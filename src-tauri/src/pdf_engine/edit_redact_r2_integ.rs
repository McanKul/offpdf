//! PR #98 follow-up locks (R-UU, R-MIXDUP, R-FLAT, R-NOROT).
//!
//! Does not replace `edit_redact_integ.rs`. New must-IDs only.

#![cfg(test)]

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_forms::FormValue;
use crate::pdf_engine::edit_overlay::{
    export_edit_pdf_with_runner, export_edit_pdf_with_runner_forms, EditDocumentIn, EditObjectIn,
    PdfRectIn,
};
use crate::pdf_engine::edit_redact::{apply_redactions, verify_redaction, RedactRegion};
use crate::pdf_engine::source_edit_fixtures::write_corpus_fixture;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-redact-r2-{}-{}-{}",
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

fn test_qpdf() -> Option<PathBuf> {
    for c in [
        "/opt/homebrew/bin/qpdf",
        "/usr/local/bin/qpdf",
        "/opt/local/bin/qpdf",
        "/usr/bin/qpdf",
    ] {
        if Path::new(c).exists() {
            return Some(PathBuf::from(c));
        }
    }
    std::process::Command::new("qpdf")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| PathBuf::from("qpdf"))
}

fn font_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf")
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

fn cover_secret() -> RedactRegion {
    region(0, 60.0, 700.0, 160.0, 40.0)
}

fn cover_path() -> RedactRegion {
    region(0, 72.0, 72.0, 100.0, 100.0)
}

fn redact_doc(page_index: u32, x: f64, y: f64, w: f64, h: f64) -> EditDocumentIn {
    EditDocumentIn {
        version: 1,
        objects: vec![EditObjectIn::Redact {
            page_index,
            rect: PdfRectIn { x, y, w, h },
            fill: Some("#000000".into()),
            label: None,
        }],
    }
}

fn apply_copy(src: &Path, dest: &Path, regions: &[RedactRegion]) -> Result<(), AppError> {
    std::fs::copy(src, dest).expect("copy dest sibling");
    apply_redactions(dest, regions)
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

fn write_user_unit_secret(path: &Path) {
    write_one_page(
        path,
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n",
        &[(b"UserUnit", Object::Real(2.0))],
    );
}

/// Two pages, identical `(SECRET) Tj` stream bytes.
fn write_two_page_identical_secret(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let stream = b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n".to_vec();
    let content1 = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), stream.clone())));
    let content2 = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), stream)));

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
    doc.save(path).expect("write identical-sibling fixture");
}

/// Intersecting text widget whose `/V` and `/AP` both carry `FIELD-LEFTOVER`.
fn write_flatten_leftover_field(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 200 Td (Hello) Tj ET\n".to_vec(),
    )));

    let mut ap_dict = Dictionary::new();
    ap_dict.set("Type", "XObject");
    ap_dict.set("Subtype", "Form");
    ap_dict.set("BBox", box_obj([0, 0, 128, 40]));
    let ap_id = doc.add_object(Object::Stream(Stream::new(
        ap_dict,
        b"BT /F1 10 Tf 2 2 Td (FIELD-LEFTOVER) Tj ET\n".to_vec(),
    )));
    let mut n = Dictionary::new();
    n.set("N", Object::Reference(ap_id));

    let mut widget = Dictionary::new();
    widget.set("Type", "Annot");
    widget.set("Subtype", "Widget");
    widget.set("FT", Object::Name(b"Tx".to_vec()));
    widget.set("T", Object::string_literal("Name"));
    widget.set("V", Object::string_literal("FIELD-LEFTOVER"));
    widget.set("Rect", box_obj([72, 700, 200, 740]));
    widget.set("AP", Object::Dictionary(n));
    widget.set("DA", Object::string_literal("/Helv 10 Tf 0 g"));
    widget.set("P", Object::Reference((0, 0)));
    let widget_id = doc.add_object(Object::Dictionary(widget));

    // Non-widget leftover so dest still has /Annots after widget flatten
    // (otherwise #34 INVALID_OUTPUT "Page annotations are missing").
    let mut keep = Dictionary::new();
    keep.set("Type", "Annot");
    keep.set("Subtype", "Text");
    keep.set("Rect", box_obj([72, 100, 192, 140]));
    keep.set("Contents", Object::string_literal("KEEP-ANNOT"));
    keep.set("P", Object::Reference((0, 0)));
    let keep_id = doc.add_object(Object::Dictionary(keep));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Annots", vec![widget_id.into(), keep_id.into()]);
    let page_id = doc.add_object(Object::Dictionary(page));
    if let Ok(Object::Dictionary(w)) = doc.get_object_mut(widget_id) {
        w.set("P", page_id);
    }
    if let Ok(Object::Dictionary(a)) = doc.get_object_mut(keep_id) {
        a.set("P", page_id);
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
    doc.save(path).expect("write flatten leftover-field fixture");
}

fn page_id_at(doc: &Document, page_index: u32) -> Option<ObjectId> {
    doc.get_pages().get(&(page_index + 1)).copied()
}

