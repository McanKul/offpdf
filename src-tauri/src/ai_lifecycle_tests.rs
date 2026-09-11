//! Unit tests for `crate::ai::lifecycle` (issue #84).
//!
//! Locked Approach A names: `preview`, `import`, `load`, `unload`,
//! `cancel`, `generate`, `remove`, `snapshot`. Injected store root +
//! `crate::ai::fake()`. Tiny fixture only — no GGUF.
//! Do not edit `ai_tests.rs`, `ai_store_tests.rs`, `store.rs`, or `fake.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::ai::lifecycle::{cancel, generate, import, load, preview, remove, snapshot, unload};
use crate::ai::store::ModelStore;
use crate::ai::BackendStatus;

const FIXTURE: &[u8] = b"offpdf-store-fixture-v1";
const FIXTURE_SHA256: &str = "9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e";
const MISSING_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const HARDWARE_STUB: &str = "Hardware check is not available in this version.";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ai_src_dir() -> PathBuf {
    manifest_dir().join("src/ai")
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        if path.is_dir() {
            walk_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn file_count(dir: &Path) -> usize {
    let mut n = 0usize;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            n += file_count(&path);
        } else {
            n += 1;
        }
    }
    n
}

fn is_under(child: &Path, root: &Path) -> bool {
    let child = match child.canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    let root = match root.canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    child.starts_with(root)
}

fn assert_ai_model_error(err: &crate::error::AppError, must_id: &str) {
    assert!(
        err.code.starts_with("AI_MODEL_"),
        "{must_id}: AppError.code must be AI_MODEL_*, got {}",
        err.code
    );
    assert_ne!(
        err.code, "ENGINE_FAILED",
        "{must_id}: must not reuse the PDF-engine code"
    );
    assert_ne!(
        err.code, "AI_FAILED",
        "{must_id}: must not reuse the Fake generate-fail code"
    );
    assert!(
        !err.title.is_empty(),
        "{must_id}: AppError.title must be non-empty"
    );
    assert!(
        !err.message.is_empty(),
        "{must_id}: AppError.message must be non-empty"
    );
}

fn assert_fake_load_has_no_path(must_id: &str) {
    let path = ai_src_dir().join("fake.rs");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{must_id}: read {}: {err}", path.display()));
    let mut rest = src.as_str();
    let mut saw = false;
    let mut bad = Vec::new();
    while let Some(idx) = rest.find("fn load") {
        let after = &rest[idx..];
        let end = after
            .find('{')
            .or_else(|| after.find(';'))
            .unwrap_or(after.len());
        let sig = &after[..end];
        saw = true;
        for token in ["PathBuf", "Path", "path:"] {
            if sig.contains(token) {
                bad.push(format!("{token} in `{sig}`"));
            }
        }
        rest = &after["fn load".len()..];
    }
    assert!(
        saw,
        "{must_id}: Fake load signature must still exist in src/ai/fake.rs"
    );
    assert!(
        bad.is_empty(),
        "{must_id}: Fake load must stay pathless; found {bad:?}"
    );
}

fn write_fixture(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, FIXTURE).unwrap_or_else(|err| {
        panic!(
            "write fixture {} ({} bytes): {err}",
            path.display(),
            FIXTURE.len()
        )
    });
    path
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-lifecycle-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// setup-facts-before-import — preview returns size / sha256 / license
/// `"imported"` / `runtime_compat` `"any"` / location / compat stub and
/// does not write a ready blob.
#[test]
fn lifecycle_preview_facts_before_import() {
    let root = Scratch::new("preview");
    let src_dir = Scratch::new("preview-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let store = ModelStore::open(root.path()).unwrap_or_else(|err| {
        panic!("setup-facts-before-import: open injected root: {err}")
    });
    let blobs_before = file_count(&root.path().join("blobs"));

    let facts = preview(root.path(), &src).unwrap_or_else(|err| {
        panic!("setup-facts-before-import: preview(path) must succeed: {err}")
    });
    assert_eq!(
        facts.size,
        FIXTURE.len() as u64,
        "setup-facts-before-import: size must be the source file length"
    );
    assert_eq!(
        facts.sha256, FIXTURE_SHA256,
        "setup-facts-before-import: sha256 must be the SHA-256 of the source file"
    );
    assert_eq!(
        facts.license, "imported",
        "setup-facts-before-import: license placeholder must be \"imported\""
    );
    assert_eq!(
        facts.runtime_compat, "any",
        "setup-facts-before-import: runtime_compat stub must be \"any\""
    );
    assert!(
        facts.location.starts_with(root.path())
            || facts.location.to_string_lossy().contains("blobs"),
        "setup-facts-before-import: location must name the injected store root or blobs/, got {}",
        facts.location.display()
    );
    assert!(
        facts.compatibility.contains(HARDWARE_STUB),
        "setup-facts-before-import: compatibility must include the #82 stub, got {:?}",
        facts.compatibility
    );

    let ready = store.list_ready().unwrap_or_else(|err| {
        panic!("setup-facts-before-import: list_ready after preview: {err}")
    });
    assert!(
        ready.is_empty(),
        "setup-facts-before-import: preview must not import; list_ready was {}, not []",
        ready.len()
    );
    assert_eq!(
        file_count(&root.path().join("blobs")),
        blobs_before,
        "setup-facts-before-import: preview must not create blobs"
    );
    assert!(
        src.exists(),
        "setup-facts-before-import: preview must not move the source file"
    );
}

