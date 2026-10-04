//! Edit text commands (SPEC §B.20): open a source, inspect a page, preview edits, release.
//! Paths in, JSON out; the work runs on a blocking worker. Save stays `edit_pdf_overlays`.

use crate::error::AppError;
use crate::pdf_engine::text_edit::cache::TextEditCache;
use crate::pdf_engine::text_edit::dto::{PageTextDto, TextEditIn, TextPreviewDto, TextSourceDto};
use crate::pdf_engine::text_edit::engines::Engines;
use crate::pdf_engine::text_edit::service;
use crate::utils::temp;

/// Runs `f` on a blocking worker with the resolved engines and the temp root.
async fn on_worker<T, F>(app: tauri::AppHandle, f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&Engines, &std::path::Path) -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let engines = Engines::resolve(&app)?;
        let temp_root = temp::root(&app)?;
        f(&engines, &temp_root)
    })
    .await
    .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))?
}

#[tauri::command]
pub async fn open_text_source(
    app: tauri::AppHandle,
    cache: tauri::State<'_, TextEditCache>,
    input_path: String,
) -> Result<TextSourceDto, AppError> {
    let cache = cache.inner().clone();
    on_worker(app, move |engines, temp_root| {
        service::open_source(&cache, engines, temp_root, &input_path)
    })
    .await
}

#[tauri::command]
pub async fn inspect_text_page(
    app: tauri::AppHandle,
    cache: tauri::State<'_, TextEditCache>,
    input_path: String,
    fingerprint: String,
    page_index: u32,
) -> Result<PageTextDto, AppError> {
    let cache = cache.inner().clone();
    on_worker(app, move |engines, temp_root| {
        service::inspect_page(
            &cache,
            engines,
            temp_root,
            &input_path,
            &fingerprint,
            page_index,
        )
    })
    .await
}

#[tauri::command]
pub async fn preview_text_edits(
    app: tauri::AppHandle,
    cache: tauri::State<'_, TextEditCache>,
    input_path: String,
    fingerprint: String,
    page_index: u32,
    edits: Vec<TextEditIn>,
) -> Result<TextPreviewDto, AppError> {
    let cache = cache.inner().clone();
    on_worker(app, move |engines, temp_root| {
        service::preview_edits(
            &cache,
            engines,
            temp_root,
            &input_path,
            &fingerprint,
            page_index,
            &edits,
        )
    })
    .await
}

#[tauri::command]
pub async fn release_text_source(
    cache: tauri::State<'_, TextEditCache>,
    input_path: String,
) -> Result<(), AppError> {
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service::release_source(&cache, &input_path))
        .await
        .map_err(|e| AppError::engine_failed(format!("worker join error: {e}")))
}
