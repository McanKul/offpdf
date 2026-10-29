//! Local-model setup IPC. Paths in, never file bytes. No HTTP.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::ai::compatibility::{evaluate, probe, CompatibilityReport, ModelRequirements};
use crate::ai::lifecycle::{
    cancel, generate, import, load, preview, remove, snapshot, unload, ModelPreview,
    ModelRemoveResult,
};
use crate::ai::store::ModelManifest;
use crate::ai::{BackendStatus, InferenceBackend};
use crate::error::AppError;

/// In-process Fake plus the checksum last passed to `load`. Not persisted.
pub struct AiSession {
    backend: Box<dyn InferenceBackend>,
    selected: Mutex<Option<String>>,
    cancel_requested: AtomicBool,
}

impl AiSession {
    pub fn new() -> Self {
        Self {
            backend: Box::new(crate::ai::fake()),
            selected: Mutex::new(None),
            cancel_requested: AtomicBool::new(false),
        }
    }

    fn backend(&self) -> &dyn InferenceBackend {
        self.backend.as_ref()
    }

    fn selected(&self) -> Option<String> {
        self.selected
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    fn set_selected(&self, checksum: Option<String>) {
        *self.selected.lock().unwrap_or_else(|err| err.into_inner()) = checksum;
    }

    fn clear_cancel(&self) {
        self.cancel_requested.store(false, Ordering::SeqCst);
    }

    fn request_cancel(&self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
    }

    fn take_cancel(&self) -> bool {
        self.cancel_requested.swap(false, Ordering::SeqCst)
    }

    fn finish_load(&self, checksum: String) -> Result<(), AppError> {
        if self.take_cancel() {
            let _ = unload(self.backend());
            self.set_selected(None);
            return Err(AppError::cancelled());
        }
        self.set_selected(Some(checksum));
        Ok(())
    }

    fn finish_generate(&self, result: Result<String, AppError>) -> Result<String, AppError> {
        if self.take_cancel() {
            return Err(AppError::cancelled());
        }
        result
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPreviewDto {
    pub size: u64,
    pub sha256: String,
    pub license: String,
    pub runtime_compat: String,
    pub location: String,
    pub compatibility: CompatibilityReport,
}

impl From<ModelPreview> for ModelPreviewDto {
    fn from(facts: ModelPreview) -> Self {
        Self {
            size: facts.size,
            sha256: facts.sha256,
            license: facts.license,
            runtime_compat: facts.runtime_compat,
            location: facts.location.to_string_lossy().into_owned(),
            compatibility: facts.compatibility,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelManifestDto {
    pub schema_version: u32,
    pub size: u64,
    pub license: String,
    pub runtime_compat: String,
    pub checksum: String,
}

impl From<ModelManifest> for ModelManifestDto {
    fn from(manifest: ModelManifest) -> Self {
        Self {
            schema_version: manifest.schema_version,
            size: manifest.size,
            license: manifest.license,
            runtime_compat: manifest.runtime_compat,
            checksum: manifest.checksum,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelHealthDto {
    pub ok: bool,
    pub backend_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSetupSnapshotDto {
    pub ready: Vec<ModelManifestDto>,
    pub backend_status: String,
    pub health: ModelHealthDto,
    pub selected_checksum: Option<String>,
    pub store_root: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRemoveResultDto {
    pub checksum: String,
    pub recovered_bytes: u64,
}

impl From<ModelRemoveResult> for ModelRemoveResultDto {
    fn from(removed: ModelRemoveResult) -> Self {
        Self {
            checksum: removed.checksum,
            recovered_bytes: removed.recovered_bytes,
        }
    }
}

fn store_root(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map_err(|err| AppError::io("Could not resolve the app data directory.", err))
}

fn join_err(err: impl std::fmt::Display) -> AppError {
    AppError::io("The local-model command was interrupted.", err)
}

fn filepath_to_string(fp: tauri_plugin_dialog::FilePath) -> Option<String> {
    fp.into_path().ok().map(|p| p.to_string_lossy().to_string())
}

fn status_label(status: BackendStatus) -> String {
    match status {
        BackendStatus::Unloaded => "Unloaded".to_string(),
        BackendStatus::Ready => "Ready".to_string(),
    }
}

/// Native picker with no PDF / GGUF filter. `null` if the user cancelled.
#[tauri::command]
pub async fn ai_pick_model_file(app: tauri::AppHandle) -> Result<Option<String>, AppError> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        // No add_filter: macOS treats ["*"] as a literal extension, so .txt stays grey.
        app.dialog().file().blocking_pick_file()
    })
    .await
    .map_err(join_err)?;

    Ok(picked.and_then(filepath_to_string))
}

/// Hash a chosen path. Does not write a blob.
#[tauri::command]
pub async fn ai_preview_model(app: tauri::AppHandle, path: String) -> Result<ModelPreviewDto, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        preview(&root, Path::new(&path)).map(ModelPreviewDto::from)
    })
    .await
    .map_err(join_err)?
}

/// Copy the chosen file into the store after the user confirms the preview hash.
#[tauri::command]
pub async fn ai_import_model(
    app: tauri::AppHandle,
    path: String,
    expected_sha256: String,
) -> Result<ModelManifestDto, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        import(&root, Path::new(&path), &expected_sha256).map(ModelManifestDto::from)
    })
    .await
    .map_err(join_err)?
}

/// Ready manifests on disk plus the current Fake status. Does not auto-load.
#[tauri::command]
pub async fn ai_list_models(
    app: tauri::AppHandle,
    session: tauri::State<'_, Arc<AiSession>>,
) -> Result<ModelSetupSnapshotDto, AppError> {
    let session = Arc::clone(&session);
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let snap = snapshot(&root, session.backend())?;
        let health = session.backend().health();
        Ok(ModelSetupSnapshotDto {
            ready: snap.ready.into_iter().map(ModelManifestDto::from).collect(),
            backend_status: status_label(snap.backend_status),
            health: ModelHealthDto {
                ok: health.ok,
                backend_id: health.backend_id.to_string(),
            },
            selected_checksum: session.selected(),
            store_root: root.to_string_lossy().into_owned(),
        })
    })
    .await
    .map_err(join_err)?
}

