//! Fail-closed publish gate for a staged PDF, before `replace_file`.
//!
//! `qpdf --check` exit policy (V4):
//! - `0` → clean (`QpdfCheckClass::Ok`)
//! - `3` → warnings only (`QpdfCheckClass::Warning`); do not block publish;
//!   keep stderr on [`ValidationResult::warnings`]
//! - `2` or any other nonzero → fatal (`QpdfCheckClass::Fatal`)
//!
//! On fatal validation: do not publish; delete the staged `.offpdf-*.pdf.tmp`;
//! leave the source PDF and any existing destination bytes untouched.

use crate::error::AppError;
use crate::pdf_engine::crop;
use lopdf::{Document, Object, ObjectId};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// FNV-1a of decoded page Contents plus that byte length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentDigest {
    pub hash: u64,
    pub len: usize,
}

/// Per-page geometry the gate compares to the reopened staged file.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSnapshot {
    pub media_box: [f64; 4],
    pub crop_box: Option<[f64; 4]>,
    pub trim_box: Option<[f64; 4]>,
    pub rotate: i64,
    pub user_unit: f64,
    pub content_digest: ContentDigest,
}

/// Catalog / trailer structures the source had and the staged file must keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogFlags {
    pub outlines: bool,
    pub info: bool,
    pub acro_form: bool,
    pub annots: bool,
}

/// Expected output after overlay: page order is `pages` order (count = `pages.len()`).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSnapshot {
    pub pages: Vec<PageSnapshot>,
    pub catalog: CatalogFlags,
}

/// Non-fatal findings from a passed gate (qpdf `--check` exit 3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationResult {
    pub warnings: Vec<String>,
}

/// Classification of a `qpdf --check` process result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpdfCheckClass {
    Ok,
    Warning,
    Fatal,
}

/// Classify `qpdf --check` from its exit code. `stderr` is recorded later on warnings.
pub fn classify_qpdf_check(exit: i32, stderr: &str) -> QpdfCheckClass {
    let _ = stderr;
    match exit {
        0 => QpdfCheckClass::Ok,
        3 => QpdfCheckClass::Warning,
        _ => QpdfCheckClass::Fatal,
    }
}

fn boxes_near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.5)
}

fn opt_boxes_near(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => boxes_near(x, y),
        _ => false,
    }
}

fn invalid_output(message: impl Into<String>) -> AppError {
    AppError::new(
        "INVALID_OUTPUT",
        "The edited PDF is not valid",
        message,
    )
    .with_suggestion("The original file was not changed. Try saving again.")
}

fn fatal_staged(staged: &Path, message: impl Into<String>) -> AppError {
    let _ = std::fs::remove_file(staged);
    invalid_output(message)
}

fn abort_if_cancelled(staged: &Path, cancel: Option<&AtomicBool>) -> Result<(), AppError> {
    if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
        let _ = std::fs::remove_file(staged);
        return Err(AppError::cancelled());
    }
    Ok(())
}

pub(crate) fn catalog_flags_from_doc(doc: &Document) -> CatalogFlags {
    CatalogFlags {
        outlines: has_catalog_key(doc, b"Outlines"),
        info: doc.trailer.get(b"Info").is_ok(),
        acro_form: has_catalog_key(doc, b"AcroForm"),
        annots: has_any_annots(doc),
    }
}

fn catalog_dict(doc: &Document) -> Option<&lopdf::Dictionary> {
    let root = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    doc.get_dictionary(root).ok()
}

fn has_catalog_key(doc: &Document, key: &[u8]) -> bool {
    catalog_dict(doc).and_then(|c| c.get(key).ok()).is_some()
}

fn has_any_annots(doc: &Document) -> bool {
    doc.get_pages().values().any(|id| {
        doc.get_dictionary(*id)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .is_some()
    })
}

/// Same FNV-1a-64 as `render::fnv1a_hex`, on raw bytes (no hex).
fn fnv1a_u64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest {
        hash: fnv1a_u64(bytes),
        len: bytes.len(),
    }
}

fn decoded_stream_bytes(stream: &lopdf::Stream) -> Vec<u8> {
    match stream.decompressed_content() {
        Ok(data) => data,
        Err(_) => stream.content.clone(),
    }
}

