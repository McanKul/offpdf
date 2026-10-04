//! Job-control commands.

use crate::error::AppError;
use crate::models::JobRegistry;

/// Longest job id the frontend sends (`crypto.randomUUID()` is 36 characters).
const JOB_ID_MAX: usize = 128;

/// The webview's job id names a work folder (`<temp>/work/<job_id>`) that the job deletes when it
/// ends: only `[A-Za-z0-9_-]{1,128}` is accepted, so it can never step out of the temp root.
pub(crate) fn check_job_id(job_id: &str) -> Result<(), AppError> {
    let ok = !job_id.is_empty()
        && job_id.len() <= JOB_ID_MAX
        && job_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        return Ok(());
    }
    Err(AppError::new(
        "INVALID_JOB",
        "Invalid job",
        "OffPDF received a job id it does not accept.",
    )
    .with_details(format!("job id of {} bytes", job_id.len())))
}

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

#[cfg(test)]
mod tests {
    use super::check_job_id;

    /// review-T5: a job id reaches `remove_dir_all(<temp>/work/<job_id>)`.
    #[test]
    fn job_ids_cannot_leave_the_work_folder() {
        for ok in [
            "3f2b8c1e-0d4a-4c6e-9a51-7b2f0e9d1c34",
            "job-1700000000000-1a2b",
            "a_b",
        ] {
            assert!(check_job_id(ok).is_ok(), "{ok}");
        }
        let long = "a".repeat(129);
        for bad in [
            "",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:x",
            "a b",
            long.as_str(),
            "x\0",
        ] {
            let e = check_job_id(bad)
                .err()
                .unwrap_or_else(|| panic!("{bad:?} accepted"));
            assert_eq!(e.code, "INVALID_JOB");
        }
    }
}
