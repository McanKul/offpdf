//! Spawning and supervising the local `qpdf` process.
//!
//! Never builds a shell string — always an argv array. Supports cooperative
//! cancellation: the `JobHandle` owns the `Child` so `cancel_job` can `kill()`
//! it; this worker reaps it and distinguishes a user cancel from a real error.

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate};
use crate::pdf_engine::qpdf;
use std::io::{BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use tauri::Emitter;

/// Spawn an already-configured command (caller sets args + `stdout(null)` +
/// `stderr(piped)`), registering the child in the `JobHandle` so `cancel_job`
/// can `kill()` it mid-run. Reads stderr to EOF, reaps the process, and returns
/// `(exit status, stderr text)`. Returns `AppError::cancelled()` if cancelled.
///
/// This is what makes the poppler/tesseract/LibreOffice steps cancellable, the
/// same way `run_qpdf` already is.
pub fn run_tracked(
    handle: &Arc<JobHandle>,
    mut cmd: Command,
) -> Result<(Option<ExitStatus>, String), AppError> {
    if handle.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::engine_missing())
        }
        Err(err) => return Err(AppError::engine_failed(err.to_string())),
    };

    let stderr = child.stderr.take();
    *handle.child.lock().unwrap() = Some(child);
    if handle.is_cancelled() {
        handle.cancel();
    }

    let mut stderr_text = String::new();
    if let Some(pipe) = stderr {
        let mut reader = BufReader::new(pipe);
        let _ = reader.read_to_string(&mut stderr_text);
    }

    let taken = handle.child.lock().unwrap().take();
    let status = taken.and_then(|mut c| c.wait().ok());

    if handle.is_cancelled() {
        return Err(AppError::cancelled());
    }
    Ok((status, stderr_text))
}

/// Run qpdf with the given args. Emits a `job:update` (state "running") before
/// spawning. Returns `Ok(())` on success or qpdf warnings (exit code 3).
pub fn run_qpdf(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    args: &[String],
    step: &str,
    percent: Option<f32>,
) -> Result<(), AppError> {
    if handle.is_cancelled() {
        return Err(AppError::cancelled());
    }

    // Progress ping for the UI.
    let mut update = JobUpdate::new(job_id, "running", step);
    if let Some(p) = percent {
        update = update.percent(p);
    }
    let _ = app.emit("job:update", update);

    let exe = qpdf::resolve_qpdf(app);
    let mut cmd = Command::new(exe);
    cmd.args(args);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: avoid flashing a console window.
        cmd.creation_flags(0x08000000);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::engine_missing())
        }
        Err(err) => return Err(AppError::engine_failed(err.to_string())),
    };

    // Own the stderr pipe ourselves; the Child goes to the handle for cancel.
    let stderr = child.stderr.take();
    *handle.child.lock().unwrap() = Some(child);

    // If cancellation raced in between spawn and storing the child, kill now.
    if handle.is_cancelled() {
        handle.cancel();
    }

    // Read stderr to EOF. This returns once the process exits or is killed.
    let mut stderr_text = String::new();
    if let Some(pipe) = stderr {
        let mut reader = BufReader::new(pipe);
        let _ = reader.read_to_string(&mut stderr_text);
    }

    // Reap the process.
    let taken = handle.child.lock().unwrap().take();
    let status = taken.and_then(|mut c| c.wait().ok());

    if handle.is_cancelled() {
        return Err(AppError::cancelled());
    }

    let code = status.and_then(|s| s.code());
    // qpdf exit code 3 means "completed with warnings" — treat as success.
    let ok = status.map(|s| s.success()).unwrap_or(false) || code == Some(3);

    if !ok {
        let detail = if stderr_text.trim().is_empty() {
            "qpdf exited with an error".to_string()
        } else {
            stderr_text.trim().to_string()
        };
        return Err(AppError::engine_failed(detail));
    }

    Ok(())
}
