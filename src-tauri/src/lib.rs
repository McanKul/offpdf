//! OffPDF Tauri backend.
//!
//! Local-first, offline-only. The backend NEVER makes a network request.
//! All PDF processing happens by spawning a local/bundled `qpdf` binary and
//! streaming its output. Only file *paths* cross the IPC boundary — never file
//! bytes — so multi-gigabyte PDFs are handled without buffering them in memory.
//!
//! ## Command contract (implemented in `commands::*`)
//!
//! Files / system (`commands::files`):
//!   - `pick_pdf_files() -> Result<Vec<String>, AppError>`
//!   - `pick_pdf_file() -> Result<Option<String>, AppError>`
//!   - `pick_output_folder() -> Result<Option<String>, AppError>`
//!   - `get_file_info(path: String) -> Result<FileInfo, AppError>`
//!   - `check_disk_space(path: String, required_bytes: u64) -> Result<DiskSpaceInfo, AppError>`
//!   - `open_path(path: String) -> Result<(), AppError>`
//!   - `clear_temp_files() -> Result<u64, AppError>`  (bytes freed)
//!   - `get_temp_dir() -> Result<String, AppError>`
//!
//! PDF operations (`commands::pdf`) — all take a frontend-generated `job_id`
//! and emit `job:update` events while running:
//!   - `merge_pdfs(window, registry, job_id, input_paths: Vec<String>, output_path: String)`
//!   - `split_pdf(window, registry, job_id, input_path: String, output_dir: String, mode: SplitMode)`
//!   - `extract_pages(window, registry, job_id, input_path, output_path, pages: String)`
//!   - `delete_pages(window, registry, job_id, input_path, output_path, pages: String)`
//!   - `rotate_pages(window, registry, job_id, input_path, output_path, pages: String, angle: i32)`
//!   - `reorder_pages(window, registry, job_id, input_path, output_path, order: String)`
//!   - `optimize_pdf(window, registry, job_id, input_path, output_path)`
//!   - all return `Result<JobResult, AppError>`
//!
//! Jobs (`commands::jobs`):
//!   - `cancel_job(registry, job_id: String) -> Result<(), AppError>`

mod commands;
mod error;
mod models;
mod pdf_engine;
mod utils;

use models::JobRegistry;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Local file dialogs (open/save). No network.
        .plugin(tauri_plugin_dialog::init())
        // Open a file or folder in the OS default handler. No network.
        .plugin(tauri_plugin_opener::init())
        // Shared, cancellable job registry.
        .manage(JobRegistry::default())
        .invoke_handler(tauri::generate_handler![
            // files / system
            commands::files::pick_pdf_files,
            commands::files::pick_pdf_file,
            commands::files::pick_output_folder,
            commands::files::get_file_info,
            commands::files::check_disk_space,
            commands::files::open_path,
            commands::files::copy_file,
            commands::files::clear_temp_files,
            commands::files::get_temp_dir,
            // pdf operations
            commands::pdf::merge_pdfs,
            commands::pdf::assemble_pdf,
            commands::pdf::edit_pdf,
            commands::pdf::split_pdf,
            commands::pdf::extract_pages,
            commands::pdf::delete_pages,
            commands::pdf::rotate_pages,
            commands::pdf::reorder_pages,
            commands::pdf::optimize_pdf,
            commands::pdf::compress_pdf,
            commands::pdf::protect_pdf,
            commands::pdf::unlock_pdf,
            commands::pdf::add_page_numbers,
            commands::pdf::stamp_pdf,
            commands::pdf::watermark_pdf,
            commands::pdf::crop_pdf,
            commands::pdf::poster_pdf,
            commands::pdf::nup_pdf,
            // jobs
            commands::jobs::cancel_job,
            // page preview / review
            commands::render::renderer_available,
            commands::render::render_thumbnails,
            commands::render::image_to_pdf,
            commands::render::pdf_to_images,
            commands::render::pdf_text,
            commands::render::page_pdf,
            commands::render::pdf_outline,
            commands::render::diff_pages,
            commands::render::office_available,
            commands::render::office_to_pdf,
            commands::render::office_to_pdf_batch,
            commands::render::pdf_to_office,
            commands::render::ocr_available,
            commands::render::ocr_pdf,
            commands::render::pdfa_pdf,
            commands::render::detect_blank_pages,
            commands::render::read_pdf_meta,
            commands::render::write_pdf_meta,
            commands::render::export_pdf_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the OffPDF application");
}
