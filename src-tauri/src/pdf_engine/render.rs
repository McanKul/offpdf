//! Page rendering for in-app review/preview.
//!
//! Renders individual PDF pages to small PNG thumbnails using poppler's
//! `pdftoppm` (spawned as a separate process, same model as qpdf). The PDF bytes
//! never enter the webview — only tiny rendered PNGs do, returned as base64
//! data URLs so no extra asset-protocol surface is needed.
//!
//! Thumbnails are cached on disk under the temp dir so re-scrolling is instant.
//!
//! `pdftoppm` is GPL and is invoked as a standalone subprocess (not linked),
//! the same way qpdf is. It is optional: if it is missing, the UI falls back to
//! typed page ranges.

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate, PagePick, RenderedThumb};
use crate::utils::temp;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn tool_exe() -> &'static str {
    if cfg!(windows) {
        "pdftoppm.exe"
    } else {
        "pdftoppm"
    }
}

/// Locate `pdftoppm`: bundled `binaries/` first, then common install dirs, then
/// PATH. Mirrors `qpdf::resolve_qpdf` (a Finder-launched .app has no shell PATH).
pub fn resolve_pdftoppm(app: &tauri::AppHandle) -> PathBuf {
    let exe = tool_exe();

    if let Some(found) = find_bundled_tool(app, exe) {
        return found;
    }
    #[cfg(not(windows))]
    {
        for candidate in [
            "/opt/homebrew/bin/pdftoppm",
            "/usr/local/bin/pdftoppm",
            "/opt/local/bin/pdftoppm",
            "/usr/bin/pdftoppm",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from(exe)
}

/// Locate poppler's `pdftotext` (same dirs as pdftoppm).
fn resolve_pdftotext(app: &tauri::AppHandle) -> PathBuf {
    let exe = if cfg!(windows) {
        "pdftotext.exe"
    } else {
        "pdftotext"
    };
    if let Some(found) = find_bundled_tool(app, exe) {
        return found;
    }
    #[cfg(not(windows))]
    for c in [
        "/opt/homebrew/bin/pdftotext",
        "/usr/local/bin/pdftotext",
        "/opt/local/bin/pdftotext",
        "/usr/bin/pdftotext",
    ] {
        let p = PathBuf::from(c);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(exe)
}

fn app_roots(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        roots.push(res);
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    roots
}

fn find_bundled_tool(app: &tauri::AppHandle, exe: &str) -> Option<PathBuf> {
    for root in app_roots(app) {
        for candidate in [
            root.join("binaries").join(exe),
            root.join("resources").join("binaries").join(exe),
            root.join("binaries").join("binaries").join(exe),
        ] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn poppler_data_dir_for_tool(exe: &Path) -> Option<PathBuf> {
    let bin_dir = exe.parent()?;
    let root = bin_dir.parent().unwrap_or(bin_dir);
    for candidate in [
        root.join("share").join("poppler"),
        root.join("share").join("share").join("poppler"),
        bin_dir.join("share").join("poppler"),
        bin_dir.join("..").join("share").join("poppler"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn fontconfig_dir_for_tool(exe: &Path) -> Option<PathBuf> {
    let bin_dir = exe.parent()?;
    let root = bin_dir.parent().unwrap_or(bin_dir);
    for candidate in [
        root.join("share").join("fontconfig"),
        root.join("share").join("share").join("fontconfig"),
        bin_dir.join("share").join("fontconfig"),
        bin_dir.join("..").join("share").join("fontconfig"),
    ] {
        if candidate.join("fonts.conf").exists() {
            return Some(candidate);
        }
    }
    None
}

pub fn configure_poppler_command(cmd: &mut Command, exe: &Path) {
    let Some(bin_dir) = exe.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };

    cmd.current_dir(bin_dir);

    if let Some(current_path) = std::env::var_os("PATH") {
        let mut paths = std::env::split_paths(&current_path).collect::<Vec<_>>();
        paths.insert(0, bin_dir.to_path_buf());
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    } else {
        cmd.env("PATH", bin_dir);
    }

    if let Some(data_dir) = poppler_data_dir_for_tool(exe) {
        cmd.env("POPPLER_DATADIR", data_dir);
    }
    if let Some(fontconfig_dir) = fontconfig_dir_for_tool(exe) {
        cmd.env("FONTCONFIG_PATH", &fontconfig_dir);
        cmd.env("FONTCONFIG_FILE", fontconfig_dir.join("fonts.conf"));
    }
}

/// Hard cap on text read for search, so a pathologically large (e.g. 100 GB)
/// text PDF can't blow up memory — we stream stdout and stop at this many bytes.
const MAX_TEXT_BYTES: u64 = 64 * 1024 * 1024;

/// Extract the text of each page (for in-app search). Returns one string per
/// page. Only text crosses IPC — never the PDF bytes. Output is capped at
/// `MAX_TEXT_BYTES`; beyond that the document is searchable up to the cap only.
pub fn page_texts(app: &tauri::AppHandle, input: &str) -> Result<Vec<String>, AppError> {
    use std::io::Read;
    if !Path::new(input).is_file() {
        return Err(AppError::invalid_pdf(input));
    }
    let exe = resolve_pdftotext(app);
    let mut cmd = Command::new(&exe);
    configure_poppler_command(&mut cmd, &exe);
    cmd.args(["-layout", "-enc", "UTF-8", input, "-"]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::engine_missing()
        } else {
            AppError::engine_failed(e.to_string())
        }
    })?;

    // Stream stdout but never buffer more than the cap, so a pathologically
    // large (e.g. 100 GB) text PDF can't blow up memory; then stop the process.
    let mut buf: Vec<u8> = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(MAX_TEXT_BYTES).read_to_end(&mut buf);
    }
    let _ = child.kill();
    let _ = child.wait();

    let text = String::from_utf8_lossy(&buf);
    let mut pages: Vec<String> = text.split('\u{000C}').map(|s| s.to_string()).collect();
    if pages.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
        pages.pop();
    }
    Ok(pages)
}

/// A single page extracted to its own small PDF is capped here; bigger (e.g. a
/// giant raster page) returns None so the viewer falls back to raster preview.
const MAX_PAGE_PDF_BYTES: u64 = 48 * 1024 * 1024;

/// Extract one page into a tiny standalone PDF and return it base64-encoded, for
/// true-vector rendering with pdf.js in the webview. Returns `None` if the page
/// is too large to safely load into the webview. Cached on disk per (file,page).
pub fn page_pdf_b64(
    app: &tauri::AppHandle,
    input: &str,
    page: u32,
) -> Result<Option<String>, AppError> {
    if !Path::new(input).is_file() {
        return Err(AppError::invalid_pdf(input));
    }
    let dir = temp::root(app)?.join("pagepdf").join(fnv1a_hex(input));
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::io("Could not create the page cache.", e))?;
    let out = dir.join(format!("p{page}.pdf"));
    let out_str = out.to_string_lossy().to_string();

    if !out.exists() {
        let mut cmd = Command::new(crate::pdf_engine::qpdf::resolve_qpdf(app));
        cmd.args([
            "--empty",
            "--pages",
            input,
            &page.to_string(),
            "--",
            &out_str,
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let status = cmd.status().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::engine_missing()
            } else {
                AppError::engine_failed(e.to_string())
            }
        })?;
        let ok = status.success() || status.code() == Some(3);
        if !ok || !out.exists() {
            return Ok(None);
        }
    }

    let meta = std::fs::metadata(&out).map_err(|e| AppError::io("Could not read the page.", e))?;
    if meta.len() > MAX_PAGE_PDF_BYTES {
        return Ok(None);
    }
    let bytes = std::fs::read(&out).map_err(|e| AppError::io("Could not read the page.", e))?;
    Ok(Some(base64(&bytes)))
}

/// Render one page to a PNG file (returns its path). Used by the compare tool.
fn render_to_png(
    app: &tauri::AppHandle,
    input: &str,
    page: u32,
    size: u32,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, AppError> {
    let exe = resolve_pdftoppm(app);
    let prefix = dir.join(name);
    let mut cmd = Command::new(&exe);
    configure_poppler_command(&mut cmd, &exe);
    cmd.args([
        "-png",
        "-scale-to",
        &size.to_string(),
        "-f",
        &page.to_string(),
        "-l",
        &page.to_string(),
        "-singlefile",
        input,
        &prefix.to_string_lossy(),
    ]);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let status = cmd.status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::engine_missing()
        } else {
            AppError::engine_failed(e.to_string())
        }
    })?;
    let png = prefix.with_extension("png");
    if !status.success() && !png.exists() {
        return Err(AppError::engine_failed("Could not render the page."));
    }
    Ok(png)
}

