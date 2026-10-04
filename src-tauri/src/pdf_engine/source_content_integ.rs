//! Source-content classifier tests (#33): the corpus contract (`corpus.rs`), the PR #97 review
//! regressions R1–R14 (`review.rs`, `review2.rs`) and the hardening regressions CLS-H01…H27 for
//! every maintainer item of #33 (`hardening.rs`, `hardening2.rs`).
//!
//! Surface (SPEC §C): `SourceOccurrence { page_index, kind, rect, locator, capability,
//! reason: Option<TextReason>, text }`; the document-wide `classify_source_content` and
//! `resolve_source_locator` (test-only) loop the per-page `classify_source_page`.

#![cfg(test)]

mod corpus;
mod hardening;
mod hardening2;
mod review;
mod review2;

use crate::error::AppError;
use crate::pdf_engine::source_content::{
    classify_source_content, resolve_source_locator, SourceOccurrence,
};
use lopdf::{Dictionary, Document, Object, Stream};
use serde::Deserialize;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Same 400 MiB gate as forms / links / outline.
const FILE_CAP_BYTES: u64 = 400 * 1024 * 1024;

/// The 14 #33 reason names that must survive (CLS-H22).
const LEGACY_REASONS: [&str; 14] = [
    "INLINE_IMAGE",
    "NESTED_FORM",
    "TYPE3",
    "VERTICAL",
    "CLIPPED",
    "PATTERN",
    "ROTATED_TEXT",
    "SKEWED_TEXT",
    "MISSING_FONT",
    "NO_TOUNICODE",
    "AMBIGUOUS_UNICODE",
    "MASKED_IMAGE",
    "SHARED_XOBJECT",
    "GEOMETRY",
];

/// Every code of `src/lib/editor/text-reasons.json` (run, page, image, file, problem, save,
/// warning lists): the frozen vocabulary (CLS-H22 — it used to be a hand list with a typo).
fn frozen_reasons() -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_str(include_str!("../../../src/lib/editor/text-reasons.json"))
            .expect("text-reasons.json parses");
    json.as_object()
        .expect("text-reasons.json is an object")
        .values()
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

const STAND_INS: &[(&str, &str, &str)] = &[
    ("text-type3.pdf", "text", "TYPE3"),
    ("text-nested-form.pdf", "text", "NESTED_FORM"),
    ("text-rotated.pdf", "text", "ROTATED_TEXT"),
    ("text-skewed.pdf", "text", "SKEWED_TEXT"),
    ("image-in-form.pdf", "image", "NESTED_FORM"),
    ("image-inline.pdf", "image", "INLINE_IMAGE"),
    ("image-mask.pdf", "image", "MASKED_IMAGE"),
];

const GEOM_ONLY: &[&str] = &[
    "geom-crop-offset.pdf",
    "geom-user-unit.pdf",
    "geom-rotate-90.pdf",
    "geom-rotate-180.pdf",
    "geom-rotate-270.pdf",
];

#[derive(Debug, Deserialize)]
struct Manifest {
    fixtures: Vec<FixtureRow>,
}

#[derive(Debug, Deserialize)]
struct FixtureRow {
    id: String,
    path: String,
    intent: String,
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-classify-{}-{}-{}",
            name,
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("source-edit")
}

fn fixture(name: &str) -> PathBuf {
    let path = corpus_dir().join(name);
    assert!(
        path.is_file(),
        "committed fixture {} must exist under fixtures/source-edit/",
        name
    );
    path
}

fn load_manifest() -> Manifest {
    let path = corpus_dir().join("manifest.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixtures/source-edit/manifest.json must be readable: {e}"));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("fixtures/source-edit/manifest.json must parse: {e}"))
}

