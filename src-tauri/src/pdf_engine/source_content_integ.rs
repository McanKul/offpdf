//! Read-only source-content classifier tests (#33).
//!
//! Production module is added by impl: `crate::pdf_engine::source_content`.
//! This file only imports the locked exports. Fail-today is a missing module.
//!
//! Locked surface (field names so impl can match):
//!
//! ```ignore
//! pub struct SourceOccurrence {
//!     pub page_index: u32,
//!     pub kind: /* enum or string: text | image */,
//!     pub rect: /* { x, y, w, h } unrotated PDF user space */,
//!     pub locator: String,
//!     pub capability: /* enum or string: supported | unsupported */,
//!     pub reason: Option<String>,
//! }
//! pub fn classify_source_content(path: &Path) -> Result<Vec<SourceOccurrence>, AppError>;
//! pub fn resolve_source_locator(path: &Path, locator: &str) -> Result<SourceOccurrence, AppError>;
//! ```

#![cfg(test)]

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

const FROZEN_REASONS: &[&str] = &[
    "NO_TOUNICODE",
    "AMBIGUOUS_UNICODE",
    "TYPE3",
    "NESTED_FORM",
    "ROTATED_TEXT",
    "SKEWED_TEXT",
    "VERTICAL",
    "CLIPPED",
    "PATTERN",
    "SHARED_XOBJECT",
    "INLINE_IMAGE",
    "MASKED_IMAGE",
    "ENCRYPTED",
    "SIGNED",
    "MALFORMED",
    "STALE",
    "GEOMETRY",
];

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
    occ.reason.as_ref().and_then(|r| {
        let trimmed = r.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
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
        FROZEN_REASONS.contains(&reason),
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

// --- CLASSIFY-API -----------------------------------------------------------

#[test]
fn classify_api_lists_occurrences() {
    let path = fixture("text-tj.pdf");
    let hits = classify(&path, "CLASSIFY-API");
    assert!(
        !hits.is_empty(),
        "CLASSIFY-API: classify_source_content(text-tj.pdf) must list occurrences"
    );
}

// --- CLASSIFY-SOURCE-PATH ---------------------------------------------------

#[test]
fn classify_uses_corpus_source_path_not_page_pdf() {
    let path = fixture("text-tj.pdf");
    let rendered = path.to_string_lossy();
    assert!(
        rendered.contains("fixtures/source-edit") && rendered.ends_with("text-tj.pdf"),
        "CLASSIFY-SOURCE-PATH: must pass the corpus source path, not pagePdf / --empty --pages; got {}",
        path.display()
    );
    // Do not spawn qpdf --empty --pages and do not call page_pdf_b64.
    let hits = classify(&path, "CLASSIFY-SOURCE-PATH");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-SOURCE-PATH");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-SOURCE-PATH: text-tj.pdf page_index is 0 on the original source"
    );
}

// --- CLASSIFY-TRY-EDIT-TJ ---------------------------------------------------

#[test]
fn classify_text_tj_supported_at_origin() {
    let path = fixture("text-tj.pdf");
    let hits = classify(&path, "CLASSIFY-TRY-EDIT-TJ");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-TRY-EDIT-TJ");
    assert_supported_text_or_image(occ, "text", "CLASSIFY-TRY-EDIT-TJ");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-TRY-EDIT-TJ: page_index must be 0"
    );
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-TJ: origin x must be ~72; got {}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 720.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-TJ: origin y must be ~720; got {}",
        occ.rect.y
    );
    assert!(
        occ.rect.w > 0.0 && occ.rect.h > 0.0,
        "CLASSIFY-TRY-EDIT-TJ: w/h must be positive; got w={} h={}",
        occ.rect.w,
        occ.rect.h
    );
}

// --- CLASSIFY-TRY-EDIT-IMAGE ------------------------------------------------