fn plain_stream(stream: &Stream) -> Vec<u8> {
    stream
        .get_plain_content()
        .or_else(|_| stream.decompressed_content())
        .unwrap_or_else(|_| stream.content.clone())
}

fn dict_from<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

/// Decoded page content + that page's XObject streams (not sibling pages).
fn page_decoded_streams(path: &Path, page_index: u32) -> String {
    let mut doc = Document::load(path).expect("load pdf");
    let _ = doc.decompress();
    let Some(page_id) = page_id_at(&doc, page_index) else {
        return String::new();
    };
    let mut out = Vec::new();
    if let Ok(bytes) = doc.get_page_content(page_id) {
        out.extend_from_slice(&bytes);
        out.push(b'\n');
    }
    for id in doc.get_page_contents(page_id) {
        if let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) {
            out.extend_from_slice(&plain_stream(stream));
            out.push(b'\n');
        }
    }
    if let Ok((inline, inherited)) = doc.get_page_resources(page_id) {
        let mut dicts: Vec<&Dictionary> = Vec::new();
        if let Some(d) = inline {
            dicts.push(d);
        }
        for id in inherited {
            if let Ok(d) = doc.get_dictionary(id) {
                dicts.push(d);
            }
        }
        for resources in dicts {
            let Ok(xo) = resources.get(b"XObject") else {
                continue;
            };
            let Some(xobjects) = dict_from(&doc, xo) else {
                continue;
            };
            for (_, obj) in xobjects.iter() {
                let Ok(id) = obj.as_reference() else {
                    continue;
                };
                let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
                    continue;
                };
                out.extend_from_slice(&plain_stream(stream));
                out.push(b'\n');
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn page_raw_content(path: &Path, page_index: u32) -> Vec<u8> {
    let doc = Document::load(path).expect("load pdf");
    let page_id = page_id_at(&doc, page_index).expect("page");
    doc.get_page_content(page_id).unwrap_or_default()
}

fn dest_has_re_op(path: &Path, page_index: u32, x: f64, y: f64, w: f64, h: f64) -> bool {
    let blob = page_decoded_streams(path, page_index);
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

fn run_qpdf(qpdf: &Path, args: &[String]) -> Result<(), AppError> {
    let out = std::process::Command::new(qpdf)
        .args(args)
        .output()
        .map_err(|e| AppError::io("qpdf failed to start", e))?;
    let code = out.status.code();
    if out.status.success() || code == Some(3) {
        Ok(())
    } else {
        Err(AppError::engine_failed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ))
    }
}

fn export_redact(
    qpdf: &Path,
    src: &Path,
    dest: &Path,
    work: &Path,
    document: &EditDocumentIn,
    pages: &str,
) -> Result<Vec<String>, AppError> {
    let groups = [PageGroup {
        path: src.to_string_lossy().into_owned(),
        pages: pages.into(),
    }];
    export_edit_pdf_with_runner(
        &groups,
        dest.to_str().unwrap(),
        document,
        &font_path(),
        work,
        "r2",
        None,
        |args| run_qpdf(qpdf, args),
    )
}

fn export_redact_flatten(
    qpdf: &Path,
    src: &Path,
    dest: &Path,
    work: &Path,
    document: &EditDocumentIn,
) -> Result<(Vec<String>, Vec<String>), AppError> {
    let groups = [PageGroup {
        path: src.to_string_lossy().into_owned(),
        pages: "1-z".into(),
    }];
    export_edit_pdf_with_runner_forms(
        &groups,
        dest.to_str().unwrap(),
        document,
        &font_path(),
        work,
        "r2-flat",
        &[] as &[FormValue],
        true,
        |args| run_qpdf(qpdf, args),
    )
}

// ---------------------------------------------------------------------------
// R-UU — /UserUnit ≠ 1 must apply_redactions successfully
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_user_unit_2_drops_secret_show_string() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-uu-secret");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_user_unit_secret(&src);
    apply_copy(&src, &dest, &[cover_secret()]).expect(
        "R-UU: apply_redactions must succeed on /UserUnit 2 (pdftoppm raster is 2× MediaBox)",
    );
    assert!(dest.is_file(), "R-UU: dest must be published");
    let page0 = page_decoded_streams(&dest, 0);
    assert!(
        !page0.contains("SECRET"),
        "R-UU: dest page-0 streams must not contain source show-string SECRET; blob={page0:?}"
    );
}

#[test]
fn apply_redactions_corpus_geom_user_unit() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-uu-corpus");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture("geom-user-unit", &src).expect("write geom-user-unit");
    apply_copy(&src, &dest, &[cover_path()]).expect(
        "R-UU: apply_redactions must succeed on corpus geom-user-unit (/UserUnit 2)",
    );
    assert!(dest.is_file(), "R-UU: dest must be published");
    assert!(
        !dest_has_re_op(&dest, 0, 72.0, 72.0, 100.0, 100.0),
        "R-UU: dest page-0 must not keep 72 72 100 100 re; blob={:?}",
        page_decoded_streams(&dest, 0)
    );
}

