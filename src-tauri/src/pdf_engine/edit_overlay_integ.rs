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
        let qpdf = self.qpdf.clone();
        let groups = [PageGroup {
            path: self.src.to_string_lossy().into_owned(),
            pages: "1".into(),
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
                    Err(AppError::engine_failed(String::from_utf8_lossy(&out.stderr).to_string()))
                }
            },
        )
        .map(|_| ())
    }

    fn overlay_pdf(&self) -> PathBuf {
        self.work.join("overlay.pdf")
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
    assert!(blob.contains("Hello"), "original page stream rasterized away: {blob:?}");
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
    fx.export(&filled_rect(72.0, 72.0, 20.0, 10.00)).expect("export");
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
    fx.export(&filled_rect(72.0, 100.0, 50.0, 20.0)).expect("export");

    let dest = Document::load(&fx.dest).expect("load dest");
    let page = first_page_dict(&dest);
    assert!((page_user_unit(&page) - 2.0).abs() < 1e-6, "UserUnit stripped");
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
    assert!(overlay.contains(MEET), "expected meet blit {MEET} in {overlay}");
    assert!(dest_ops.contains(MEET), "qpdf dest missing meet blit {MEET}: {dest_ops}");
    assert!(!overlay.contains(STRETCH) && !dest_ops.contains(STRETCH), "must not stretch");
    assert!(overlay.contains("/Im0 Do") && dest_ops.contains("/Im0 Do"), "missing image Do");
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
    fx.export(&filled_rect(rect.x, rect.y, rect.w, rect.h)).expect("export");
    // Independent of pdf_rect_to_overlay: displayed AABB after Rotate 270.
    const EXPECTED: &str = "700.00 72.00 20.00 40.00 re";
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert!(overlay.contains(EXPECTED), "expected {EXPECTED} in {overlay}");
    assert!(dest_ops.contains(EXPECTED), "qpdf dest missing {EXPECTED}: {dest_ops}");
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
    assert_eq!(overlay_text.matches("/Subtype /Image").count(), 1, "{overlay_text}");
    let overlay = dump_streams(&fx.overlay_pdf());
    let dest_ops = dump_streams(&fx.dest);
    assert_eq!(overlay.matches("/Im0 Do").count(), 2, "{overlay}");
    assert_eq!(dest_ops.matches("/Im0 Do").count(), 2, "qpdf dest should stamp both Dos: {dest_ops}");
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
    assert!(!fx.dest.exists(), "dest must not be created on image reject");
    fx.cleanup();
}