#[test]
fn classify_image_unique_supported() {
    let path = fixture("image-unique.pdf");
    let hits = classify(&path, "CLASSIFY-TRY-EDIT-IMAGE");
    let occ = first_of_kind(&hits, "image", "CLASSIFY-TRY-EDIT-IMAGE");
    assert_supported_text_or_image(occ, "image", "CLASSIFY-TRY-EDIT-IMAGE");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-TRY-EDIT-IMAGE: page_index must be 0"
    );
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: x must be ~72; got {}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 400.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: y must be ~400; got {}",
        occ.rect.y
    );
    assert!(
        (occ.rect.w - 40.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: w must be ~40; got {}",
        occ.rect.w
    );
    assert!(
        (occ.rect.h - 40.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: h must be ~40; got {}",
        occ.rect.h
    );
}

// --- CLASSIFY-NO-TOUNICODE --------------------------------------------------

#[test]
fn classify_cid_no_tounicode_is_unsupported() {
    let path = fixture("text-cid-no-tounicode.pdf");
    let hits = classify(&path, "CLASSIFY-NO-TOUNICODE");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-NO-TOUNICODE");
    assert_unsupported(occ, "text", "NO_TOUNICODE", "CLASSIFY-NO-TOUNICODE");
}

// --- CLASSIFY-SHARED-IMAGE --------------------------------------------------

#[test]
fn classify_image_reused_shared_xobject() {
    let path = fixture("image-reused.pdf");
    let hits = classify(&path, "CLASSIFY-SHARED-IMAGE");
    let images: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "image").collect();
    assert_eq!(
        images.len(),
        2,
        "CLASSIFY-SHARED-IMAGE: image-reused.pdf must yield two image occurrences; got {:?}",
        hits.iter()
            .map(|o| (
                o.page_index,
                kind_token(o),
                capability_token(o),
                reason_code(o)
            ))
            .collect::<Vec<_>>()
    );
    for occ in images {
        assert_unsupported(occ, "image", "SHARED_XOBJECT", "CLASSIFY-SHARED-IMAGE");
    }
}

// --- CLASSIFY-STAND-INS -----------------------------------------------------

#[test]
fn classify_stand_ins_locked_reasons() {
    for (file, kind, reason) in STAND_INS {
        let path = fixture(file);
        let hits = classify(&path, "CLASSIFY-STAND-INS");
        let occ = first_of_kind(&hits, kind, "CLASSIFY-STAND-INS");
        assert_unsupported(occ, kind, reason, &format!("CLASSIFY-STAND-INS: {file}"));
        for other in &hits {
            assert_ne!(
                capability_token(other),
                "supported",
                "CLASSIFY-STAND-INS: {file} must never return supported"
            );
        }
    }
}

// --- CLASSIFY-NO-SILENT-OVERLAY ---------------------------------------------

#[test]
fn classify_stand_ins_never_supported_or_overlay_fallback() {
    let manifest = load_manifest();
    let stand_in_rows: Vec<&FixtureRow> = manifest
        .fixtures
        .iter()
        .filter(|r| r.intent == "unsupported-stand-in")
        .collect();
    assert!(
        !stand_in_rows.is_empty(),
        "CLASSIFY-NO-SILENT-OVERLAY: manifest must list unsupported-stand-in rows"
    );
    for row in stand_in_rows {
        let path = fixture(&row.path);
        let hits = classify(&path, "CLASSIFY-NO-SILENT-OVERLAY");
        for occ in &hits {
            assert_ne!(
                capability_token(occ),
                "supported",
                "CLASSIFY-NO-SILENT-OVERLAY: {} must not be supported",
                row.id
            );
            if let Some(code) = reason_code(occ) {
                assert!(
                    !looks_like_overlay_fallback(&code),
                    "CLASSIFY-NO-SILENT-OVERLAY: {} reason must not be an overlay fallback; got {code}",
                    row.id
                );
                assert!(
                    FROZEN_REASONS.contains(&code.as_str()),
                    "CLASSIFY-NO-SILENT-OVERLAY: {} reason {code} is not a frozen code",
                    row.id
                );
            }
            assert!(
                !looks_like_overlay_fallback(&capability_token(occ)),
                "CLASSIFY-NO-SILENT-OVERLAY: {} capability must not be an overlay fallback",
                row.id
            );
        }
    }
}

// --- CLASSIFY-NO-SAVE -------------------------------------------------------

