//! PR #98 review-fold r4 locks (R-SHARE, R-ZIP, R-GEOM-PX, hunt IDs).
//!
//! Does not replace `edit_redact_integ.rs`, `edit_redact_r2_integ.rs`,
//! or `edit_redact_r3_integ.rs`.

#![cfg(test)]

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_overlay::{
    export_edit_pdf_with_runner, EditDocumentIn, EditObjectIn, PdfRectIn,
};
use crate::pdf_engine::edit_redact::{
    apply_redactions, collect_redact_probes_for_pages, verify_redaction, RedactRegion,
};
use crate::pdf_engine::source_edit_fixtures::write_corpus_fixture;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::path::{Path, PathBuf};

/// Corpus `image-unique` / R-IMAGE samples (`source_edit_fixtures.rs`).
const TINY_RGB: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];

const FORM_ZIP_PROBE: &[u8] = b"FORM-ZIP-PROBE";

/// Distinctive burn (not `#000000`; corpus geom fixtures already paint that rect black).
const GEOM_FILL: &str = "#CC3344";
const GEOM_FILL_RGB: [u8; 3] = [0xCC, 0x33, 0x44];
/// Unrotated `{72,72,100,100}` interior in MediaBox pixel space (72 DPI, UU=1).
const GEOM_INTERIOR: (u32, u32) = (122, 670);
const GEOM_CONTROL: (u32, u32) = (20, 20);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-redact-r4-{}-{}-{}",
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

fn region_fill(page_index: u32, x: f64, y: f64, w: f64, h: f64, fill: &str) -> RedactRegion {
    RedactRegion {
        page_index,
        rect: PdfRectIn { x, y, w, h },
        fill: Some(fill.into()),
        label: None,
    }
}

fn cover_secret() -> RedactRegion {
    region(0, 60.0, 700.0, 160.0, 40.0)
}

fn cover_path_fill() -> RedactRegion {
    region_fill(0, 72.0, 72.0, 100.0, 100.0, GEOM_FILL)
}

fn zip_regions() -> [RedactRegion; 2] {
    [
        region(0, 72.0, 72.0, 100.0, 100.0),
        region(1, 72.0, 72.0, 100.0, 100.0),
    ]
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

fn add_form(doc: &mut Document, body: &[u8]) -> ObjectId {
    let mut fm_dict = Dictionary::new();
    fm_dict.set("Type", "XObject");
    fm_dict.set("Subtype", "Form");
    fm_dict.set("BBox", box_obj([0, 0, 612, 792]));
    doc.add_object(Object::Stream(Stream::new(fm_dict, body.to_vec())))
}

fn add_tiny_rgb_image(doc: &mut Document) -> ObjectId {
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", "Image");
    d.set("Width", 2);
    d.set("Height", 2);
    d.set("ColorSpace", "DeviceRGB");
    d.set("BitsPerComponent", 8);
    doc.add_object(Object::Stream(Stream::new(d, TINY_RGB.to_vec())))
}

fn xobject_resources(entries: &[(&str, ObjectId)]) -> Dictionary {
    let mut xobjects = Dictionary::new();
    for (name, id) in entries {
        xobjects.set(*name, Object::Reference(*id));
    }
    let mut resources = Dictionary::new();
    resources.set("XObject", Object::Dictionary(xobjects));
    resources
}

fn add_page(
    doc: &mut Document,
    pages_id: ObjectId,
    content: &[u8],
    resources: Option<Dictionary>,
) -> ObjectId {
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    if let Some(res) = resources {
        page.set("Resources", Object::Dictionary(res));
    }
    doc.add_object(Object::Dictionary(page))
}

fn finish_two_page(
    doc: &mut Document,
    pages_id: ObjectId,
    p0: ObjectId,
    p1: ObjectId,
    pages_resources: Option<Dictionary>,
    path: &Path,
    what: &str,
) {
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![p0.into(), p1.into()]);
    pages.set("Count", 2);
    if let Some(res) = pages_resources {
        pages.set("Resources", Object::Dictionary(res));
    }
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path).unwrap_or_else(|e| panic!("write {what}: {e}"));
}