fn debug_token<T: std::fmt::Debug>(v: &T) -> String {
    format!("{v:?}")
        .trim_matches('"')
        .split("::")
        .last()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn kind_token(occ: &SourceOccurrence) -> String {
    debug_token(&occ.kind)
}

fn capability_token(occ: &SourceOccurrence) -> String {
    debug_token(&occ.capability)
}

fn reason_code(occ: &SourceOccurrence) -> Option<String> {
    occ.reason.map(|r| r.as_str().to_string())
}

fn classify(path: &Path, must_id: &str) -> Vec<SourceOccurrence> {
    classify_source_content(path).unwrap_or_else(|e| {
        panic!(
            "{must_id}: classify_source_content({}) must succeed: {e}",
            path.display()
        )
    })
}

fn first_of_kind<'a>(
    hits: &'a [SourceOccurrence],
    kind: &str,
    must_id: &str,
) -> &'a SourceOccurrence {
    hits.iter()
        .find(|o| kind_token(o) == kind)
        .unwrap_or_else(|| {
            panic!(
                "{must_id}: expected a {kind} occurrence; got {:?}",
                hits.iter()
                    .map(|o| (kind_token(o), capability_token(o), reason_code(o)))
                    .collect::<Vec<_>>()
            )
        })
}

fn assert_supported_text_or_image(occ: &SourceOccurrence, kind: &str, must_id: &str) {
    assert_eq!(kind_token(occ), kind, "{must_id}: kind must be {kind}");
    assert_eq!(
        capability_token(occ),
        "supported",
        "{must_id}: capability must be supported; got {} reason={:?}",
        capability_token(occ),
        reason_code(occ)
    );
    assert!(
        reason_code(occ).is_none(),
        "{must_id}: supported must not carry a refuse reason; got {:?}",
        reason_code(occ)
    );
    assert!(
        !occ.locator.trim().is_empty(),
        "{must_id}: locator must be a non-empty opaque string"
    );
    assert!(
        occ.rect.w > 0.0 && occ.rect.h > 0.0,
        "{must_id}: rect w/h must be positive; got w={} h={}",
        occ.rect.w,
        occ.rect.h
    );
}

fn assert_unsupported(occ: &SourceOccurrence, kind: &str, reason: &str, must_id: &str) {
    assert_eq!(kind_token(occ), kind, "{must_id}: kind must be {kind}");
    assert_eq!(
        capability_token(occ),
        "unsupported",
        "{must_id}: capability must be unsupported; got {}",
        capability_token(occ)
    );
    assert_ne!(
        capability_token(occ),
        "supported",
        "{must_id}: must never be supported"
    );
    assert_eq!(
        reason_code(occ).as_deref(),
        Some(reason),
        "{must_id}: reason must be {reason}; got {:?}",
        reason_code(occ)
    );
    assert!(
        frozen_reasons().iter().any(|f| f == reason),
        "{must_id}: {reason} is not a frozen reason code"
    );
    assert!(
        !occ.locator.trim().is_empty(),
        "{must_id}: locator must be present even when unsupported"
    );
}

fn looks_like_overlay_fallback(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("use overlay")
        || lower.contains("cover-and-overlay")
        || lower.contains("cover and overlay")
        || lower.contains("overlay fallback")
        || lower.contains("fallback to overlay")
}

fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn flip_one_payload_byte(bytes: &mut [u8]) {
    if let Some(i) = bytes.windows(2).position(|w| w == b"Hi") {
        bytes[i] ^= 0x01;
        return;
    }
    let i = bytes.len() / 2;
    bytes[i] ^= 0x01;
}

fn expect_err_code(result: Result<Vec<SourceOccurrence>, AppError>, code: &str, must_id: &str) {
    match result {
        Ok(hits) => panic!(
            "{must_id}: expected AppError.code={code}, got Ok({} occurrences)",
            hits.len()
        ),
        Err(err) => assert_eq!(
            err.code, code,
            "{must_id}: AppError.code must be {code}; got {} ({})",
            err.code, err.message
        ),
    }
}

fn expect_bounds_err(
    result: Result<Vec<SourceOccurrence>, AppError>,
    allowed: &[&str],
    must_id: &str,
) {
    match result {
        Ok(hits) => panic!(
            "{must_id}: broken/missing input must be AppError {:?}, not Ok({} occurrences)",
            allowed,
            hits.len()
        ),
        Err(err) => assert!(
            allowed.iter().any(|c| err.code == *c),
            "{must_id}: AppError.code must be one of {allowed:?}; got {} ({})",
            err.code,
            err.message
        ),
    }
}
