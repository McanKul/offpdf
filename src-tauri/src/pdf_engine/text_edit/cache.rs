//! Cache and concurrency (SPEC §B.19): the snapshots of the files open in Edit text, their page
//! models and their background `qpdf --check`, shared by the inspect and preview commands.
//!
//! - Keyed by canonical path; an entry is reused only while `stat_matches` holds (length, mtime
//!   and the first/last 64 KiB unchanged), otherwise the file is read again.
//! - At most `CACHE_SNAPSHOTS_MAX` snapshots are kept, only of files of at most
//!   `CACHE_SNAPSHOT_BYTES_MAX` bytes. A larger file is not kept between calls, but calls that
//!   overlap share one read of it, and only one read of a file runs at a time (review-T5 M3).
//!   Each source keeps at most `CACHE_PAGE_MODELS_MAX` page models and at most
//!   `CACHE_SNAPSHOT_BYTES_MAX` bytes of them (each model's `walk.model_bytes`: everything it
//!   keeps, its font models included).
//! - Every source gets `<temp>/textedit/<fingerprint>/` with `source.pdf`, written once from the
//!   snapshot bytes (temp name + rename): qpdf never reads the user's file. The page-map agreement
//!   check (`qpdf --json` vs lopdf) runs there before anything is inspected; `qpdf --check` runs
//!   in the background and only the preview waits for it (Save reuses its memoised result).
//! - The mutex is held only to look up or swap `Arc`s; reads, parses and subprocesses run
//!   outside it. Evicting or releasing a source deletes its folder unless another open source has
//!   the same bytes; a folder whose background check is still reading `source.pdf` is deleted by
//!   a later call. A running preview leases its folder: a release or an eviction during it
//!   deletes the folder when the preview ends, not before (review-T5 M2); folders left by a crash
//!   are deleted at startup (`clear_stale_folders`). Save never uses this cache.