/// Check local hardware and resources against caller-supplied model
/// requirements. Unavailable system information is reported in the result;
/// the check itself does not fail and never touches the PDF tools.
#[tauri::command]
pub async fn ai_check_compatibility(
    app: tauri::AppHandle,
    requirements: ModelRequirements,
) -> Result<CompatibilityReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || -> Result<CompatibilityReport, AppError> {
        // The store root only selects the volume to inspect. If app data is
        // unavailable, the report still explains that free space is unknown.
        let root = store_root(&app).ok();
        Ok(evaluate(&requirements, &probe(root.as_deref())))
    })
    .await
    .map_err(join_err)?
}

/// Load a ready checksum into the Fake. Checksum is remembered in process memory.
///
/// A cancel that arrives while the blocking load is in flight wins: the backend
/// is unloaded again and the command returns `CANCELLED`.
#[tauri::command]
pub async fn ai_load_model(
    app: tauri::AppHandle,
    session: tauri::State<'_, Arc<AiSession>>,
    checksum: String,
) -> Result<(), AppError> {
    let session = Arc::clone(&session);
    // Clear the previous operation before queuing work. A quick Cancel must not
    // be erased when spawn_blocking eventually starts the worker.
    session.clear_cancel();
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        load(&root, session.backend(), &checksum)?;
        session.finish_load(checksum)
    })
    .await
    .map_err(join_err)?
}

/// Unload the Fake and clear the selected checksum.
#[tauri::command]
pub async fn ai_unload_model(
    session: tauri::State<'_, Arc<AiSession>>,
) -> Result<(), AppError> {
    let session = Arc::clone(&session);
    tauri::async_runtime::spawn_blocking(move || {
        unload(session.backend())?;
        session.set_selected(None);
        Ok(())
    })
    .await
    .map_err(join_err)?
}

/// Cooperative cancel of an in-flight load or generate.
#[tauri::command]
pub fn ai_cancel(session: tauri::State<'_, Arc<AiSession>>) -> Result<(), AppError> {
    session.request_cancel();
    cancel(session.backend());
    Ok(())
}

/// Generate from a prompt. No PDF path.
#[tauri::command]
pub async fn ai_generate(
    session: tauri::State<'_, Arc<AiSession>>,
    prompt: String,
) -> Result<String, AppError> {
    let session = Arc::clone(&session);
    session.clear_cancel();
    tauri::async_runtime::spawn_blocking(move || {
        session.finish_generate(generate(session.backend(), &prompt))
    })
    .await
    .map_err(join_err)?
}

/// Delete one imported model and report recovered bytes.
#[tauri::command]
pub async fn ai_remove_model(
    app: tauri::AppHandle,
    session: tauri::State<'_, Arc<AiSession>>,
    checksum: String,
) -> Result<ModelRemoveResultDto, AppError> {
    let session = Arc::clone(&session);
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let removed = remove(&root, &checksum)?;
        if session.selected().as_deref() == Some(checksum.as_str()) {
            let _ = unload(session.backend());
            session.set_selected(None);
        }
        Ok(ModelRemoveResultDto::from(removed))
    })
    .await
    .map_err(join_err)?
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc};

    use super::{cancel, generate, AiSession};

    #[test]
    fn cancel_while_load_waits_for_worker_is_not_lost() {
        let session = Arc::new(AiSession::new());
        session.backend().load().unwrap();
        session.set_selected(Some("previous-model".to_string()));
        let (start_tx, start_rx) = mpsc::channel();
        session.clear_cancel();

        let worker_session = Arc::clone(&session);
        let worker = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            worker_session.backend().load().unwrap();
            worker_session.finish_load("queued-model".to_string())
        });

        session.request_cancel();
        cancel(session.backend());
        start_tx.send(()).unwrap();

        let err = worker
            .join()
            .unwrap()
            .expect_err("queued load must be cancelled");
        assert_eq!(err.code, "CANCELLED");
        assert_eq!(
            session.backend().status(),
            crate::ai::BackendStatus::Unloaded
        );
        assert_eq!(session.selected(), None);
    }

    #[test]
    fn cancel_while_generate_waits_for_worker_is_not_lost() {
        let session = Arc::new(AiSession::new());
        session.backend().load().unwrap();
        let (start_tx, start_rx) = mpsc::channel();
        session.clear_cancel();

        let worker_session = Arc::clone(&session);
        let worker = std::thread::spawn(move || {
            start_rx.recv().unwrap();
            worker_session.finish_generate(generate(worker_session.backend(), "health check"))
        });

        session.request_cancel();
        cancel(session.backend());
        start_tx.send(()).unwrap();

        let err = worker
            .join()
            .unwrap()
            .expect_err("queued inference must be cancelled");
        assert_eq!(err.code, "CANCELLED");
    }
}
