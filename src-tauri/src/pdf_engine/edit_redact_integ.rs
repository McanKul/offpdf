//! Issue #10 secure redaction must-IDs.
//!
//! Named product surface (missing module is accepted until impl):
//! `edit_redact::{apply_redactions, verify_redaction, RedactRegion}`.
//!
//! `apply_redactions` integ skips when `pdftoppm` is missing (no new CI
//! poppler requirement). `verify_redaction` uses synthetic lopdf dests.

#![cfg(test)]

use crate::error::AppError;
use crate::pdf_engine::edit_overlay::PdfRectIn;
use crate::pdf_engine::edit_redact::{apply_redactions, verify_redaction, RedactRegion};
use super::source_edit_fixtures::write_corpus_fixture;
use lopdf::{Dictionary, Document, Object, Stream};
use std::path::{Path, PathBuf};

/// Corpus `image-unique` / `TINY_RGB` samples (`source_edit_fixtures.rs`).
const TINY_RGB: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-redact-{}-{}-{}",
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

    fn pdf(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_pdftoppm() -> Option<PathBuf> {
    for c in [
        "/opt/homebrew/bin/pdftoppm",
        "/usr/local/bin/pdftoppm",
        "/opt/local/bin/pdftoppm",
        "/usr/bin/pdftoppm",
    ] {
        if Path::new(c).exists() {
            return Some(PathBuf::from(c));
        }
    }
    std::process::Command::new("pdftoppm")
        .arg("-v")
        .output()
        .ok()
        .filter(|o| o.status.success() || !o.stderr.is_empty())
        .map(|_| PathBuf::from("pdftoppm"))
}

fn box_obj(b: [i64; 4]) -> Object {
    Object::Array(b.into_iter().map(Object::Integer).collect())
}

fn region(page_index: u32, x: f64, y: f64, w: f64, h: f64) -> RedactRegion {
    RedactRegion {
        page_index,
        rect: PdfRectIn { x, y, w, h },
        fill: Some("#000000".into()),
        label: None,
    }
}

fn write_one_page(path: &Path, content: &[u8], extras: &[(&[u8], Object)]) {
    let mut doc = Document::with_version("1.5");
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

fn write_secret_text(path: &Path) {
    write_one_page(
        path,
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n",
        &[],
    );
}

fn write_ocr_probe(path: &Path) {
    write_one_page(
        path,
        b"BT /F1 12 Tf 3 Tr 72 720 Td (OCRPROBE) Tj ET\n",
        &[],
    );
}

fn write_vector_path(path: &Path) {
    write_one_page(path, b"0 0 0 rg 72 72 100 100 re f\n", &[]);
}

fn write_two_page_mixed(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content1 = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n".to_vec(),
    )));
    let content2 = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n".to_vec(),
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
    doc.save(path).expect("write two-page mixed fixture");
}

fn write_leftover_annot(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 200 Td (Hello) Tj ET\n".to_vec(),
    )));

    let mut annot = Dictionary::new();
    annot.set("Type", "Annot");
    annot.set("Subtype", "Text");
    annot.set("Rect", box_obj([72, 700, 192, 740]));
    annot.set("Contents", Object::string_literal("LEAKANNOT"));
    let annot_id = doc.add_object(Object::Dictionary(annot));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Annots", vec![annot_id.into()]);
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
    doc.save(path).expect("write leftover-annot fixture");
}

fn write_intersecting_field(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 200 Td (Hello) Tj ET\n".to_vec(),
    )));

    let mut widget = Dictionary::new();
    widget.set("Type", "Annot");
    widget.set("Subtype", "Widget");
    widget.set("FT", Object::Name(b"Tx".to_vec()));
    widget.set("T", Object::string_literal("Name"));
    widget.set("V", Object::string_literal("LEAKFIELD"));
    widget.set("Rect", box_obj([72, 700, 200, 740]));
    widget.set("P", Object::Reference((0, 0)));
    let widget_id = doc.add_object(Object::Dictionary(widget));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Annots", vec![widget_id.into()]);
    let page_id = doc.add_object(Object::Dictionary(page));
    if let Ok(Object::Dictionary(w)) = doc.get_object_mut(widget_id) {
        w.set("P", page_id);
    }

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut acro = Dictionary::new();
    acro.set("Fields", vec![widget_id.into()]);
    let acro_id = doc.add_object(Object::Dictionary(acro));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("AcroForm", acro_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write intersecting-field fixture");
}

fn decoded_blob(path: &Path) -> String {
    let mut doc = Document::load(path).expect("load pdf");
    let _ = doc.decompress();
    let mut out = String::new();
    for obj in doc.objects.values() {
        if let Object::Stream(s) = obj {
            let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
            out.push_str(&String::from_utf8_lossy(&bytes));
            out.push('\n');
        }
    }
    out
}