use crate::error::AppError;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::{qpdf_page_map, Engines, PendingCheck, RunOpts};
use crate::pdf_engine::text_edit::limits::{
    CACHE_PAGE_MODELS_MAX, CACHE_SNAPSHOTS_MAX, CACHE_SNAPSHOT_BYTES_MAX, CHECK_MEMO_MAX,
    PREVIEW_SHARED_MODEL_MAX,
};
use crate::pdf_engine::text_edit::preview::{cache_dir_for, write_once};
use crate::pdf_engine::text_edit::reasons;
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::snapshot::{
    check_page_map, read_snapshot, stat_matches, Fingerprint, SourceSnapshot,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

/// Byte budget of the page models one source keeps.
const PAGE_MODEL_BYTES_MAX: usize = CACHE_SNAPSHOT_BYTES_MAX as usize;

/// One open source: its snapshot context, page models and temporary folder.
pub struct CachedSource {
    pub ctx: SnapshotContext,
    /// `<temp>/textedit/<fingerprint>/`.
    pub dir: PathBuf,
    pages: Mutex<LruPages>,
}

#[derive(Default)]
struct LruPages {
    /// (page index, model, approximate bytes), most recently used first.
    entries: VecDeque<(u32, Arc<PageModel>, usize)>,
    bytes: usize,
}

impl LruPages {
    fn get(&mut self, page: u32) -> Option<Arc<PageModel>> {
        let pos = self.entries.iter().position(|(p, ..)| *p == page)?;
        let entry = self.entries.remove(pos)?;
        let model = Arc::clone(&entry.1);
        self.entries.push_front(entry);
        Some(model)
    }

    /// Keeps `model` unless it alone exceeds the budget; a model another call stored meanwhile
    /// wins (both were built from the same bytes).
    fn insert(&mut self, page: u32, model: Arc<PageModel>) -> Arc<PageModel> {
        if let Some(existing) = self.get(page) {
            return existing;
        }
        let bytes = model.walk.model_bytes;
        if bytes > PAGE_MODEL_BYTES_MAX {
            return model;
        }
        self.entries.push_front((page, Arc::clone(&model), bytes));
        self.bytes = self.bytes.saturating_add(bytes);
        while self.entries.len() > CACHE_PAGE_MODELS_MAX || self.bytes > PAGE_MODEL_BYTES_MAX {
            match self.entries.pop_back() {
                Some((_, _, b)) => self.bytes = self.bytes.saturating_sub(b),
                None => break,
            }
        }
        model
    }

    /// Removes `page`'s model, if cached.
    fn take(&mut self, page: u32) -> Option<Arc<PageModel>> {
        let pos = self.entries.iter().position(|(p, ..)| *p == page)?;
        let (_, model, bytes) = self.entries.remove(pos)?;
        self.bytes = self.bytes.saturating_sub(bytes);
        Some(model)
    }
}

struct Entry {
    key: PathBuf,
    src: Arc<CachedSource>,
}

/// An open source too large to cache: its folder stays until release; the source itself lives
/// only while a call holds it (`src`), and calls that overlap share it.
struct Pinned {
    key: PathBuf,
    dir: PathBuf,
    src: Weak<CachedSource>,
}

#[derive(Default)]
struct CacheInner {
    /// Cached snapshots, most recently used first.
    sources: VecDeque<Entry>,
    pinned: Vec<Pinned>,
    /// Folders a running preview writes into, with the number of previews.
    leases: Vec<(PathBuf, usize)>,
    /// One load lock per key being read (single flight).
    loading: Vec<(PathBuf, Arc<Mutex<()>>)>,
    /// Background checks by fingerprint and folder (shared by every open of the same bytes).
    checks: VecDeque<(Fingerprint, PathBuf, PendingCheck)>,
    /// Fingerprints whose page map lopdf and qpdf agree on.
    maps_ok: VecDeque<Fingerprint>,
    /// Unused folders whose background check was still reading `source.pdf`.
    deferred: Vec<PathBuf>,
    #[cfg(test)]
    seams: seams::CacheSeams,
}

impl CacheInner {
    fn in_use(&self, dir: &Path) -> bool {
        self.sources.iter().any(|e| e.src.dir == dir)
            || self.pinned.iter().any(|p| p.dir == dir)
            || self.leases.iter().any(|(d, _)| d == dir)
    }

    /// The source of `key` (cached, or pinned and held by a running call).
    fn lookup(&self, key: &Path) -> Option<Arc<CachedSource>> {
        let cached = self.sources.iter().find(|e| e.key == key);
        cached.map(|e| Arc::clone(&e.src)).or_else(|| {
            self.pinned
                .iter()
                .find(|p| p.key == key)
                .and_then(|p| p.src.upgrade())
        })
    }

    /// Whether a file of `len` bytes is kept between calls.
    fn cacheable(&self, len: u64) -> bool {
        #[cfg(test)]
        if let Some(max) = self.seams.snapshot_bytes_max {
            return len <= max;
        }
        len <= CACHE_SNAPSHOT_BYTES_MAX
    }

    fn busy(&self, dir: &Path) -> bool {
        self.checks
            .iter()
            .any(|(_, d, c)| d == dir && c.peek().is_none())
    }

    /// The folders to delete now: `candidates` and earlier deferred folders that no open source
    /// uses; busy ones are deferred to a later call.
    fn collect(&mut self, candidates: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut all = std::mem::take(&mut self.deferred);
        all.extend(candidates);
        all.sort();
        all.dedup();
        let mut now = Vec::new();
        for dir in all {
            if self.in_use(&dir) {
                continue;
            }
            if self.busy(&dir) {
                self.deferred.push(dir);
            } else {
                now.push(dir);
            }
        }
        now
    }

    /// Removes the entries of `key`, returning their folders.
    fn forget(&mut self, key: &Path) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        self.sources.retain(|e| {
            let keep = e.key != key;
            if !keep {
                dirs.push(e.src.dir.clone());
            }
            keep
        });
        self.pinned.retain(|p| {
            let keep = p.key != key;
            if !keep {
                dirs.push(p.dir.clone());
            }
            keep
        });
        dirs
    }
}

/// Keeps a source's folder while a preview writes into it (`TextEditCache::lease`).
pub struct DirLease {
    cache: TextEditCache,
    dir: PathBuf,
}

