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

/// File stem of `path` ("report" for "/x/report.docx"), or "document".
fn stem_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string())
}

/// Move a file, falling back to copy + delete when `rename` crosses volumes.
fn move_file(src: &str, dst: &str) -> Result<(), AppError> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).map_err(|e| AppError::io("Could not save the converted file.", e))?;
    let _ = std::fs::remove_file(src);
    Ok(())
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

/// Supported URI / in-document GoTo `/Link` annots. Paths in, JSON out; never
/// fetches a URI. Files over 400 MiB or with more than 5,000 supported links
/// return an actionable error (not a silent empty or truncated list).
#[tauri::command]
pub async fn list_pdf_links(
    input_path: String,
) -> Result<Vec<crate::pdf_engine::edit_links::PdfLinkDto>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::edit_links::list_pdf_links_cmd(&input_path)
    })
    .await
    .map_err(|e| AppError::io("Could not read the links.", e))?
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
        let work = temp::root(&app2)?.join("work").join(&jid);
        let result = (|| -> Result<Vec<String>, AppError> {
            let mut outputs = Vec::with_capacity(n);
            // Output paths produced by THIS batch — used to detect inputs that
            // share a stem (report.docx + report.xlsx) so the second one gets a
            // "report (2).pdf" name instead of silently overwriting the first.
            let mut taken = std::collections::HashSet::<String>::new();
            for (i, inp) in input_paths.iter().enumerate() {
                if handle.is_cancelled() {
                    return Err(AppError::cancelled());
                }
                let _ = app2.emit(
                    "job:update",
                    JobUpdate::new(&jid, "running", &format!("Converting {} of {n}", i + 1))
                        .percent(i as f32 / n as f32 * 100.0),
                );

                let stem = stem_of(inp);
                let expected = std::path::Path::new(&output_dir).join(format!("{stem}.pdf"));
                if !taken.contains(&expected.to_string_lossy().to_string()) {
                    let out = office::to_pdf(&app2, Some(&handle), inp, &output_dir)?;
                    taken.insert(out.clone());
                    outputs.push(out);
                    continue;
                }

                // Stem collision: convert into a scratch dir, then move the
                // result to a unique "<stem> (N).pdf" in the output folder.
                let scratch = work.join(i.to_string());
                std::fs::create_dir_all(&scratch)
                    .map_err(|e| AppError::io("Could not create a temp directory.", e))?;
                let produced =
                    office::to_pdf(&app2, Some(&handle), inp, &scratch.to_string_lossy())?;
                let mut k = 2;
                let target = loop {
                    let cand =
                        std::path::Path::new(&output_dir).join(format!("{stem} ({k}).pdf"));
                    let cand_str = cand.to_string_lossy().to_string();
                    if !taken.contains(&cand_str) && !cand.exists() {
                        break cand_str;
                    }
                    k += 1;
                };
                move_file(&produced, &target)?;
                taken.insert(target.clone());
                outputs.push(target);
            }
            Ok(outputs)
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

// ---------------------------------------------------------------------------
// Utility tools: blank pages / metadata / text export
// ---------------------------------------------------------------------------

/// Detect blank pages of one PDF (1-based page numbers). Cancellable via
/// `cancel_job` with the same job id. `sensitivity`: "strict" | "normal" |
/// "aggressive".
#[tauri::command]
pub async fn detect_blank_pages(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    sensitivity: String,
) -> Result<Vec<u32>, AppError> {
    let handle = registry.register(&job_id);
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::blank::detect_blank_pages(&app, &handle, &jid, &input_path, &sensitivity)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    res
}

/// Read a PDF's /Info metadata (Title, Author, Subject, Keywords, Creator,
/// Producer). Missing entries come back as null.
#[tauri::command]
pub async fn read_pdf_meta(
    input_path: String,
) -> Result<crate::pdf_engine::metadata::PdfMeta, AppError> {
    tauri::async_runtime::spawn_blocking(move || crate::pdf_engine::metadata::read_meta(&input_path))
        .await
        .map_err(|e| AppError::io("Could not read the metadata.", e))?
}

/// Write /Info metadata to a copy of the PDF. Empty/null fields are removed;
/// `clear_all` strips the whole /Info ("sanitize"). Turkish and any other
/// non-ASCII text is stored as UTF-16BE so it survives.
#[tauri::command]
pub async fn write_pdf_meta(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    fields: crate::pdf_engine::metadata::PdfMeta,
    clear_all: bool,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::metadata::write_meta(
            &app2, &handle, &jid, &input_path, &output_path, &fields, clear_all,
        )
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(JobResult { job_id, output_paths, status: "completed".to_string() })
}

/// Export a PDF's text (whole document or a page range) to a UTF-8 .txt file.
#[tauri::command]
pub async fn export_pdf_text(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    first_page: Option<u32>,
    last_page: Option<u32>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::textexport::export_text(
            &app2, &handle, &jid, &input_path, &output_path, first_page, last_page,
        )
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
        // LibreOffice names its output after the input stem, so name the
        // intermediate merged PDF after the user's first source file — the
        // converted document then gets a meaningful name (not "merged.docx").
        let base_stem = groups.first().map(|g| stem_of(&g.path)).unwrap_or_else(|| "document".into());
        // Never silently overwrite an existing file in the output folder.
        let mut stem = base_stem.clone();
        let mut k = 2;
        while std::path::Path::new(&output_dir)
            .join(format!("{stem}.{format}"))
            .exists()
        {
            stem = format!("{base_stem} ({k})");
            k += 1;
        }
        let merged = work.join(format!("{stem}.pdf"));
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
