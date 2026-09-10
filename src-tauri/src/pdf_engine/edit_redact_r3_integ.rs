//! PR #98 review-fold r3 locks (R-EMPTY, R-INHERIT, R-FORMV, R-FATT).
//!
//! Does not replace `edit_redact_integ.rs` or `edit_redact_r2_integ.rs`.

#![cfg(test)]

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_forms::FormValue;
use crate::pdf_engine::edit_overlay::{
    export_edit_pdf_with_runner, export_edit_pdf_with_runner_forms, EditDocumentIn, EditObjectIn,
    PdfRectIn,
};
use crate::pdf_engine::edit_redact::{apply_redactions, verify_redaction, RedactRegion};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-redact-r3-{}-{}-{}",
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

fn out_of_range() -> RedactRegion {
    region(99, 60.0, 700.0, 160.0, 40.0)
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

fn write_secret_text(path: &Path) {
    write_one_page(path, b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n", &[]);
}

/// Pages `/Resources /XObject /Fm0` holds `(SECRET) Tj`; page Contents is `/Fm0 Do`.
fn write_inherited_form_secret(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let mut fm_dict = Dictionary::new();
    fm_dict.set("Type", "XObject");
    fm_dict.set("Subtype", "Form");
    fm_dict.set("BBox", box_obj([0, 0, 612, 792]));
    let fm_id = doc.add_object(Object::Stream(Stream::new(
        fm_dict,
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n".to_vec(),
    )));

    let mut xobjects = Dictionary::new();
    xobjects.set("Fm0", Object::Reference(fm_id));
    let mut resources = Dictionary::new();
    resources.set("XObject", Object::Dictionary(xobjects));

    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"/Fm0 Do\n".to_vec(),
    )));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    pages.set("Resources", Object::Dictionary(resources));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write inherited-form fixture");
}

/// Widget `/V` LEAKFIELD at `[72 700 200 740]` plus a non-intersecting leftover annot
/// so flatten does not trip #34 INVALID_OUTPUT “Page annotations are missing”.
fn write_flatten_leakfield(path: &Path) {
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
        b"BT /F1 10 Tf 2 2 Td (AP) Tj ET\n".to_vec(),
    )));
    let mut n = Dictionary::new();
    n.set("N", Object::Reference(ap_id));

    let mut widget = Dictionary::new();
    widget.set("Type", "Annot");
    widget.set("Subtype", "Widget");
    widget.set("FT", Object::Name(b"Tx".to_vec()));
    widget.set("T", Object::string_literal("Name"));
    widget.set("V", Object::string_literal("LEAKFIELD"));
    widget.set("Rect", box_obj([72, 700, 200, 740]));
    widget.set("AP", Object::Dictionary(n));
    widget.set("DA", Object::string_literal("/Helv 10 Tf 0 g"));
    widget.set("P", Object::Reference((0, 0)));
    let widget_id = doc.add_object(Object::Dictionary(widget));

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
    doc.save(path).expect("write flatten leakfield fixture");
}

/// Page `/FileAttachment` with `/FS` `/EF` `ATTACH-SECRET`. No catalog `/AF` or EmbeddedFiles.
fn write_file_attachment(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n".to_vec(),
    )));

    let mut ef_dict = Dictionary::new();
    ef_dict.set("Type", "EmbeddedFile");
    let ef_id = doc.add_object(Object::Stream(Stream::new(
        ef_dict,
        b"ATTACH-SECRET".to_vec(),
    )));

    let mut ef = Dictionary::new();
    ef.set("F", Object::Reference(ef_id));
    let mut fs = Dictionary::new();
    fs.set("Type", "Filespec");
    fs.set("F", Object::string_literal("secret.txt"));
    fs.set("EF", Object::Dictionary(ef));
    let fs_id = doc.add_object(Object::Dictionary(fs));

    let mut annot = Dictionary::new();
    annot.set("Type", "Annot");
    annot.set("Subtype", "FileAttachment");
    annot.set("Rect", box_obj([72, 700, 192, 740]));
    annot.set("FS", Object::Reference(fs_id));
    annot.set("P", Object::Reference((0, 0)));
    let annot_id = doc.add_object(Object::Dictionary(annot));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Annots", vec![annot_id.into()]);
    let page_id = doc.add_object(Object::Dictionary(page));
    if let Ok(Object::Dictionary(a)) = doc.get_object_mut(annot_id) {
        a.set("P", page_id);
    }

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
    doc.save(path).expect("write file-attachment fixture");
}