/// Visually compare page A and page B: render both at the same size, then build
/// an overlay where differing pixels are painted red over a faded version of B.
pub fn diff_pages(
    app: &tauri::AppHandle,
    a_path: &str,
    a_page: u32,
    b_path: &str,
    b_page: u32,
    size: u32,
) -> Result<crate::models::DiffResult, AppError> {
    let size = size.clamp(200, 4000);
    let dir = temp::root(app)?
        .join("cmp")
        .join(fnv1a_hex(&format!("{a_path}|{b_path}")));
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;

    let pa = render_to_png(app, a_path, a_page, size, &dir, &format!("a{a_page}"))?;
    let pb = render_to_png(app, b_path, b_page, size, &dir, &format!("b{b_page}"))?;

    let ia = image::open(&pa)
        .map_err(|e| AppError::engine_failed(format!("read A: {e}")))?
        .to_rgba8();
    let ib = image::open(&pb)
        .map_err(|e| AppError::engine_failed(format!("read B: {e}")))?
        .to_rgba8();
    let (wa, ha) = ia.dimensions();
    let (wb, hb) = ib.dimensions();
    let w = wa.max(wb);
    let h = ha.max(hb);

    let mut out = image::RgbaImage::new(w, h);
    let mut changed: u64 = 0;
    const THRESH: i32 = 60;
    for y in 0..h {
        for x in 0..w {
            let ap = if x < wa && y < ha {
                ia.get_pixel(x, y).0
            } else {
                [255, 255, 255, 255]
            };
            let bp = if x < wb && y < hb {
                ib.get_pixel(x, y).0
            } else {
                [255, 255, 255, 255]
            };
            let d = (ap[0] as i32 - bp[0] as i32).abs()
                + (ap[1] as i32 - bp[1] as i32).abs()
                + (ap[2] as i32 - bp[2] as i32).abs();
            if d > THRESH {
                changed += 1;
                out.put_pixel(x, y, image::Rgba([220, 30, 30, 255]));
            } else {
                // light grayscale of B so real differences stand out
                let luma = (bp[0] as u16 * 3 + bp[1] as u16 * 6 + bp[2] as u16) / 10;
                let fv = ((luma + 255 * 2) / 3) as u8;
                out.put_pixel(x, y, image::Rgba([fv, fv, fv, 255]));
            }
        }
    }
    let changed_percent = changed as f32 / (w as f32 * h as f32) * 100.0;

    let mut buf: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(out)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| AppError::engine_failed(format!("encode diff: {e}")))?;

    Ok(crate::models::DiffResult {
        data_url: format!("data:image/png;base64,{}", base64(&buf)),
        changed_percent,
    })
}