/// Pages `/Fm0` = `(SECRET) Tj`; page 0 `/Fm0 Do`; page 1 `(KEEPME)` no `/Fm0`.
fn write_share_inherited_form(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let fm0 = add_form(&mut doc, b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n");
    let p0 = add_page(&mut doc, pages_id, b"/Fm0 Do\n", None);
    let p1 = add_page(
        &mut doc,
        pages_id,
        b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n",
        None,
    );
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        Some(xobject_resources(&[("Fm0", fm0)])),
        path,
        "R-SHARE inherited-form fixture",
    );
}

/// Each page has own `/Resources /Fm0` → the same Form `(SECRET) Tj`.
fn write_share_own_form(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let fm0 = add_form(&mut doc, b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n");
    let p0 = add_page(
        &mut doc,
        pages_id,
        b"/Fm0 Do\n",
        Some(xobject_resources(&[("Fm0", fm0)])),
    );
    let p1 = add_page(
        &mut doc,
        pages_id,
        b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n",
        Some(xobject_resources(&[("Fm0", fm0)])),
    );
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        None,
        path,
        "R-SHARE-OWN own-form fixture",
    );
}

/// Pages `/Fm0` SECRET and `/Fm1` KEEPME; page 0 `/Fm0 Do`; page 1 `/Fm1 Do`.
fn write_share_mix_forms(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let fm0 = add_form(&mut doc, b"BT /F1 12 Tf 72 720 Td (SECRET) Tj ET\n");
    let fm1 = add_form(&mut doc, b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n");
    let p0 = add_page(&mut doc, pages_id, b"/Fm0 Do\n", None);
    let p1 = add_page(&mut doc, pages_id, b"/Fm1 Do\n", None);
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        Some(xobject_resources(&[("Fm0", fm0), ("Fm1", fm1)])),
        path,
        "R-SHARE-MIX inherited-forms fixture",
    );
}

/// Pages `/Im0` = TINY_RGB; page 0 `/Im0 Do`; page 1 `(KEEPME)` no `/Im0`.
fn write_share_inherited_image(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let im0 = add_tiny_rgb_image(&mut doc);
    let p0 = add_page(
        &mut doc,
        pages_id,
        b"q 40 0 0 40 72 400 cm /Im0 Do Q\n",
        None,
    );
    let p1 = add_page(
        &mut doc,
        pages_id,
        b"BT /F1 12 Tf 72 720 Td (KEEPME) Tj ET\n",
        None,
    );
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        Some(xobject_resources(&[("Im0", im0)])),
        path,
        "R-SHARE-IMG inherited-image fixture",
    );
}

/// Page 0: non-empty Contents + Form body. Page 1: empty Contents.
fn write_zip_content_and_form(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let fm0 = add_form(
        &mut doc,
        b"BT /F1 12 Tf 72 200 Td (FORM-ZIP-PROBE) Tj ET\n",
    );
    let p0 = add_page(
        &mut doc,
        pages_id,
        b"BT /F1 12 Tf 72 720 Td (PAGE0) Tj ET\n/Fm0 Do\n",
        Some(xobject_resources(&[("Fm0", fm0)])),
    );
    let p1 = add_page(&mut doc, pages_id, b"", None);
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        None,
        path,
        "R-ZIP content+form fixture",
    );
}

fn write_successful_looking_burn(path: &Path) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut img = Dictionary::new();
    img.set("Type", "XObject");
    img.set("Subtype", "Image");
    img.set("Width", 1);
    img.set("Height", 1);
    img.set("ColorSpace", "DeviceRGB");
    img.set("BitsPerComponent", 8);
    let img_id = doc.add_object(Object::Stream(Stream::new(img, vec![255, 255, 255])));
    let res = xobject_resources(&[("ImR", img_id)]);
    let p0 = add_page(&mut doc, pages_id, b"/ImR Do\n", Some(res.clone()));
    let p1 = add_page(&mut doc, pages_id, b"/ImR Do\n", Some(res));
    finish_two_page(
        &mut doc,
        pages_id,
        p0,
        p1,
        None,
        path,
        "successful-looking burn",
    );
}