impl Drop for DirLease {
    /// The last lease of a folder that no open source owns any more deletes it (or defers it while
    /// its check runs).
    fn drop(&mut self) {
        let doomed = {
            let mut inner = self.cache.lock();
            if let Some(pos) = inner.leases.iter().position(|(d, _)| *d == self.dir) {
                let left = inner
                    .leases
                    .get(pos)
                    .map_or(0, |(_, n)| n.saturating_sub(1));
                match inner.leases.get_mut(pos) {
                    Some(lease) if left > 0 => lease.1 = left,
                    _ => {
                        inner.leases.remove(pos);
                    }
                }
            }
            inner.collect(vec![self.dir.clone()])
        };
        remove_dirs(&doomed);
    }
}

/// Deletes `<temp_root>/textedit/` (startup: no source is open yet, so every copy there was left
/// by a crash or an interrupted call).
pub fn clear_stale_folders(temp_root: &Path) {
    remove_dirs(&[temp_root.join("textedit")]);
}

/// Snapshots of the files open in Edit text (managed Tauri state).
#[derive(Clone, Default)]
pub struct TextEditCache {
    inner: Arc<Mutex<CacheInner>>,
}

fn key_of(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The file name shown in `STALE` messages.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn remove_dirs(dirs: &[PathBuf]) {
    for dir in dirs {
        if let Err(e) = std::fs::remove_dir_all(dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "offpdf: could not remove the text-edit folder {}: {e}",
                    dir.display()
                );
            }
        }
    }
}

impl TextEditCache {
    fn lock(&self) -> MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The source at `path`, read again when `stat_matches` fails: `read_snapshot` → write
    /// `source.pdf` once → page-map agreement (synchronous; a disagreement is `PDF_NEEDS_REPAIR`)
    /// → background `qpdf --check` (memoised by fingerprint). Returns before the check finishes.
    pub fn open(
        &self,
        path: &Path,
        temp_root: &Path,
        engines: &Engines,
    ) -> Result<Arc<CachedSource>, AppError> {
        let key = key_of(path);
        if let Some(src) = self.current(&key) {
            return Ok(src);
        }
        // Single flight: one read of a file at a time; a read that finished meanwhile is shared.
        let gate = {
            let mut inner = self.lock();
            match inner.loading.iter().find(|(k, _)| *k == key) {
                Some((_, g)) => Arc::clone(g),
                None => {
                    let g = Arc::new(Mutex::new(()));
                    inner.loading.push((key.clone(), Arc::clone(&g)));
                    g
                }
            }
        };
        let loaded = {
            let _one = gate.lock().unwrap_or_else(|e| e.into_inner());
            match self.current(&key) {
                Some(src) => Ok(src),
                None => self.load(path, key.clone(), temp_root, engines),
            }
        };
        let mut inner = self.lock();
        // The map's copy and ours: nobody else waits on this key.
        if Arc::strong_count(&gate) <= 2 {
            inner.loading.retain(|(k, _)| *k != key);
        }
        loaded
    }

    /// The source of `key` when it is still the file on disk (`stat_matches`).
    fn current(&self, key: &Path) -> Option<Arc<CachedSource>> {
        let src = self.lock().lookup(key)?;
        if !stat_matches(&src.ctx.snap) {
            return None;
        }
        self.touch(key);
        Some(src)
    }

