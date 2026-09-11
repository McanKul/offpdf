//! Opt-in model lifecycle helpers. Injected store root; no HTTP.
//!
//! Preview hashes a user-chosen path and does not write. Import is a second
//! explicit call. Load requires a ready store entry.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::AppError;

use super::store::{ModelManifest, ModelStore};
use super::{BackendStatus, InferenceBackend};

const LICENSE_PLACEHOLDER: &str = "imported";
const RUNTIME_COMPAT_PLACEHOLDER: &str = "any";
const HARDWARE_STUB: &str = "Hardware check is not available in this version.";

/// Facts shown before an explicit import. Preview must not create blobs.
#[derive(Debug, Clone)]
pub struct ModelPreview {
    pub size: u64,
    pub sha256: String,
    pub license: String,
    pub runtime_compat: String,
    pub location: PathBuf,
    pub compatibility: String,
}

/// Disk bytes recovered by deleting one checksum (blob size, at least).
#[derive(Debug, Clone)]
pub struct ModelRemoveResult {
    pub checksum: String,
    pub recovered_bytes: u64,
}

/// Store-on-disk plus in-memory backend status. Restart does not auto-load.
#[derive(Debug, Clone)]
pub struct ModelSetupSnapshot {
    pub ready: Vec<ModelManifest>,
    pub backend_status: BackendStatus,
}

/// Hash `path` and report the store location it would occupy. Does not write.
pub fn preview(root: &Path, path: &Path) -> Result<ModelPreview, AppError> {
    let (size, sha256) = sha256_file(path)?;
    Ok(ModelPreview {
        size,
        location: root.join("blobs").join(&sha256),
        sha256,
        license: LICENSE_PLACEHOLDER.to_string(),
        runtime_compat: RUNTIME_COMPAT_PLACEHOLDER.to_string(),
        compatibility: format!("{RUNTIME_COMPAT_PLACEHOLDER}. {HARDWARE_STUB}"),
    })
}

/// Copy `path` into the store after the caller confirms `expected_sha256`.
pub fn import(root: &Path, path: &Path, expected_sha256: &str) -> Result<ModelManifest, AppError> {
    let store = ModelStore::open(root)?;
    store.import_file(path, expected_sha256)
}

/// Require a ready checksum, then load the in-process backend.
pub fn load(
    root: &Path,
    backend: &(impl InferenceBackend + ?Sized),
    checksum: &str,
) -> Result<(), AppError> {
    let store = ModelStore::open(root)?;
    let _ready = store.get(checksum)?;
    backend.load()
}

/// Unload the in-process backend. Disk entries are unchanged.
pub fn unload(backend: &(impl InferenceBackend + ?Sized)) -> Result<(), AppError> {
    backend.unload()
}

/// Cooperative cancel of an in-flight `generate`.
pub fn cancel(backend: &(impl InferenceBackend + ?Sized)) {
    backend.cancel();
}

/// UTF-8 prompt in, UTF-8 text out. No document path.
pub fn generate(backend: &(impl InferenceBackend + ?Sized), prompt: &str) -> Result<String, AppError> {
    backend.generate(prompt)
}

/// Delete one checksum and report recovered blob bytes.
pub fn remove(root: &Path, checksum: &str) -> Result<ModelRemoveResult, AppError> {
    let store = ModelStore::open(root)?;
    let recovered_bytes = recorded_remove_size(root, checksum);
    store.remove(checksum)?;
    Ok(ModelRemoveResult {
        checksum: checksum.to_string(),
        recovered_bytes,
    })
}

/// Size from dest `manifests/<checksum>.json`, else blob `metadata.len()`.
/// Does not hash. Truncated blobs still report the recorded size.
fn recorded_remove_size(root: &Path, checksum: &str) -> u64 {
    let manifest_path = root.join("manifests").join(format!("{checksum}.json"));
    if let Ok(json) = fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = ModelManifest::parse(&json) {
            return manifest.size;
        }
    }
    fs::metadata(root.join("blobs").join(checksum))
        .map(|meta| meta.len())
        .unwrap_or(0)
}

/// Ready manifests on disk plus the current backend status.
pub fn snapshot(
    root: &Path,
    backend: &(impl InferenceBackend + ?Sized),
) -> Result<ModelSetupSnapshot, AppError> {
    let store = ModelStore::open(root)?;
    Ok(ModelSetupSnapshot {
        ready: store.list_ready()?,
        backend_status: backend.status(),
    })
}

fn sha256_file(path: &Path) -> Result<(u64, String), AppError> {
    let mut file = File::open(path).map_err(|err| {
        AppError::ai_model_invalid()
            .with_details(format!("could not read {}: {err}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, hex_lower(&hasher.finalize())))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
