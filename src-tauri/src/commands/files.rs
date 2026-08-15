//! File and system commands: native file/folder pickers, file metadata, disk
//! space, opening paths in the OS, and temp-directory management.
//!
//! Only paths cross IPC. No bytes, no network.
//!
//! macOS note: synchronous (`fn`) Tauri commands run on the MAIN thread. A
//! native file dialog (NSOpenPanel) also needs the main thread's run loop, so
//! calling `blocking_pick_*` from a sync command deadlocks the UI. Anything that
//! shows a dialog or does noticeable I/O is therefore an `async` command whose
//! work runs in `spawn_blocking` (off the main thread, leaving the run loop free
//! to present the panel).

use crate::error::AppError;
use crate::models::{DiskSpaceInfo, FileInfo, ImagePreview};
use crate::pdf_engine::qpdf;
use crate::utils::{disk, temp};
use std::path::Path;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Convert a dialog `FilePath` to a plain string, dropping non-file URLs.
fn filepath_to_string(fp: tauri_plugin_dialog::FilePath) -> Option<String> {
    fp.into_path().ok().map(|p| p.to_string_lossy().to_string())
}

/// Open a multi-select PDF picker. Returns the chosen paths (empty if cancelled).
#[tauri::command]
pub async fn pick_pdf_files(app: tauri::AppHandle) -> Result<Vec<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(
                "PDF, images & Office",
                &[
                    "pdf", "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "doc", "docx",
                    "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "csv", "html", "htm",
                ],
            )
            .add_filter("PDF documents", &["pdf"])
            .blocking_pick_files()
    })
    .await
    .map_err(|e| AppError::io("The file dialog could not be opened.", e))?;

    Ok(picked
        .unwrap_or_default()
        .into_iter()
        .filter_map(filepath_to_string)
        .collect())
}

/// Open a single-file PDF picker. Returns `None` if cancelled.
#[tauri::command]
pub async fn pick_pdf_file(app: tauri::AppHandle) -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter(
                "PDF, images & Office",
                &[
                    "pdf", "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "doc", "docx",
                    "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "csv", "html", "htm",
                ],
            )
            .add_filter("PDF documents", &["pdf"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| AppError::io("The file dialog could not be opened.", e))?;

    Ok(picked.and_then(filepath_to_string))
}

/// Single PNG/JPEG picker for editor image overlays.
#[tauri::command]
pub async fn pick_image_file(app: tauri::AppHandle) -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("PNG and JPEG", &["png", "jpg", "jpeg"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| AppError::io("The file dialog could not be opened.", e))?;

    Ok(picked.and_then(filepath_to_string))
}

/// Decode a PNG/JPEG at a bounded size for the editor preview (never full PDF bytes).
#[tauri::command]
pub async fn preview_image(path: String) -> Result<ImagePreview, AppError> {
    tauri::async_runtime::spawn_blocking(move || preview_image_sync(&path))
        .await
        .map_err(|e| AppError::io("Could not read the image.", e))?
}

fn preview_image_sync(path: &str) -> Result<ImagePreview, AppError> {
    let inspected = crate::pdf_engine::edit_image::inspect_image(path)?;
    let img = crate::pdf_engine::edit_image::decode_bounded(&inspected.bytes)?;
    let w0 = img.width().max(1);
    let h0 = img.height().max(1);
    let max_edge = 800u32;
    let img = if img.width().max(img.height()) > max_edge {
        img.thumbnail(max_edge, max_edge)
    } else {
        img
    };
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| AppError::io("Could not preview the image.", e))?;
    Ok(ImagePreview {
        width: w0,
        height: h0,
        data_url: format!("data:image/png;base64,{}", crate::pdf_engine::render::base64(&buf)),
    })
}

/// Open a folder picker for choosing an output directory. `None` if cancelled.
#[tauri::command]
pub async fn pick_output_folder(app: tauri::AppHandle) -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().blocking_pick_folder()
    })
    .await
    .map_err(|e| AppError::io("The folder dialog could not be opened.", e))?;

    Ok(picked.and_then(filepath_to_string))
}

/// Read best-effort metadata for a file. `pageCount`/`isValidPdf` are populated
/// only when the extension is `.pdf` and qpdf can read the document. Runs off
/// the main thread because reading the page count shells out to qpdf.
#[tauri::command]
pub async fn get_file_info(app: tauri::AppHandle, path: String) -> Result<FileInfo, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let meta = std::fs::metadata(&path)
            .map_err(|e| AppError::io("Could not read file information.", e))?;
        let size_bytes = meta.len();

        let name = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        let is_pdf_ext = Path::new(&path)
            .extension()
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);

        let page_count = if is_pdf_ext {
            qpdf::npages(&app, &path).ok()
        } else {
            None
        };

        let is_valid_pdf = is_pdf_ext && page_count.is_some();

        Ok(FileInfo {
            path,
            name,
            size_bytes,
            page_count,
            is_valid_pdf,
        })
    })
    .await
    .map_err(|e| AppError::io("Could not read file information.", e))?
}

/// Check whether `required_bytes` will fit on the volume of `path`. Fast (no
/// process spawn), so it can stay synchronous.
#[tauri::command]
pub fn check_disk_space(path: String, required_bytes: u64) -> Result<DiskSpaceInfo, AppError> {
    disk::check(&path, required_bytes)
}

/// Open a file or folder in the OS default handler.
#[tauri::command]
pub async fn open_path(app: tauri::AppHandle, path: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        app.opener()
            .open_path(path, None::<&str>)
            .map_err(|e| AppError::io("Could not open the path.", e))
    })
    .await
    .map_err(|e| AppError::io("Could not open the path.", e))?
}

/// Copy a file to a new path (used to save an on-the-fly OCR'd searchable copy).
#[tauri::command]
pub async fn copy_file(src: String, dst: String) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(parent) = Path::new(&dst).parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io("Could not create the output folder.", e))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| AppError::io("Could not save the file.", e))?;
        Ok(dst)
    })
    .await
    .map_err(|e| AppError::io("Could not save the file.", e))?
}

/// Clear all temp/job scratch files. Returns the number of bytes freed. Runs off
/// the main thread because it may walk and delete many files.
#[tauri::command]
pub async fn clear_temp_files(app: tauri::AppHandle) -> Result<u64, AppError> {
    tauri::async_runtime::spawn_blocking(move || temp::clear(&app))
        .await
        .map_err(|e| AppError::io("Could not clear temp files.", e))?
}

/// Return the temp/job scratch directory path. Fast, stays synchronous.
#[tauri::command]
pub fn get_temp_dir(app: tauri::AppHandle) -> Result<String, AppError> {
    temp::get(&app)
}
