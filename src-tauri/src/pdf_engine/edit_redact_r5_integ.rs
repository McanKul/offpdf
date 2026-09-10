//! PR #98 review-fold r5 lock (R-PAGETYPE).
//!
//! Does not replace `edit_redact_integ.rs`, `edit_redact_r2_integ.rs`,
//! `edit_redact_r3_integ.rs`, or `edit_redact_r4_integ.rs`.

#![cfg(test)]

use crate::error::AppError;
use crate::pdf_engine::edit_overlay::PdfRectIn;
use crate::pdf_engine::edit_redact::{apply_redactions, RedactRegion};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-redact-r5-{}-{}-{}",
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

fn cover_page0() -> RedactRegion {
    region(0, 0.0, 0.0, 612.0, 792.0)
}

fn apply_copy(src: &Path, dest: &Path, regions: &[RedactRegion]) -> Result<(), AppError> {
    std::fs::copy(src, dest).expect("copy dest sibling");
    apply_redactions(dest, regions)
}

fn add_form(doc: &mut Document, body: &[u8]) -> ObjectId {
    let mut fm_dict = Dictionary::new();
    fm_dict.set("Type", "XObject");
    fm_dict.set("Subtype", "Form");
    fm_dict.set("BBox", box_obj([0, 0, 612, 792]));
    doc.add_object(Object::Stream(Stream::new(fm_dict, body.to_vec())))
}

fn dummy_fm0_resources(doc: &mut Document) -> Dictionary {
    let fm0 = add_form(doc, b"% dummy sibling Form\n");
    let mut xobjects = Dictionary::new();
    xobjects.set("Fm0", Object::Reference(fm0));
    let mut resources = Dictionary::new();
    resources.set("XObject", Object::Dictionary(xobjects));
    resources
}

/// Kids[0] `/Type /Page` `(SECRET) Tj`. Kids[1] typeless, own `/Resources /Fm0`,
/// `(KEEPME) Tj`. lopdf `get_pages()` yields only Kids[0].
fn write_typed_page_and_typeless_sibling(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();

    let secret_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n".to_vec(),
    )));
    let mut page0 = Dictionary::new();
    page0.set("Type", "Page");
    page0.set("Parent", pages_id);
    page0.set("MediaBox", box_obj([0, 0, 612, 792]));
    page0.set("Contents", secret_id);
    let p0 = doc.add_object(Object::Dictionary(page0));

    let keep_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n".to_vec(),
    )));
    let sibling_res = dummy_fm0_resources(&mut doc);
    let mut page1 = Dictionary::new();
    page1.set("Parent", pages_id);
    page1.set("MediaBox", box_obj([0, 0, 612, 792]));
    page1.set("Contents", keep_id);
    page1.set("Resources", Object::Dictionary(sibling_res));
    let p1 = doc.add_object(Object::Dictionary(page1));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![p0.into(), p1.into()]);
    pages.set("Count", 2);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .unwrap_or_else(|e| panic!("write typed+typeless-sibling fixture: {e}"));
}

fn dict_from<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn catalog_kid_id(doc: &Document, index: usize) -> Option<ObjectId> {
    let pages_id = doc.catalog().ok()?.get(b"Pages").ok()?.as_reference().ok()?;
    let kids = doc
        .get_dictionary(pages_id)
        .ok()?
        .get(b"Kids")
        .ok()?
        .as_array()
        .ok()?;
    kids.get(index)?.as_reference().ok()
}

fn page_own_resources<'a>(doc: &'a Document, page_id: ObjectId) -> Option<&'a Dictionary> {
    let page = doc.get_dictionary(page_id).ok()?;
    dict_from(doc, page.get(b"Resources").ok()?)
}

fn resources_names_imr(doc: &Document, page_id: ObjectId) -> bool {
    let Some(resources) = page_own_resources(doc, page_id) else {
        return false;
    };
    if resources.get(b"ImR").is_ok() {
        return true;
    }
    let Ok(xo) = resources.get(b"XObject") else {
        return false;
    };
    let Some(xobjects) = dict_from(doc, xo) else {
        return false;
    };
    xobjects.get(b"ImR").is_ok()
}

fn resources_names_fm0(doc: &Document, page_id: ObjectId) -> bool {
    let Some(resources) = page_own_resources(doc, page_id) else {
        return false;
    };
    if resources.get(b"Fm0").is_ok() {
        return true;
    }
    let Ok(xo) = resources.get(b"XObject") else {
        return false;
    };
    let Some(xobjects) = dict_from(doc, xo) else {
        return false;
    };
    xobjects.get(b"Fm0").is_ok()
}

// ---------------------------------------------------------------------------
// R-PAGETYPE — typeless sibling must keep `/Resources` when page 0 is redacted
// ---------------------------------------------------------------------------

#[test]
fn apply_redactions_typeless_sibling_keeps_resources() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-pagetype");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_typed_page_and_typeless_sibling(&src);

    let src_doc = Document::load(&src).expect("load source");
    assert_eq!(
        src_doc.get_pages().len(),
        1,
        "R-PAGETYPE: get_pages() must yield only the /Type /Page kid"
    );
    let src_p0 = catalog_kid_id(&src_doc, 0).expect("Kids[0]");
    let src_p1 = catalog_kid_id(&src_doc, 1).expect("Kids[1]");
    assert_eq!(
        src_doc.get_dictionary(src_p0).ok().and_then(|d| d.type_name().ok()),
        Some("Page"),
        "R-PAGETYPE: Kids[0] must be /Type /Page"
    );
    assert!(
        src_doc
            .get_dictionary(src_p1)
            .expect("Kids[1] dict")
            .get(b"Type")
            .is_err(),
        "R-PAGETYPE: Kids[1] must omit /Type"
    );
    assert!(
        resources_names_fm0(&src_doc, src_p1),
        "R-PAGETYPE: fixture Kids[1] must own /Resources /Fm0"
    );

    let src_before = std::fs::read(&src).expect("read source before apply");
    apply_copy(&src, &dest, &[cover_page0()]).unwrap_or_else(|e| {
        panic!("R-PAGETYPE: apply_redactions must be Ok on /Type /Page 0; {e:?}")
    });
    let src_after = std::fs::read(&src).expect("read source after apply");
    assert_eq!(
        src_before, src_after,
        "R-PAGETYPE: source bytes must be unchanged (copy-then-apply)"
    );

    let dest_doc = Document::load(&dest).expect("load dest");
    let dest_p0 = catalog_kid_id(&dest_doc, 0).expect("dest Kids[0]");
    let dest_p1 = catalog_kid_id(&dest_doc, 1).expect("dest Kids[1]");
    assert!(
        resources_names_imr(&dest_doc, dest_p0),
        "R-PAGETYPE: dest Kids[0] /Resources must name /ImR"
    );
    assert!(
        page_own_resources(&dest_doc, dest_p1).is_some() && resources_names_fm0(&dest_doc, dest_p1),
        "R-PAGETYPE: dest typeless Kids[1] /Resources must still name /Fm0 (must not be stripped as a tree node)"
    );
}