    /// Keeps `dir` (a source's folder) while the returned guard lives: a preview writes into it.
    pub fn lease(&self, dir: &Path) -> DirLease {
        let mut inner = self.lock();
        match inner.leases.iter_mut().find(|(d, _)| d == dir) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => inner.leases.push((dir.to_path_buf(), 1)),
        }
        DirLease {
            cache: self.clone(),
            dir: dir.to_path_buf(),
        }
    }

    /// `open`, then `STALE` unless the file still has `fingerprint`.
    pub fn get(
        &self,
        path: &Path,
        temp_root: &Path,
        fingerprint: &str,
        engines: &Engines,
    ) -> Result<Arc<CachedSource>, AppError> {
        let src = self.open(path, temp_root, engines)?;
        if src.ctx.snap.fingerprint.to_string() != fingerprint {
            return Err(reasons::stale(&file_name(path)));
        }
        Ok(src)
    }

    /// The model of `page_index` (`INVALID_PAGES` when the file has no such page).
    pub fn page(&self, src: &CachedSource, page_index: u32) -> Result<Arc<PageModel>, AppError> {
        let lock = || src.pages.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(model) = lock().get(page_index) {
            return Ok(model);
        }
        let model = Arc::new(build_page_model(&src.ctx, page_index, None)?);
        Ok(lock().insert(page_index, model))
    }

    /// The model of `page_index` for a call that may release it: a model over
    /// `PREVIEW_SHARED_MODEL_MAX` leaves the cache (built again on the next visit), so the caller
    /// holds the last reference and frees it when done (review-final MEDIUM-3).
    pub fn page_to_release(
        &self,
        src: &CachedSource,
        page_index: u32,
    ) -> Result<Arc<PageModel>, AppError> {
        let model = self.page(src, page_index)?;
        if model.walk.model_bytes > PREVIEW_SHARED_MODEL_MAX {
            let mut pages = src.pages.lock().unwrap_or_else(|e| e.into_inner());
            pages.take(page_index);
        }
        Ok(model)
    }

    /// The background `qpdf --check` of `src` (started at open). A check that ended in an error
    /// (timeout, missing qpdf) is started again; `source.pdf` is written again first when an
    /// eviction removed it.
    pub fn source_check(
        &self,
        src: &CachedSource,
        engines: &Engines,
    ) -> Result<PendingCheck, AppError> {
        let source = src.dir.join("source.pdf");
        std::fs::create_dir_all(&src.dir)
            .map_err(|e| AppError::io("OffPDF could not create a temporary folder.", e))?;
        write_once(&source, &src.ctx.snap.bytes)?;
        Ok(self.pending_check(engines, src.ctx.snap.fingerprint, &src.dir))
    }

    /// Forgets `path` and deletes its folder unless another open source has the same bytes
    /// (best effort; a failure is logged).
    pub fn release(&self, path: &Path) {
        let key = key_of(path);
        let doomed = {
            let mut inner = self.lock();
            let candidates = inner.forget(&key);
            inner.collect(candidates)
        };
        remove_dirs(&doomed);
    }

    fn touch(&self, key: &Path) {
        let mut inner = self.lock();
        if let Some(pos) = inner.sources.iter().position(|e| e.key == key) {
            if let Some(entry) = inner.sources.remove(pos) {
                inner.sources.push_front(entry);
            }
        }
    }

    fn load(
        &self,
        path: &Path,
        key: PathBuf,
        temp_root: &Path,
        engines: &Engines,
    ) -> Result<Arc<CachedSource>, AppError> {
        let snap = read_snapshot(path)?;
        #[cfg(test)]
        {
            self.lock().seams.loads += 1;
        }
        let fp = snap.fingerprint;
        let dir = cache_dir_for(temp_root, &fp.to_string());
        let prepared = std::fs::create_dir_all(&dir)
            .map_err(|e| AppError::io("OffPDF could not create a temporary folder.", e))
            .and_then(|()| write_once(&dir.join("source.pdf"), &snap.bytes))
            .and_then(|()| self.check_map(&snap, &dir, engines));
        if let Err(e) = prepared {
            let doomed = self.lock().collect(vec![dir]);
            remove_dirs(&doomed);
            return Err(e);
        }
        self.pending_check(engines, fp, &dir);
        let src = Arc::new(CachedSource {
            ctx: SnapshotContext::new(snap),
            dir,
            pages: Mutex::new(LruPages::default()),
        });
        let doomed = {
            let mut inner = self.lock();
            let mut candidates = inner.forget(&key);
            if inner.cacheable(fp.len) {
                inner.sources.push_front(Entry {
                    key,
                    src: Arc::clone(&src),
                });
                while inner.sources.len() > CACHE_SNAPSHOTS_MAX {
                    if let Some(evicted) = inner.sources.pop_back() {
                        candidates.push(evicted.src.dir.clone());
                    }
                }
            } else {
                inner.pinned.push(Pinned {
                    key,
                    dir: src.dir.clone(),
                    src: Arc::downgrade(&src),
                });
            }
            inner.collect(candidates)
        };
        remove_dirs(&doomed);
        Ok(src)
    }

    /// Page-map agreement of the snapshot with qpdf's reading of `dir/source.pdf` (once per
    /// fingerprint: the answer depends only on the bytes).
    fn check_map(
        &self,
        snap: &SourceSnapshot,
        dir: &Path,
        engines: &Engines,
    ) -> Result<(), AppError> {
        let fp = snap.fingerprint;
        if self.lock().maps_ok.contains(&fp) {
            return Ok(());
        }
        let pages = qpdf_page_map(engines, &dir.join("source.pdf"), &RunOpts::default())?;
        check_page_map(snap, &pages)?;
        let mut inner = self.lock();
        if !inner.maps_ok.contains(&fp) {
            inner.maps_ok.push_front(fp);
            inner.maps_ok.truncate(CHECK_MEMO_MAX);
        }
        Ok(())
    }

    /// The background `qpdf --check` of `dir/source.pdf`, started once per fingerprint and
    /// folder (again after an error).
    fn pending_check(&self, engines: &Engines, fp: Fingerprint, dir: &Path) -> PendingCheck {
        let mut inner = self.lock();
        if let Some((_, _, check)) = inner.checks.iter().find(|(f, d, _)| *f == fp && d == dir) {
            if !matches!(check.peek(), Some(Err(_))) {
                return check.clone();
            }
        }
        inner.checks.retain(|(f, d, _)| *f != fp || d != dir);
        #[cfg(test)]
        let engines = &seams::background_engines(engines);
        let check = PendingCheck::spawn(engines.clone(), fp, dir.join("source.pdf"));
        inner
            .checks
            .push_front((fp, dir.to_path_buf(), check.clone()));
        // Only finished checks are dropped: a running one must stay visible to `busy`.
        while inner.checks.len() > CHECK_MEMO_MAX {
            match inner
                .checks
                .iter()
                .rposition(|(_, _, c)| c.peek().is_some())
            {
                Some(i) => {
                    inner.checks.remove(i);
                }
                None => break,
            }
        }
        check
    }

    /// Number of cached snapshots (tests).
    #[cfg(test)]
    pub(crate) fn cached_len(&self) -> usize {
        self.lock().sources.len()
    }

    /// Waits until every background check this cache started has finished (tests delete their
    /// folders afterwards; a check whose input vanished must never be what a test observes).
    #[cfg(test)]
    pub(crate) fn wait_checks(&self) {
        let checks: Vec<PendingCheck> = self.lock().checks.iter().map(|c| c.2.clone()).collect();
        for check in checks {
            let _ = check.wait(None);
        }
    }

    /// Bytes the page models of `src` are charged in its LRU (tests).
    #[cfg(test)]
    pub(crate) fn page_model_bytes(&self, src: &CachedSource) -> usize {
        src.pages.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }

    /// Background checks still running (tests).
    #[cfg(test)]
    pub(crate) fn running_checks(&self) -> usize {
        let inner = self.lock();
        inner.checks.iter().filter(|c| c.2.peek().is_none()).count()
    }

    /// Test knobs of this cache only (other tests' caches are unaffected).
    #[cfg(test)]
    pub(crate) fn with_seams<T>(&self, f: impl FnOnce(&mut seams::CacheSeams) -> T) -> T {
        f(&mut self.lock().seams)
    }
}

#[cfg(test)]
pub(crate) mod seams {
    use crate::pdf_engine::text_edit::engines::Engines;
    use std::cell::RefCell;
    use std::path::PathBuf;

    #[derive(Default)]
    pub(crate) struct CacheSeams {
        /// Replaces `CACHE_SNAPSHOT_BYTES_MAX` for this cache.
        pub snapshot_bytes_max: Option<u64>,
        /// Number of file reads (`read_snapshot`) this cache made.
        pub loads: usize,
    }

    thread_local! {
        static BACKGROUND_QPDF: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// The qpdf the background checks started on this thread run (CMD-08: a check held until
    /// the test lets it go).
    pub(crate) fn set_background_qpdf(qpdf: Option<PathBuf>) {
        BACKGROUND_QPDF.with(|q| *q.borrow_mut() = qpdf);
    }

    pub(super) fn background_engines(engines: &Engines) -> Engines {
        let mut out = engines.clone();
        if let Some(q) = BACKGROUND_QPDF.with(|q| q.borrow().clone()) {
            out.qpdf = q;
        }
        out
    }
}