#[test]
fn classify_does_not_write_source_or_dest() {
    let path = fixture("text-tj.pdf");
    let parent = path
        .parent()
        .expect("CLASSIFY-NO-SAVE: fixture has a parent dir");
    let before_names = dir_names(parent);
    let before_bytes = fs::read(&path).unwrap();
    let before_meta = fs::metadata(&path).unwrap();
    let before_mtime = before_meta.modified().ok();
    let before_len = before_meta.len();

    // Entry point must not write, whether it returns Ok or Err.
    let _ = classify_source_content(&path);

    let after_bytes = fs::read(&path).unwrap();
    assert_eq!(
        after_bytes, before_bytes,
        "CLASSIFY-NO-SAVE: source bytes of text-tj.pdf must be unchanged"
    );
    let after_meta = fs::metadata(&path).unwrap();
    assert_eq!(
        after_meta.len(),
        before_len,
        "CLASSIFY-NO-SAVE: source length must be unchanged"
    );
    if let (Some(before), Ok(after)) = (before_mtime, after_meta.modified()) {
        assert_eq!(
            after, before,
            "CLASSIFY-NO-SAVE: source mtime must be unchanged"
        );
    }
    let after_names = dir_names(parent);
    assert_eq!(
        after_names, before_names,
        "CLASSIFY-NO-SAVE: no dest sibling may be created next to the fixture; before={before_names:?} after={after_names:?}"
    );
}

// --- CLASSIFY-NO-EDITABLE-CLAIM ---------------------------------------------

#[test]
fn classify_try_edit_is_not_auto_supported() {
    let cid = classify(
        &fixture("text-cid-tounicode.pdf"),
        "CLASSIFY-NO-EDITABLE-CLAIM",
    );
    let cid_occ = first_of_kind(&cid, "text", "CLASSIFY-NO-EDITABLE-CLAIM");
    assert_unsupported(
        cid_occ,
        "text",
        "AMBIGUOUS_UNICODE",
        "CLASSIFY-NO-EDITABLE-CLAIM: text-cid-tounicode.pdf is try-edit but must not be treated as supported",
    );
    assert_ne!(
        reason_code(cid_occ).as_deref(),
        Some("NO_TOUNICODE"),
        "CLASSIFY-NO-EDITABLE-CLAIM: text-cid-tounicode.pdf has a ToUnicode CMap; NO_TOUNICODE is the wrong reason"
    );

    let kerned = classify(&fixture("text-tj-kerned.pdf"), "CLASSIFY-NO-EDITABLE-CLAIM");
    let kerned_occ = first_of_kind(&kerned, "text", "CLASSIFY-NO-EDITABLE-CLAIM");
    assert_supported_text_or_image(
        kerned_occ,
        "text",
        "CLASSIFY-NO-EDITABLE-CLAIM: text-tj-kerned.pdf is the human pick for supported",
    );

    let manifest = load_manifest();
    let try_edit: Vec<&FixtureRow> = manifest
        .fixtures
        .iter()
        .filter(|r| r.intent == "try-edit")
        .collect();
    assert!(
        try_edit.iter().any(|r| r.id == "text-cid-tounicode"),
        "CLASSIFY-NO-EDITABLE-CLAIM: manifest still marks text-cid-tounicode as try-edit"
    );
    assert!(
        try_edit.iter().any(|r| r.id == "text-tj-kerned"),
        "CLASSIFY-NO-EDITABLE-CLAIM: manifest still marks text-tj-kerned as try-edit"
    );
}

// --- CLASSIFY-STALE ---------------------------------------------------------

#[test]
fn classify_mutated_copy_locator_is_stale() {
    let src = fixture("text-tj.pdf");
    let hits = classify(&src, "CLASSIFY-STALE");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-STALE");
    let locator = occ.locator.clone();
    assert!(
        !locator.trim().is_empty(),
        "CLASSIFY-STALE: locator must be a non-empty opaque string"
    );

    match resolve_source_locator(&src, &locator) {
        Ok(_) => {}
        Err(err) => assert_ne!(
            err.code.as_str(),
            "STALE",
            "CLASSIFY-STALE: original source must not be STALE"
        ),
    }

    let scratch = Scratch::new("stale");
    let copy = scratch.file("copy.pdf");
    fs::copy(&src, &copy).unwrap();
    let mut bytes = fs::read(&copy).unwrap();
    flip_one_payload_byte(&mut bytes);
    fs::write(&copy, &bytes).unwrap();

    let err = resolve_source_locator(&copy, &locator)
        .expect_err("CLASSIFY-STALE: resolve_source_locator on a mutated copy must be Err, not Ok");
    assert_eq!(
        err.code, "STALE",
        "CLASSIFY-STALE: AppError.code must be STALE; got {} ({})",
        err.code, err.message
    );
}

// --- CLASSIFY-BOUNDS --------------------------------------------------------

