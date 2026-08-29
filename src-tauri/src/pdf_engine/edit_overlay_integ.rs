//! Real-qpdf Edit PDF fixtures. Skip silently when qpdf is not on PATH.
//! Hand-rolled PDFs live in temp dirs (nothing binary in git).

#![cfg(test)]

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_overlay::{
    export_edit_pdf_with_runner, EditDocumentIn, EditObjectIn, PdfRectIn,
};
use lopdf::{Dictionary, Document, Object, Stream};
use std::path::{Path, PathBuf};

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

fn write_letter_page(path: &Path, extras: &[(&[u8], Object)]) {
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
    doc.save(path).expect("write letter fixture");
}

fn write_visual_page(path: &Path, rotate: i64, crop: Option<[i64; 4]>, trim: Option<[i64; 4]>) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    // All paint is resource-free vector content. The green rectangle is where
    // the edit is placed; the yellow marker proves the far corner survives.
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"q 0 0 1 rg 72 36 468 684 re f Q\nq 0 1 0 rg 82 46 40 30 re f Q\nq 1 1 0 rg 490 660 40 30 re f Q\n".to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    if let Some(b) = crop {
        page.set("CropBox", box_obj(b));
    }
    if let Some(b) = trim {
        page.set("TrimBox", box_obj(b));
    }
    page.set("Rotate", Object::Integer(rotate));
    page.set("Resources", Object::Dictionary(Dictionary::new()));
    page.set("Contents", content_id);
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
    doc.save(path).expect("write visual fixture");
}

fn write_catalog_fixture(path: &Path) {
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
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut item = Dictionary::new();
    item.set("Title", Object::string_literal("Chapter 1"));
    item.set("Dest", vec![page_id.into(), Object::Name(b"Fit".to_vec())]);
    let item_id = doc.add_object(Object::Dictionary(item));

    let mut outlines = Dictionary::new();
    outlines.set("Type", "Outlines");
    outlines.set("First", item_id);
    outlines.set("Last", item_id);
    outlines.set("Count", 1);
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(item_id) {
        d.set("Parent", outlines_id);
    }

    let mut acro = Dictionary::new();
    acro.set("Fields", Vec::<Object>::new());
    let acro_id = doc.add_object(Object::Dictionary(acro));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("Outlines", outlines_id);
    catalog.set("AcroForm", acro_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);

    let mut info = Dictionary::new();
    info.set("Title", Object::string_literal("Fixture Doc"));
    info.set("Author", Object::string_literal("OffPDF"));
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", info_id);
    doc.save(path).expect("write catalog fixture");
}

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
    doc.save(path).expect("write two-page letter fixture");
}

fn write_catalog_annots_fixture(
    path: &Path,
    rotate: i64,
    crop: Option<[i64; 4]>,
    trim: Option<[i64; 4]>,
) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
    )));

    let mut annot = Dictionary::new();
    annot.set("Type", "Annot");
    annot.set("Subtype", "Text");
    annot.set("Rect", box_obj([72, 700, 120, 740]));
    annot.set("Contents", Object::string_literal("note"));
    let annot_id = doc.add_object(Object::Dictionary(annot));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    if let Some(b) = crop {
        page.set("CropBox", box_obj(b));
    }
    if let Some(b) = trim {
        page.set("TrimBox", box_obj(b));
    }
    if rotate != 0 {
        page.set("Rotate", Object::Integer(rotate));
    }
    page.set("Contents", content_id);
    page.set("Annots", vec![annot_id.into()]);
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut item = Dictionary::new();
    item.set("Title", Object::string_literal("Chapter 1"));
    item.set("Dest", vec![page_id.into(), Object::Name(b"Fit".to_vec())]);
    let item_id = doc.add_object(Object::Dictionary(item));

    let mut outlines = Dictionary::new();
    outlines.set("Type", "Outlines");
    outlines.set("First", item_id);
    outlines.set("Last", item_id);
    outlines.set("Count", 1);
    let outlines_id = doc.add_object(Object::Dictionary(outlines));
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(item_id) {
        d.set("Parent", outlines_id);
    }

    let mut acro = Dictionary::new();
    acro.set("Fields", Vec::<Object>::new());
    let acro_id = doc.add_object(Object::Dictionary(acro));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("Outlines", outlines_id);
    catalog.set("AcroForm", acro_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);

    let mut info = Dictionary::new();
    info.set("Title", Object::string_literal("Fixture Doc"));
    info.set("Author", Object::string_literal("OffPDF"));
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", info_id);
    doc.save(path).expect("write catalog+annots fixture");
}

/// Catalog fixture plus leftover `/Highlight` + leftover `/Link` (C3 / C5).
fn write_leftover_highlight_link_fixture(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
    )));

    let mut highlight = Dictionary::new();
    highlight.set("Type", "Annot");
    highlight.set("Subtype", "Highlight");
    highlight.set("Rect", box_obj([100, 200, 180, 240]));
    highlight.set("Contents", Object::string_literal("keep-me"));
    highlight.set(
        "QuadPoints",
        Object::Array(vec![
            Object::Integer(100),
            Object::Integer(200),
            Object::Integer(180),
            Object::Integer(200),
            Object::Integer(180),
            Object::Integer(240),
            Object::Integer(100),
            Object::Integer(240),
        ]),
    );
    let highlight_id = doc.add_object(Object::Dictionary(highlight));

    let mut action = Dictionary::new();
    action.set("S", "URI");
    action.set("URI", Object::string_literal("https://keep.example/"));
    let mut link = Dictionary::new();
    link.set("Type", "Annot");
    link.set("Subtype", "Link");
    link.set("Rect", box_obj([200, 300, 280, 360]));
    link.set("A", Object::Dictionary(action));
    let link_id = doc.add_object(Object::Dictionary(link));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Annots", vec![highlight_id.into(), link_id.into()]);
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
        .expect("write leftover highlight+link fixture");
}