fn dict_from<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a lopdf::Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn page_form_xobject_bytes(doc: &Document, page_id: ObjectId) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let Ok((inline, inherited_ids)) = doc.get_page_resources(page_id) else {
        return out;
    };
    let mut resource_dicts: Vec<&lopdf::Dictionary> = Vec::new();
    if let Some(d) = inline {
        resource_dicts.push(d);
    }
    for id in inherited_ids {
        if let Ok(d) = doc.get_dictionary(id) {
            resource_dicts.push(d);
        }
    }
    for resources in resource_dicts {
        let Ok(xo_obj) = resources.get(b"XObject") else {
            continue;
        };
        let Some(xobjects) = dict_from(doc, xo_obj) else {
            continue;
        };
        for (_, obj) in xobjects.iter() {
            let Ok(id) = obj.as_reference() else {
                continue;
            };
            let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
                continue;
            };
            let subtype = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok());
            if subtype != Some(b"Form") {
                continue;
            }
            out.push(decoded_stream_bytes(stream));
        }
    }
    out
}

fn dest_page_digests(doc: &Document, page_id: ObjectId) -> Vec<ContentDigest> {
    let mut out = Vec::new();
    if let Ok(bytes) = doc.get_page_content(page_id) {
        out.push(content_digest(&bytes));
    }
    for id in doc.get_page_contents(page_id) {
        if let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) {
            out.push(content_digest(&decoded_stream_bytes(stream)));
        }
    }
    for bytes in page_form_xobject_bytes(doc, page_id) {
        out.push(content_digest(&bytes));
    }
    out
}