fn page_id_at(doc: &Document, page_index: u32) -> Option<ObjectId> {
    doc.get_pages().get(&(page_index + 1)).copied()
}

fn dict_from<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn as_u32(obj: &Object) -> Option<u32> {
    match obj {
        Object::Integer(i) if *i > 0 => Some(*i as u32),
        Object::Real(r) if *r > 0.0 => Some(*r as u32),
        _ => None,
    }
}

fn plain_stream(stream: &Stream) -> Vec<u8> {
    stream
        .get_plain_content()
        .or_else(|_| stream.decompressed_content())
        .unwrap_or_else(|_| stream.content.clone())
}

fn decoded_blob(path: &Path) -> String {
    let mut doc = Document::load(path).expect("load pdf");
    let _ = doc.decompress();
    let mut out = String::new();
    for obj in doc.objects.values() {
        if let Object::Stream(s) = obj {
            out.push_str(&String::from_utf8_lossy(&plain_stream(s)));
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
                let bytes = plain_stream(s);
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

fn collect_source_probes(src: &Path, regions: &[RedactRegion]) -> Vec<Vec<u8>> {
    let doc = Document::load(src).expect("load source for collect_redact_probes_for_pages");
    collect_redact_probes_for_pages(&doc, regions)
        .expect("collect_redact_probes_for_pages on source")
}

fn plant_leftover_form_on_page0(src_dest: &Path, planted: &Path, body: &[u8]) {
    let mut doc = Document::load(src_dest).expect("load dest to plant leftover Form");
    let page_id = page_id_at(&doc, 0).expect("page 0");
    let fm_id = add_form(&mut doc, body);

    let mut resources = Dictionary::new();
    if let Ok(page) = doc.get_dictionary(page_id) {
        if let Ok(res_obj) = page.get(b"Resources") {
            if let Some(d) = dict_from(&doc, res_obj) {
                resources = d.clone();
            }
        }
    }
    let mut xobjects = Dictionary::new();
    if let Ok(xo) = resources.get(b"XObject") {
        if let Some(d) = dict_from(&doc, xo) {
            xobjects = d.clone();
        }
    }
    xobjects.set("FmLeak", Object::Reference(fm_id));
    resources.set("XObject", Object::Dictionary(xobjects));
    if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
        page.set("Resources", Object::Dictionary(resources));
    }
    doc.save(planted)
        .expect("save dest with leftover Form planted on page 0");
}

fn filter_label(dict: &Dictionary) -> String {
    match dict.get(b"Filter") {
        Ok(Object::Name(n)) => String::from_utf8_lossy(n).into_owned(),
        Ok(Object::Array(arr)) => arr
            .iter()
            .filter_map(|o| o.as_name().ok())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect::<Vec<_>>()
            .join(","),
        Ok(other) => format!("{other:?}"),
        Err(_) => "none".into(),
    }
}

fn decode_device_rgb(stream: &Stream) -> Result<(u32, u32, Vec<u8>), String> {
    let w = stream
        .dict
        .get(b"Width")
        .ok()
        .and_then(as_u32)
        .ok_or_else(|| "image missing Width".to_string())?;
    let h = stream
        .dict
        .get(b"Height")
        .ok()
        .and_then(as_u32)
        .ok_or_else(|| "image missing Height".to_string())?;
    let raw = stream.content.clone();
    let expected = (w as usize)
        .checked_mul(h as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or_else(|| "dest image dimensions overflow".to_string())?;
    if raw.len() == expected {
        return Ok((w, h, raw));
    }
    // lopdf will not inflate Image streams (`decompressed_content` returns Type).
    let inflated = inflate_flate(&raw).or_else(|_| inflate_raw_deflate(&raw));
    if let Ok(bytes) = inflated {
        if bytes.len() == expected {
            return Ok((w, h, bytes));
        }
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgb = img.to_rgb8();
            return Ok((rgb.width(), rgb.height(), rgb.into_raw()));
        }
    }
    for bytes in [&raw, &plain_stream(stream)] {
        if let Ok(img) = image::load_from_memory(bytes) {
            let rgb = img.to_rgb8();
            return Ok((rgb.width(), rgb.height(), rgb.into_raw()));
        }
    }
    Err(format!(
        "could not decode dest image {w}×{h}: bytes={} filter={}",
        raw.len(),
        filter_label(&stream.dict)
    ))
}

fn inflate_flate(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::ZlibDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib inflate: {e}"))?;
    Ok(out)
}

fn inflate_raw_deflate(data: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("deflate inflate: {e}"))?;
    Ok(out)
}

fn image_rgb_from_obj(doc: &Document, obj: &Object) -> Option<(u32, u32, Vec<u8>)> {
    let id = obj.as_reference().ok()?;
    let stream = doc.get_object(id).ok().and_then(|o| o.as_stream().ok())?;
    let subtype = stream.dict.get(b"Subtype").ok()?.as_name().ok()?;
    if subtype != b"Image" {
        return None;
    }
    decode_device_rgb(stream).ok()
}

fn dest_page0_image_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let mut doc = Document::load(path).map_err(|e| format!("load dest: {e}"))?;
    let _ = doc.decompress();
    let page_id = page_id_at(&doc, 0).ok_or_else(|| "dest missing page 0".to_string())?;

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
            if let Ok(imr) = xobjects.get(b"ImR") {
                if let Ok(id) = imr.as_reference() {
                    if let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) {
                        return decode_device_rgb(stream);
                    }
                }
                if let Some(rgb) = image_rgb_from_obj(&doc, imr) {
                    return Ok(rgb);
                }
            }
            let mut found: Option<(u32, u32, Vec<u8>)> = None;
            for (name, obj) in xobjects.iter() {
                if name == b"ImR" {
                    continue;
                }
                if let Some(rgb) = image_rgb_from_obj(&doc, obj) {
                    if found.is_some() {
                        return Err("page 0 has multiple Image XObjects and no usable /ImR".into());
                    }
                    found = Some(rgb);
                }
            }
            if let Some(rgb) = found {
                return Ok(rgb);
            }
        }
    }

    let mut sole: Option<(u32, u32, Vec<u8>)> = None;
    for obj in doc.objects.values() {
        let Object::Stream(stream) = obj else {
            continue;
        };
        let Ok(subtype) = stream.dict.get(b"Subtype").and_then(|o| o.as_name()) else {
            continue;
        };
        if subtype != b"Image" {
            continue;
        }
        let Ok(rgb) = decode_device_rgb(stream) else {
            continue;
        };
        if sole.is_some() {
            return Err("dest has multiple Image XObjects and page 0 has no /ImR".into());
        }
        sole = Some(rgb);
    }
    sole.ok_or_else(|| "dest page 0 has no /ImR Image XObject".into())
}

