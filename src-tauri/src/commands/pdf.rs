//! Async PDF operation commands.
//!
//! Each command registers a cancellable job, runs the synchronous engine op on
//! a blocking worker thread, emits a final `completed` update, and returns a
//! `JobResult`. The job is registered BEFORE spawning (so cancel works) and
//! removed AFTER the worker joins.

use crate::error::AppError;
use crate::models::{JobRegistry, JobResult, JobUpdate, PageGroup, PagePick, RotateGroup, SplitMode};
use tauri::Emitter;

/// Helper: emit the final "completed" update and build the JobResult.
fn completed(app: &tauri::AppHandle, job_id: String, output_paths: Vec<String>) -> JobResult {
    let _ = app.emit("job:update", JobUpdate::new(&job_id, "completed", "Done"));
    JobResult {
        job_id,
        output_paths,
        status: "completed".to_string(),
    }
}

#[tauri::command]
pub async fn merge_pdfs(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_paths: Vec<String>,
    output_path: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::merge(&app2, &handle, &jid, &input_paths, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn split_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_dir: String,
    picks: Vec<PagePick>,
    mode: SplitMode,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::split(&app2, &handle, &jid, &picks, &output_dir, &mode)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn assemble_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::assemble(&app2, &handle, &jid, &groups, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn edit_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    rotations: Vec<RotateGroup>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::edit(&app2, &handle, &jid, &groups, &rotations, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn unlock_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    password: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::unlock(&app2, &handle, &jid, &input_path, &output_path, &password)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn watermark_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    text: String,
    opacity: f64,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::overlay::add_watermark(&app2, &handle, &jid, &groups, &output_path, &text, opacity)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn crop_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::crop::crop(&app2, &handle, &jid, &groups, &output_path, left, top, right, bottom)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn stamp_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    page: u32,
    anchor: String,
    text: String,
    color: [f64; 3],
    size_pct: f64,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::stamp::stamp_text(&app2, &handle, &jid, &groups, &output_path, page, &anchor, &text, color, size_pct)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn poster_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    page: u32,
    tile_w: f64,
    tile_h: f64,
    overlap: f64,
    marks: bool,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::poster::poster(&app2, &handle, &jid, &groups, &output_path, page, tile_w, tile_h, overlap, marks)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn nup_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    mode: String,
    sheet_w: f64,
    sheet_h: f64,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::nup::nup(&app2, &handle, &jid, &groups, &output_path, &mode, sheet_w, sheet_h)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn add_page_numbers(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    position: String,
    start: i64,
    prefix: Option<String>,
    pad_width: Option<u32>,
    with_date: Option<bool>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        // Any format arg present → Bates mode; all absent → plain numbers.
        let format = if prefix.is_some() || pad_width.is_some() || with_date.unwrap_or(false) {
            Some(crate::pdf_engine::overlay::NumberFormat {
                prefix: prefix.unwrap_or_default(),
                pad_width: pad_width.unwrap_or(0) as usize,
                with_date: with_date.unwrap_or(false),
            })
        } else {
            None
        };
        crate::pdf_engine::overlay::add_page_numbers_formatted(
            &app2, &handle, &jid, &groups, &output_path, &position, start, format,
        )
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn protect_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    user_password: String,
    owner_password: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::protect(&app2, &handle, &jid, &groups, &user_password, &owner_password, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn extract_pages(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    pages: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::extract(&app2, &handle, &jid, &input_path, &output_path, &pages)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn delete_pages(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    pages: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::delete(&app2, &handle, &jid, &input_path, &output_path, &pages)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn rotate_pages(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
    angle: i32,
    rotate_pages: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::rotate(&app2, &handle, &jid, &groups, angle, &rotate_pages, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn reorder_pages(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    input_path: String,
    output_path: String,
    order: String,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::reorder(&app2, &handle, &jid, &input_path, &output_path, &order)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn compress_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    picks: Vec<PagePick>,
    dpi: u32,
    quality: u32,
    target_bytes: Option<u64>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::compress::compress(
            &app2, &handle, &jid, &picks, &output_path, dpi, quality, target_bytes,
        )
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}

#[tauri::command]
pub async fn optimize_pdf(
    app: tauri::AppHandle,
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
    output_path: String,
    groups: Vec<PageGroup>,
) -> Result<JobResult, AppError> {
    let handle = registry.register(&job_id);
    let app2 = app.clone();
    let jid = job_id.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        crate::pdf_engine::optimize(&app2, &handle, &jid, &groups, &output_path)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?;
    registry.remove(&job_id);
    let output_paths = res?;
    Ok(completed(&app, job_id, output_paths))
}
