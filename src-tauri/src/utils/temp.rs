//! Temp/job scratch directory management. Everything lives under the app cache
//! directory so it is per-user and easy to clear.

use crate::error::AppError;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// The root scratch directory: `<app_cache_dir>/jobs`. Created if missing.
pub fn root(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| AppError::io("Could not resolve the app cache directory.", e))?;
    let dir = cache.join("jobs");
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::io("Could not create the temp directory.", e))?;
    Ok(dir)
}

/// Return the temp root as a string (for the frontend).
pub fn get(app: &tauri::AppHandle) -> Result<String, AppError> {
    Ok(root(app)?.to_string_lossy().to_string())
}

/// Recursively sum the size of every file under `dir`. Missing dirs count as 0.
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => total += dir_size(&path),
            Ok(ft) if ft.is_file() => {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
            _ => {}
        }
    }
    total
}

/// Delete everything inside the temp root (keeping the root itself) and return
/// the number of bytes freed. Robust to a missing directory (returns 0).
pub fn clear(app: &tauri::AppHandle) -> Result<u64, AppError> {
    let dir = root(app)?;
    if !dir.exists() {
        return Ok(0);
    }

    let freed = dir_size(&dir);

    for entry in fs::read_dir(&dir)
        .map_err(|e| AppError::io("Could not read the temp directory.", e))?
        .flatten()
    {
        let path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                let _ = fs::remove_dir_all(&path);
            }
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    Ok(freed)
}
