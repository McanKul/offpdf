//! Size-reducing compression for image-heavy / large-format PDFs (e.g. scanned
//! architectural plans where each page is one big image).
//!
//! Approach (matches the proven playground script, but pure Rust + CLI):
//!   1. Render each page to a JPEG at a chosen DPI/quality via poppler
//!      `pdftoppm -jpeg` — this flattens all visible content into one clean
//!      image and drops hidden/residual layers.
//!   2. Wrap the JPEGs into a single PDF, one image per page, embedding each as a
//!      /DCTDecode image XObject (no re-encoding, so no extra quality loss).
//!
//! This is intentionally LOSSY/rasterizing: selectable text becomes part of the
//! image. It is exposed as a separate "Compress" tool, distinct from the
//! non-destructive "Optimize". Bytes never enter the webview.

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate};
use crate::pdf_engine::render;
use crate::utils::temp;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Compress `input` into `output` by rasterizing each page to JPEG at `dpi`
/// and wrapping the JPEGs into one PDF.
///
/// - `target_bytes = None`: use a fixed JPEG `quality` (1–100) for every page.
/// - `target_bytes = Some(t)`: aim for a total output near `t` by giving each
///   page a byte budget (`t / pages`) and binary-searching the highest JPEG
///   quality (≤ `quality`) that fits it — the playground `compress_pdf.py`
///   strategy. `quality` acts as the upper quality cap in this mode.
pub fn compress(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    picks: &[crate::models::PagePick],
    output: &str,
    dpi: u32,
    quality: u32,
    target_bytes: Option<u64>,
) -> Result<Vec<String>, AppError> {
    if picks.is_empty() {
        return Err(AppError::new(
            "NO_PAGES",
            "No pages to compress",
            "Add a PDF or image first.",
        ));
    }
    super::ensure_output_dir(output)?;

    let dpi = dpi.clamp(36, 600);
    let quality = quality.clamp(1, 100);
    let n = picks.len();

    // Validate each distinct source file exists.
    let mut seen = std::collections::HashSet::new();
    for pk in picks {
        if seen.insert(pk.path.as_str()) && !Path::new(&pk.path).is_file() {
            return Err(AppError::invalid_pdf(&pk.path));
        }
    }

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp work directory.", e))?;

    let result = (|| -> Result<Vec<String>, AppError> {
        let mut images: Vec<PageImage> = Vec::with_capacity(n);

        for (i, pk) in picks.iter().enumerate() {
            if handle.is_cancelled() {
                return Err(AppError::cancelled());
            }
            let _ = app.emit(
                "job:update",
                JobUpdate::new(
                    job_id,
                    "running",
                    &format!("Compressing page {} of {n}", i + 1),
                )
                .percent(i as f32 / n as f32 * 100.0),
            );

            // Track the DPI each page was ACTUALLY rendered at — in target mode
            // render_auto picks its own per-page DPI, and the page's physical
            // size in the output must be computed from that value.
            let (jpg, page_dpi) = match target_bytes {
                Some(total) => {
                    let budget = (total / n as u64).max(15_000);
                    // Auto: estimate a DPI for the budget (cap at `dpi`), then tune
                    // quality (≤ `quality`). Resolution drops before quality is crushed.
                    // qmin 40: below ~q40 JPEG ringing eats hairlines even at
                    // adequate DPI. Combined with the 150-dpi floor this keeps
                    // technical-drawing lines visible; tight targets may
                    // overshoot slightly instead of destroying detail.
                    render_auto(
                        app, handle, &pk.path, pk.page, budget, dpi, 40, quality, &work, i,
                    )?
                }
                None => (
                    render_jpeg(app, handle, &pk.path, pk.page, dpi, quality, &work, i)?,
                    dpi,
                ),
            };
            let bytes = std::fs::read(&jpg)
                .map_err(|e| AppError::io("Could not read a rendered page.", e))?;
            let (w, h, comps) = jpeg_info(&bytes)
                .ok_or_else(|| AppError::engine_failed("Rendered page was not a valid JPEG."))?;
            images.push(PageImage {
                path: jpg,
                w,
                h,
                comps,
                dpi: page_dpi,
            });
        }

        let _ = app.emit(
            "job:update",
            JobUpdate::new(job_id, "running", "Assembling PDF").percent(99.0),
        );
        write_image_pdf(output, &images)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

pub(crate) struct PageImage {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub comps: u8,
    /// DPI this page was rendered at — physical page size derives from it.
    pub dpi: u32,
}

/// Wrap a single uploaded image into a one-page PDF (so it can be previewed,
/// merged and edited exactly like any PDF). JPEGs are embedded losslessly; other
/// formats are decoded and re-encoded to JPEG. Returns the output PDF path.
pub fn image_to_pdf(app: &tauri::AppHandle, input: &str) -> Result<String, AppError> {
    use std::io::BufWriter;

    let in_path = Path::new(input);
    if !in_path.is_file() {
        return Err(AppError::new(
            "INVALID_IMAGE",
            "Image not found",
            format!("\"{input}\" could not be read."),
        ));
    }

    let dir = temp::root(app)?.join("images").join(hash_hex(input));
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;

    let stem = in_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());

    let ext = in_path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let jpg_path = dir.join("page.jpg");

    if ext == "jpg" || ext == "jpeg" {
        // Already JPEG — embed as-is (lossless).
        std::fs::copy(in_path, &jpg_path)
            .map_err(|e| AppError::io("Could not read the image.", e))?;
    } else {
        // Decode any supported format and re-encode to JPEG.
        let img = image::open(in_path).map_err(|e| {
            AppError::new("INVALID_IMAGE", "Could not read this image", e.to_string())
                .with_suggestion("Supported: PNG, JPEG, GIF, BMP, WebP, TIFF.")
        })?;
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width(), rgb.height());
        let file = std::fs::File::create(&jpg_path)
            .map_err(|e| AppError::io("Could not write the converted image.", e))?;
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(BufWriter::new(file), 90);
        image::ImageEncoder::write_image(
            encoder,
            rgb.as_raw(),
            w,
            h,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| AppError::engine_failed(e.to_string()))?;
    }

    let bytes = std::fs::read(&jpg_path)
        .map_err(|e| AppError::io("Could not read the converted image.", e))?;
    let (w, h, comps) = jpeg_info(&bytes)
        .ok_or_else(|| AppError::engine_failed("Converted image was not valid JPEG."))?;

    let out_pdf = dir.join(format!("{stem}.pdf"));
    let out_pdf_str = out_pdf.to_string_lossy().to_string();
    let pages = [PageImage {
        path: jpg_path,
        w,
        h,
        comps,
        dpi: 96,
    }];
    write_image_pdf(&out_pdf_str, &pages)?;
    Ok(out_pdf_str)
}

/// Render one page to a JPEG at the given DPI/quality. `idx` keeps file names
/// unique across source files. Returns the file path.
fn render_jpeg(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    input: &str,
    page: u32,
    dpi: u32,
    quality: u32,
    dir: &Path,
    idx: usize,
) -> Result<PathBuf, AppError> {
    let prefix = dir.join(format!("c{idx}"));
    let exe = render::resolve_pdftoppm(app);
    let mut cmd = Command::new(&exe);
    render::configure_poppler_command(&mut cmd, &exe);
    cmd.args([
        "-jpeg",
        "-jpegopt",
        &format!("quality={quality}"),
        "-r",
        &dpi.to_string(),
        "-f",
        &page.to_string(),
        "-l",
        &page.to_string(),
        "-singlefile",
        input,
        &prefix.to_string_lossy(),
    ]);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    // Cancellable: the child is registered so cancel_job can kill it mid-render.
    let (_status, stderr) = crate::utils::process::run_tracked(handle, cmd)?;

    let jpg = prefix.with_extension("jpg");
    if !jpg.exists() {
        return Err(AppError::engine_failed(stderr.trim().to_string()));
    }
    Ok(jpg)
}

/// Auto-fit a page to `budget`: probe once to estimate a DPI for the budget
/// (capped at `max_dpi`), then binary-search JPEG quality at that DPI. This
/// reduces resolution to roughly hit the size, then trims quality only as
/// needed — keeping line/text legibility better than crushing quality alone.
#[allow(clippy::too_many_arguments)]
fn render_auto(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    input: &str,
    page: u32,
    budget: u64,
    max_dpi: u32,
    qmin: u32,
    qmax: u32,
    dir: &Path,
    idx: usize,
) -> Result<(PathBuf, u32), AppError> {
    if handle.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let probe = max_dpi.min(150).max(50);
    let p = render_jpeg(app, handle, input, page, probe, 75, dir, idx)?;
    let s = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    if s == 0 {
        return Ok((p, probe));
    }
    // JPEG size scales ~ with pixel count (∝ dpi²); leave 10% headroom.
    let scale = ((budget as f64 * 0.9) / s as f64).sqrt();
    let est = ((probe as f64) * scale).round() as i64;
    // Floor at 150 dpi: below that, hairlines (0.3-0.5pt in technical
    // drawings) fall under one pixel and vanish entirely. Preferring a
    // slightly-over-target file over an unreadable plan.
    let floor = 150.min(max_dpi) as i64;
    let dpi = est.clamp(floor, max_dpi as i64) as u32;
    let jpg = render_to_budget(app, handle, input, page, dpi, budget, qmin, qmax, dir, idx)?;
    Ok((jpg, dpi))
}

/// Render a page choosing the highest JPEG quality (≤ qmax) whose file fits
/// `budget` bytes, via binary search. Falls back to qmin if nothing fits.
#[allow(clippy::too_many_arguments)]
fn render_to_budget(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    input: &str,
    page: u32,
    dpi: u32,
    budget: u64,
    qmin: u32,
    qmax: u32,
    dir: &Path,
    idx: usize,
) -> Result<PathBuf, AppError> {
    let size_of = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX);

    // If the highest quality already fits, use it.
    let jpg = render_jpeg(app, handle, input, page, dpi, qmax, dir, idx)?;
    if size_of(&jpg) <= budget {
        return Ok(jpg);
    }

    let (mut lo, mut hi) = (qmin, qmax.saturating_sub(1).max(qmin));
    let mut best: Option<u32> = None;
    while lo <= hi {
        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }
        let mid = (lo + hi) / 2;
        let p = render_jpeg(app, handle, input, page, dpi, mid, dir, idx)?;
        if size_of(&p) <= budget {
            best = Some(mid);
            lo = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }

    // Re-render at the chosen quality so the file on disk matches it.
    let q = best.unwrap_or(qmin);
    render_jpeg(app, handle, input, page, dpi, q, dir, idx)
}