// ---------------------------------------------------------------------------
// R-MIXDUP — identical content on an unredacted sibling must not fail-close
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_identical_sibling_secret_does_not_fail_close() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-mixdup-verify");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_two_page_identical_secret(&src);
    // Same probe collect_redact_probes takes today: the whole page-0 stream.
    let probe = page_raw_content(&src, 0);
    assert!(
        probe.windows(b"SECRET".len()).any(|w| w == b"SECRET"),
        "fixture page 0 must contain SECRET"
    );
    apply_copy(&src, &dest, &[cover_secret()])
        .expect("R-MIXDUP: apply_redactions (page 0 only) must succeed");
    verify_redaction(&dest, &[probe.as_slice()], &[cover_secret()]).expect(
        "R-MIXDUP: verify must not fail-close because an unredacted sibling still has SECRET",
    );
    assert!(dest.is_file(), "R-MIXDUP: dest must exist");
    let page0 = page_decoded_streams(&dest, 0);
    let page1 = page_decoded_streams(&dest, 1);
    assert!(
        !page0.contains("SECRET"),
        "R-MIXDUP: page 0 dest streams must lack SECRET; blob={page0:?}"
    );
    assert!(
        page1.contains("SECRET"),
        "R-MIXDUP: page 1 dest streams must still have SECRET; blob={page1:?}"
    );
}

#[test]
fn export_redact_page0_keeps_identical_sibling_secret() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-mixdup-export");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_two_page_identical_secret(&src);
    export_redact(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(0, 60.0, 700.0, 160.0, 40.0),
        "1-z",
    )
    .expect("R-MIXDUP: export apply+verify must succeed when only page 0 is redacted");
    assert!(dest.is_file(), "R-MIXDUP: dest must be published");
    let page0 = page_decoded_streams(&dest, 0);
    let page1 = page_decoded_streams(&dest, 1);
    assert!(
        !page0.contains("SECRET"),
        "R-MIXDUP: page 0 dest streams must lack SECRET; blob={page0:?}"
    );
    assert!(
        page1.contains("SECRET"),
        "R-MIXDUP: page 1 dest streams must still have SECRET; blob={page1:?}"
    );
}

// ---------------------------------------------------------------------------
// R-FLAT — flatten-after-redact must not restore intersecting field /AP text
// ---------------------------------------------------------------------------

#[test]
fn flatten_after_redact_does_not_restore_field_leftover() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-flat");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_flatten_leftover_field(&src);
    let result = export_redact_flatten(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(0, 60.0, 700.0, 160.0, 40.0),
    );
    // Flatten-before-redact (or an explicit flatten+redact refusal) are both
    // acceptable. Publishing dest whose page-content still has FIELD-LEFTOVER
    // is not. Other export errors (e.g. INVALID_OUTPUT) are not a refusal.
    match result {
        Ok(_) => {
            assert!(dest.is_file(), "R-FLAT: dest must be published on Ok");
            let page0 = page_decoded_streams(&dest, 0);
            assert!(
                !page0.contains("FIELD-LEFTOVER"),
                "R-FLAT: dest page-content must not contain intersecting field /V appearance text FIELD-LEFTOVER after flatten+redact (flatten painted /Ff0 after /ImR); has_ff0={} has_imr={}",
                page0.contains("/Ff0"),
                page0.contains("/ImR")
            );
        }
        Err(e) => {
            assert!(
                !dest.is_file(),
                "R-FLAT: failed flatten+redact must not publish dest"
            );
            let blob = format!("{} {} {}", e.code, e.title, e.message).to_lowercase();
            assert!(
                (blob.contains("flatten") && blob.contains("redact"))
                    || blob.contains("flatten+redact")
                    || blob.contains("flatten_redact"),
                "R-FLAT: refuse flatten+redact must say so; got {e:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// R-NOROT — redact preview rotation must not exist / must not affect burn
// ---------------------------------------------------------------------------

#[test]
fn redact_kind_serde_has_no_object_rotate() {
    let json = r##"{"version":1,"objects":[{"kind":"redact","pageIndex":0,"rect":{"x":72,"y":700,"w":120,"h":40},"fill":"#000000","objectRotate":45}]}"##;
    let d: EditDocumentIn =
        serde_json::from_str(json).expect("R-NOROT: redact JSON must deserialize");
    assert_eq!(d.objects.len(), 1);
    match &d.objects[0] {
        EditObjectIn::Redact {
            page_index, rect, ..
        } => {
            assert_eq!(*page_index, 0);
            assert!((rect.x - 72.0).abs() < 1e-6);
            assert!((rect.y - 700.0).abs() < 1e-6);
            assert!((rect.w - 120.0).abs() < 1e-6);
            assert!((rect.h - 40.0).abs() < 1e-6);
        }
        other => panic!("R-NOROT: expected Redact, got {other:?}"),
    }
    let dbg = format!("{:?}", d.objects[0]);
    assert!(
        !dbg.contains("object_rotate"),
        "R-NOROT: EditObjectIn::Redact must not carry objectRotate (Rust burns AABB only); got {dbg}"
    );
}