#[test]
fn classify_missing_path_is_invalid_pdf() {
    let scratch = Scratch::new("missing");
    let missing = scratch.file("no-such.pdf");
    expect_err_code(
        classify_source_content(&missing),
        "INVALID_PDF",
        "CLASSIFY-BOUNDS: missing path",
    );
}

#[test]
fn classify_broken_tiny_pdf_is_app_error() {
    let scratch = Scratch::new("broken");
    let path = scratch.file("tiny.pdf");
    fs::write(&path, b"%PDF-1.4\n%% truncated").unwrap();
    expect_bounds_err(
        classify_source_content(&path),
        &["MALFORMED_CONTENT", "INVALID_PDF"],
        "CLASSIFY-BOUNDS: broken tiny PDF",
    );
}

#[test]
fn classify_geom_only_pages_are_empty() {
    for name in [
        "geom-crop-offset.pdf",
        "geom-user-unit.pdf",
        "geom-rotate-90.pdf",
    ] {
        let hits = classify(&fixture(name), "CLASSIFY-BOUNDS");
        assert!(
            hits.is_empty(),
            "CLASSIFY-BOUNDS: {name} is geom-only (re f) and must return an empty list, not a fake supported/GEOMETRY row; got {:?}",
            hits.iter()
                .map(|o| (kind_token(o), capability_token(o), reason_code(o)))
                .collect::<Vec<_>>()
        );
    }
    for name in GEOM_ONLY {
        let hits = classify(&fixture(name), "CLASSIFY-BOUNDS");
        assert!(
            hits.iter().all(|o| capability_token(o) != "supported"),
            "CLASSIFY-BOUNDS: {name} must not mark a geom-only page supported"
        );
    }
}

#[test]
fn classify_oversize_sparse_file_is_file_too_large() {
    // Sparse tempfile — do not commit a 400 MiB PDF.
    let scratch = Scratch::new("huge");
    let path = scratch.file("huge.pdf");
    let f = File::create(&path).unwrap();
    f.set_len(FILE_CAP_BYTES + 1).unwrap();
    drop(f);
    expect_err_code(
        classify_source_content(&path),
        "FILE_TOO_LARGE",
        "CLASSIFY-BOUNDS: set_len(400MiB+1)",
    );
}

// --- PR 97 review fold (R1–R5) ---------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

fn box_obj(b: [i64; 4]) -> Object {
    Object::Array(b.into_iter().map(Object::Integer).collect())
}

fn helvetica_resources() -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    res
}

fn write_helvetica_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write generated classifier fixture");
}

fn write_text_with_empty_sig_widget(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n".to_vec(),
    )));
    let widget_id = doc.new_object_id();
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    page.set("Annots", vec![Object::Reference(widget_id)]);
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut widget = Dictionary::new();
    widget.set("Type", "Annot");
    widget.set("Subtype", "Widget");
    widget.set("FT", "Sig");
    widget.set("T", Object::string_literal("Sig1"));
    widget.set("Rect", box_obj([72, 72, 172, 92]));
    widget.set("P", page_id);
    doc.objects.insert(widget_id, Object::Dictionary(widget));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut acro = Dictionary::new();
    acro.set("Fields", vec![Object::Reference(widget_id)]);
    let acro_id = doc.add_object(Object::Dictionary(acro));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("AcroForm", acro_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write empty-sig-widget classifier fixture");
}

fn write_applied_signature(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n".to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut sig = Dictionary::new();
    sig.set("Type", "Sig");
    sig.set(
        "ByteRange",
        vec![
            Object::Integer(0),
            Object::Integer(10),
            Object::Integer(20),
            Object::Integer(30),
        ],
    );
    let _sig_id = doc.add_object(Object::Dictionary(sig));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write applied-signature classifier fixture");
}

// --- R1 --------------------------------------------------------------------

#[test]
fn classify_180_degree_text_is_rotated() {
    let scratch = Scratch::new("r1-180");
    let path = scratch.file("text-180.pdf");
    write_helvetica_page(&path, b"BT /F1 12 Tf -1 0 0 -1 200 400 Tm (Hi) Tj ET\n");
    let hits = classify(&path, "R1");
    let occ = first_of_kind(&hits, "text", "R1");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R1: 180° Tm [-1 0 0 -1 200 400] must not be supported"
    );
    assert_unsupported(occ, "text", "ROTATED_TEXT", "R1");
}

// --- R2 --------------------------------------------------------------------