fn sample_rgb(data: &[u8], width: u32, height: u32, x: u32, y: u32) -> Result<[u8; 3], String> {
    if x >= width || y >= height {
        return Err(format!(
            "sample ({x},{y}) outside dest image {width}×{height}"
        ));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(3))
        .ok_or_else(|| "dest image dimensions overflow".to_string())?;
    if data.len() < expected {
        return Err(format!(
            "dest image bytes {} < {} (expected DeviceRGB {width}×{height})",
            data.len(),
            expected
        ));
    }
    let i = (y as usize * width as usize + x as usize) * 3;
    Ok([data[i], data[i + 1], data[i + 2]])
}

fn rgb_close(got: [u8; 3], want: [u8; 3], tol: u8) -> bool {
    got.iter()
        .zip(want)
        .all(|(g, w)| g.abs_diff(w) <= tol)
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
        "r4",
        None,
        |args| run_qpdf(qpdf, args),
    )
}

fn apply_verify_must_not_keep(
    id: &str,
    src: &Path,
    dest: &Path,
    regions: &[RedactRegion],
    leak: &[u8],
    leak_name: &str,
) {
    let probes = collect_source_probes(src, regions);
    let refs: Vec<&[u8]> = probes.iter().map(|p| p.as_slice()).collect();
    match apply_copy(src, dest, regions) {
        Ok(()) => match verify_redaction(dest, &refs, regions) {
            Ok(_) => {
                assert!(
                    dest.is_file(),
                    "{id}: dest must exist after apply+verify Ok"
                );
                assert!(
                    !dest_has_bytes(dest, leak),
                    "{id}: dest must not stay Ok while any dest object still contains {leak_name}"
                );
                assert!(
                    dest_has_str(dest, "KEEPME"),
                    "{id}: page 1 KEEPME must remain"
                );
            }
            Err(_) => {
                // Fail-closed is acceptable if the leak cannot be swept.
            }
        },
        Err(_) => {
            // Fail-closed apply is acceptable.
        }
    }
}

