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