/// setup-import-explicit-only — empty root lists []; import is a second
/// explicit call with path + expected sha; then one ready entry.
#[test]
fn lifecycle_import_explicit_only() {
    let root = Scratch::new("import-explicit");
    let src_dir = Scratch::new("import-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let backend = crate::ai::fake();

    let store = ModelStore::open(root.path()).unwrap_or_else(|err| {
        panic!("setup-import-explicit-only: open empty root: {err}")
    });
    let snap = snapshot(root.path(), &backend).unwrap_or_else(|err| {
        panic!("setup-import-explicit-only: snapshot on empty root: {err}")
    });
    assert!(
        snap.ready.is_empty(),
        "setup-import-explicit-only: empty root snapshot.ready must be [], got {} entries",
        snap.ready.len()
    );
    assert!(
        store.list_ready().unwrap().is_empty(),
        "setup-import-explicit-only: ModelStore list_ready on empty root must be []"
    );

    let facts = preview(root.path(), &src).unwrap_or_else(|err| {
        panic!("setup-import-explicit-only: preview before import: {err}")
    });
    assert!(
        store.list_ready().unwrap().is_empty(),
        "setup-import-explicit-only: preview still must not import"
    );

    let imported = import(root.path(), &src, &facts.sha256).unwrap_or_else(|err| {
        panic!("setup-import-explicit-only: explicit import(path, expected sha): {err}")
    });
    assert_eq!(imported.checksum, FIXTURE_SHA256);
    assert_eq!(imported.license, "imported");
    assert_eq!(imported.runtime_compat, "any");
    assert!(
        src.exists(),
        "setup-import-explicit-only: import copies, it must not move the source"
    );

    let ready = store.list_ready().unwrap_or_else(|err| {
        panic!("setup-import-explicit-only: list_ready after import: {err}")
    });
    assert_eq!(
        ready.len(),
        1,
        "setup-import-explicit-only: explicit import must be the only ready entry"
    );
    let loadable = store
        .loadable_path(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-import-explicit-only: loadable_path: {err}"));
    assert!(
        is_under(&loadable, root.path()),
        "setup-import-explicit-only: loadable_path {} must be under the injected root {}",
        loadable.display(),
        root.path().display()
    );
}

/// setup-load-ready-then-fake — missing checksum is AI_MODEL_*; ready
/// checksum pathless-loads Fake to Ready.
#[test]
fn lifecycle_load_ready_then_fake() {
    let root = Scratch::new("load");
    let src_dir = Scratch::new("load-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let backend = crate::ai::fake();

    let err = load(root.path(), &backend, MISSING_SHA256).expect_err(
        "setup-load-ready-then-fake: load of an unknown checksum must be AppError, not Ok",
    );
    assert_ai_model_error(&err, "setup-load-ready-then-fake");
    assert!(
        matches!(backend.status(), BackendStatus::Unloaded),
        "setup-load-ready-then-fake: failed load must leave Fake Unloaded"
    );

    import(root.path(), &src, FIXTURE_SHA256).unwrap_or_else(|err| {
        panic!("setup-load-ready-then-fake: import before load: {err}")
    });
    load(root.path(), &backend, FIXTURE_SHA256).unwrap_or_else(|err| {
        panic!("setup-load-ready-then-fake: load of a ready checksum: {err}")
    });
    assert!(
        matches!(backend.status(), BackendStatus::Ready),
        "setup-load-ready-then-fake: Fake status after load must be Ready"
    );
    assert_fake_load_has_no_path("setup-load-ready-then-fake");

    unload(&backend).unwrap_or_else(|err| {
        panic!("setup-load-ready-then-fake: unload must succeed: {err}")
    });
    assert!(
        matches!(backend.status(), BackendStatus::Unloaded),
        "setup-load-ready-then-fake: Fake status after unload must be Unloaded"
    );
}

/// setup-cancel-generate — in-flight generate("SLOW") + cancel → CANCELLED.
#[test]
fn lifecycle_cancel_generate() {
    let root = Scratch::new("cancel");
    let src_dir = Scratch::new("cancel-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let backend = Arc::new(crate::ai::fake());

    import(root.path(), &src, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-cancel-generate: import: {err}"));
    load(root.path(), backend.as_ref(), FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-cancel-generate: load: {err}"));

    let worker = {
        let backend = Arc::clone(&backend);
        std::thread::spawn(move || generate(backend.as_ref(), "SLOW"))
    };
    std::thread::sleep(Duration::from_millis(50));
    cancel(backend.as_ref());

    let result = worker
        .join()
        .expect("setup-cancel-generate: generate thread must not panic");
    let err = result.expect_err("setup-cancel-generate: cancelled SLOW must be Err");
    assert_eq!(
        err.code,
        crate::error::AppError::cancelled().code,
        "setup-cancel-generate: must reuse AppError::cancelled"
    );
    assert_eq!(
        err.code, "CANCELLED",
        "setup-cancel-generate: .code must be CANCELLED"
    );
    assert_ne!(
        err.code, "ENGINE_FAILED",
        "setup-cancel-generate: must not reuse ENGINE_FAILED"
    );
}

/// setup-cancel-no-latch — import+load, do not call `cancel`;
/// `generate("hello")` is Ok. Settings dismiss of preview must not
/// call `lifecycle::cancel`. Keep SLOW+cancel → CANCELLED in
/// `lifecycle_cancel_generate`.
#[test]
fn lifecycle_cancel_no_latch() {
    let root = Scratch::new("cancel-no-latch");
    let src_dir = Scratch::new("cancel-no-latch-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let backend = crate::ai::fake();

    import(root.path(), &src, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-cancel-no-latch: import: {err}"));
    load(root.path(), &backend, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-cancel-no-latch: load: {err}"));

    // Settings-style dismiss: drop preview only. Do not call cancel().
    let out = generate(&backend, "hello").unwrap_or_else(|err| {
        panic!(
            "setup-cancel-no-latch: generate(\"hello\") after dismiss-without-cancel must be Ok: {err}"
        )
    });
    assert!(
        !out.is_empty(),
        "setup-cancel-no-latch: generate(\"hello\") must return UTF-8"
    );
    assert_ne!(
        out, "CANCELLED",
        "setup-cancel-no-latch: generate must not look like a cancel error"
    );
}

/// setup-remove-reports-bytes — remove returns recovered_bytes >= size;
/// second remove is AI_MODEL_*.
#[test]
fn lifecycle_remove_reports_bytes() {
    let root = Scratch::new("remove");
    let src_dir = Scratch::new("remove-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let store = ModelStore::open(root.path())
        .unwrap_or_else(|err| panic!("setup-remove-reports-bytes: open: {err}"));

    import(root.path(), &src, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-remove-reports-bytes: import: {err}"));
    let size = store
        .get(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-remove-reports-bytes: get before remove: {err}"))
        .size;

    let removed = remove(root.path(), FIXTURE_SHA256).unwrap_or_else(|err| {
        panic!("setup-remove-reports-bytes: remove must succeed: {err}")
    });
    assert_eq!(
        removed.checksum, FIXTURE_SHA256,
        "setup-remove-reports-bytes: result.checksum must name the removed model"
    );
    assert!(
        removed.recovered_bytes >= size,
        "setup-remove-reports-bytes: recovered_bytes {} must be >= manifest.size {}",
        removed.recovered_bytes,
        size
    );
    assert!(
        store.list_ready().unwrap().is_empty(),
        "setup-remove-reports-bytes: list_ready must be empty after remove"
    );

    let err = remove(root.path(), FIXTURE_SHA256)
        .expect_err("setup-remove-reports-bytes: second remove must be AppError, not Ok");
    assert_ai_model_error(&err, "setup-remove-reports-bytes");
}

/// setup-restart-store-not-backend — reopen store keeps the ready entry;
/// a new Fake is Unloaded.
#[test]
fn lifecycle_restart_store_not_backend() {
    let root = Scratch::new("restart");
    let src_dir = Scratch::new("restart-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let backend = crate::ai::fake();

    import(root.path(), &src, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-restart-store-not-backend: import: {err}"));
    load(root.path(), &backend, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("setup-restart-store-not-backend: load: {err}"));
    assert!(
        matches!(backend.status(), BackendStatus::Ready),
        "setup-restart-store-not-backend: first process Fake must be Ready after load"
    );

    let store = ModelStore::open(root.path()).unwrap_or_else(|err| {
        panic!("setup-restart-store-not-backend: reopen store: {err}")
    });
    let fresh = crate::ai::fake();
    let snap = snapshot(root.path(), &fresh).unwrap_or_else(|err| {
        panic!("setup-restart-store-not-backend: snapshot after reopen: {err}")
    });
    assert_eq!(
        snap.ready.len(),
        1,
        "setup-restart-store-not-backend: reopened store must still list the ready entry"
    );
    assert_eq!(snap.ready[0].checksum, FIXTURE_SHA256);
    assert_eq!(
        store.list_ready().unwrap().len(),
        1,
        "setup-restart-store-not-backend: ModelStore::list_ready must still see the blob"
    );
    assert!(
        matches!(snap.backend_status, BackendStatus::Unloaded),
        "setup-restart-store-not-backend: snapshot backend_status must be Unloaded for a new Fake"
    );
    assert!(
        matches!(fresh.status(), BackendStatus::Unloaded),
        "setup-restart-store-not-backend: a new Fake must be Unloaded"
    );
}

/// setup-zero-models-tools-ok — empty root lists []; Fake load/generate
/// still works with zero blobs.
#[test]
fn lifecycle_zero_models_ok() {
    let root = Scratch::new("zero");
    let backend = crate::ai::fake();
    let snap = snapshot(root.path(), &backend).unwrap_or_else(|err| {
        panic!("setup-zero-models-tools-ok: snapshot on empty root: {err}")
    });
    assert!(
        snap.ready.is_empty(),
        "setup-zero-models-tools-ok: empty root must have no default model, got {} entries",
        snap.ready.len()
    );
    assert!(
        !root.path().ends_with("jobs"),
        "setup-zero-models-tools-ok: store root must not be <app_cache_dir>/jobs"
    );

    backend
        .load()
        .expect("setup-zero-models-tools-ok: Fake load must succeed with zero store blobs");
    let out = backend
        .generate("zero-models")
        .expect("setup-zero-models-tools-ok: Fake generate must work with zero store blobs");
    assert!(
        !out.is_empty(),
        "setup-zero-models-tools-ok: Fake generate must return UTF-8 with zero models"
    );
}

/// setup-no-http-no-gguf — new `ai/` files stay in the #81/#83 lock;
/// tests use the tiny in-memory fixture.
#[test]
fn lifecycle_no_http_no_gguf() {
    assert_eq!(
        FIXTURE, b"offpdf-store-fixture-v1",
        "setup-no-http-no-gguf: tests must reuse the tiny in-memory FIXTURE"
    );
    assert!(
        FIXTURE.len() < 64,
        "setup-no-http-no-gguf: FIXTURE must stay tiny, got {} bytes",
        FIXTURE.len()
    );

    let ai_dir = ai_src_dir();
    assert!(
        ai_dir.is_dir(),
        "setup-no-http-no-gguf: src-tauri/src/ai/ must exist so the source lock can scan it"
    );
    let lifecycle_rs = ai_dir.join("lifecycle.rs");
    let lifecycle_mod = ai_dir.join("lifecycle").join("mod.rs");
    assert!(
        lifecycle_rs.is_file() || lifecycle_mod.is_file(),
        "setup-no-http-no-gguf: src-tauri/src/ai/lifecycle.rs (or lifecycle/mod.rs) must exist"
    );

    let mut files = Vec::new();
    walk_files(&ai_dir, &mut files);
    let rust_files: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    assert!(
        !rust_files.is_empty(),
        "setup-no-http-no-gguf: src-tauri/src/ai/ must contain Rust sources to scan"
    );

    const FORBIDDEN: &[&str] = &[
        "reqwest",
        "ureq",
        "std::net",
        "std::process::Command",
        "XAI_API_KEY",
        "api.openai.com",
        "api.x.ai",
        "api.anthropic.com",
        "generativelanguage.googleapis.com",
        "api.groq.com",
    ];
    let mut hits = Vec::new();
    for path in &rust_files {
        let src = std::fs::read_to_string(path).unwrap_or_else(|err| {
            panic!("setup-no-http-no-gguf: read {}: {err}", path.display())
        });
        for token in FORBIDDEN {
            if src.contains(token) {
                hits.push(format!("{}: {token}", path.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "setup-no-http-no-gguf: src-tauri/src/ai/** must not contain network, shell, or cloud-host tokens; found {hits:?}"
    );

    let crate_root = manifest_dir();
    let mut crate_files = Vec::new();
    walk_src_tauri_lock(&crate_root, &mut crate_files);
    let mut gguf = Vec::new();
    let mut huge = Vec::new();
    for path in &crate_files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".gguf") || lower.ends_with(".ggml") || lower.ends_with(".safetensors") {
            gguf.push(path.display().to_string());
        }
        if let Ok(meta) = path.metadata() {
            if meta.len() >= 1_000_000 {
                huge.push(format!("{} ({} bytes)", path.display(), meta.len()));
            }
        }
    }
    assert!(
        gguf.is_empty(),
        "setup-no-http-no-gguf: src-tauri must not contain weight files; found {gguf:?}"
    );
    assert!(
        huge.is_empty(),
        "setup-no-http-no-gguf: src-tauri/src and test fixtures must not add a multi-MB weight; found {huge:?}"
    );
}

/// lifecycle-remove-size-without-hash — import fixture, note `size`.
/// Truncate dest blob to 0 bytes. `lifecycle::remove`. `recovered_bytes`
/// must equal the original recorded size (not 0). Fail-today: `get`
/// hashes the truncated blob and recovered collapses to 0.
#[test]
fn lifecycle_remove_size_without_hash() {
    let root = Scratch::new("remove-size");
    let src_dir = Scratch::new("remove-size-src");
    let src = write_fixture(src_dir.path(), "model-bytes");
    let store = ModelStore::open(root.path()).unwrap_or_else(|err| {
        panic!("lifecycle-remove-size-without-hash: open: {err}")
    });

    import(root.path(), &src, FIXTURE_SHA256).unwrap_or_else(|err| {
        panic!("lifecycle-remove-size-without-hash: import: {err}")
    });
    let size = store
        .get(FIXTURE_SHA256)
        .unwrap_or_else(|err| {
            panic!("lifecycle-remove-size-without-hash: get before truncate: {err}")
        })
        .size;
    assert_eq!(
        size,
        FIXTURE.len() as u64,
        "lifecycle-remove-size-without-hash: recorded size must be the fixture length"
    );
    assert_ne!(
        size, 0,
        "lifecycle-remove-size-without-hash: original size must be non-zero so a 0 recovered_bytes is distinguishable"
    );

    let blobs = root.path().join("blobs");
    let mut blob_files = Vec::new();
    walk_files(&blobs, &mut blob_files);
    assert!(
        !blob_files.is_empty(),
        "lifecycle-remove-size-without-hash: dest blob must exist after import"
    );
    for path in &blob_files {
        std::fs::write(path, b"").unwrap_or_else(|err| {
            panic!(
                "lifecycle-remove-size-without-hash: truncate {} to 0 bytes: {err}",
                path.display()
            )
        });
        let meta = std::fs::metadata(path).unwrap_or_else(|err| {
            panic!(
                "lifecycle-remove-size-without-hash: metadata after truncate {}: {err}",
                path.display()
            )
        });
        assert_eq!(
            meta.len(),
            0,
            "lifecycle-remove-size-without-hash: dest blob {} must be 0 bytes after truncate",
            path.display()
        );
    }

    let removed = remove(root.path(), FIXTURE_SHA256).unwrap_or_else(|err| {
        panic!(
            "lifecycle-remove-size-without-hash: remove of a truncated blob must succeed: {err}"
        )
    });
    assert_eq!(
        removed.checksum, FIXTURE_SHA256,
        "lifecycle-remove-size-without-hash: result.checksum must name the removed model"
    );
    assert_eq!(
        removed.recovered_bytes, size,
        "lifecycle-remove-size-without-hash: recovered_bytes must equal the original recorded size {size}, not the truncated blob length"
    );
}

fn walk_src_tauri_lock(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(_) => continue,
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            name,
            "binaries"
                | "share"
                | "tesseract"
                | "libreoffice"
                | "windows-runtime"
                | "gen"
                | "target"
                | "vendor"
                | "resources"
        ) {
            continue;
        }
        if path.is_dir() {
            walk_src_tauri_lock(&path, out);
        } else {
            out.push(path);
        }
    }
}