#[test]
fn classify_second_tj_advances_tm() {
    let scratch = Scratch::new("r2-advance");
    let path = scratch.file("two-tj.pdf");
    write_helvetica_page(&path, b"BT /F1 12 Tf 72 720 Td (Hel) Tj (lo) Tj ET\n");
    let hits = classify(&path, "R2");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R2: (Hel) Tj (lo) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    assert!(
        (texts[0].rect.x - 72.0).abs() <= 1.0,
        "R2: first origin x must be ~72; got {}",
        texts[0].rect.x
    );
    assert!(
        texts[1].rect.x > texts[0].rect.x,
        "R2: second rect.x must be > first (must not share origin 72); first x={} second x={}",
        texts[0].rect.x,
        texts[1].rect.x
    );
    assert!(
        (texts[1].rect.x - 72.0).abs() > 1.0,
        "R2: second show must not reuse origin 72; first x={} second x={}",
        texts[0].rect.x,
        texts[1].rect.x
    );
}

// --- R3 --------------------------------------------------------------------

#[test]
fn classify_empty_sig_widget_does_not_refuse_file() {
    let scratch = Scratch::new("r3-empty-sig");
    let path = scratch.file("empty-sig.pdf");
    write_text_with_empty_sig_widget(&path);
    let hits = match classify_source_content(&path) {
        Ok(hits) => hits,
        Err(err) => panic!(
            "R3: empty /FT /Sig widget (Type Annot, no ByteRange) + Helvetica (Hi) Tj must be Ok with a text occurrence, not AppError SIGNED; got {} ({})",
            err.code, err.message
        ),
    };
    let occ = first_of_kind(&hits, "text", "R3");
    assert_ne!(
        reason_code(occ).as_deref(),
        Some("SIGNED"),
        "R3: Helvetica text on a file with an empty Sig widget must not be SIGNED"
    );
}

#[test]
fn classify_applied_signature_is_signed() {
    let scratch = Scratch::new("r3-applied-sig");
    let path = scratch.file("applied-sig.pdf");
    write_applied_signature(&path);
    expect_err_code(
        classify_source_content(&path),
        "SIGNED",
        "R3: /Type /Sig + ByteRange still refuses",
    );
}

// --- R4 --------------------------------------------------------------------

#[test]
fn classify_text_after_inline_image_is_kept() {
    let scratch = Scratch::new("r4-inline-rest");
    let path = scratch.file("inline-then-text.pdf");
    write_helvetica_page(
        &path,
        b"q 24 0 0 12 72 400 cm\n\
BI\n\
/W 2 /H 1 /CS /DeviceRGB /BPC 8 /F /AHx\n\
ID\n\
C8101010C810>\n\
EI\n\
Q\n\
BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n",
    );
    let hits = classify(&path, "R4");
    let _text = first_of_kind(&hits, "text", "R4");
}

// --- R5 --------------------------------------------------------------------

#[test]
fn classify_text_bounds_use_tm_scale() {
    let scratch = Scratch::new("r5-tm-scale");
    let path = scratch.file("tf1-tm12.pdf");
    write_helvetica_page(&path, b"BT /F1 1 Tf 12 0 0 12 72 720 Tm (Hi) Tj ET\n");
    let hits = classify(&path, "R5");
    let occ = first_of_kind(&hits, "text", "R5");
    assert!(
        (occ.rect.h - 12.0).abs() <= 1.0,
        "R5: /F1 1 Tf + 12 0 0 12 Tm must report height ~12, not ~1; got h={}",
        occ.rect.h
    );
    assert!(
        (occ.rect.h - 1.0).abs() > 1.0,
        "R5: rect.h must not stay at Tf size ~1; got h={}",
        occ.rect.h
    );
}

// --- PR 97 review fold r2 (R6–R9) ------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