fn write_malformed_annots_fixture(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
    )));

    let mut missing_rect = Dictionary::new();
    missing_rect.set("Type", "Annot");
    missing_rect.set("Subtype", "Text");
    missing_rect.set("Contents", Object::string_literal("no-rect"));
    let missing_id = doc.add_object(Object::Dictionary(missing_rect));

    let mut odd_quads = Dictionary::new();
    odd_quads.set("Type", "Annot");
    odd_quads.set("Subtype", "Highlight");
    odd_quads.set("Rect", box_obj([100, 200, 180, 240]));
    odd_quads.set(
        "QuadPoints",
        Object::Array(vec![
            Object::Integer(100),
            Object::Integer(200),
            Object::Integer(180),
            Object::Integer(200),
            Object::Integer(180),
            Object::Integer(240),
            Object::Integer(100),
        ]),
    );
    odd_quads.set("Contents", Object::string_literal("odd-quads"));
    let odd_id = doc.add_object(Object::Dictionary(odd_quads));

    let mut unknown = Dictionary::new();
    unknown.set("Type", "Annot");
    unknown.set("Subtype", "FooBar");
    unknown.set("Rect", box_obj([10, 10, 40, 40]));
    unknown.set("Contents", Object::string_literal("mystery"));
    let unknown_id = doc.add_object(Object::Dictionary(unknown));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set(
        "Annots",
        vec![missing_id.into(), odd_id.into(), unknown_id.into()],
    );
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
    doc.save(path).expect("write malformed annots fixture");
}

fn dest_annot_subtypes(path: &Path) -> Vec<String> {
    let doc = Document::load(path).expect("load dest");
    let page = first_page_dict(&doc);
    let Ok(annots) = page.get(b"Annots") else {
        return Vec::new();
    };
    let arr = match annots {
        Object::Array(a) => a.clone(),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Array(a)) => a.clone(),
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    arr.iter()
        .filter_map(|raw| {
            let dict = match raw {
                Object::Dictionary(d) => d,
                Object::Reference(id) => doc.get_dictionary(*id).ok()?,
                _ => return None,
            };
            match dict.get(b"Subtype") {
                Ok(Object::Name(n)) => Some(String::from_utf8_lossy(n).into_owned()),
                _ => None,
            }
        })
        .collect()
}

fn assert_catalog_annots_survived(dest: &Path) {
    let out = Document::load(dest).expect("load dest");
    let info = match out.trailer.get(b"Info").ok() {
        Some(Object::Reference(id)) => out.get_dictionary(*id).ok().cloned(),
        Some(Object::Dictionary(d)) => Some(d.clone()),
        _ => None,
    }
    .expect("Info");
    let title = match info.get(b"Title").ok() {
        Some(Object::String(b, _)) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    };
    assert!(title.contains("Fixture Doc"), "title={title:?}");
    let root_id = out.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let cat = out.get_dictionary(root_id).unwrap();
    assert!(cat.get(b"Outlines").is_ok(), "Outlines missing");
    assert!(cat.get(b"AcroForm").is_ok(), "AcroForm missing");
    let page = first_page_dict(&out);
    assert!(page.get(b"Annots").is_ok(), "page /Annots missing");
}

fn page_rotate(dict: &Dictionary) -> i64 {
    match dict.get(b"Rotate") {
        Ok(Object::Integer(i)) => *i,
        _ => 0,
    }
}

