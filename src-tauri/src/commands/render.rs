//! Page-preview commands. Render PDF pages to small PNG thumbnails locally so
//! users can review and pick pages visually. Source PDF bytes never cross IPC —
//! only tiny rendered PNGs (as base64 data URLs) do.

use crate::error::AppError;
use crate::models::{JobRegistry, JobResult, JobUpdate, PageGroup, PagePick, RenderedThumb};
use crate::pdf_engine::{ocr, office, render};
use crate::utils::temp;
use tauri::Emitter;

fn hashed(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Whether a local page renderer (poppler `pdftoppm`) is available.
#[tauri::command]
pub async fn renderer_available(app: tauri::AppHandle) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || render::available(&app))
        .await
        .map_err(|e| AppError::io("Could not probe the preview engine.", e))
}

/// Render thumbnails for the given 1-based pages of `input_path`.
/// `size` is the longest-side pixel size (default 240).
#[tauri::command]
pub async fn render_thumbnails(
    app: tauri::AppHandle,
    input_path: String,
    pages: Vec<u32>,
    size: Option<u32>,
) -> Result<Vec<RenderedThumb>, AppError> {
    let size = size.unwrap_or(240);
    tauri::async_runtime::spawn_blocking(move || {
        render::render_thumbnails(&app, &input_path, &pages, size)
    })
    .await
    .map_err(|e| AppError::io("Could not render the page preview.", e))?
}

/// Wrap an uploaded image into a one-page PDF and return its path. The frontend
/// then treats it like any PDF (preview/merge/edit), so images show up as PDFs.
#[tauri::command]
pub async fn image_to_pdf(app: tauri::AppHandle, image_path: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::compress::image_to_pdf(&app, &image_path)
    })
    .await
    .map_err(|e| AppError::io("Could not convert the image.", e))?
}

/// Export each page of the combined document to an image file in `output_dir`.
#[tauri::command]
pub async fn pdf_to_images(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_dir: String,
    picks: Vec<PagePick>,
    format: String,
    dpi: u32,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        render::to_images(&app2, &handle, &jid, &picks, &output_dir, &format, dpi)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}

/// Per-page text of a PDF, for in-app search. Only text crosses IPC.
#[tauri::command]
pub async fn pdf_text(app: tauri::AppHandle, input_path: String) -> Result<Vec<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || render::page_texts(&app, &input_path))
        .await
        .map_err(|e| AppError::io("Could not read the document text.", e))?
}

/// One page as a small standalone PDF (base64), for true-vector zoom with pdf.js.
/// Returns null when the page is too large to load into the webview.
#[tauri::command]
pub async fn page_pdf(app: tauri::AppHandle, input_path: String, page: u32) -> Result<Option<String>, AppError> {
    tauri::async_runtime::spawn_blocking(move || render::page_pdf_b64(&app, &input_path, page))
        .await
        .map_err(|e| AppError::io("Could not read the page.", e))?
}

/// A PDF's bookmarks/outline (flattened). Empty for files with no outline or
/// files too large to parse safely.
#[tauri::command]
pub async fn pdf_outline(input_path: String) -> Result<Vec<crate::models::OutlineItem>, AppError> {
    tauri::async_runtime::spawn_blocking(move || crate::pdf_engine::outline::extract(&input_path))
        .await
        .map_err(|e| AppError::io("Could not read the outline.", e))?
}

/// Visually compare two pages; returns a diff-overlay image + changed percent.
#[tauri::command]
pub async fn diff_pages(
    app: tauri::AppHandle,
    a_path: String,
    a_page: u32,
    b_path: String,
    b_page: u32,
    size: u32,
) -> Result<crate::models::DiffResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || render::diff_pages(&app, &a_path, a_page, &b_path, b_page, size))
        .await
        .map_err(|e| AppError::io("Could not compare the pages.", e))?
}