fn write_type3_and_helvetica_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let proc_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"10 0 0 0 10 10 d1\n0 0 10 10 re f\n".to_vec(),
    )));
    let mut char_procs = Dictionary::new();
    char_procs.set("x", proc_id);

    let mut enc = Dictionary::new();
    enc.set("Type", "Encoding");
    enc.set(
        "Differences",
        vec![Object::Integer(120), Object::Name(b"x".to_vec())],
    );

    let mut t3 = Dictionary::new();
    t3.set("Type", "Font");
    t3.set("Subtype", "Type3");
    t3.set("FontBBox", box_obj([0, 0, 10, 10]));
    t3.set(
        "FontMatrix",
        Object::Array(vec![
            Object::Real(1.0),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(1.0),
            Object::Real(0.0),
            Object::Real(0.0),
        ]),
    );
    t3.set("CharProcs", Object::Dictionary(char_procs));
    t3.set("Encoding", Object::Dictionary(enc));
    t3.set("FirstChar", 120);
    t3.set("LastChar", 120);
    t3.set("Widths", vec![Object::Integer(10)]);

    let mut f1 = Dictionary::new();
    f1.set("Type", "Font");
    f1.set("Subtype", "Type1");
    f1.set("BaseFont", "Helvetica");

    let mut fonts = Dictionary::new();
    fonts.set("T3", Object::Dictionary(t3));
    fonts.set("F1", Object::Dictionary(f1));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write Type3+Helvetica classifier fixture");
}

// --- R6 --------------------------------------------------------------------

#[test]
fn classify_inline_image_uses_ctm_at_bi() {
    let path = fixture("image-inline.pdf");
    let hits = classify(&path, "R6");
    let occ = first_of_kind(&hits, "image", "R6");
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.x must be ~72 (CTM at BI), not the unit square at origin; got x={}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 400.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.y must be ~400 (CTM at BI), not the unit square at origin; got y={}",
        occ.rect.y
    );
    assert!(
        (occ.rect.w - 24.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.w must be ~24 (CTM at BI), not the unit square; got w={}",
        occ.rect.w
    );
    assert!(
        (occ.rect.h - 12.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.h must be ~12 (CTM at BI), not the unit square; got h={}",
        occ.rect.h
    );
}

// --- R7 --------------------------------------------------------------------

#[test]
fn classify_q_restores_type3_after_helvetica() {
    let scratch = Scratch::new("r7-q-type3");
    let path = scratch.file("q-type3.pdf");
    write_type3_and_helvetica_page(
        &path,
        b"BT /T3 12 Tf (x) Tj q /F1 12 Tf (y) Tj Q (z) Tj ET\n",
    );
    let hits = classify(&path, "R7");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        3,
        "R7: (x) Tj q /F1 (y) Tj Q (z) Tj must emit three text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), capability_token(o), reason_code(o)))
            .collect::<Vec<_>>()
    );
    let last = texts[2];
    assert_ne!(
        capability_token(last),
        "supported",
        "R7: last show (z) after Q must not be Helvetica supported; got {} reason={:?}",
        capability_token(last),
        reason_code(last)
    );
    assert_unsupported(last, "text", "TYPE3", "R7");
}

// --- R8 --------------------------------------------------------------------

#[test]
fn classify_tc_advances_second_tj() {
    let scratch = Scratch::new("r8-tc");
    let path = scratch.file("tc-two-tj.pdf");
    write_helvetica_page(&path, b"BT /F1 12 Tf 2 Tc 72 720 Td (Hi) Tj (there) Tj ET\n");
    let hits = classify(&path, "R8");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R8: (Hi) Tj (there) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    // Helvetica H=667 i=278 → 11.34 at Tf=12. 2 Tc on two glyphs adds 4
    // user units, so second.x ≈ first.x + 15.34, not first.x + 11.34.
    assert!(
        texts[1].rect.x > texts[0].rect.x + 13.0,
        "R8: 2 Tc must push second.x past first.x + no-Tc Hi width 11.34; first.x={} second.x={} (need second.x > first.x + 13)",
        texts[0].rect.x,
        texts[1].rect.x
    );
}

// --- R9 --------------------------------------------------------------------

#[test]
fn classify_source_drops_rotated_fixture_parenthetical() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pdf_engine/source_content.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "R9: must read source_content.rs via CARGO_MANIFEST_DIR ({}): {e}",
            path.display()
        )
    });
    assert!(
        !src.contains("keeps text-rotated.pdf green"),
        "R9: source_content.rs must not contain the exact substring `keeps text-rotated.pdf green`"
    );
}

// --- PR 97 review fold r3 (R10–R12) ----------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

/// 2×2 DeviceRGB, same bytes as the #32 unique/mask fixtures.
const R3_TINY_RGB: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];