fn dest_has_bytes(path: &Path, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    if let Ok(raw) = std::fs::read(path) {
        if raw.windows(needle.len()).any(|w| w == needle) {
            return true;
        }
    }
    let Ok(mut doc) = Document::load(path) else {
        return false;
    };
    let _ = doc.decompress();
    for obj in doc.objects.values() {
        match obj {
            Object::Stream(s) => {
                let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
                if bytes.windows(needle.len()).any(|w| w == needle) {
                    return true;
                }
            }
            Object::String(b, _) if b.windows(needle.len()).any(|w| w == needle) => return true,
            _ => {}
        }
    }
    false
}

fn dest_has_str(path: &Path, needle: &str) -> bool {
    dest_has_bytes(path, needle.as_bytes()) || decoded_blob(path).contains(needle)
}

fn dest_has_re_op(path: &Path, x: f64, y: f64, w: f64, h: f64) -> bool {
    let blob = decoded_blob(path);
    for (i, _) in blob.match_indices(" re") {
        let start = blob[..i]
            .rfind(|c: char| c == '\n' || c == '\r')
            .map(|j| j + 1)
            .unwrap_or(0);
        let frag = blob[start..i + 3].trim();
        let nums: Vec<f64> = frag
            .split_whitespace()
            .take(4)
            .filter_map(|t| t.parse().ok())
            .collect();
        if nums.len() == 4
            && (nums[0] - x).abs() < 1.0
            && (nums[1] - y).abs() < 1.0
            && (nums[2] - w).abs() < 1.0
            && (nums[3] - h).abs() < 1.0
        {
            return true;
        }
    }
    false
}

fn apply_copy(src: &Path, dest: &Path, regions: &[RedactRegion]) -> Result<(), AppError> {
    std::fs::copy(src, dest).expect("copy dest sibling");
    apply_redactions(dest, regions)
}

fn cover_secret() -> RedactRegion {
    region(0, 60.0, 700.0, 160.0, 40.0)
}

fn cover_path() -> RedactRegion {
    region(0, 72.0, 72.0, 100.0, 100.0)
}

// ---------------------------------------------------------------------------
// R-TEXT
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_drops_secret_show_string() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-text-secret");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_secret_text(&src);
    apply_copy(&src, &dest, &[cover_secret()]).expect("R-TEXT: apply_redactions must succeed");
    assert!(
        !dest_has_str(&dest, "SECRET"),
        "R-TEXT: dest decoded streams must not contain the source show-string SECRET; blob={:?}",
        decoded_blob(&dest)
    );
}

#[test]
fn apply_redactions_drops_corpus_text_tj_hi() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-text-tj");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture("text-tj", &src).expect("write text-tj");
    apply_copy(&src, &dest, &[cover_secret()]).expect("R-TEXT: apply_redactions must succeed");
    assert!(
        !dest_has_str(&dest, "Hi"),
        "R-TEXT: dest decoded streams must not contain corpus text-tj show-string Hi; blob={:?}",
        decoded_blob(&dest)
    );
}

// ---------------------------------------------------------------------------
// R-VECTOR
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_drops_original_path_ops() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-vector");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_vector_path(&src);
    apply_copy(&src, &dest, &[cover_path()]).expect("R-VECTOR: apply_redactions must succeed");
    assert!(
        !dest_has_re_op(&dest, 72.0, 72.0, 100.0, 100.0),
        "R-VECTOR: dest must not keep the original path ops under the region; blob={:?}",
        decoded_blob(&dest)
    );
}

// ---------------------------------------------------------------------------
// R-IMAGE
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_drops_image_unique_tiny_rgb_samples() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-image");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture("image-unique", &src).expect("write image-unique");
    apply_copy(&src, &dest, &[region(0, 72.0, 400.0, 40.0, 40.0)])
        .expect("R-IMAGE: apply_redactions must succeed");
    assert!(
        !dest_has_bytes(&dest, TINY_RGB),
        "R-IMAGE: dest must not keep recoverable source image samples TINY_RGB [200, 16, 16, …]"
    );
}

// ---------------------------------------------------------------------------
// R-OCR
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_drops_invisible_ocrprobe() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-ocr");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_ocr_probe(&src);
    apply_copy(&src, &dest, &[cover_secret()]).expect("R-OCR: apply_redactions must succeed");
    assert!(
        !dest_has_str(&dest, "OCRPROBE"),
        "R-OCR: dest must not contain the invisible OCR show-string OCRPROBE; blob={:?}",
        decoded_blob(&dest)
    );
}

// ---------------------------------------------------------------------------
// R-ANNOT
// ---------------------------------------------------------------------------

