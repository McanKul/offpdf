//! OCR: make a scanned PDF searchable. Each page is rendered to an image
//! (poppler), Tesseract produces a one-page searchable PDF (image + invisible
//! text layer), and the pages are merged back with qpdf. All local, no network.

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate, PagePick};
use crate::pdf_engine::render;
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn tesseract_exe() -> &'static str {
    if cfg!(windows) {
        "tesseract.exe"
    } else {
        "tesseract"
    }
}

/// Locate the Tesseract binary.
pub fn resolve_tesseract(app: &tauri::AppHandle) -> PathBuf {
    let exe = tesseract_exe();

    if let Some(found) = find_bundled_tesseract(app, exe) {
        return found;
    }

    #[cfg(not(windows))]
    for c in [
        "/opt/homebrew/bin/tesseract",
        "/usr/local/bin/tesseract",
        "/usr/bin/tesseract",
    ] {
        let p = PathBuf::from(c);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(exe)
}

/// Whether Tesseract is available.
pub fn available(app: &tauri::AppHandle) -> bool {
    let exe = resolve_tesseract(app);
    let mut cmd = Command::new(&exe);
    configure_tesseract_command(&mut cmd, &exe);
    cmd.arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    matches!(cmd.status(), Ok(status) if status.success())
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

fn find_bundled_tesseract(app: &tauri::AppHandle, exe: &str) -> Option<PathBuf> {
    for root in app_roots(app) {
        for candidate in [
            root.join("tesseract").join(exe),
            root.join("resources").join("tesseract").join(exe),
            root.join("binaries").join(exe),
            root.join("resources").join("binaries").join(exe),
        ] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn tessdata_dir_for_tool(exe: &Path) -> Option<PathBuf> {
    let bin_dir = exe.parent()?;
    for candidate in [
        bin_dir.join("tessdata"),
        bin_dir.join("..").join("tessdata"),
    ] {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn configure_tesseract_command(cmd: &mut Command, exe: &Path) {
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

    if let Some(tessdata) = tessdata_dir_for_tool(exe) {
        cmd.env("TESSDATA_PREFIX", &tessdata);
    }
}

fn missing() -> AppError {
    AppError::new(
        "OCR_MISSING",
        "Tesseract not found",
        "OCR needs Tesseract, which couldn't be located.",
    )
    .with_suggestion("Install it — on macOS: brew install tesseract tesseract-lang.")
}

/// OCR every page of the combined document into one searchable PDF. `lang` is a
/// Tesseract language code (e.g. "eng", "tur", or "eng+tur").
pub fn ocr(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    picks: &[PagePick],
    output: &str,
    lang: &str,
) -> Result<Vec<String>, AppError> {
    if picks.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    super::ensure_output_dir(output)?;

    let pdftoppm = render::resolve_pdftoppm(app);
    let tess = resolve_tesseract(app);
    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;

    let result = (|| -> Result<Vec<String>, AppError> {
        let n = picks.len();
        let mut page_pdfs: Vec<String> = Vec::with_capacity(n);

        for (i, pk) in picks.iter().enumerate() {
            if handle.is_cancelled() {
                return Err(AppError::cancelled());
            }
            let _ = app.emit(
                "job:update",
                JobUpdate::new(
                    job_id,
                    "running",
                    &format!("Reading text — page {} of {n}", i + 1),
                )
                .percent(i as f32 / n as f32 * 100.0),
            );

            // 1. render page → PNG at 300 DPI (good OCR accuracy)
            let img_prefix = work.join(format!("p{i}"));
            let mut r = Command::new(&pdftoppm);
            render::configure_poppler_command(&mut r, &pdftoppm);
            r.args([
                "-png",
                "-r",
                "300",
                "-f",
                &pk.page.to_string(),
                "-l",
                &pk.page.to_string(),
                "-singlefile",
                &pk.path,
                &img_prefix.to_string_lossy(),
            ]);
            r.stdout(Stdio::null()).stderr(Stdio::piped());
            #[cfg(windows)]
            r.creation_flags(0x08000000);
            let (_rs, rerr) = crate::utils::process::run_tracked(handle, r)?;
            let png = img_prefix.with_extension("png");
            if !png.exists() {
                return Err(AppError::engine_failed(rerr.trim().to_string()));
            }

            // 2. tesseract PNG → searchable one-page PDF
            let out_base = work.join(format!("o{i}"));
            let mut t = Command::new(&tess);
            configure_tesseract_command(&mut t, &tess);
            t.arg(png.to_string_lossy().to_string())
                .arg(out_base.to_string_lossy().to_string());
            if let Some(tessdata) = tessdata_dir_for_tool(&tess) {
                t.arg("--tessdata-dir").arg(tessdata);
            }
            t.arg("-l").arg(lang).arg("pdf");
            t.stdout(Stdio::null()).stderr(Stdio::piped());
            #[cfg(windows)]
            t.creation_flags(0x08000000);
            let (_ts, terr) = match crate::utils::process::run_tracked(handle, t) {
                Ok(v) => v,
                Err(e) if e.code == "ENGINE_MISSING" => return Err(missing()),
                Err(e) => return Err(e),
            };
            let page_pdf = out_base.with_extension("pdf");
            if !page_pdf.exists() {
                return Err(AppError::engine_failed(terr.trim().to_string()));
            }
            page_pdfs.push(page_pdf.to_string_lossy().to_string());
            let _ = std::fs::remove_file(&png);
        }

        // 3. merge the per-page searchable PDFs
        let _ = app.emit(
            "job:update",
            JobUpdate::new(job_id, "running", "Merging pages").percent(99.0),
        );
        let mut args: Vec<String> = vec!["--empty".into(), "--pages".into()];
        for p in &page_pdfs {
            args.push(p.clone());
            args.push("1".into());
        }
        args.push("--".into());
        args.push(output.to_string());
        run_qpdf(app, handle, job_id, &args, "Merging pages", None)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}