fn write_type1_indirect_widths(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    // FirstChar 'H' (72) … LastChar 'i' (105): 34 glyph slots, all 1000.
    const FIRST_CHAR: i64 = 72;
    const LAST_CHAR: i64 = 105;
    let widths: Vec<Object> = (FIRST_CHAR..=LAST_CHAR)
        .map(|_| Object::Integer(1000))
        .collect();
    let widths_id = doc.add_object(Object::Array(widths));

    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    font.set("FirstChar", FIRST_CHAR);
    font.set("LastChar", LAST_CHAR);
    font.set("Widths", Object::Reference(widths_id));

    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write Type1 indirect-Widths classifier fixture");
}

fn write_pattern_cs_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));

    let mut cs = Dictionary::new();
    cs.set("Cs1", Object::Name(b"Pattern".to_vec()));

    let mut pat = Dictionary::new();
    pat.set("Type", "Pattern");
    pat.set("PatternType", 1);
    pat.set("PaintType", 1);
    pat.set("TilingType", 1);
    pat.set("BBox", box_obj([0, 0, 10, 10]));
    pat.set("XStep", 10);
    pat.set("YStep", 10);
    pat.set("Resources", Object::Dictionary(Dictionary::new()));
    let pat_id = doc.add_object(Object::Stream(Stream::new(
        pat,
        b"0 0 10 10 re f\n".to_vec(),
    )));
    let mut patterns = Dictionary::new();
    patterns.set("P1", Object::Reference(pat_id));

    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    res.set("ColorSpace", Object::Dictionary(cs));
    res.set("Pattern", Object::Dictionary(patterns));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write Pattern ColorSpace classifier fixture");
}

fn write_extgstate_smask_image(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut img = Dictionary::new();
    img.set("Type", "XObject");
    img.set("Subtype", "Image");
    img.set("Width", 2);
    img.set("Height", 2);
    img.set("ColorSpace", "DeviceRGB");
    img.set("BitsPerComponent", 8);
    let img_id = doc.add_object(Object::Stream(Stream::new(img, R3_TINY_RGB.to_vec())));

    let mut sm = Dictionary::new();
    sm.set("Type", "XObject");
    sm.set("Subtype", "Image");
    sm.set("Width", 2);
    sm.set("Height", 2);
    sm.set("ColorSpace", "DeviceGray");
    sm.set("BitsPerComponent", 8);
    let smask_id = doc.add_object(Object::Stream(Stream::new(sm, vec![255, 200, 180, 255])));

    let mut gs = Dictionary::new();
    gs.set("Type", "ExtGState");
    gs.set("SMask", Object::Reference(smask_id));
    let gs_id = doc.add_object(Object::Dictionary(gs));

    let mut xobjects = Dictionary::new();
    xobjects.set("Im0", Object::Reference(img_id));
    let mut extg = Dictionary::new();
    extg.set("Gs1", Object::Reference(gs_id));
    let mut res = Dictionary::new();
    res.set("XObject", Object::Dictionary(xobjects));
    res.set("ExtGState", Object::Dictionary(extg));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write ExtGState SMask + unique Image classifier fixture");
}

// --- R10 -------------------------------------------------------------------

#[test]
fn classify_indirect_widths_not_helvetica_fallback() {
    let scratch = Scratch::new("r10-widths");
    let path = scratch.file("indirect-widths.pdf");
    write_type1_indirect_widths(&path, b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n");
    let hits = classify(&path, "R10");
    let occ = first_of_kind(&hits, "text", "R10");
    assert!(
        (occ.rect.w - 24.0).abs() <= 1.0,
        "R10: Type1 indirect /Widths 1000,1000 at Tf=12 must report w≈24, not Helvetica fallback ≈11.34; got w={}",
        occ.rect.w
    );
    assert!(
        (occ.rect.w - 11.34).abs() > 1.0,
        "R10: rect.w must not stay on the Helvetica table ≈11.34; got w={}",
        occ.rect.w
    );
}

// --- R11 -------------------------------------------------------------------

#[test]
fn classify_named_pattern_cs_is_unsupported() {
    let scratch = Scratch::new("r11-pattern");
    let path = scratch.file("pattern-cs.pdf");
    write_pattern_cs_page(
        &path,
        b"BT /F1 12 Tf /Cs1 cs /P1 scn 72 720 Td (Hi) Tj ET\n",
    );
    let hits = classify(&path, "R11");
    let occ = first_of_kind(&hits, "text", "R11");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R11: /Cs1 cs Pattern resource + (Hi) Tj must not be supported; got {} reason={:?}",
        capability_token(occ),
        reason_code(occ)
    );
    assert_unsupported(occ, "text", "PATTERN", "R11");
}

