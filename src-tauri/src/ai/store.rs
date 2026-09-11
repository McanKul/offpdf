//! Content-addressed on-disk model store. Injected root; no HTTP.
//!
//! Layout: `<root>/{blobs,manifests,staging}/`. Identity is SHA-256.
//! Ready means a manifest and blob are both present and the blob re-hashes
//! to the checksum. Leftover `staging/` is never ready.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

const SCHEMA_VERSION: u32 = 1;
const CHECKSUM_LEN: usize = 64;
const LICENSE_PLACEHOLDER: &str = "imported";
const RUNTIME_COMPAT_PLACEHOLDER: &str = "any";

/// Versioned model metadata. `schema_version` must be 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub size: u64,
    pub license: String,
    pub runtime_compat: String,
    pub checksum: String,
}

impl ModelManifest {
    pub fn parse(json: &str) -> Result<Self, AppError> {
        let parsed: Self = serde_json::from_str(json)
            .map_err(|err| AppError::ai_model_invalid().with_details(err.to_string()))?;
        if parsed.schema_version != SCHEMA_VERSION {
            return Err(AppError::ai_model_invalid().with_details(format!(
                "unsupported schema_version {}",
                parsed.schema_version
            )));
        }
        validate_checksum(&parsed.checksum)?;
        Ok(parsed)
    }
}