/// Whether a page renderer is available (controls the "Pick visually" UI).
pub fn available(app: &tauri::AppHandle) -> bool {
    let exe = resolve_pdftoppm(app);
    let mut cmd = Command::new(&exe);
    configure_poppler_command(&mut cmd, &exe);
    cmd.arg("-v").stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    // Any successful spawn (even a non-zero exit from `-v`) means it's present.
    !matches!(cmd.status(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// Stable per-document cache folder: `<temp>/preview/<hash-of-path>`.
fn cache_dir(app: &tauri::AppHandle, input: &str) -> Result<PathBuf, AppError> {
    let dir = temp::root(app)?.join("preview").join(fnv1a_hex(input));
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::io("Could not create the preview cache directory.", e))?;
    Ok(dir)
}

/// Render one page to a PNG (cached) and return it as a base64 data URL.
fn render_one(
    app: &tauri::AppHandle,
    input: &str,
    page: u32,
    size: u32,
    dir: &Path,
) -> Result<String, AppError> {
    let png = dir.join(format!("p{page}_s{size}.png"));

    if !png.exists() {
        // pdftoppm writes "<prefix>.png" when -singlefile is set.
        let prefix = dir.join(format!("p{page}_s{size}"));
        let exe = resolve_pdftoppm(app);
        let mut cmd = Command::new(&exe);
        configure_poppler_command(&mut cmd, &exe);
        cmd.args([
            "-png",
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-singlefile",
            "-scale-to",
            &size.to_string(),
            input,
            &prefix.to_string_lossy(),
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::new(
                    "RENDERER_MISSING",
                    "Preview engine not found",
                    "pdftoppm (poppler) could not be located.",
                )
                .with_suggestion("Install poppler to enable visual page preview.")
            } else {
                AppError::engine_failed(e.to_string())
            }
        })?;

        if !out.status.success() && out.status.code() != Some(0) && !png.exists() {
            return Err(AppError::engine_failed(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
    }

    let bytes = std::fs::read(&png)
        .map_err(|e| AppError::io("Could not read the rendered preview image.", e))?;
    Ok(format!("data:image/png;base64,{}", base64(&bytes)))
}

/// Render thumbnails for the given pages. `size` is the longest-side pixel size.
pub fn render_thumbnails(
    app: &tauri::AppHandle,
    input: &str,
    pages: &[u32],
    size: u32,
) -> Result<Vec<RenderedThumb>, AppError> {
    if !Path::new(input).is_file() {
        return Err(AppError::invalid_pdf(input));
    }
    let dir = cache_dir(app, input)?;
    // Up to 6000 px on the long side so the viewer can re-render crisply as the
    // user zooms in (raster, but at the resolution actually being displayed).
    let size = size.clamp(64, 6000);

    let mut out = Vec::with_capacity(pages.len());
    for &page in pages {
        let data_url = render_one(app, input, page, size, &dir)?;
        out.push(RenderedThumb { page, data_url });
    }
    Ok(out)
}

/// Export each page of the combined document to an image file in `out_dir`.
/// `format` is "png" or "jpg". Returns the produced file paths.
pub fn to_images(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    picks: &[PagePick],
    out_dir: &str,
    format: &str,
    dpi: u32,
) -> Result<Vec<String>, AppError> {
    if picks.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    std::fs::create_dir_all(out_dir).map_err(|_| AppError::output_not_writable(out_dir))?;

    let is_jpg = format.eq_ignore_ascii_case("jpg") || format.eq_ignore_ascii_case("jpeg");
    let ext = if is_jpg { "jpg" } else { "png" };
    let dpi = dpi.clamp(36, 600);
    let exe = resolve_pdftoppm(app);
    let n = picks.len();
    let mut outputs: Vec<String> = Vec::with_capacity(n);

    for (i, pk) in picks.iter().enumerate() {
        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }
        let _ = app.emit(
            "job:update",
            JobUpdate::new(
                job_id,
                "running",
                &format!("Exporting page {} of {n}", i + 1),
            )
            .percent(i as f32 / n as f32 * 100.0),
        );

        let prefix = Path::new(out_dir).join(format!("page-{:03}", i + 1));
        let prefix_str = prefix.to_string_lossy().to_string();

        let mut cmd = Command::new(&exe);
        configure_poppler_command(&mut cmd, &exe);
        if is_jpg {
            cmd.args(["-jpeg", "-jpegopt", "quality=92"]);
        } else {
            cmd.arg("-png");
        }
        cmd.args([
            "-r",
            &dpi.to_string(),
            "-f",
            &pk.page.to_string(),
            "-l",
            &pk.page.to_string(),
            "-singlefile",
            &pk.path,
            &prefix_str,
        ]);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);

        let (_st, stderr) = crate::utils::process::run_tracked(handle, cmd)?;

        let outfile = prefix.with_extension(ext);
        if !outfile.exists() {
            return Err(AppError::engine_failed(stderr.trim().to_string()));
        }
        outputs.push(outfile.to_string_lossy().to_string());
    }
    Ok(outputs)
}

// ---- tiny dependency-free helpers ----------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        s.push(B64[((n >> 18) & 63) as usize] as char);
        s.push(B64[((n >> 12) & 63) as usize] as char);
        s.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}

fn fnv1a_hex(s: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
