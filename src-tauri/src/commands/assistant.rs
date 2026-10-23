//! Private-assistant ingestion commands.
//!
//! Native text extraction is bounded to a small byte cap. OCR is an explicit
//! user action: one temporary searchable PDF is built, its per-page text is
//! extracted, and the PDF is always removed before the command returns — even
//! when the job is cancelled or OCR fails.
//!
//! No network access and no permanent index: only the requested files are
//! touched, and only text crosses IPC.

use crate::error::AppError;
use crate::models::{AssistantText, JobRegistry, JobUpdate, PagePick};
use crate::pdf_engine::{ocr, qpdf, render};
use crate::utils::temp;
use std::path::Path;
use tauri::Emitter;

/// Hard cap on text returned for one source or one OCR pass. The assistant only
/// needs a bounded context, so this is far smaller than the search cap.
const MAX_ASSISTANT_TEXT_BYTES: u64 = 8 * 1024 * 1024;

/// Cap OCR work per request so a scanned book cannot fill the disk or block a
/// session indefinitely. The frontend applies a smaller default budget.
const MAX_OCR_PAGES: usize = 200;

/// Extract native text for the assistant, one string per page, bounded.
#[tauri::command]
pub async fn assistant_pdf_text(
    app: tauri::AppHandle,
    input_path: String,
) -> Result<AssistantText, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        require_file(&input_path)?;
        let (pages, truncated) =
            render::page_texts_bounded(&app, &input_path, MAX_ASSISTANT_TEXT_BYTES)?;
        Ok(AssistantText { pages, truncated })
    })
    .await
    .map_err(|e| AppError::io("Could not read the document text.", e))?
}

/// OCR only the requested pages into a temporary searchable PDF, extract their
/// text, and delete the temporary PDF. Cancellable with `cancel_job(job_id)`.
#[tauri::command]
pub async fn assistant_ocr_text(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    pages: Vec<u32>,
    lang: String,
) -> Result<AssistantText, AppError> {
    super::jobs::check_job_id(&job_id)?;
    // The frontend filters and sorts pages, but keep repeated page numbers from
    // wasting OCR work if this command is called directly.
    let mut seen = std::collections::HashSet::new();
    let pages: Vec<u32> = pages
        .into_iter()
        .filter(|page| seen.insert(*page))
        .collect();
    if pages.is_empty() {
        return Err(AppError::new(
            "NO_PAGES",
            "No pages selected",
            "Choose at least one page to OCR.",
        ));
    }
    if pages.len() > MAX_OCR_PAGES {
        return Err(AppError::new(
            "OCR_TOO_MANY_PAGES",
            "Too many pages for OCR",
            format!("OCR is limited to {MAX_OCR_PAGES} pages per request."),
        ));
    }
    if pages.iter().any(|page| *page == 0) {
        return Err(AppError::new(
            "INVALID_PAGES",
            "Invalid page selection",
            "Page numbers start at 1.",
        ));
    }

    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        require_file(&input_path)?;

        // qpdf also rejects encrypted or malformed PDFs before OCR starts.
        let total = qpdf::npages(&app2, &input_path)?;
        if let Some(page) = pages.iter().find(|page| **page > total) {
            return Err(AppError::new(
                "INVALID_PAGES",
                "Invalid page selection",
                format!("Page {page} is outside this {total}-page document."),
            ));
        }

        let dir = temp::root(&app2)?.join("assistant");
        std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::io("Could not create a temporary OCR folder.", e))?;
        // `job_id` is restricted to `[A-Za-z0-9_-]`, so this can never escape
        // the assistant temp folder.
        let output = dir.join(format!("{jid}.pdf"));
        let output_str = output.to_string_lossy().to_string();
        let picks: Vec<PagePick> = pages
            .iter()
            .map(|page| PagePick {
                path: input_path.clone(),
                page: *page,
            })
            .collect();

        let text = (|| -> Result<AssistantText, AppError> {
            ocr::ocr(&app2, &handle, &jid, &picks, &output_str, &lang)?;
            let (pages, truncated) =
                render::page_texts_bounded(&app2, &output_str, MAX_ASSISTANT_TEXT_BYTES)?;
            Ok(AssistantText { pages, truncated })
        })();

        // Always remove the temporary searchable PDF, including on cancel/fail.
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_dir(&dir);
        text
    })
    .await;

    // Always free the job slot, including if the blocking task panicked.
    registry.remove(&job_id);
    let result =
        joined.map_err(|e| AppError::io("Could not read the document text with OCR.", e))?;
    let text = result?;
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    Ok(text)
}

fn require_file(path: &str) -> Result<(), AppError> {
    if Path::new(path).is_file() {
        Ok(())
    } else {
        Err(AppError::invalid_pdf(path))
    }
}