fn page_id_at(doc: &Document, page_index: u32) -> Option<ObjectId> {
    doc.get_pages().get(&(page_index + 1)).copied()
}

fn page_raw_content(path: &Path, page_index: u32) -> Vec<u8> {
    let doc = Document::load(path).expect("load pdf");
    let page_id = page_id_at(&doc, page_index).expect("page");
    doc.get_page_content(page_id).unwrap_or_default()
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

fn warnings_blob(warnings: &[String]) -> String {
    warnings.join(" | ")
}

fn mentions_form_or_field(warnings: &[String]) -> bool {
    warnings.iter().any(|w| {
        let l = w.to_lowercase();
        l.contains("form") || l.contains("field")
    })
}

fn dest_page_has_widget(path: &Path) -> bool {
    let Ok(doc) = Document::load(path) else {
        return false;
    };
    let Some(page_id) = page_id_at(&doc, 0) else {
        return false;
    };
    let Ok(page) = doc.get_dictionary(page_id) else {
        return false;
    };
    let Ok(Object::Array(arr)) = page.get(b"Annots") else {
        return false;
    };
    for obj in arr {
        let dict = match obj {
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            Object::Dictionary(d) => Some(d),
            _ => None,
        };
        if let Some(d) = dict {
            if d.get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .is_some_and(|n| n == b"Widget")
            {
                return true;
            }
        }
    }
    false
}

fn mentions_attachment(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.to_lowercase().contains("attach"))
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
) -> Result<Vec<String>, AppError> {
    let groups = [PageGroup {
        path: src.to_string_lossy().into_owned(),
        pages: "1-z".into(),
    }];
    export_edit_pdf_with_runner(
        &groups,
        dest.to_str().unwrap(),
        document,
        &font_path(),
        work,
        "r3",
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
        "r3-flat",
        &[] as &[FormValue],
        true,
        |args| run_qpdf(qpdf, args),
    )
}

