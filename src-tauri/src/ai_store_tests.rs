//! Unit tests for `crate::ai::store` (issue #83).
//!
//! Locked Approach A names: `ModelStore::open`, `import_file`,
//! `install_from_reader`, `list_ready`, `get`, `loadable_path`, `remove`,
//! and the versioned `ModelManifest` JSON (`schema_version`, size, license,
//! `runtime_compat`, SHA-256 hex `checksum`).
//! Do not edit `ai_tests.rs`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ai::store::{ModelManifest, ModelStore};

const FIXTURE: &[u8] = b"offpdf-store-fixture-v1";
const FIXTURE_SHA256: &str = "9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e";
const WRONG_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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

fn tree_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total += tree_size(&path);
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
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

fn child_names(dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return names,
    };
    for entry in entries.flatten() {
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
    names
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

fn valid_manifest_json() -> String {
    format!(
        r#"{{"schema_version":1,"size":{size},"license":"CC0-1.0","runtime_compat":"test","checksum":"{sum}"}}"#,
        size = FIXTURE.len(),
        sum = FIXTURE_SHA256
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-store-{}-{}-{}",
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

/// store-manifest-parse — valid fixture JSON parses; truncated / missing
/// fields / bad schema_version rejected.
#[test]
fn store_manifest_parse() {
    let parsed = ModelManifest::parse(&valid_manifest_json())
        .unwrap_or_else(|err| panic!("store-manifest-parse: valid fixture JSON must parse: {err}"));
    assert_eq!(
        parsed.schema_version, 1,
        "store-manifest-parse: schema_version must be 1"
    );
    assert_eq!(
        parsed.size,
        FIXTURE.len() as u64,
        "store-manifest-parse: size must match the fixture byte length"
    );
    assert_eq!(
        parsed.license, "CC0-1.0",
        "store-manifest-parse: license must round-trip"
    );
    assert_eq!(
        parsed.runtime_compat, "test",
        "store-manifest-parse: runtime_compat is an opaque string"
    );
    assert_eq!(
        parsed.checksum, FIXTURE_SHA256,
        "store-manifest-parse: checksum must be the SHA-256 hex"
    );

    let truncated = r#"{"schema_version":1,"size":23,"license":"CC0-1.0""#;
    let err = ModelManifest::parse(truncated)
        .expect_err("store-manifest-parse: truncated JSON must be rejected");
    assert_ai_model_error(&err, "store-manifest-parse");

    let not_json = "not-json";
    let err = ModelManifest::parse(not_json)
        .expect_err("store-manifest-parse: non-JSON must be rejected");
    assert_ai_model_error(&err, "store-manifest-parse");

    let missing = [
        r#"{"size":23,"license":"CC0-1.0","runtime_compat":"test","checksum":"9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e"}"#,
        r#"{"schema_version":1,"license":"CC0-1.0","runtime_compat":"test","checksum":"9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e"}"#,
        r#"{"schema_version":1,"size":23,"runtime_compat":"test","checksum":"9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e"}"#,
        r#"{"schema_version":1,"size":23,"license":"CC0-1.0","checksum":"9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e"}"#,
        r#"{"schema_version":1,"size":23,"license":"CC0-1.0","runtime_compat":"test"}"#,
    ];
    for json in missing {
        let err = ModelManifest::parse(json)
            .expect_err("store-manifest-parse: JSON missing a required field must be rejected");
        assert_ai_model_error(&err, "store-manifest-parse");
    }

    for version in [0_u32, 999] {
        let json = format!(
            r#"{{"schema_version":{version},"size":23,"license":"CC0-1.0","runtime_compat":"test","checksum":"{FIXTURE_SHA256}"}}"#
        );
        let err = ModelManifest::parse(&json).expect_err(&format!(
            "store-manifest-parse: schema_version {version} must be rejected"
        ));
        assert_ai_model_error(&err, "store-manifest-parse");
    }
}

/// store-import-ready — import FIXTURE + matching sha256 → only ready entry;
/// loadable_path under the injected root.
#[test]
fn store_import_ready() {
    let root = Scratch::new("import-ready");
    let store = ModelStore::open(root.path())
        .unwrap_or_else(|err| panic!("store-import-ready: open injected root must succeed: {err}"));

    let mut reader = std::io::Cursor::new(FIXTURE);
    let installed = store
        .install_from_reader(&mut reader, FIXTURE_SHA256)
        .unwrap_or_else(|err| {
            panic!("store-import-ready: install_from_reader + matching sha256: {err}")
        });
    assert_eq!(
        installed.checksum, FIXTURE_SHA256,
        "store-import-ready: checksum is the id"
    );
    assert_eq!(
        installed.size,
        FIXTURE.len() as u64,
        "store-import-ready: size must be the fixture length"
    );

    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-import-ready: list_ready: {err}"));
    assert_eq!(
        ready.len(),
        1,
        "store-import-ready: matching import must be the only ready entry"
    );
    assert_eq!(ready[0].checksum, FIXTURE_SHA256);

    let got = store
        .get(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-import-ready: get(checksum) after import: {err}"));
    assert_eq!(got.checksum, FIXTURE_SHA256);
    assert_eq!(got.size, FIXTURE.len() as u64);

    let path = store
        .loadable_path(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-import-ready: loadable_path: {err}"));
    assert!(
        is_under(&path, root.path()),
        "store-import-ready: loadable_path {} must be under the injected root {}",
        path.display(),
        root.path().display()
    );
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|err| panic!("store-import-ready: loadable blob must exist: {err}"));
    assert_eq!(
        bytes, FIXTURE,
        "store-import-ready: loadable blob must be the fixture bytes"
    );

    let src_dir = Scratch::new("import-src");
    let src = src_dir.path().join("model-bytes");
    std::fs::write(&src, FIXTURE).unwrap();
    let names_before = child_names(src_dir.path());
    let imported = store
        .import_file(&src, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-import-ready: import_file + matching sha256: {err}"));
    assert_eq!(imported.checksum, FIXTURE_SHA256);
    assert!(
        src.exists(),
        "store-import-ready: import_file copies, it must not move the source"
    );
    assert_eq!(
        child_names(src_dir.path()),
        names_before,
        "store-import-ready: import must not write next to the source file"
    );
    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-import-ready: list_ready after file import: {err}"));
    assert_eq!(
        ready.len(),
        1,
        "store-import-ready: same bytes via import_file stay one ready entry"
    );
    let loadable = store
        .loadable_path(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-import-ready: loadable_path after file import: {err}"));
    assert!(
        is_under(&loadable, root.path()),
        "store-import-ready: file-import loadable_path must stay under the store root"
    );
    assert_ne!(
        loadable.canonicalize().ok(),
        src.canonicalize().ok(),
        "store-import-ready: loadable_path must not be the caller's source file"
    );
}

/// store-checksum-mismatch-not-ready — wrong digest → AppError (AI_MODEL_*,
/// not ENGINE_FAILED), not in list_ready.
#[test]
fn store_checksum_mismatch_not_ready() {
    let root = Scratch::new("checksum-mismatch");
    let store = ModelStore::open(root.path())
        .unwrap_or_else(|err| panic!("store-checksum-mismatch-not-ready: open: {err}"));

    let mut reader = std::io::Cursor::new(FIXTURE);
    let err = store
        .install_from_reader(&mut reader, WRONG_SHA256)
        .expect_err("store-checksum-mismatch-not-ready: wrong digest must return AppError, not Ok");
    assert_ai_model_error(&err, "store-checksum-mismatch-not-ready");

    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-checksum-mismatch-not-ready: list_ready: {err}"));
    assert!(
        ready.is_empty(),
        "store-checksum-mismatch-not-ready: mismatched install must not appear in list_ready, got {} entries",
        ready.len()
    );
    assert!(
        store.get(WRONG_SHA256).is_err(),
        "store-checksum-mismatch-not-ready: wrong digest must not be get-able"
    );
    assert!(
        store.get(FIXTURE_SHA256).is_err(),
        "store-checksum-mismatch-not-ready: fixture checksum must not be ready after a mismatch"
    );
}

/// store-partial-not-ready — leftover staging/ or a truncated blob → not in
/// list_ready.
#[test]
fn store_partial_not_ready() {
    let leftover = Scratch::new("partial-staging");
    std::fs::create_dir_all(leftover.path().join("staging").join("interrupted")).unwrap();
    std::fs::write(
        leftover
            .path()
            .join("staging")
            .join("interrupted")
            .join("blob"),
        FIXTURE,
    )
    .unwrap();
    let store = ModelStore::open(leftover.path())
        .unwrap_or_else(|err| panic!("store-partial-not-ready: open leftover staging root: {err}"));
    let ready = store.list_ready().unwrap_or_else(|err| {
        panic!("store-partial-not-ready: list_ready on leftover staging: {err}")
    });
    assert!(
        ready.is_empty(),
        "store-partial-not-ready: leftover staging/ must not appear in list_ready, got {} entries",
        ready.len()
    );

    let truncated = Scratch::new("partial-trunc");
    let store = ModelStore::open(truncated.path())
        .unwrap_or_else(|err| panic!("store-partial-not-ready: open truncate root: {err}"));
    let mut reader = std::io::Cursor::new(FIXTURE);
    store
        .install_from_reader(&mut reader, FIXTURE_SHA256)
        .unwrap_or_else(|err| {
            panic!("store-partial-not-ready: matching install before truncate: {err}")
        });
    let blobs = truncated.path().join("blobs");
    assert!(
        blobs.is_dir(),
        "store-partial-not-ready: committed blob must live under <root>/blobs"
    );
    let mut blob_files = Vec::new();
    walk_files(&blobs, &mut blob_files);
    assert!(
        !blob_files.is_empty(),
        "store-partial-not-ready: blobs/ must contain the committed fixture"
    );
    for path in &blob_files {
        std::fs::write(path, &FIXTURE[..5]).unwrap_or_else(|err| {
            panic!(
                "store-partial-not-ready: truncate {}: {err}",
                path.display()
            )
        });
    }
    let store = ModelStore::open(truncated.path())
        .unwrap_or_else(|err| panic!("store-partial-not-ready: reopen after truncate: {err}"));
    let ready = store.list_ready().unwrap_or_else(|err| {
        panic!("store-partial-not-ready: list_ready after truncated blob: {err}")
    });
    assert!(
        ready.is_empty(),
        "store-partial-not-ready: truncated blob must not appear in list_ready, got {} entries",
        ready.len()
    );
}

/// store-dedup-no-waste — import the same bytes twice → one blob; blob-tree
/// size does not double.
#[test]
fn store_dedup_no_waste() {
    let root = Scratch::new("dedup");
    let store = ModelStore::open(root.path())
        .unwrap_or_else(|err| panic!("store-dedup-no-waste: open: {err}"));

    let mut first = std::io::Cursor::new(FIXTURE);
    store
        .install_from_reader(&mut first, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-dedup-no-waste: first install: {err}"));
    let blobs = root.path().join("blobs");
    let size_after_first = tree_size(&blobs);
    let files_after_first = file_count(&blobs);
    assert!(
        size_after_first >= FIXTURE.len() as u64,
        "store-dedup-no-waste: blobs/ must hold at least the fixture after the first import"
    );
    assert!(
        files_after_first >= 1,
        "store-dedup-no-waste: first import must create a blob file"
    );

    let mut second = std::io::Cursor::new(FIXTURE);
    store
        .install_from_reader(&mut second, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-dedup-no-waste: second install: {err}"));

    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-dedup-no-waste: list_ready: {err}"));
    assert_eq!(
        ready.len(),
        1,
        "store-dedup-no-waste: same checksum is one ready model, got {} entries",
        ready.len()
    );

    let size_after_second = tree_size(&blobs);
    let files_after_second = file_count(&blobs);
    assert_eq!(
        files_after_second, files_after_first,
        "store-dedup-no-waste: second import must not add another blob file"
    );
    assert_eq!(
        size_after_second, size_after_first,
        "store-dedup-no-waste: blob-tree size must not grow on a duplicate import"
    );
    assert!(
        size_after_second < (FIXTURE.len() as u64).saturating_mul(2),
        "store-dedup-no-waste: blob-tree size must not reach two copies of the fixture"
    );
}

/// store-remove-all — remove deletes that model's manifest, blob, staging;
/// list_ready empty. Second remove is a structured not-found.
#[test]
fn store_remove_all() {
    let root = Scratch::new("remove");
    let store =
        ModelStore::open(root.path()).unwrap_or_else(|err| panic!("store-remove-all: open: {err}"));

    let mut reader = std::io::Cursor::new(FIXTURE);
    store
        .install_from_reader(&mut reader, FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-remove-all: install: {err}"));

    std::fs::create_dir_all(root.path().join("staging").join(FIXTURE_SHA256)).unwrap();
    std::fs::write(
        root.path()
            .join("staging")
            .join(FIXTURE_SHA256)
            .join("leftover"),
        FIXTURE,
    )
    .unwrap();

    store
        .remove(FIXTURE_SHA256)
        .unwrap_or_else(|err| panic!("store-remove-all: first remove: {err}"));

    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-remove-all: list_ready after remove: {err}"));
    assert!(
        ready.is_empty(),
        "store-remove-all: list_ready must be empty after remove, got {} entries",
        ready.len()
    );
    assert_eq!(
        tree_size(&root.path().join("blobs")),
        0,
        "store-remove-all: blob for this checksum must be gone"
    );
    assert_eq!(
        tree_size(&root.path().join("manifests")),
        0,
        "store-remove-all: manifest for this checksum must be gone"
    );
    assert_eq!(
        tree_size(&root.path().join("staging")),
        0,
        "store-remove-all: leftover staging for this model must be gone"
    );

    let err = store
        .remove(FIXTURE_SHA256)
        .expect_err("store-remove-all: second remove must be a structured not-found, not Ok");
    assert_ai_model_error(&err, "store-remove-all");
}

/// store-no-auto-download — open empty root + list_ready: no URL, no default
/// model, no writes outside the injected root.
#[test]
fn store_no_auto_download() {
    let parent = Scratch::new("no-auto-parent");
    let root = parent.path().join("store-root");
    std::fs::create_dir_all(&root).unwrap();
    let parent_before = child_names(parent.path());

    let store = ModelStore::open(&root).unwrap_or_else(|err| {
        panic!("store-no-auto-download: open empty root must not need a URL: {err}")
    });
    let ready = store
        .list_ready()
        .unwrap_or_else(|err| panic!("store-no-auto-download: list_ready on empty root: {err}"));
    assert!(
        ready.is_empty(),
        "store-no-auto-download: empty root must have no default model, got {} entries",
        ready.len()
    );

    assert_eq!(
        child_names(parent.path()),
        parent_before,
        "store-no-auto-download: open + list_ready must not write outside the injected root"
    );
    assert!(
        !root.ends_with("jobs"),
        "store-no-auto-download: injected root must not be <app_cache_dir>/jobs"
    );
}

/// store-no-network-no-shell — `src-tauri/src/ai/**` still has no
/// reqwest/ureq/std::net/Command/XAI_API_KEY/cloud host. store.rs must exist.
#[test]
fn store_no_network_no_shell() {
    let ai_dir = ai_src_dir();
    assert!(
        ai_dir.is_dir(),
        "store-no-network-no-shell: src-tauri/src/ai/ must exist so the source lock can scan it"
    );

    let store_rs = ai_dir.join("store.rs");
    let store_mod = ai_dir.join("store").join("mod.rs");
    assert!(
        store_rs.is_file() || store_mod.is_file(),
        "store-no-network-no-shell: src-tauri/src/ai/store.rs (or store/mod.rs) must exist"
    );

    let mut files = Vec::new();
    walk_files(&ai_dir, &mut files);
    let rust_files: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    assert!(
        !rust_files.is_empty(),
        "store-no-network-no-shell: src-tauri/src/ai/ must contain Rust sources to scan"
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
            panic!("store-no-network-no-shell: read {}: {err}", path.display())
        });
        for token in FORBIDDEN {
            if src.contains(token) {
                hits.push(format!("{}: {token}", path.display()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "store-no-network-no-shell: src-tauri/src/ai/** must not contain network, shell, or cloud-host tokens; found {hits:?}"
    );
}

/// store-no-gguf-in-clone — walk crate sources (skip gitignored bundle dirs):
/// no `.gguf` / `.ggml` / `.safetensors` and no huge weight. Tests use
/// in-memory FIXTURE.
#[test]
fn store_no_gguf_in_clone() {
    assert_eq!(
        FIXTURE, b"offpdf-store-fixture-v1",
        "store-no-gguf-in-clone: tests must use the tiny in-memory FIXTURE"
    );
    assert!(
        FIXTURE.len() < 64,
        "store-no-gguf-in-clone: FIXTURE must stay tiny, got {} bytes",
        FIXTURE.len()
    );

    let crate_root = manifest_dir();
    let mut files = Vec::new();
    walk_src_tauri_lock(&crate_root, &mut files);

    let mut gguf = Vec::new();
    let mut huge = Vec::new();
    for path in &files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.to_ascii_lowercase().ends_with(".gguf")
            || name.to_ascii_lowercase().ends_with(".ggml")
            || name.to_ascii_lowercase().ends_with(".safetensors")
        {
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
        "store-no-gguf-in-clone: src-tauri must not contain weight files; found {gguf:?}"
    );
    assert!(
        huge.is_empty(),
        "store-no-gguf-in-clone: src-tauri/src and test fixtures must not add a multi-MB weight; found {huge:?}"
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
        // Local prepare-* trees drop multi-MB tools here; they are not weights.
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