#[test]
fn verify_redaction_warns_leftover_intersecting_annot_contents_and_does_not_strip() {
    let scratch = Scratch::new("r-annot");
    let dest = scratch.pdf("dest.pdf");
    write_leftover_annot(&dest);
    let regions = [cover_secret()];
    let warnings = verify_redaction(&dest, &[], &regions).expect(
        "R-ANNOT: leftover intersecting /Contents must warn, not fail-closed and not auto-strip",
    );
    assert!(
        !warnings.is_empty(),
        "R-ANNOT: leftover /Contents intersecting the region must produce a warning"
    );
    assert!(
        dest_has_str(&dest, "LEAKANNOT"),
        "R-ANNOT: leftover annot /Contents string may remain; do not auto-strip"
    );
}

// ---------------------------------------------------------------------------
// R-FORM
// ---------------------------------------------------------------------------

#[test]
fn verify_redaction_warns_intersecting_field_v_and_does_not_strip() {
    let scratch = Scratch::new("r-form");
    let dest = scratch.pdf("dest.pdf");
    write_intersecting_field(&dest);
    let regions = [cover_secret()];
    let warnings = verify_redaction(&dest, &[], &regions)
        .expect("R-FORM: intersecting field /V must warn, not fail-closed and not auto-strip");
    assert!(
        !warnings.is_empty(),
        "R-FORM: intersecting field /V must produce a warning"
    );
    assert!(
        dest_has_str(&dest, "LEAKFIELD"),
        "R-FORM: intersecting field /V may remain; do not auto-strip"
    );
}

// ---------------------------------------------------------------------------
// R-GEOM
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_unrotated_rect_on_rotate_90() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-geom-rot90");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture("geom-rotate-90", &src).expect("write geom-rotate-90");
    apply_copy(&src, &dest, &[cover_path()]).expect("R-GEOM: apply_redactions must succeed");
    assert!(
        !dest_has_re_op(&dest, 72.0, 72.0, 100.0, 100.0),
        "R-GEOM: unrotated rect on rotate-90 must cover 72 72 100 100 re; blob={:?}",
        decoded_blob(&dest)
    );
}

#[test]
fn apply_redactions_unrotated_rect_on_crop_neq_media() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-geom-crop");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture("geom-crop-offset", &src).expect("write geom-crop-offset");
    apply_copy(&src, &dest, &[cover_path()]).expect("R-GEOM: apply_redactions must succeed");
    assert!(
        !dest_has_re_op(&dest, 72.0, 72.0, 100.0, 100.0),
        "R-GEOM: unrotated rect on Crop≠Media must cover 72 72 100 100 re; blob={:?}",
        decoded_blob(&dest)
    );
}

// ---------------------------------------------------------------------------
// R-VERIFY
// ---------------------------------------------------------------------------

#[test]
fn verify_redaction_fails_closed_when_page_content_probe_remains() {
    let scratch = Scratch::new("r-verify-probe");
    let dest = scratch.pdf("dest.pdf");
    write_secret_text(&dest);
    let err = verify_redaction(&dest, &[b"SECRET".as_slice()], &[cover_secret()]).expect_err(
        "R-VERIFY: verification must fail-closed if the page-content probe remains",
    );
    assert!(
        !err.code.trim().is_empty(),
        "R-VERIFY: fail-closed AppError must have a code"
    );
}

#[test]
fn verify_redaction_fail_closed_leaves_source_and_existing_dest() {
    let scratch = Scratch::new("r-verify-publish");
    let source = scratch.pdf("source.pdf");
    let dest = scratch.pdf("out.pdf");
    let staged = scratch.pdf(".offpdf-redact.pdf.tmp");
    write_secret_text(&source);
    std::fs::write(&dest, b"OLD-DEST").unwrap();
    std::fs::copy(&source, &staged).unwrap();

    let result = verify_redaction(&staged, &[b"SECRET".as_slice()], &[cover_secret()]);
    result
        .as_ref()
        .expect_err("R-VERIFY: dest must not publish when the page-content probe remains");

    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&staged).unwrap(),
        "R-VERIFY: source bytes must be unchanged"
    );
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        b"OLD-DEST",
        "R-VERIFY: existing dest must be untouched (same contract as #34 V3)"
    );
    let _ = result;
}

// ---------------------------------------------------------------------------
// keep-green: unredacted page on a mixed redact job
// ---------------------------------------------------------------------------

#[test]
fn unredacted_page_on_mixed_redact_job_keeps_source_stream() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("keep-green-mixed");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_two_page_mixed(&src);
    apply_copy(&src, &dest, &[cover_secret()]).expect("mixed redact apply must succeed");
    assert!(
        dest_has_str(&dest, "KEEPME"),
        "keepGreen: unredacted page 2 on a mixed job must keep its source stream KEEPME; blob={:?}",
        decoded_blob(&dest)
    );
}