fn assert_fail_closed_code(err: &AppError, id: &str) {
    let blob = format!("{} {} {}", err.code, err.title, err.message).to_uppercase();
    assert!(
        blob.contains("REDACTION_INCOMPLETE")
            || (blob.contains("PAGE")
                && (blob.contains("NOT")
                    || blob.contains("MISSING")
                    || blob.contains("FOUND")
                    || blob.contains("RANGE"))),
        "{id}: Err must be REDACTION_INCOMPLETE or a page-not-found code; got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// R-EMPTY — out-of-range page_index is not a successful redaction
// ---------------------------------------------------------------------------

#[test]
fn verify_empty_probes_out_of_range_page_must_err() {
    let scratch = Scratch::new("r-empty-verify");
    let dest = scratch.pdf("dest.pdf");
    write_secret_text(&dest);
    let err = verify_redaction(&dest, &[], &[out_of_range()]).expect_err(
        "R-EMPTY: verify_redaction must not treat an empty probe list as success when the job had regions (page_index=99)",
    );
    assert_fail_closed_code(&err, "R-EMPTY");
    assert!(
        dest_has_str(&dest, "SECRET"),
        "R-EMPTY: dest must still hold source SECRET after fail-closed verify"
    );
}

#[test]
fn apply_redactions_out_of_range_page_must_err() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-empty-apply");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_secret_text(&src);
    match apply_copy(&src, &dest, &[out_of_range()]) {
        Ok(()) => panic!(
            "R-EMPTY: apply_redactions must Err for page_index=99 on a one-page PDF (empty rewrite is not success); dest_has_SECRET={}",
            dest.is_file() && dest_has_str(&dest, "SECRET")
        ),
        Err(e) => {
            assert_fail_closed_code(&e, "R-EMPTY");
            if dest.is_file() {
                assert!(
                    dest_has_str(&dest, "SECRET"),
                    "R-EMPTY: source SECRET still in dest if a dest file was written"
                );
            }
        }
    }
}

#[test]
fn export_out_of_range_page_must_not_publish_dest() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-empty-export");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_secret_text(&src);
    match export_redact(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(99, 60.0, 700.0, 160.0, 40.0),
    ) {
        Ok(_) => panic!(
            "R-EMPTY: export_edit_pdf_with_runner must not publish dest Ok for page_index=99; dest_exists={} dest_has_SECRET={}",
            dest.is_file(),
            dest.is_file() && dest_has_str(&dest, "SECRET")
        ),
        Err(e) => {
            assert_fail_closed_code(&e, "R-EMPTY");
            assert!(
                !dest.is_file(),
                "R-EMPTY: dest must not be published as a successful redaction"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// R-INHERIT — inherited Pages Form XObject is page content
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_inherited_form_xobject_must_not_keep_secret() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-inherit-apply");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_inherited_form_secret(&src);
    let probe = page_raw_content(&src, 0);
    let probe_s = String::from_utf8_lossy(&probe);
    assert!(
        probe_s.contains("/Fm0") && probe_s.contains("Do"),
        "R-INHERIT fixture Contents must be /Fm0 Do; got {probe_s:?}"
    );
    assert!(
        !probe.windows(b"SECRET".len()).any(|w| w == b"SECRET"),
        "R-INHERIT fixture Contents-only probe must not already be SECRET; got {probe_s:?}"
    );
    match apply_copy(&src, &dest, &[cover_secret()]) {
        Ok(()) => {
            match verify_redaction(&dest, &[probe.as_slice()], &[cover_secret()]) {
                Ok(_) => {
                    assert!(
                        dest.is_file(),
                        "R-INHERIT: dest must exist after apply+verify Ok"
                    );
                    assert!(
                        !dest_has_str(&dest, "SECRET"),
                        "R-INHERIT: dest must not be published Ok while any dest object still contains SECRET"
                    );
                }
                Err(_) => {
                    if dest.is_file() {
                        // Fail-closed is acceptable; dest is the in-place working copy.
                    }
                }
            }
        }
        Err(_) => {
            // Fail-closed apply is acceptable; dest is the in-place working copy.
        }
    }
}

#[test]
fn export_inherited_form_xobject_must_not_publish_secret() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-inherit-export");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_inherited_form_secret(&src);
    match export_redact(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(0, 60.0, 700.0, 160.0, 40.0),
    ) {
        Ok(_) => {
            assert!(dest.is_file(), "R-INHERIT: dest must exist on export Ok");
            assert!(
                !dest_has_str(&dest, "SECRET"),
                "R-INHERIT: dest must not be published Ok while any dest object still contains SECRET"
            );
        }
        Err(_) => {
            assert!(
                !dest.is_file(),
                "R-INHERIT: fail-closed export must not publish dest"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// R-FORMV — leftover field /V after flatten-before-burn must warn
// ---------------------------------------------------------------------------

#[test]
fn flatten_redact_leftover_field_v_must_warn() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-formv");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_flatten_leakfield(&src);
    let (paths, warnings) = export_redact_flatten(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(0, 60.0, 700.0, 160.0, 40.0),
    )
    .expect("R-FORMV: flatten+redact export must complete so leftover /V can warn (not fail-closed)");
    assert!(
        dest.is_file() || !paths.is_empty(),
        "R-FORMV: dest must be published on Ok"
    );
    if dest.is_file() {
        assert!(
            dest_has_str(&dest, "LEAKFIELD"),
            "R-FORMV: leftover field /V may remain; do not auto-strip LEAKFIELD"
        );
    }
    assert!(
        dest.is_file() || !paths.is_empty(),
        "R-FORMV: dest must be published on Ok"
    );
    if dest.is_file() {
        assert!(
            !dest_page_has_widget(&dest),
            "R-FORMV: fixture must flatten the widget off page /Annots so leftover catalog /V is what is locked"
        );
        assert!(
            dest_has_str(&dest, "LEAKFIELD"),
            "R-FORMV: leftover field /V may remain; do not auto-strip LEAKFIELD"
        );
    }
    assert!(
        mentions_form_or_field(&warnings),
        "R-FORMV: warnings must mention form/field after flatten+redact leftover /V; got {:?}",
        warnings_blob(&warnings)
    );
}

// ---------------------------------------------------------------------------
// R-FATT — page FileAttachment is an attachment leftover (warn, do not strip)
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_file_attachment_must_warn() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-fatt");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_file_attachment(&src);
    let probe = page_raw_content(&src, 0);
    apply_copy(&src, &dest, &[cover_secret()]).expect("R-FATT: apply_redactions must succeed");
    let warnings = verify_redaction(&dest, &[probe.as_slice()], &[cover_secret()])
        .expect("R-FATT: leftover FileAttachment must warn, not fail-closed and not auto-strip");
    assert!(
        dest_has_str(&dest, "ATTACH-SECRET"),
        "R-FATT: dest must still have ATTACH-SECRET (do not auto-strip)"
    );
    assert!(
        mentions_attachment(&warnings),
        "R-FATT: leftover page FileAttachment must produce an attachment warning; got {:?}",
        warnings_blob(&warnings)
    );
}