/// Validate a dest-sibling staged PDF against `snapshot` using `run_check` for `qpdf --check`.
///
/// `run_check` receives an argv array (no shell), typically `["--check", <staged>]`,
/// and returns `(exit, stderr)`.
pub fn validate_staged_pdf(
    staged: &Path,
    snapshot: &OutputSnapshot,
    cancel: Option<&AtomicBool>,
    mut run_check: impl FnMut(&[String]) -> Result<(i32, String), AppError>,
) -> Result<ValidationResult, AppError> {
    abort_if_cancelled(staged, cancel)?;

    let staged_arg = staged.to_string_lossy().into_owned();
    let args = ["--check".to_string(), staged_arg];
    let (exit, stderr) = match run_check(&args) {
        Ok(v) => v,
        Err(e) if e.code == "CANCELLED" => {
            let _ = std::fs::remove_file(staged);
            return Err(AppError::cancelled());
        }
        Err(e) => return Err(e),
    };
    abort_if_cancelled(staged, cancel)?;

    let mut warnings = Vec::new();
    match classify_qpdf_check(exit, &stderr) {
        QpdfCheckClass::Fatal => {
            return Err(fatal_staged(
                staged,
                if stderr.trim().is_empty() {
                    "qpdf --check reported errors in the edited PDF.".to_string()
                } else {
                    format!("qpdf --check reported errors: {}", stderr.trim())
                },
            ));
        }
        QpdfCheckClass::Warning => warnings.push(stderr),
        QpdfCheckClass::Ok => {}
    }

    let doc = match Document::load(staged) {
        Ok(d) => d,
        Err(e) => {
            return Err(fatal_staged(
                staged,
                format!("The edited PDF could not be reopened ({e})."),
            ));
        }
    };

    let page_map = doc.get_pages();
    if page_map.len() != snapshot.pages.len() {
        return Err(fatal_staged(
            staged,
            format!(
                "Page count changed: expected {}, found {}.",
                snapshot.pages.len(),
                page_map.len()
            ),
        ));
    }

    for (i, expected) in snapshot.pages.iter().enumerate() {
        let page_no = (i as u32) + 1;
        let Some(&id) = page_map.get(&page_no) else {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} is missing from the edited PDF."),
            ));
        };

        if !boxes_near(crop::media_box(&doc, id), expected.media_box) {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} MediaBox does not match the source."),
            ));
        }
        if !opt_boxes_near(crop::crop_box(&doc, id), expected.crop_box) {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} CropBox does not match the source."),
            ));
        }
        if !opt_boxes_near(crop::page_trim_box(&doc, id), expected.trim_box) {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} TrimBox does not match the source."),
            ));
        }
        if crop::page_rotation(&doc, id) != expected.rotate {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} /Rotate does not match the source."),
            ));
        }
        if (crop::page_user_unit(&doc, id) - expected.user_unit).abs() > 0.0001 {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} /UserUnit does not match the source."),
            ));
        }
        let candidates = dest_page_digests(&doc, id);
        if !candidates.iter().any(|d| *d == expected.content_digest) {
            return Err(fatal_staged(
                staged,
                format!("Page {page_no} content does not match the source."),
            ));
        }
        abort_if_cancelled(staged, cancel)?;
    }

    if snapshot.catalog.outlines && !has_catalog_key(&doc, b"Outlines") {
        return Err(fatal_staged(
            staged,
            "Bookmarks (Outlines) are missing from the edited PDF.",
        ));
    }
    if snapshot.catalog.info && doc.trailer.get(b"Info").is_err() {
        return Err(fatal_staged(
            staged,
            "Document Info metadata is missing from the edited PDF.",
        ));
    }
    if snapshot.catalog.acro_form && !has_catalog_key(&doc, b"AcroForm") {
        return Err(fatal_staged(
            staged,
            "AcroForm is missing from the edited PDF.",
        ));
    }
    if snapshot.catalog.annots && !has_any_annots(&doc) {
        return Err(fatal_staged(
            staged,
            "Page annotations are missing from the edited PDF.",
        ));
    }

    abort_if_cancelled(staged, cancel)?;
    Ok(ValidationResult { warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document, Object, Stream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "offpdf-validate-{}-{}-{}",
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

    fn box_obj(b: [i64; 4]) -> Object {
        Object::Array(b.into_iter().map(Object::Integer).collect())
    }

    fn write_one_page_pdf(path: &Path, extras: &[(&[u8], Object)]) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
        )));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set("MediaBox", box_obj([0, 0, 612, 792]));
        page.set("Contents", content_id);
        for (k, v) in extras {
            page.set(*k, v.clone());
        }
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
        doc.save(path).expect("write one-page fixture");
    }

    fn letter_page() -> PageSnapshot {
        PageSnapshot {
            media_box: [0.0, 0.0, 612.0, 792.0],
            crop_box: None,
            trim_box: None,
            rotate: 0,
            user_unit: 1.0,
            content_digest: content_digest(b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET"),
        }
    }

    fn letter_page_labeled(label: &str) -> PageSnapshot {
        let bytes = format!("BT /F1 12 Tf 72 720 Td ({label}) Tj ET").into_bytes();
        PageSnapshot {
            content_digest: content_digest(&bytes),
            ..letter_page()
        }
    }

    fn empty_catalog() -> CatalogFlags {
        CatalogFlags {
            outlines: false,
            info: false,
            acro_form: false,
            annots: false,
        }
    }

    fn letter_snapshot() -> OutputSnapshot {
        OutputSnapshot {
            pages: vec![letter_page()],
            catalog: empty_catalog(),
        }
    }

    fn assert_invalid_output(err: &AppError) {
        assert_eq!(err.code, "INVALID_OUTPUT");
        assert!(
            !err.title.trim().is_empty(),
            "INVALID_OUTPUT must have a title"
        );
        assert!(
            !err.message.trim().is_empty(),
            "INVALID_OUTPUT must have a message"
        );
        assert!(
            err.suggestion
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            "INVALID_OUTPUT must have a suggestion"
        );
    }

    // --- V4 -----------------------------------------------------------------

    #[test]
    fn classify_qpdf_check_exit_0_is_ok() {
        assert_eq!(
            classify_qpdf_check(0, ""),
            QpdfCheckClass::Ok,
            "exit 0 must be Ok"
        );
    }

    #[test]
    fn classify_qpdf_check_exit_3_is_warning() {
        assert_eq!(
            classify_qpdf_check(3, "WARNING: linearized"),
            QpdfCheckClass::Warning,
            "exit 3 must be Warning"
        );
    }

    #[test]
    fn classify_qpdf_check_exit_2_is_fatal() {
        assert_eq!(
            classify_qpdf_check(2, "ERROR: damaged"),
            QpdfCheckClass::Fatal,
            "exit 2 must be Fatal"
        );
    }

    #[test]
    fn classify_qpdf_check_other_nonzero_is_fatal() {
        assert_eq!(
            classify_qpdf_check(1, "unexpected"),
            QpdfCheckClass::Fatal,
            "other nonzero must be Fatal"
        );
        assert_eq!(
            classify_qpdf_check(99, "other"),
            QpdfCheckClass::Fatal,
            "other nonzero must be Fatal"
        );
    }

    #[test]
    fn validate_qpdf_exit_3_records_warning() {
        let scratch = Scratch::new("v4-warn");
        let staged = scratch.path().join("staged.pdf");
        write_one_page_pdf(&staged, &[]);
        let snapshot = letter_snapshot();
        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| {
            Ok((3, "WARNING: file has warnings\n".into()))
        })
        .expect("V4: qpdf --check exit 3 must not block");
        assert!(
            result.warnings.iter().any(|w| w.contains("WARNING")),
            "V4: exit 3 stderr must be recorded on ValidationResult.warnings; got {:?}",
            result.warnings
        );
    }

    // --- V1 / V8 ------------------------------------------------------------

    #[test]
    fn validate_staged_pdf_is_reachable_and_leaves_dest_untouched() {
        // V8: public path crate::pdf_engine::validate_output::validate_staged_pdf
        let scratch = Scratch::new("v1-v8");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        write_one_page_pdf(&staged, &[]);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let dest_mtime = std::fs::metadata(&dest).unwrap().modified().unwrap();
        let snapshot = letter_snapshot();

        let result = crate::pdf_engine::validate_output::validate_staged_pdf(
            &staged,
            &snapshot,
            None,
            |_| Ok((0, String::new())),
        );

        assert!(
            result.is_ok(),
            "V1: matching snapshot + check exit 0 is not fatal; {result:?}"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "V1: gate must not publish; dest bytes must stay OLD"
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().modified().unwrap(),
            dest_mtime,
            "V1: dest mtime must stay unchanged"
        );
    }

    #[test]
    fn validate_err_means_caller_must_not_publish() {
        // Caller contract around the gate (export runner is impl's job).
        let scratch = Scratch::new("v1-no-publish");
        let dest = scratch.path().join("out.pdf");
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let gate: Result<ValidationResult, AppError> = Err(AppError::new(
            "INVALID_OUTPUT",
            "The edited PDF is not valid",
            "Validation rejected the staged file.",
        )
        .with_suggestion("Try saving again, or pick a different destination."));
        if gate.is_err() {
            // do not replace dest
        } else {
            std::fs::write(&dest, b"NEW-PUBLISHED").unwrap();
        }
        assert_eq!(std::fs::read(&dest).unwrap(), b"OLD-DEST");
    }

    // --- V2 -----------------------------------------------------------------

    #[test]
    fn validate_truncated_staged_pdf_is_invalid_output() {
        let scratch = Scratch::new("v2-trunc");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        std::fs::write(&staged, b"%PDF-1.4\n%% truncated").unwrap();
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let dest_mtime = std::fs::metadata(&dest).unwrap().modified().unwrap();
        let snapshot = letter_snapshot();

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| {
            Ok((2, "qpdf --check: file is damaged".into()))
        });
        let err = result.expect_err("V2: truncated staged PDF must be INVALID_OUTPUT");
        assert_invalid_output(&err);
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "V2: dest bytes must stay OLD"
        );
        assert_eq!(
            std::fs::metadata(&dest).unwrap().modified().unwrap(),
            dest_mtime,
            "V2: dest mtime must stay unchanged"
        );
    }

    #[test]
    fn validate_truncated_does_not_create_dest() {
        let scratch = Scratch::new("v2-nodest");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        std::fs::write(&staged, b"%PDF-1.4\n%% truncated").unwrap();
        let snapshot = letter_snapshot();

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| {
            Ok((2, "qpdf --check: file is damaged".into()))
        });
        let err = result.expect_err("V2: truncated staged PDF must be INVALID_OUTPUT");
        assert_invalid_output(&err);
        assert!(
            !dest.exists(),
            "V2: dest must not be created when it did not exist"
        );
    }

    // --- V3 -----------------------------------------------------------------

    #[test]
    fn validate_fatal_deletes_staging_leaves_source_and_dest() {
        let scratch = Scratch::new("v3-cleanup");
        let source = scratch.path().join("source.pdf");
        let dest = scratch.path().join("out.pdf");
        let staged = scratch.path().join(".offpdf-job.pdf.tmp");
        std::fs::write(&source, b"SOURCE-BYTES").unwrap();
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        std::fs::write(&staged, b"%PDF-1.4\n%% truncated").unwrap();
        let snapshot = letter_snapshot();

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| {
            Ok((2, "qpdf --check: file is damaged".into()))
        });

        assert!(
            !staged.exists(),
            "V3: fatal validate must delete staged .offpdf-*.pdf.tmp"
        );
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"SOURCE-BYTES",
            "V3: source bytes must be unchanged"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "V3: dest bytes must be unchanged"
        );
        let err = result.expect_err("V3: truncated staging must be INVALID_OUTPUT");
        assert_invalid_output(&err);
    }

    // --- V5 -----------------------------------------------------------------

    #[test]
    fn validate_page_count_mismatch_is_invalid_output() {
        let scratch = Scratch::new("v5-pages");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        write_one_page_pdf(&staged, &[]);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let snapshot = OutputSnapshot {
            pages: vec![letter_page(), letter_page()],
            catalog: empty_catalog(),
        };

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| Ok((0, String::new())));
        let err = result.expect_err("V5: page-count mismatch must be INVALID_OUTPUT");
        assert_invalid_output(&err);
        assert_eq!(std::fs::read(&dest).unwrap(), b"OLD-DEST");
    }

    #[test]
    fn validate_cropbox_mismatch_is_invalid_output() {
        let scratch = Scratch::new("v5-crop");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        write_one_page_pdf(&staged, &[(b"CropBox", box_obj([0, 0, 612, 792]))]);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let snapshot = OutputSnapshot {
            pages: vec![PageSnapshot {
                media_box: [0.0, 0.0, 612.0, 792.0],
                crop_box: Some([72.0, 72.0, 540.0, 720.0]),
                trim_box: None,
                rotate: 0,
                user_unit: 1.0,
                content_digest: content_digest(b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET"),
            }],
            catalog: empty_catalog(),
        };

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| Ok((0, String::new())));
        let err = result.expect_err("V5: CropBox mismatch must be INVALID_OUTPUT");
        assert_invalid_output(&err);
        assert_eq!(std::fs::read(&dest).unwrap(), b"OLD-DEST");
    }

    // --- V6 -----------------------------------------------------------------

    #[test]
    fn validate_missing_catalog_keys_is_invalid_output() {
        let scratch = Scratch::new("v6-catalog");
        let staged = scratch.path().join("staged.pdf");
        let dest = scratch.path().join("out.pdf");
        // Valid one-page PDF: no Outlines, no Info, no AcroForm, no Annots.
        write_one_page_pdf(&staged, &[]);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let snapshot = OutputSnapshot {
            pages: vec![letter_page()],
            catalog: CatalogFlags {
                outlines: true,
                info: true,
                acro_form: true,
                annots: true,
            },
        };

        let result = validate_staged_pdf(&staged, &snapshot, None, |_args| Ok((0, String::new())));
        let err = result.expect_err(
            "V6: snapshot Outlines+Info+AcroForm+Annots missing on staged file must be INVALID_OUTPUT",
        );
        assert_invalid_output(&err);
        assert_eq!(std::fs::read(&dest).unwrap(), b"OLD-DEST");
    }

    // --- C1 -----------------------------------------------------------------

    #[test]
    fn validate_cancel_after_check_does_not_pass() {
        let scratch = Scratch::new("c1-cancel");
        let dest = scratch.path().join("out.pdf");
        let staged = scratch.path().join(".offpdf-job.pdf.tmp");
        write_one_page_pdf(&staged, &[]);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        let snapshot = letter_snapshot();
        let cancel = AtomicBool::new(false);

        let result = validate_staged_pdf(&staged, &snapshot, Some(&cancel), |_args| {
            cancel.store(true, Ordering::SeqCst);
            Ok((0, String::new()))
        });

        let err = result.expect_err("C1: cancel after successful --check must be CANCELLED");
        assert_eq!(
            err.code, "CANCELLED",
            "C1: cancel after check must be CANCELLED, not {}",
            err.code
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "C1: dest bytes must stay OLD-DEST"
        );
        assert!(
            !staged.exists(),
            "C1: cancel must delete staged .offpdf-*.pdf.tmp"
        );
    }

    // --- R1 / R1b -----------------------------------------------------------

    fn write_two_page_letter(path: &Path, label1: &str, label2: &str) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content1 = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            format!("BT /F1 12 Tf 72 720 Td ({label1}) Tj ET").into_bytes(),
        )));
        let content2 = doc.add_object(Object::Stream(Stream::new(
            Dictionary::new(),
            format!("BT /F1 12 Tf 72 720 Td ({label2}) Tj ET").into_bytes(),
        )));

        let mut page1 = Dictionary::new();
        page1.set("Type", "Page");
        page1.set("Parent", pages_id);
        page1.set("MediaBox", box_obj([0, 0, 612, 792]));
        page1.set("Contents", content1);
        let page1_id = doc.add_object(Object::Dictionary(page1));

        let mut page2 = Dictionary::new();
        page2.set("Type", "Page");
        page2.set("Parent", pages_id);
        page2.set("MediaBox", box_obj([0, 0, 612, 792]));
        page2.set("Contents", content2);
        let page2_id = doc.add_object(Object::Dictionary(page2));

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", vec![page1_id.into(), page2_id.into()]);
        pages.set("Count", 2);
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", pages_id);
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", catalog_id);
        doc.save(path).expect("write two-page fixture");
    }

    fn two_letter_snapshot() -> OutputSnapshot {
        OutputSnapshot {
            pages: vec![
                letter_page_labeled("ALPHA-PAGE"),
                letter_page_labeled("BETA-PAGE"),
            ],
            catalog: empty_catalog(),
        }
    }

    fn swap_page_kids(path: &Path) {
        let mut doc = Document::load(path).expect("load two-page pdf");
        let root = doc
            .trailer
            .get(b"Root")
            .expect("Root")
            .as_reference()
            .expect("Root ref");
        let pages_id = doc
            .get_dictionary(root)
            .expect("catalog")
            .get(b"Pages")
            .expect("Pages")
            .as_reference()
            .expect("Pages ref");
        let kids = match doc.get_dictionary(pages_id).expect("pages dict").get(b"Kids") {
            Ok(Object::Array(a)) => a.clone(),
            other => panic!("Kids must be an array, got {other:?}"),
        };
        assert_eq!(kids.len(), 2, "fixture must have two Kids");
        let swapped = vec![kids[1].clone(), kids[0].clone()];
        match doc.get_object_mut(pages_id) {
            Ok(Object::Dictionary(pages)) => pages.set("Kids", swapped),
            other => panic!("Pages object is not a dictionary: {other:?}"),
        }
        doc.save(path).expect("rewrite swapped Kids");
    }

    fn page_stream_text(path: &Path, page: u32) -> String {
        let doc = Document::load(path).expect("load pdf");
        let id = *doc.get_pages().get(&page).expect("page id");
        let bytes = doc.get_page_content(id).expect("page contents");
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn validate_swapped_same_geometry_pages_is_invalid_output() {
        let scratch = Scratch::new("r1-swap");
        let dest = scratch.path().join("out.pdf");
        let staged = scratch.path().join(".offpdf-r1.pdf.tmp");
        write_two_page_letter(&staged, "ALPHA-PAGE", "BETA-PAGE");
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        swap_page_kids(&staged);
        assert!(
            page_stream_text(&staged, 1).contains("BETA-PAGE"),
            "swap must put BETA on dest page 1"
        );
        assert!(
            page_stream_text(&staged, 2).contains("ALPHA-PAGE"),
            "swap must put ALPHA on dest page 2"
        );

        let result = validate_staged_pdf(&staged, &two_letter_snapshot(), None, |_args| {
            Ok((0, String::new()))
        });
        let err = result.expect_err(
            "R1: same-geometry Kids swap must be INVALID_OUTPUT (presence of source Contents digest)",
        );
        assert_invalid_output(&err);
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "R1: dest bytes must stay OLD-DEST"
        );
        assert!(
            !staged.exists(),
            "R1: fatal validate must delete staged .offpdf-*.pdf.tmp"
        );
    }

    #[test]
    fn validate_in_order_same_geometry_pages_is_ok() {
        let scratch = Scratch::new("r1b-order");
        let dest = scratch.path().join("out.pdf");
        let staged = scratch.path().join("staged.pdf");
        write_two_page_letter(&staged, "ALPHA-PAGE", "BETA-PAGE");
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        assert!(page_stream_text(&staged, 1).contains("ALPHA-PAGE"));
        assert!(page_stream_text(&staged, 2).contains("BETA-PAGE"));

        let result = validate_staged_pdf(&staged, &two_letter_snapshot(), None, |_args| {
            Ok((0, String::new()))
        });
        assert!(
            result.is_ok(),
            "R1b: in-order two-page same-geometry dest must pass the gate; {result:?}"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "R1b: gate must not publish; dest bytes must stay OLD"
        );
    }
}
