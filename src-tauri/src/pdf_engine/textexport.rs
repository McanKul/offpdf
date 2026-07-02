//! Export a PDF's text to a .txt file (UTF-8, layout-preserving) via poppler's
//! `pdftotext`. The tool writes straight to the output file on disk, so there
//! is no size cap and nothing is buffered in RAM — unlike the in-app search
//! path (`render::page_texts`), which streams stdout with a hard byte cap.

use crate::error::AppError;
use crate::models::JobUpdate;
use crate::models::JobHandle;
use crate::pdf_engine::render;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Extract text from `input` into the `.txt` file at `output`. `first`/`last`
/// bound the 1-based page range; `None` means from the start / to the end.
/// Cancellable via the `JobHandle` (kills the pdftotext process).
pub fn export_text(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    first: Option<u32>,
    last: Option<u32>,
) -> Result<Vec<String>, AppError> {
    super::require_input(input)?;
    super::ensure_output_dir(output)?;
    if let (Some(f), Some(l)) = (first, last) {
        if f > l {
            return Err(AppError::new(
                "INVALID_RANGE",
                "Invalid page range",
                format!("The range starts at page {f} but ends at page {l}."),
            ));
        }
    }

    let _ = app.emit(
        "job:update",
        JobUpdate::new(job_id, "running", "Extracting text"),
    );

    let exe = render::resolve_pdftotext(app);
    let mut cmd = Command::new(&exe);
    render::configure_poppler_command(&mut cmd, &exe);
    cmd.args(["-layout", "-enc", "UTF-8"]);
    if let Some(f) = first {
        cmd.args(["-f", &f.max(1).to_string()]);
    }
    if let Some(l) = last {
        cmd.args(["-l", &l.to_string()]);
    }
    // pdftotext streams pages directly into the output file — nothing in RAM.
    cmd.arg(input).arg(output);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let (status, stderr) = crate::utils::process::run_tracked(handle, cmd)?;
    let ok = status.map(|s| s.success()).unwrap_or(false);
    if !ok || !Path::new(output).is_file() {
        let detail = if stderr.trim().is_empty() {
            "pdftotext exited with an error".to_string()
        } else {
            stderr.trim().to_string()
        };
        return Err(AppError::engine_failed(detail));
    }
    Ok(vec![output.to_string()])
}