/// Stable short hash for naming a per-image temp folder.
fn hash_hex(s: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Parse width, height and component count from a JPEG's SOF marker.
fn jpeg_info(data: &[u8]) -> Option<(u32, u32, u8)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // Standalone markers (no length).
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = ((data[i + 2] as usize) << 8) | (data[i + 3] as usize);
        // SOF0..SOF15 carry the frame size; exclude DHT(C4), DAC(CC), DNL(C8).
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let h = ((data[i + 5] as u32) << 8) | (data[i + 6] as u32);
            let w = ((data[i + 7] as u32) << 8) | (data[i + 8] as u32);
            let comps = data[i + 9];
            return Some((w, h, comps));
        }
        i += 2 + len;
    }
    None
}

/// Build a minimal PDF that places one JPEG per page (DCTDecode XObjects).
/// Each page's physical size derives from its own render DPI.
fn write_image_pdf(output: &str, pages: &[PageImage]) -> Result<(), AppError> {
    let n = pages.len();
    let total_objs = 2 + 3 * n; // catalog + pages + (image, content, page) per page
    let mut buf: Vec<u8> = Vec::new();
    let mut offsets = vec![0usize; total_objs + 1];

    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");

    offsets[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let kids: String = (0..n)
        .map(|i| format!("{} 0 R", 5 + i * 3))
        .collect::<Vec<_>>()
        .join(" ");
    offsets[2] = buf.len();
    buf.extend_from_slice(
        format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {n} >>\nendobj\n").as_bytes(),
    );

    for (i, pg) in pages.iter().enumerate() {
        let img_obj = 3 + i * 3;
        let content_obj = 4 + i * 3;
        let page_obj = 5 + i * 3;

        let wpt = pg.w as f64 * 72.0 / pg.dpi as f64;
        let hpt = pg.h as f64 * 72.0 / pg.dpi as f64;
        let cs = if pg.comps == 1 {
            "/DeviceGray"
        } else {
            "/DeviceRGB"
        };

        let jpeg = std::fs::read(&pg.path)
            .map_err(|e| AppError::io("Could not read a rendered page.", e))?;

        // Image XObject (raw JPEG via DCTDecode).
        offsets[img_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{img_obj} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} \
                 /ColorSpace {cs} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                pg.w,
                pg.h,
                jpeg.len()
            )
            .as_bytes(),
        );
        buf.extend_from_slice(&jpeg);
        buf.extend_from_slice(b"\nendstream\nendobj\n");

        // Content stream: scale the unit image to the full page box.
        let content = format!("q\n{wpt:.2} 0 0 {hpt:.2} 0 0 cm\n/Im0 Do\nQ\n");
        offsets[content_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{content_obj} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );

        // Page.
        offsets[page_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {wpt:.2} {hpt:.2}] \
                 /Resources << /XObject << /Im0 {img_obj} 0 R >> >> /Contents {content_obj} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );
    }

    // Cross-reference table.
    let xref_pos = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for num in 1..=total_objs {
        buf.extend_from_slice(format!("{:010} 00000 n \n", offsets[num]).as_bytes());
    }
    buf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            total_objs + 1
        )
        .as_bytes(),
    );

    std::fs::write(output, &buf)
        .map_err(|e| AppError::output_not_writable(&format!("{output} ({e})")))?;
    Ok(())
}
