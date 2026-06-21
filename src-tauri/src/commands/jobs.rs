//! Job-control commands.

use crate::error::AppError;
use crate::models::JobRegistry;

/// Cancel a running job by id. Killing the child (if any) is handled by the
/// `JobHandle`; the worker thread reaps it and returns `AppError::cancelled`.
#[tauri::command]
pub async fn cancel_job(
    registry: tauri::State<'_, JobRegistry>,
    job_id: String,
) -> Result<(), AppError> {
    if let Some(h) = registry.get(&job_id) {
        h.cancel();
    }
    Ok(())
}