/// Whether Tesseract (OCR) is available.
#[tauri::command]
pub async fn ocr_available(app: tauri::AppHandle) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || ocr::available(&app))
        .await
        .map_err(|e| AppError::io("Could not probe Tesseract.", e))
}

/// OCR the combined document into one searchable PDF.
#[tauri::command]
pub async fn ocr_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    picks: Vec<PagePick>,
    lang: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        ocr::ocr(&app2, &handle, &jid, &picks, &output_path, &lang)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}

/// Whether LibreOffice is available (controls Office conversion features).
#[tauri::command]
pub async fn office_available(app: tauri::AppHandle) -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(move || office::available(&app))
        .await
        .map_err(|e| AppError::io("Could not probe LibreOffice.", e))
}

/// Convert an Office document to a one-page-or-more PDF in temp; returns its
/// path. Used on import so Office files behave like PDFs everywhere.
#[tauri::command]
pub async fn office_to_pdf(app: tauri::AppHandle, input_path: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = temp::root(&app)?.join("office").join(hashed(&input_path));
        office::to_pdf(&app, None, &input_path, &dir.to_string_lossy())
    })
    .await
    .map_err(|e| AppError::io("Could not convert the document.", e))?
}

/// Explicit "Office → PDF": convert each input file to a PDF in `output_dir`.
#[tauri::command]
pub async fn office_to_pdf_batch(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_dir: String,
    input_paths: Vec<String>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, AppError> {
        let n = input_paths.len();
        let mut outputs = Vec::with_capacity(n);
        for (i, inp) in input_paths.iter().enumerate() {
            if handle.is_cancelled() {
                return Err(AppError::cancelled());
            }
            let _ = app2.emit(
                "job:update",
                JobUpdate::new(&jid, "running", &format!("Converting {} of {n}", i + 1))
                    .percent(i as f32 / n as f32 * 100.0),
            );
            outputs.push(office::to_pdf(&app2, Some(&handle), inp, &output_dir)?);
        }
        Ok(outputs)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}

/// Convert the combined document to PDF/A-2b (via LibreOffice).
#[tauri::command]
pub async fn pdfa_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, AppError> {
        let out = std::path::Path::new(&output_path);
        let out_dir = out
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".into());
        let stem = out
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "output".into());
        let work = temp::root(&app2)?.join("work").join(&jid);
        std::fs::create_dir_all(&work).map_err(|e| AppError::io("Could not create a temp directory.", e))?;
        let merged = work.join(format!("{stem}.pdf")).to_string_lossy().to_string();
        let result = (|| -> Result<Vec<String>, AppError> {
            crate::pdf_engine::assemble(&app2, &handle, &jid, &groups, &merged)?;
            let _ = app2.emit("job:update", JobUpdate::new(&jid, "running", "Converting to PDF/A"));
            let produced = office::to_pdfa(&app2, Some(&handle), &merged, &out_dir)?;
            Ok(vec![produced])
        })();
        let _ = std::fs::remove_dir_all(&work);
        result
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}

/// "PDF → Office": assemble the combined document, then convert to docx/pptx/xlsx.
#[tauri::command]
pub async fn pdf_to_office(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_dir: String,
    groups: Vec<PageGroup>,
    format: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, AppError> {
        let work = temp::root(&app2)?.join("work").join(&jid);
        std::fs::create_dir_all(&work)
            .map_err(|e| AppError::io("Could not create a temp directory.", e))?;
        let merged = work.join("merged.pdf");
        let merged_str = merged.to_string_lossy().to_string();
        let result = (|| -> Result<Vec<String>, AppError> {
            crate::pdf_engine::assemble(&app2, &handle, &jid, &groups, &merged_str)?;
            let _ = app2.emit("job:update", JobUpdate::new(&jid, "running", "Converting"));
            let out = office::from_pdf(&app2, Some(&handle), &merged_str, &output_dir, &format)?;
            Ok(vec![out])
        })();
        let _ = std::fs::remove_dir_all(&work);
        result
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}
