//! Shared data models exchanged with the frontend and the job registry.
//!
//! Every serialized struct uses `camelCase` so the TypeScript types in
//! `src/lib/types.ts` map 1:1 without any manual conversion.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Metadata about a single file on disk. `pageCount`/`isValidPdf` are best
/// effort: if the engine cannot read the file they fall back to None/false.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub page_count: Option<u32>,
    pub is_valid_pdf: bool,
}

/// One entry in a PDF's bookmarks/outline tree (flattened with a depth level).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineItem {
    pub title: String,
    /// 1-based page within the source file, or None if it couldn't be resolved.
    pub page: Option<u32>,
    pub level: u32,
}

/// Result of a visual page comparison: a diff-overlay image (base64 data URL)
/// plus the fraction of pixels that changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub data_url: String,
    pub changed_percent: f32,
}

/// Result of a free-disk-space check against the volume of a given path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpaceInfo {
    pub path: String,
    pub available_bytes: u64,
    pub required_bytes: u64,
    pub sufficient: bool,
}

/// Returned when a job finishes successfully.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub job_id: String,
    /// One or more produced files (split can produce many).
    pub output_paths: Vec<String>,
    pub status: String,
}

/// Live job state pushed to the frontend over the `job:update` event.
/// `percent` of `None` means the UI should render an indeterminate bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUpdate {
    pub job_id: String,
    /// idle | preparing | running | completed | failed | cancelled
    pub state: String,
    /// Human-readable current step, e.g. "Merging 4 files".
    pub step: String,
    pub percent: Option<f32>,
    pub message: Option<String>,
}

impl JobUpdate {
    pub fn new(job_id: &str, state: &str, step: &str) -> Self {
        Self {
            job_id: job_id.to_string(),
            state: state.to_string(),
            step: step.to_string(),
            percent: None,
            message: None,
        }
    }
    pub fn percent(mut self, p: f32) -> Self {
        self.percent = Some(p);
        self
    }
    #[allow(dead_code)] // Part of the JobUpdate builder API.
    pub fn message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }
}

/// One file + an ordered qpdf page spec, for assembling pages across multiple
/// source files (powers cross-document reorder / delete / extract).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGroup {
    pub path: String,
    /// qpdf page selection in the desired order, e.g. "2,5,1" or "1-3".
    pub pages: String,
}

/// A rotation to apply to specific output pages, e.g. angle 90 for pages "1,3".
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RotateGroup {
    pub angle: i32,
    /// Output (assembled-document) page numbers, e.g. "1,3,5-8".
    pub pages: String,
}

/// A single page of a single source file (an ordered list of these is the
/// combined working document used by compress/split across files).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePick {
    pub path: String,
    pub page: u32,
}

/// A rendered page preview: page number + a base64 PNG data URL. Only tiny
/// thumbnails cross IPC this way — never the source PDF bytes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedThumb {
    pub page: u32,
    pub data_url: String,
}

/// An inclusive page range (1-based, over the combined document).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RangePair {
    pub start: u32,
    pub end: u32,
}

/// Split mode, tagged union matching the TS `SplitMode`.
///
/// ```ts
/// type SplitMode =
///   | { type: "everyN"; n: number }
///   | { type: "ranges"; ranges: { start: number; end: number }[] };
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SplitMode {
    /// Split into chunks of `n` pages, producing multiple output files.
    EveryN { n: u32 },
    /// One output file per range.
    Ranges { ranges: Vec<RangePair> },
}

// ---------------------------------------------------------------------------
// Job registry: tracks running child processes so jobs can be cancelled.
// ---------------------------------------------------------------------------

/// A cancellable handle to a running engine process.
///
/// The worker thread reads the process's stderr through a separately-owned pipe
/// handle, so the `Child` sits behind `child` purely so `cancel_job` can
/// `kill()` it. `cancelled` lets the worker distinguish a user cancel from a
/// genuine engine failure once the process exits.
///
/// Usage in the engine worker (inside `spawn_blocking`):
///   1. spawn the child, `take()` its stderr pipe;
///   2. `*handle.child.lock().unwrap() = Some(child);`
///   3. read stderr to EOF (returns when the process exits or is killed);
///   4. `let child = handle.child.lock().unwrap().take();` then `child.wait()`;
///   5. if `handle.is_cancelled()` -> `AppError::cancelled()`, else inspect status.
pub struct JobHandle {
    pub child: Mutex<Option<Child>>,
    pub cancelled: AtomicBool,
}

impl JobHandle {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            child: Mutex::new(None),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Mark cancelled and kill the child if it is currently running. The worker
    /// still owns reaping (`wait`) the process after its pipes close.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

/// Maps `job_id -> JobHandle`. Managed by Tauri via `app.manage(...)`.
#[derive(Default)]
pub struct JobRegistry {
    pub jobs: Mutex<HashMap<String, Arc<JobHandle>>>,
}

impl JobRegistry {
    /// Register a job slot and return its shared handle.
    pub fn register(&self, job_id: &str) -> Arc<JobHandle> {
        let handle = JobHandle::new();
        self.jobs
            .lock()
            .expect("job registry poisoned")
            .insert(job_id.to_string(), handle.clone());
        handle
    }

    /// Remove a job slot once it has finished (success, failure or cancel).
    pub fn remove(&self, job_id: &str) {
        self.jobs
            .lock()
            .expect("job registry poisoned")
            .remove(job_id);
    }

    /// Look up a job's handle (used by `cancel_job`).
    pub fn get(&self, job_id: &str) -> Option<Arc<JobHandle>> {
        self.jobs
            .lock()
            .expect("job registry poisoned")
            .get(job_id)
            .cloned()
    }
}
