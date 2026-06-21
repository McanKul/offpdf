//! Free-disk-space checks. Uses `fs2::available_space` against the nearest
//! existing ancestor of the target path (the path itself may not exist yet).

use crate::error::AppError;
use crate::models::DiskSpaceInfo;
use std::path::Path;

/// Walk up from `path` until an existing directory/file is found, then query the
/// available space on that volume. Reports whether `required` bytes will fit.
pub fn check(path: &str, required: u64) -> Result<DiskSpaceInfo, AppError> {
    let mut probe = Path::new(path);

    // Find the nearest existing ancestor so fs2 has a real path to stat.
    let existing = loop {
        if probe.exists() {
            break probe;
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => {
                return Err(AppError::io(
                    "Could not locate a volume for the given path.",
                    format!("No existing ancestor of \"{path}\""),
                ))
            }
        }
    };

    let available = fs2::available_space(existing)
        .map_err(|e| AppError::io("Could not read free disk space.", e))?;

    Ok(DiskSpaceInfo {
        path: path.to_string(),
        available_bytes: available,
        required_bytes: required,
        sufficient: available >= required,
    })
}