/// Persistent store under an injected root. Tests pass a temp path.
#[derive(Debug, Clone)]
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    pub fn open(root: &Path) -> Result<Self, AppError> {
        let store = Self {
            root: root.to_path_buf(),
        };
        fs::create_dir_all(store.blobs_dir())?;
        fs::create_dir_all(store.manifests_dir())?;
        fs::create_dir_all(store.staging_dir())?;
        Ok(store)
    }

    pub fn install_from_reader(
        &self,
        r: &mut impl Read,
        expected_sha256: &str,
    ) -> Result<ModelManifest, AppError> {
        let expected = validate_checksum(expected_sha256)?;
        let staging = self.new_staging_dir()?;
        let _guard = StagingGuard(staging.clone());
        let staged_blob = staging.join("blob");
        let (size, actual) = write_hashed(r, &staged_blob)?;
        if actual != expected {
            return Err(AppError::ai_model_invalid().with_details(format!(
                "checksum mismatch: expected {expected}, got {actual}"
            )));
        }

        let manifest = ModelManifest {
            schema_version: SCHEMA_VERSION,
            size,
            license: LICENSE_PLACEHOLDER.to_string(),
            runtime_compat: RUNTIME_COMPAT_PLACEHOLDER.to_string(),
            checksum: expected.clone(),
        };
        self.commit_ready(&manifest, &staged_blob)?;
        Ok(manifest)
    }

    pub fn import_file(
        &self,
        path: &Path,
        expected_sha256: &str,
    ) -> Result<ModelManifest, AppError> {
        let mut file = File::open(path).map_err(|err| {
            AppError::ai_model_invalid()
                .with_details(format!("could not read {}: {err}", path.display()))
        })?;
        self.install_from_reader(&mut file, expected_sha256)
    }

    pub fn list_ready(&self) -> Result<Vec<ModelManifest>, AppError> {
        let mut ready = Vec::new();
        let dir = match fs::read_dir(self.manifests_dir()) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ready);
            }
            Err(err) => return Err(err.into()),
        };
        for entry in dir {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if validate_checksum(stem).is_err() {
                continue;
            }
            if let Some(manifest) = self.ready_manifest(stem)? {
                ready.push(manifest);
            }
        }
        ready.sort_by(|a, b| a.checksum.cmp(&b.checksum));
        Ok(ready)
    }

    pub fn get(&self, checksum: &str) -> Result<ModelManifest, AppError> {
        let checksum = validate_checksum(checksum)?;
        self.ready_manifest(&checksum)?
            .ok_or_else(AppError::ai_model_not_found)
    }

    pub fn loadable_path(&self, checksum: &str) -> Result<PathBuf, AppError> {
        let manifest = self.get(checksum)?;
        Ok(self.blob_path(&manifest.checksum))
    }

    pub fn remove(&self, checksum: &str) -> Result<(), AppError> {
        let checksum = validate_checksum(checksum)?;
        let blob = self.blob_path(&checksum);
        let manifest = self.manifest_path(&checksum);
        let staging = self.staging_dir().join(&checksum);
        let blob_exists = blob.exists();
        let manifest_exists = manifest.exists();
        let staging_exists = staging.exists();
        if !blob_exists && !manifest_exists && !staging_exists {
            return Err(AppError::ai_model_not_found());
        }
        if blob_exists {
            fs::remove_file(&blob)?;
        }
        if manifest_exists {
            fs::remove_file(&manifest)?;
        }
        if staging_exists {
            fs::remove_dir_all(&staging)?;
        }
        Ok(())
    }

    fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    fn manifests_dir(&self) -> PathBuf {
        self.root.join("manifests")
    }

    fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }

    fn blob_path(&self, checksum: &str) -> PathBuf {
        self.blobs_dir().join(checksum)
    }

    fn manifest_path(&self, checksum: &str) -> PathBuf {
        self.manifests_dir().join(format!("{checksum}.json"))
    }

    fn new_staging_dir(&self) -> Result<PathBuf, AppError> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = self
            .staging_dir()
            .join(format!("install-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn commit_ready(&self, manifest: &ModelManifest, staged_blob: &Path) -> Result<(), AppError> {
        fs::create_dir_all(self.blobs_dir())?;
        fs::create_dir_all(self.manifests_dir())?;
        let dest_blob = self.blob_path(&manifest.checksum);
        if !self.blob_matches(&dest_blob, manifest)? {
            if dest_blob.exists() {
                fs::remove_file(&dest_blob)?;
            }
            fs::rename(staged_blob, &dest_blob)?;
        }
        let dest_manifest = self.manifest_path(&manifest.checksum);
        if dest_manifest.exists() {
            if let Ok(existing) = fs::read_to_string(&dest_manifest) {
                if let Ok(parsed) = ModelManifest::parse(&existing) {
                    if parsed == *manifest {
                        return Ok(());
                    }
                }
            }
        }
        let json = serde_json::to_string(manifest)
            .map_err(|err| AppError::ai_model_invalid().with_details(err.to_string()))?;
        let staged_manifest = staged_blob
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("manifest.json");
        fs::write(&staged_manifest, json.as_bytes())?;
        fs::rename(&staged_manifest, &dest_manifest)?;
        Ok(())
    }

    fn ready_manifest(&self, checksum: &str) -> Result<Option<ModelManifest>, AppError> {
        let man_path = self.manifest_path(checksum);
        if !man_path.is_file() {
            return Ok(None);
        }
        let json = match fs::read_to_string(&man_path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let manifest = match ModelManifest::parse(&json) {
            Ok(parsed) => parsed,
            Err(_) => return Ok(None),
        };
        if manifest.checksum != checksum {
            return Ok(None);
        }
        if !self.blob_matches(&self.blob_path(checksum), &manifest)? {
            return Ok(None);
        }
        Ok(Some(manifest))
    }

    fn blob_matches(&self, path: &Path, manifest: &ModelManifest) -> Result<bool, AppError> {
        if !path.is_file() {
            return Ok(false);
        }
        let meta = fs::metadata(path)?;
        if meta.len() != manifest.size {
            return Ok(false);
        }
        let actual = hash_path(path)?;
        Ok(actual == manifest.checksum)
    }
}

struct StagingGuard(PathBuf);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn validate_checksum(value: &str) -> Result<String, AppError> {
    let valid = value.len() == CHECKSUM_LEN
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if valid {
        Ok(value.to_string())
    } else {
        Err(AppError::ai_model_invalid()
            .with_details("checksum must be 64 lowercase hex characters"))
    }
}

fn write_hashed(src: &mut impl Read, dest: &Path) -> Result<(u64, String), AppError> {
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    file.flush()?;
    Ok((size, hex_lower(&hasher.finalize())))
}

fn hash_path(path: &Path) -> Result<String, AppError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
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