// ---------------------------------------------------------------------------
// R-SHARE — inherited Pages Form used only by the redacted page
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_shared_inherited_form_must_not_keep_secret() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-share-apply");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_share_inherited_form(&src);
    apply_verify_must_not_keep(
        "R-SHARE",
        &src,
        &dest,
        &[cover_secret()],
        b"SECRET",
        "SECRET",
    );
}

#[test]
fn export_shared_inherited_form_must_not_publish_secret() {
    let Some(qpdf) = test_qpdf() else {
        eprintln!("skip: qpdf not available");
        return;
    };
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-share-export");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    let work = scratch.0.join("work");
    std::fs::create_dir_all(&work).unwrap();
    write_share_inherited_form(&src);
    match export_redact(
        &qpdf,
        &src,
        &dest,
        &work,
        &redact_doc(0, 60.0, 700.0, 160.0, 40.0),
    ) {
        Ok(_) => {
            assert!(dest.is_file(), "R-SHARE: dest must exist on export Ok");
            assert!(
                !dest_has_str(&dest, "SECRET"),
                "R-SHARE: dest must not be published Ok while any dest object still contains SECRET"
            );
            assert!(
                dest_has_str(&dest, "KEEPME"),
                "R-SHARE: page 1 KEEPME must remain"
            );
        }
        Err(_) => {
            assert!(
                !dest.is_file(),
                "R-SHARE: fail-closed export must not publish dest"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// R-ZIP — leftover Form on page 0 must fail-close (no 1:1 probe/page zip)
// ---------------------------------------------------------------------------

#[test]
fn verify_leftover_form_on_page0_must_err_when_probe_page_counts_match() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-zip");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_zip_content_and_form(&src);
    let regions = zip_regions();
    let probes = collect_source_probes(&src, &regions);
    assert_eq!(
        probes.len(),
        2,
        "R-ZIP: collect on source must emit two probes (page-0 Contents + Form body) for two redacted pages (zip condition); got {}",
        probes.len()
    );
    let form_body = probes
        .iter()
        .find(|p| p.windows(FORM_ZIP_PROBE.len()).any(|w| w == FORM_ZIP_PROBE))
        .cloned()
        .expect("R-ZIP: collect on source must include the Form body probe FORM-ZIP-PROBE");

    if apply_copy(&src, &dest, &regions).is_err() {
        write_successful_looking_burn(&dest);
    }
    assert!(dest.is_file(), "R-ZIP: dest must exist to plant leftover Form");

    let planted = scratch.pdf("planted.pdf");
    plant_leftover_form_on_page0(&dest, &planted, &form_body);
    let refs: Vec<&[u8]> = probes.iter().map(|p| p.as_slice()).collect();
    let err = verify_redaction(&planted, &refs, &regions).expect_err(
        "R-ZIP: leftover Form planted on page 0 must fail-close; zip must not search that probe only on empty page 1",
    );
    assert_eq!(
        err.code, "REDACTION_INCOMPLETE",
        "R-ZIP: fail-closed code must be REDACTION_INCOMPLETE; got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// R-GEOM-PX — dest /ImR interior is the distinctive fill; control is not
// ---------------------------------------------------------------------------

fn assert_geom_px(label: &str, corpus: &str) {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new(label);
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_corpus_fixture(corpus, &src).unwrap_or_else(|e| panic!("write {corpus}: {e}"));
    apply_copy(&src, &dest, &[cover_path_fill()]).unwrap_or_else(|e| {
        panic!("R-GEOM-PX: apply_redactions must succeed so dest /ImR can be sampled; {e:?}")
    });
    let (width, height, rgb) = dest_page0_image_rgb(&dest).unwrap_or_else(|e| {
        panic!("R-GEOM-PX {corpus}: dest page-0 /ImR (or sole Image) required; {e}")
    });
    let (ix, iy) = GEOM_INTERIOR;
    let (cx, cy) = GEOM_CONTROL;
    assert!(
        width > ix && height > iy,
        "R-GEOM-PX {corpus}: dest /ImR must be MediaBox pixel space so ({ix},{iy}) is in-bounds; got {width}×{height}"
    );
    let interior = sample_rgb(&rgb, width, height, ix, iy)
        .unwrap_or_else(|e| panic!("R-GEOM-PX {corpus} interior: {e}"));
    let control = sample_rgb(&rgb, width, height, cx, cy)
        .unwrap_or_else(|e| panic!("R-GEOM-PX {corpus} control: {e}"));
    assert!(
        rgb_close(interior, GEOM_FILL_RGB, 16),
        "R-GEOM-PX {corpus}: interior raster ({ix},{iy}) must be fill {GEOM_FILL} {:?}; got {:?}",
        GEOM_FILL_RGB,
        interior
    );
    assert!(
        !rgb_close(control, GEOM_FILL_RGB, 16),
        "R-GEOM-PX {corpus}: control raster ({cx},{cy}) must not be the fill; got {:?}",
        control
    );
}

#[test]
fn apply_redactions_geom_rotate_90_fill_lands_in_unrotated_region() {
    assert_geom_px("r-geom-px-rot90", "geom-rotate-90");
}

#[test]
fn apply_redactions_geom_crop_offset_fill_lands_in_mediabox_region() {
    assert_geom_px("r-geom-px-crop", "geom-crop-offset");
}

// ---------------------------------------------------------------------------
// R-SHARE-OWN — page-local unused /Fm0 on the sibling is still page content
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_own_resources_unused_form_must_not_keep_secret() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-share-own");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_share_own_form(&src);
    apply_verify_must_not_keep(
        "R-SHARE-OWN",
        &src,
        &dest,
        &[cover_secret()],
        b"SECRET",
        "SECRET",
    );
}

// ---------------------------------------------------------------------------
// R-SHARE-MIX — sibling-used /Fm1 KEEPME stays; unused /Fm0 SECRET must not
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_mixed_inherited_forms_must_drop_secret_keep_keepme() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-share-mix");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_share_mix_forms(&src);
    apply_verify_must_not_keep(
        "R-SHARE-MIX",
        &src,
        &dest,
        &[cover_secret()],
        b"SECRET",
        "SECRET",
    );
}

// ---------------------------------------------------------------------------
// R-SHARE-IMG — inherited Image used only by the redacted page
// ---------------------------------------------------------------------------

#[test]
fn apply_verify_shared_inherited_image_must_not_keep_tiny_rgb() {
    if test_pdftoppm().is_none() {
        eprintln!("skip: pdftoppm not available");
        return;
    }
    let scratch = Scratch::new("r-share-img");
    let src = scratch.pdf("src.pdf");
    let dest = scratch.pdf("dest.pdf");
    write_share_inherited_image(&src);
    apply_verify_must_not_keep(
        "R-SHARE-IMG",
        &src,
        &dest,
        &[region(0, 72.0, 400.0, 40.0, 40.0)],
        TINY_RGB,
        "TINY_RGB",
    );
}