// --- R12 -------------------------------------------------------------------

#[test]
fn classify_extgstate_smask_image_is_masked() {
    let scratch = Scratch::new("r12-gs-smask");
    let path = scratch.file("gs-smask.pdf");
    write_extgstate_smask_image(&path, b"q 40 0 0 40 72 400 cm /Gs1 gs /Im0 Do Q\n");
    let hits = classify(&path, "R12");
    let occ = first_of_kind(&hits, "image", "R12");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R12: unique Image after ExtGState /Gs1 /SMask must not be supported; got {} reason={:?}",
        capability_token(occ),
        reason_code(occ)
    );
    assert_unsupported(occ, "image", "MASKED_IMAGE", "R12");
}

// --- PR 97 review fold r4 (R13) --------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

fn write_unique_rgb_image(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut img = Dictionary::new();
    img.set("Type", "XObject");
    img.set("Subtype", "Image");
    img.set("Width", 2);
    img.set("Height", 2);
    img.set("ColorSpace", "DeviceRGB");
    img.set("BitsPerComponent", 8);
    let img_id = doc.add_object(Object::Stream(Stream::new(img, R3_TINY_RGB.to_vec())));

    let mut xobjects = Dictionary::new();
    xobjects.set("Im0", Object::Reference(img_id));
    let mut res = Dictionary::new();
    res.set("XObject", Object::Dictionary(xobjects));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write unique 2×2 DeviceRGB Image classifier fixture");
}

// --- R13a ------------------------------------------------------------------

#[test]
fn classify_stacked_cm_image_origin() {
    let scratch = Scratch::new("r13a-stacked-cm");
    let path = scratch.file("stacked-cm.pdf");
    write_unique_rgb_image(
        &path,
        b"q 2 0 0 2 0 0 cm 20 0 0 20 36 200 cm /Im0 Do Q\n",
    );
    let hits = classify(&path, "R13a");
    let occ = first_of_kind(&hits, "image", "R13a");
    assert_supported_text_or_image(occ, "image", "R13a");
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0
            && (occ.rect.y - 400.0).abs() <= 1.0
            && (occ.rect.w - 40.0).abs() <= 1.0
            && (occ.rect.h - 40.0).abs() <= 1.0,
        "R13a: stacked cm image rect must be ~{{x:72, y:400, w:40, h:40}}, not origin ~(36, 200); got {{x:{}, y:{}, w:{}, h:{}}}",
        occ.rect.x,
        occ.rect.y,
        occ.rect.w,
        occ.rect.h
    );
    assert!(
        (occ.rect.x - 36.0).abs() > 1.0 || (occ.rect.y - 200.0).abs() > 1.0,
        "R13a: stacked cm must not leave the image at the second-cm translation (36, 200); got {{x:{}, y:{}, w:{}, h:{}}}",
        occ.rect.x,
        occ.rect.y,
        occ.rect.w,
        occ.rect.h
    );
}

// --- R13b ------------------------------------------------------------------

#[test]
fn classify_scaled_tm_second_show_x() {
    let scratch = Scratch::new("r13b-scaled-tm");
    let path = scratch.file("scaled-tm-two-tj.pdf");
    write_helvetica_page(
        &path,
        b"BT /F1 1 Tf 12 0 0 12 72 720 Tm (Hel) Tj (lo) Tj ET\n",
    );
    let hits = classify(&path, "R13b");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R13b: (Hel) Tj (lo) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    let first = texts[0];
    let second = texts[1];
    // Helvetica H=667 e=556 l=278 → 1.501 at Tf=1. Scaled Tm 12× must
    // advance ~18.012 user units → second.x ≈ 90, not text-space 1.501
    // added in user space (≈73.5).
    assert!(
        second.rect.x > first.rect.x + 15.0 || (second.rect.x - 90.0).abs() <= 2.0,
        "R13b: second rect.x after 12 0 0 12 72 720 Tm (Hel) Tj must be ≈90 (±2), not ≈73.5; first.x={} second.x={}",
        first.rect.x,
        second.rect.x
    );
    assert!(
        (second.rect.x - 73.5).abs() > 1.0,
        "R13b: second rect.x must not stay at origin+text-space width ≈73.5; first.x={} second.x={}",
        first.rect.x,
        second.rect.x
    );
}