fn dump_streams(path: &Path) -> String {
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

/// Content-stream `re` operators only (keeps failure messages readable).
fn listed_re_ops(blob: &str) -> String {
    let mut ops = Vec::new();
    for (i, _) in blob.match_indices(" re") {
        let start = blob[..i]
            .rfind(|c: char| c == '\n' || c == '\r')
            .map(|j| j + 1)
            .unwrap_or(0);
        let frag = blob[start..i + 3].trim();
        if frag
            .split_whitespace()
            .take(4)
            .all(|t| t.parse::<f64>().is_ok())
        {
            ops.push(frag.to_string());
        }
    }
    if ops.is_empty() {
        "<no re ops>".into()
    } else {
        ops.join(" | ")
    }
}

fn first_page_dict(doc: &Document) -> Dictionary {
    let pages = doc.get_pages();
    let id = *pages.get(&1).expect("page 1");
    doc.get_dictionary(id).expect("page dict").clone()
}

fn page_box(dict: &Dictionary, name: &[u8]) -> Option<Vec<f64>> {
    let obj = dict.get(name).ok()?;
    match obj {
        Object::Array(a) => Some(
            a.iter()
                .filter_map(|o| match o {
                    Object::Integer(i) => Some(*i as f64),
                    Object::Real(r) => Some(*r as f64),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn form_streams(path: &Path) -> Vec<(Dictionary, String)> {
    let mut doc = Document::load(path).expect("load pdf");
    let _ = doc.decompress();
    let page_id = *doc.get_pages().get(&1).expect("page 1");
    let page = doc.get_dictionary(page_id).expect("page dict");
    let resources_obj = page.get(b"Resources").expect("page resources");
    let resources = match resources_obj {
        Object::Dictionary(d) => d,
        Object::Reference(id) => doc.get_dictionary(*id).expect("resources dict"),
        _ => panic!("invalid page resources"),
    };
    let xobjects_obj = resources.get(b"XObject").expect("page XObjects");
    let xobjects = match xobjects_obj {
        Object::Dictionary(d) => d,
        Object::Reference(id) => doc.get_dictionary(*id).expect("XObject dict"),
        _ => panic!("invalid page XObjects"),
    };
    xobjects
        .iter()
        .filter_map(|(_, obj)| {
            let id = obj.as_reference().ok()?;
            let stream = doc.get_object(id).ok()?.as_stream().ok()?;
            let bytes = stream
                .get_plain_content()
                .unwrap_or_else(|_| stream.content.clone());
            Some((
                stream.dict.clone(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ))
        })
        .collect()
}

fn form_matrix(dict: &Dictionary) -> [f64; 6] {
    let Some(values) = page_box(dict, b"Matrix") else {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    };
    if values.len() != 6 {
        return [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    }
    [
        values[0], values[1], values[2], values[3], values[4], values[5],
    ]
}

fn transform_rect(rect: [f64; 4], m: [f64; 6]) -> [f64; 4] {
    let [a, b, c, d, e, f] = m;
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[2], rect[3]),
        (rect[0], rect[3]),
    ];
    let mapped = corners.map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let min_x = mapped.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = mapped.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_x = mapped.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let max_y = mapped.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    [min_x, min_y, max_x, max_y]
}

fn assert_rect_near(got: [f64; 4], want: [f64; 4], context: &str) {
    assert!(
        got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.5),
        "{context}: got {got:?}, want {want:?}"
    );
}

fn assert_box_near(dict: &Dictionary, name: &[u8], want: [f64; 4]) {
    let got = page_box(dict, name).unwrap_or_default();
    assert!(
        got.len() == 4
            && (got[0] - want[0]).abs() < 1.0
            && (got[1] - want[1]).abs() < 1.0
            && (got[2] - want[2]).abs() < 1.0
            && (got[3] - want[3]).abs() < 1.0,
        "{} = {got:?}, want {want:?}",
        String::from_utf8_lossy(name)
    );
}

fn page_user_unit(dict: &Dictionary) -> f64 {
    match dict.get(b"UserUnit") {
        Ok(Object::Real(r)) => *r as f64,
        Ok(Object::Integer(i)) => *i as f64,
        _ => 1.0,
    }
}

struct Harness {
    root: PathBuf,
    src: PathBuf,
    dest: PathBuf,
    work: PathBuf,
    qpdf: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Option<Self> {
        let qpdf = test_qpdf()?;
        let root = std::env::temp_dir().join(format!(
            "offpdf-integ-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        Some(Self {
            src: root.join("src.pdf"),
            dest: root.join("out.pdf"),
            work,
            root,
            qpdf,
        })
    }

    fn export(&self, document: &EditDocumentIn) -> Result<(), AppError> {
        self.export_pages("1", document)
    }

    fn export_pages(&self, pages: &str, document: &EditDocumentIn) -> Result<(), AppError> {
        let qpdf = self.qpdf.clone();
        let groups = [PageGroup {
            path: self.src.to_string_lossy().into_owned(),
            pages: pages.into(),
        }];
        export_edit_pdf_with_runner(
            &groups,
            self.dest.to_str().unwrap(),
            document,
            &font_path(),
            &self.work,
            "integ",
            None,
            |args| {
                assert!(!args.iter().any(|a| a == "--empty"), "argv={args:?}");
                let out = std::process::Command::new(&qpdf)
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
            },
        )
        .map(|_| ())
    }

    fn overlay_pdf(&self) -> PathBuf {
        self.work.join("overlay.pdf")
    }

    fn assert_dest_checks(&self) {
        let out = std::process::Command::new(&self.qpdf)
            .args(["--check"])
            .arg(&self.dest)
            .output()
            .expect("run qpdf --check");
        assert!(
            out.status.success(),
            "qpdf --check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text_box() -> EditDocumentIn {
    EditDocumentIn {
        version: 1,
        objects: vec![EditObjectIn::Text {
            page_index: 0,
            rect: PdfRectIn {
                x: 72.0,
                y: 700.0,
                w: 200.0,
                h: 24.0,
            },
            content: "Hello".into(),
            font_size: 14.0,
            color: Some("#111827".into()),
            align: None,
            opacity: None,
            object_rotate: 0.0,
        }],
    }
}

fn filled_rect(x: f64, y: f64, w: f64, h: f64) -> EditDocumentIn {
    EditDocumentIn {
        version: 1,
        objects: vec![EditObjectIn::Rect {
            page_index: 0,
            rect: PdfRectIn { x, y, w, h },
            fill: Some("#ff0000".into()),
            stroke: None,
            stroke_width: None,
            opacity: None,
            object_rotate: 0.0,
        }],
    }
}

fn write_tiny_png(path: &Path, w: u32, h: u32) {
    image::RgbImage::from_pixel(w, h, image::Rgb([20, 180, 40]))
        .save(path)
        .unwrap();
}

// L4 / C3 keepGreen: overlay-only stamp save keeps catalog keys.
#[test]
fn integ_catalog_survives_and_original_stream_stays() {
    let Some(fx) = Harness::new("catalog") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_catalog_fixture(&fx.src);
    let before = std::fs::read(&fx.src).unwrap();
    fx.export(&text_box()).expect("export");
    assert_eq!(std::fs::read(&fx.src).unwrap(), before);
    assert!(fx.dest.exists());

    let out = Document::load(&fx.dest).expect("load dest");
    let info = match out.trailer.get(b"Info").ok() {
        Some(Object::Reference(id)) => out.get_dictionary(*id).ok().cloned(),
        Some(Object::Dictionary(d)) => Some(d.clone()),
        _ => None,
    }
    .expect("Info");
    let title = match info.get(b"Title").ok() {
        Some(Object::String(b, _)) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    };
    assert!(title.contains("Fixture Doc"), "title={title:?}");
    let root_id = out.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let cat = out.get_dictionary(root_id).unwrap();
    assert!(cat.get(b"Outlines").is_ok(), "Outlines missing");
    assert!(cat.get(b"AcroForm").is_ok(), "AcroForm missing");

    let blob = dump_streams(&fx.dest);
    assert!(
        blob.contains("Hello"),
        "original page stream rasterized away: {blob:?}"
    );
    fx.cleanup();
}

#[test]
fn integ_crop_trim_places_rect_at_visible_origin() {
    let Some(fx) = Harness::new("croptrim") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(
        &fx.src,
        &[
            (b"CropBox", box_obj([72, 72, 540, 720])),
            (b"TrimBox", box_obj([0, 0, 612, 792])),
        ],
    );
    // Visible origin is (72,72). Align is full Trim, so overlay must keep 72,72
    // (not 0,0 as if we had used Crop as align).
    fx.export(&filled_rect(72.0, 72.0, 20.0, 10.00))
        .expect("export");
    let overlay = dump_streams(&fx.overlay_pdf());
    assert!(
        overlay.contains("72.00 72.00 20.00 10.00 re"),
        "overlay should stamp visible origin in dest Trim space: {overlay}"
    );
    assert!(
        !overlay.contains("0.00 0.00 20.00 10.00 re"),
        "must not shift by crop origin: {overlay}"
    );
    let dest = dump_streams(&fx.dest);
    assert!(
        dest.contains("72.00 72.00 20.00 10.00 re"),
        "qpdf overlay should carry the rect onto dest: {dest}"
    );
    let page = first_page_dict(&Document::load(&fx.dest).unwrap());
    assert_box_near(&page, b"MediaBox", [0.0, 0.0, 612.0, 792.0]);
    assert_box_near(&page, b"CropBox", [72.0, 72.0, 540.0, 720.0]);
    assert_box_near(&page, b"TrimBox", [0.0, 0.0, 612.0, 792.0]);
    fx.cleanup();
}

#[test]
fn integ_user_unit_not_double_scaled() {
    let Some(fx) = Harness::new("userunit") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[(b"UserUnit", Object::Real(2.0))]);
    fx.export(&filled_rect(72.0, 100.0, 50.0, 20.0))
        .expect("export");

    let dest = Document::load(&fx.dest).expect("load dest");
    let page = first_page_dict(&dest);
    assert!(
        (page_user_unit(&page) - 2.0).abs() < 1e-6,
        "UserUnit stripped"
    );
    let media = page_box(&page, b"MediaBox").unwrap_or_default();
    assert!(
        media.len() == 4 && (media[2] - 612.0).abs() < 1.0 && (media[3] - 792.0).abs() < 1.0,
        "MediaBox scaled unexpectedly: {media:?}"
    );

    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(
        overlay.contains("72.00 100.00 50.00 20.00 re"),
        "rect must stay in raw user units, not 2x: {overlay}"
    );
    assert!(
        dest_ops.contains("72.00 100.00 50.00 20.00 re"),
        "qpdf dest must keep unscaled overlay ops: {dest_ops}"
    );
    assert!(
        !overlay.contains("144.00 200.00 100.00 40.00 re")
            && !dest_ops.contains("144.00 200.00 100.00 40.00 re"),
        "UserUnit must not multiply overlay coords"
    );
    fx.cleanup();
}

#[test]
fn integ_rotate_90_image_meets_not_stretches() {
    let Some(fx) = Harness::new("rot90") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[(b"Rotate", Object::Integer(90))]);
    let png = fx.root.join("wide.png");
    write_tiny_png(&png, 40, 20);
    let rect = PdfRectIn {
        x: 100.0,
        y: 100.0,
        w: 200.0,
        h: 200.0,
    };
    let doc = EditDocumentIn {
        version: 1,
        objects: vec![EditObjectIn::Image {
            page_index: 0,
            rect: rect.clone(),
            path: png.to_string_lossy().into_owned(),
            opacity: None,
            object_rotate: 0.0,
            keep_aspect: Some(true),
        }],
    };
    fx.export(&doc).expect("export");

    let dest_page = first_page_dict(&Document::load(&fx.dest).unwrap());
    let rot = match dest_page.get(b"Rotate") {
        Ok(Object::Integer(i)) => *i,
        _ => 0,
    };
    assert_eq!(rot, 90, "page /Rotate should survive overlay");

    // Independent of pdf_rect_to_overlay / image_meet_blit: 40×20 meet in the
    // displayed 200² AABB after Rotate 90 of unrotated (100,100,200,200).
    const MEET: &str = "200.00 0 0 100.00 100.00 362.00 cm";
    const STRETCH: &str = "200.00 0 0 200.00 100.00 312.00 cm";
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(
        overlay.contains(MEET),
        "expected meet blit {MEET} in {overlay}"
    );
    assert!(
        dest_ops.contains(MEET),
        "qpdf dest missing meet blit {MEET}: {dest_ops}"
    );
    assert!(
        !overlay.contains(STRETCH) && !dest_ops.contains(STRETCH),
        "must not stretch"
    );
    assert!(
        overlay.contains("/Im0 Do") && dest_ops.contains("/Im0 Do"),
        "missing image Do"
    );
    fx.cleanup();
}

#[test]
fn integ_rotate_270_shape_uses_displayed_overlay() {
    let Some(fx) = Harness::new("rot270") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[(b"Rotate", Object::Integer(270))]);
    let rect = PdfRectIn {
        x: 72.0,
        y: 72.0,
        w: 40.0,
        h: 20.0,
    };
    fx.export(&filled_rect(rect.x, rect.y, rect.w, rect.h))
        .expect("export");
    // Independent of pdf_rect_to_overlay: displayed AABB after Rotate 270.
    const EXPECTED: &str = "700.00 72.00 20.00 40.00 re";
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(
        overlay.contains(EXPECTED),
        "expected {EXPECTED} in {overlay}"
    );
    assert!(
        dest_ops.contains(EXPECTED),
        "qpdf dest missing {EXPECTED}: {dest_ops}"
    );
    let dest_page = first_page_dict(&Document::load(&fx.dest).unwrap());
    let rot = match dest_page.get(b"Rotate") {
        Ok(Object::Integer(i)) => *i,
        _ => 0,
    };
    assert_eq!(rot, 270);
    fx.cleanup();
}

#[test]
fn integ_repeated_image_embeds_once() {
    let Some(fx) = Harness::new("repeatimg") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[]);
    let png = fx.root.join("a.png");
    write_tiny_png(&png, 8, 8);
    let path = png.to_string_lossy().into_owned();
    let img = |x: f64| EditObjectIn::Image {
        page_index: 0,
        rect: PdfRectIn {
            x,
            y: 400.0,
            w: 80.0,
            h: 80.0,
        },
        path: path.clone(),
        opacity: None,
        object_rotate: 0.0,
        keep_aspect: Some(true),
    };
    let doc = EditDocumentIn {
        version: 1,
        objects: vec![img(72.0), img(200.0)],
    };
    fx.export(&doc).expect("export");
    let overlay_bytes = std::fs::read(fx.overlay_pdf()).unwrap();
    let overlay_text = String::from_utf8_lossy(&overlay_bytes);
    assert_eq!(
        overlay_text.matches("/Subtype /Image").count(),
        1,
        "{overlay_text}"
    );
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert_eq!(overlay.matches("/Im0 Do").count(), 2, "{overlay}");
    assert_eq!(
        dest_ops.matches("/Im0 Do").count(),
        2,
        "qpdf dest should stamp both Dos: {dest_ops}"
    );
    assert!(!overlay.contains("/Im1 Do") && !dest_ops.contains("/Im1 Do"));
    assert!(fx.dest.exists());
    fx.cleanup();
}

#[test]
fn integ_oversized_jpeg_is_rejected_without_dest() {
    let Some(fx) = Harness::new("bigimg") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[]);
    let jpg = fx.root.join("big.jpg");
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
    b.extend_from_slice(&6000u16.to_be_bytes());
    b.extend_from_slice(&4000u16.to_be_bytes());
    b.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    std::fs::write(&jpg, &b).unwrap();
    let doc = EditDocumentIn {
        version: 1,
        objects: vec![EditObjectIn::Image {
            page_index: 0,
            rect: PdfRectIn {
                x: 72.0,
                y: 400.0,
                w: 100.0,
                h: 80.0,
            },
            path: jpg.to_string_lossy().into_owned(),
            opacity: None,
            object_rotate: 0.0,
            keep_aspect: Some(true),
        }],
    };
    let err = fx.export(&doc).expect_err("oversized must fail");
    assert_eq!(err.code, "IMAGE_TOO_LARGE");
    assert!(
        !fx.dest.exists(),
        "dest must not be created on image reject"
    );
    fx.cleanup();
}

fn write_letter_trim_inside_crop(path: &Path) {
    write_letter_page(
        path,
        &[
            (b"CropBox", box_obj([0, 0, 612, 792])),
            (b"TrimBox", box_obj([100, 100, 400, 500])),
        ],
    );
}

fn assert_dest_boxes_trim_inside_crop(dest: &Path) {
    let page = first_page_dict(&Document::load(dest).unwrap());
    assert_box_near(&page, b"MediaBox", [0.0, 0.0, 612.0, 792.0]);
    assert_box_near(&page, b"CropBox", [0.0, 0.0, 612.0, 792.0]);
    assert_box_near(&page, b"TrimBox", [100.0, 100.0, 400.0, 500.0]);
}

/// Page `/Contents` only — qpdf compositor (`cm` + `Do`), not form XObject bodies.
fn first_page_contents(path: &Path) -> String {
    let mut doc = Document::load(path).expect("load dest");
    let _ = doc.decompress();
    let id = *doc.get_pages().get(&1).expect("page 1");
    let page = doc.get_dictionary(id).expect("page dict");
    let contents = page.get(b"Contents").ok().cloned();
    let mut refs = Vec::new();
    match contents {
        Some(Object::Reference(r)) => refs.push(r),
        Some(Object::Array(a)) => {
            for o in a {
                if let Object::Reference(r) = o {
                    refs.push(r);
                }
            }
        }
        _ => {}
    }
    let mut out = String::new();
    for r in refs {
        if let Ok(Object::Stream(s)) = doc.get_object(r) {
            let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
            out.push_str(&String::from_utf8_lossy(&bytes));
            out.push('\n');
        }
    }
    out
}

/// `a b c d e f cm` matrices from a content stream (whitespace-tokenized).
fn parse_cm_ops(blob: &str) -> Vec<[f64; 6]> {
    let tokens: Vec<&str> = blob.split_whitespace().collect();
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        if tokens[i] != "cm" || i < 6 {
            continue;
        }
        let parsed = [
            tokens[i - 6].parse::<f64>(),
            tokens[i - 5].parse::<f64>(),
            tokens[i - 4].parse::<f64>(),
            tokens[i - 3].parse::<f64>(),
            tokens[i - 2].parse::<f64>(),
            tokens[i - 1].parse::<f64>(),
        ];
        if let [Ok(a), Ok(b), Ok(c), Ok(d), Ok(e), Ok(f)] = parsed {
            out.push([a, b, c, d, e, f]);
        }
    }
    out
}

fn assert_source_overlay_cm_match(path: &Path, context: &str) {
    let cms = parse_cm_ops(&first_page_contents(path));
    assert_eq!(cms.len(), 2, "{context}: expected source + overlay cm ops");
    for i in 0..6 {
        assert!(
            (cms[0][i] - cms[1][i]).abs() < 0.01,
            "{context}: qpdf shifted source and overlay differently: {cms:?}"
        );
    }
}

#[test]
fn integ_trim_inside_crop_stamp_inside_trim_unshifted() {
    let Some(fx) = Harness::new("trim-in") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_trim_inside_crop(&fx.src);
    fx.export(&filled_rect(120.0, 120.0, 20.0, 10.0))
        .expect("export");

    let overlay = dump_streams(&fx.overlay_pdf());
    let dest = dump_streams(&fx.dest);
    assert!(
        overlay.contains("120.00 120.00 20.00 10.00 re"),
        "overlay must keep dest user-space (120,120), not shift by Trim origin −100; re={}",
        listed_re_ops(&overlay)
    );
    assert!(
        dest.contains("120.00 120.00 20.00 10.00 re"),
        "dest must keep the rect at (120,120) in dest user space, not −100; re={}",
        listed_re_ops(&dest)
    );
    assert_dest_boxes_trim_inside_crop(&fx.dest);
    fx.cleanup();
}

#[test]
fn integ_trim_inside_crop_stamp_outside_trim_survives() {
    let Some(fx) = Harness::new("trim-out") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_trim_inside_crop(&fx.src);
    fx.export(&filled_rect(72.0, 72.0, 20.0, 10.0))
        .expect("export");

    let dest = dump_streams(&fx.dest);
    assert!(
        dest.contains("72.00 72.00 20.00 10.00 re"),
        "Crop-visible stamp outside Trim must survive on dest at (72,72); re={}",
        listed_re_ops(&dest)
    );
    assert_dest_boxes_trim_inside_crop(&fx.dest);
    fx.cleanup();
}

/// T1v: dest page-level `cm` must not translate the source page under the stamp.
#[test]
fn integ_t1v_dest_page_cm_no_translation() {
    let Some(fx) = Harness::new("t1v") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_trim_inside_crop(&fx.src);
    fx.export(&filled_rect(120.0, 120.0, 20.0, 10.0))
        .expect("export");

    let overlay = dump_streams(&fx.overlay_pdf());
    let dest = dump_streams(&fx.dest);
    assert!(
        overlay.contains("120.00 120.00 20.00 10.00 re"),
        "overlay must keep dest user-space (120,120); re={}",
        listed_re_ops(&overlay)
    );
    assert!(
        dest.contains("120.00 120.00 20.00 10.00 re"),
        "dest must keep the rect at (120,120) in dest user space; re={}",
        listed_re_ops(&dest)
    );

    let compositor = first_page_contents(&fx.dest);
    assert!(
        compositor.contains("cm") && compositor.contains("Do"),
        "dest page /Contents must be the qpdf compositor (cm + Do), not a form body: {compositor:?}"
    );
    let cms = parse_cm_ops(&compositor);
    assert!(
        !cms.is_empty(),
        "dest page compositor has Do but no parseable `a b c d e f cm`: {compositor:?}"
    );
    for m in &cms {
        let [_, _, _, _, e, f] = *m;
        assert!(
            e.abs() < 0.5 && f.abs() < 0.5,
            "dest page-level cm must not translate the source page (human dest had `1 0 0 1 56 96 cm` before /Fx0 Do); got e={e} f={f} matrix={m:?} compositor={compositor}"
        );
    }
    assert_dest_boxes_trim_inside_crop(&fx.dest);
    fx.cleanup();
}

/// R1: Rotate 90 + Trim ⊂ Crop must map against visible (Crop∩Media), not Trim size.
#[test]
fn integ_r1_rotate_90_trim_inside_crop_uses_visible() {
    let Some(fx) = Harness::new("r1-rot90-trim") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(
        &fx.src,
        &[
            (b"CropBox", box_obj([0, 0, 612, 792])),
            (b"TrimBox", box_obj([100, 100, 400, 500])),
            (b"Rotate", Object::Integer(90)),
        ],
    );
    fx.export(&filled_rect(72.0, 72.0, 20.0, 10.0))
        .expect("export");

    // Independent of pdf_rect_to_overlay: Rotate 90 of (72,72,20,10)
    // against visible 612×792 → (72, 612-72-20, 10, 20).
    const EXPECTED: &str = "72.00 520.00 10.00 20.00 re";
    // Same stamp against Trim 300×400 → (72, 300-72-20, 10, 20).
    const WRONG_TRIM: &str = "72.00 208.00 10.00 20.00 re";
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(
        overlay.contains(EXPECTED),
        "overlay must rotate against visible 612×792, not Trim 300×400; expected {EXPECTED}; re={}",
        listed_re_ops(&overlay)
    );
    assert!(
        dest_ops.contains(EXPECTED),
        "dest must rotate against visible 612×792, not Trim 300×400; expected {EXPECTED}; re={}",
        listed_re_ops(&dest_ops)
    );
    assert!(
        !overlay.contains(WRONG_TRIM) && !dest_ops.contains(WRONG_TRIM),
        "must not use Trim-sized rotation {WRONG_TRIM}; overlay re={} dest re={}",
        listed_re_ops(&overlay),
        listed_re_ops(&dest_ops)
    );

    let dest_page = first_page_dict(&Document::load(&fx.dest).unwrap());
    let rot = match dest_page.get(b"Rotate") {
        Ok(Object::Integer(i)) => *i,
        _ => 0,
    };
    assert_eq!(rot, 90, "page /Rotate should survive overlay");
    assert_dest_boxes_trim_inside_crop(&fx.dest);
    fx.cleanup();
}

#[test]
fn integ_offset_crop_smaller_trim_preserves_source_and_overlay_all_rotations() {
    let cases = [
        (
            0,
            "82.00 46.00 40.00 30.00 re",
            [82.0, 46.0, 122.0, 76.0],
            [72.0, 36.0, 540.0, 720.0],
        ),
        (
            90,
            "46.00 346.00 30.00 40.00 re",
            [46.0, 346.0, 76.0, 386.0],
            [36.0, -72.0, 720.0, 396.0],
        ),
        (
            180,
            "346.00 608.00 40.00 30.00 re",
            [346.0, 608.0, 386.0, 638.0],
            [-72.0, -36.0, 396.0, 648.0],
        ),
        (
            270,
            "608.00 82.00 30.00 40.00 re",
            [608.0, 82.0, 638.0, 122.0],
            [-36.0, 72.0, 648.0, 540.0],
        ),
    ];

    for (rotate, expected_re, expected_rect, expected_overlay_box) in cases {
        let Some(fx) = Harness::new(&format!("offset-crop-rot-{rotate}")) else {
            eprintln!("skip: qpdf not available");
            return;
        };
        write_visual_page(
            &fx.src,
            rotate,
            Some([72, 36, 540, 720]),
            Some([100, 100, 400, 500]),
        );
        fx.export(&filled_rect(82.0, 46.0, 40.0, 30.0))
            .expect("export");

        let out = Document::load(&fx.dest).expect("load dest");
        let page = first_page_dict(&out);
        assert_box_near(&page, b"MediaBox", [0.0, 0.0, 612.0, 792.0]);
        assert_box_near(&page, b"CropBox", [72.0, 36.0, 540.0, 720.0]);
        assert_box_near(&page, b"TrimBox", [100.0, 100.0, 400.0, 500.0]);
        let saved_rotate = match page.get(b"Rotate") {
            Ok(Object::Integer(v)) => *v,
            _ => 0,
        };
        assert_eq!(saved_rotate, rotate, "Rotate {rotate} must survive");

        let forms = form_streams(&fx.dest);
        let (source_dict, source_stream) = forms
            .iter()
            .find(|(_, stream)| stream.contains("0 1 0 rg 82 46 40 30 re f"))
            .expect("source vector Form");
        assert!(
            source_stream.contains("1 1 0 rg 490 660 40 30 re f"),
            "Rotate {rotate}: far source marker was removed"
        );
        assert_box_near(source_dict, b"BBox", [72.0, 36.0, 540.0, 720.0]);

        let (overlay_dict, overlay_stream) = forms
            .iter()
            .find(|(_, stream)| stream.contains("/GS100 gs"))
            .expect("overlay Form");
        assert!(
            overlay_stream.contains(expected_re),
            "Rotate {rotate}: expected {expected_re}; re={}",
            listed_re_ops(overlay_stream)
        );
        assert_box_near(overlay_dict, b"BBox", expected_overlay_box);

        let mapped_source = transform_rect([82.0, 46.0, 122.0, 76.0], form_matrix(source_dict));
        assert_rect_near(
            mapped_source,
            expected_rect,
            &format!("Rotate {rotate}: source marker transform"),
        );

        assert_source_overlay_cm_match(&fx.dest, &format!("Rotate {rotate}"));

        fx.assert_dest_checks();
        fx.cleanup();
    }
}

#[test]
fn integ_box_normalization_restores_missing_crop_and_trim_entries() {
    let Some(no_trim) = Harness::new("restore-no-trim") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_visual_page(&no_trim.src, 0, Some([72, 36, 540, 720]), None);
    no_trim
        .export(&filled_rect(82.0, 46.0, 40.0, 30.0))
        .expect("export without TrimBox");
    let page = first_page_dict(&Document::load(&no_trim.dest).unwrap());
    assert_box_near(&page, b"MediaBox", [0.0, 0.0, 612.0, 792.0]);
    assert_box_near(&page, b"CropBox", [72.0, 36.0, 540.0, 720.0]);
    assert!(page.get(b"TrimBox").is_err(), "must not invent TrimBox");
    assert_source_overlay_cm_match(&no_trim.dest, "missing TrimBox");
    no_trim.assert_dest_checks();
    no_trim.cleanup();

    let Some(no_crop) = Harness::new("restore-no-crop") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_visual_page(&no_crop.src, 0, None, Some([100, 100, 400, 500]));
    no_crop
        .export(&filled_rect(82.0, 46.0, 40.0, 30.0))
        .expect("export without CropBox");
    let page = first_page_dict(&Document::load(&no_crop.dest).unwrap());
    assert_box_near(&page, b"MediaBox", [0.0, 0.0, 612.0, 792.0]);
    assert!(page.get(b"CropBox").is_err(), "must not invent CropBox");
    assert_box_near(&page, b"TrimBox", [100.0, 100.0, 400.0, 500.0]);
    assert_source_overlay_cm_match(&no_crop.dest, "missing CropBox");
    no_crop.assert_dest_checks();
    no_crop.cleanup();
}

#[test]
fn integ_user_unit_10000_copied_not_clamped() {
    let Some(fx) = Harness::new("userunit-10000") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_letter_page(&fx.src, &[(b"UserUnit", Object::Real(10000.0))]);
    fx.export(&filled_rect(72.0, 100.0, 50.0, 20.0))
        .expect("export");

    let dest_doc = Document::load(&fx.dest).expect("load dest");
    let dest_page = first_page_dict(&dest_doc);
    assert!(
        (page_user_unit(&dest_page) - 10000.0).abs() < 1.0,
        "dest UserUnit must remain 10000, got {}",
        page_user_unit(&dest_page)
    );

    let overlay_doc = Document::load(&fx.overlay_pdf()).expect("load overlay");
    let overlay_page = first_page_dict(&overlay_doc);
    assert!(
        (page_user_unit(&overlay_page) - 10000.0).abs() < 1.0,
        "overlay page dict must copy UserUnit 10000, got {}",
        page_user_unit(&overlay_page)
    );

    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(
        overlay.contains("72.00 100.00 50.00 20.00 re"),
        "overlay rect must stay in raw user units, not divided by UserUnit: {overlay}"
    );
    assert!(
        dest_ops.contains("72.00 100.00 50.00 20.00 re"),
        "dest rect must stay in raw user units, not divided by UserUnit: {dest_ops}"
    );
    fx.cleanup();
}

#[test]
fn integ_two_page_overlay_keeps_source_streams_by_presence() {
    let Some(fx) = Harness::new("r1b-twopage") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_two_page_letter(&fx.src, "ALPHA-PAGE", "BETA-PAGE");
    fx.export_pages("1-z", &text_box())
        .expect("R1b: in-order two-page overlay must publish");
    assert!(fx.dest.exists(), "R1b: dest must be published");

    let dest = Document::load(&fx.dest).expect("load dest");
    assert_eq!(dest.get_pages().len(), 2, "R1b: dest must keep two pages");
    let blob = dump_streams(&fx.dest);
    assert!(
        blob.contains("ALPHA-PAGE"),
        "R1b: source page 1 stream must still be present after overlay (Form XObject or Contents): {blob:?}"
    );
    assert!(
        blob.contains("BETA-PAGE"),
        "R1b: source page 2 stream must still be present after overlay (Form XObject or Contents): {blob:?}"
    );
    fx.cleanup();
}

// C keepGreen: overlay stamp save must still leave the fixture /Subtype /Text /Annots key.
#[test]
fn integ_catalog_annots_survive_rotate_90() {
    let Some(fx) = Harness::new("r2-rot90-annots") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_catalog_annots_fixture(&fx.src, 90, None, None);
    fx.export(&text_box())
        .expect("export catalog+annots rotate 90");
    assert_catalog_annots_survived(&fx.dest);
    let page = first_page_dict(&Document::load(&fx.dest).unwrap());
    assert_eq!(
        page_rotate(&page),
        90,
        "page /Rotate 90 must survive overlay"
    );
    fx.cleanup();
}

// C keepGreen: Trim⊂Crop overlay must still leave /Annots.
#[test]
fn integ_catalog_annots_survive_trim_inside_crop() {
    let Some(fx) = Harness::new("r2b-crop-annots") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_catalog_annots_fixture(
        &fx.src,
        0,
        Some([0, 0, 612, 792]),
        Some([100, 100, 400, 500]),
    );
    fx.export(&text_box())
        .expect("export catalog+annots Trim⊂Crop");
    assert_catalog_annots_survived(&fx.dest);
    assert_dest_boxes_trim_inside_crop(&fx.dest);
    fx.cleanup();
}

#[test]
fn integ_leftover_highlight_and_link_survive_stamp_only_save() {
    let Some(fx) = Harness::new("c3-stamp-leftover") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_leftover_highlight_link_fixture(&fx.src);
    fx.export(&text_box())
        .expect("C3: stamp-only save must publish");
    let kinds = dest_annot_subtypes(&fx.dest);
    assert!(
        kinds.iter().any(|s| s == "Highlight"),
        "C3: leftover Highlight must survive stamp-only Save; got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|s| s == "Link"),
        "C3: leftover Link must survive stamp-only Save; got {kinds:?}"
    );
    fx.cleanup();
}

#[test]
fn integ_flatten_default_off_keeps_annots() {
    let Some(fx) = Harness::new("c5-flatten-off") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_leftover_highlight_link_fixture(&fx.src);
    fx.export(&text_box())
        .expect("C5: default Save (flatten off) must publish");
    let page = first_page_dict(&Document::load(&fx.dest).unwrap());
    assert!(
        page.get(b"Annots").is_ok(),
        "C5: flatten default off must keep dest /Annots"
    );
    let kinds = dest_annot_subtypes(&fx.dest);
    assert!(
        kinds.iter().any(|s| s == "Highlight"),
        "C5: flatten default off must keep leftover markup; got {kinds:?}"
    );
    fx.cleanup();
}

#[test]
fn integ_malformed_annots_stamp_save_still_validates() {
    let Some(fx) = Harness::new("c8-malformed-save") else {
        eprintln!("skip: qpdf not available");
        return;
    };
    write_malformed_annots_fixture(&fx.src);
    fx.export(&text_box()).expect(
        "C8: Save of a file with malformed /Annots must still hit validate_staged_pdf and publish",
    );
    let kinds = dest_annot_subtypes(&fx.dest);
    assert!(
        kinds.iter().any(|s| s == "FooBar"),
        "C8: leftover unknown /Subtype must copy through Save; got {kinds:?}"
    );
    fx.cleanup();
}
