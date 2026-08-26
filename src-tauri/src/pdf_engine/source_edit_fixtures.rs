//! Source-editing fixture corpus (#32).
//!
//! Committed files live at clone `fixtures/source-edit/` (manifest + tiny PDFs).
//! This module is the generator + lib tests. It does not claim a fixture is editable.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write one named corpus fixture (`manifest.json` `id`) to `dest`.
///
/// After lopdf `save`, runs `qpdf in out` when qpdf is present so `qpdf --check`
/// passes (same rewrite as crop.rs / metadata.rs). Missing qpdf still writes a PDF.
pub(crate) fn write_corpus_fixture(id: &str, dest: &Path) -> Result<(), String> {
    let mut doc = match id {
        "text-tj" => build_text_tj(),
        "text-tj-kerned" => build_text_tj_kerned(),
        "text-cid-tounicode" => build_cid(true),
        "text-cid-no-tounicode" => build_cid(false),
        "text-type3" => build_type3(),
        "text-nested-form" => build_nested_form(),
        "text-rotated" => build_text_tm("0 1 -1 0 200 400 Tm"),
        "text-skewed" => build_text_tm("1 0.3 0 1 72 400 Tm"),
        "image-unique" => build_image_unique(),
        "image-reused" => build_image_reused(),
        "image-in-form" => build_image_in_form(),
        "image-inline" => build_image_inline(),
        "image-mask" => build_image_mask(),
        "geom-crop-offset" => build_geom_crop_offset(),
        "geom-user-unit" => build_geom_user_unit(),
        "geom-rotate-90" => build_geom_rotate(90),
        "geom-rotate-180" => build_geom_rotate(180),
        "geom-rotate-270" => build_geom_rotate(270),
        other => return Err(format!("unknown corpus fixture id: {other}")),
    };
    save_normalized(&mut doc, dest)
}

fn find_qpdf() -> Option<PathBuf> {
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

fn save_normalized(doc: &mut Document, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create dest dir: {e}"))?;
        }
    }
    let seq = WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_dir = std::env::temp_dir().join(format!(
        "offpdf-corpus-write-{}-{}-{}",
        std::process::id(),
        seq,
        dest.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("fixture")
    ));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| format!("create temp dir: {e}"))?;
    let tmp = tmp_dir.join("in.pdf");
    let result = (|| {
        doc.save(&tmp).map_err(|e| format!("lopdf save: {e}"))?;
        if let Some(qpdf) = find_qpdf() {
            let out = std::process::Command::new(qpdf)
                .arg(&tmp)
                .arg(dest)
                .output()
                .map_err(|e| format!("qpdf start: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "qpdf rewrite failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
        } else {
            std::fs::copy(&tmp, dest).map_err(|e| format!("copy dest: {e}"))?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp_dir);
    result
}

fn box_obj(b: [i64; 4]) -> Object {
    Object::Array(b.into_iter().map(Object::Integer).collect())
}

fn helvetica() -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    font
}

fn font_resources(name: &str, font: Object) -> Dictionary {
    let mut fonts = Dictionary::new();
    fonts.set(name.as_bytes().to_vec(), font);
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    res
}

fn xobject_resources(name: &str, id: ObjectId) -> Dictionary {
    let mut xobj = Dictionary::new();
    xobj.set(name.as_bytes().to_vec(), Object::Reference(id));
    let mut res = Dictionary::new();
    res.set("XObject", Object::Dictionary(xobj));
    res
}

fn new_doc() -> (Document, ObjectId) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    (doc, pages_id)
}

fn finish_doc(doc: &mut Document, pages_id: ObjectId, page_ids: Vec<ObjectId>) {
    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set(
        "Kids",
        page_ids
            .iter()
            .map(|id| Object::Reference(*id))
            .collect::<Vec<Object>>(),
    );
    pages.set("Count", Object::Integer(page_ids.len() as i64));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
}

fn add_page(
    doc: &mut Document,
    pages_id: ObjectId,
    content: &[u8],
    resources: Dictionary,
    extras: &[(&[u8], Object)],
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
    page.set("Resources", Object::Dictionary(resources));
    for (k, v) in extras {
        page.set(k.to_vec(), v.clone());
    }
    doc.add_object(Object::Dictionary(page))
}

fn one_page_doc(content: &[u8], resources: Dictionary, extras: &[(&[u8], Object)]) -> Document {
    let (mut doc, pages_id) = new_doc();
    let page_id = add_page(&mut doc, pages_id, content, resources, extras);
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn build_text_tj() -> Document {
    one_page_doc(
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n",
        font_resources("F1", Object::Dictionary(helvetica())),
        &[(b"Rotate", Object::Integer(0))],
    )
}

fn build_text_tj_kerned() -> Document {
    one_page_doc(
        b"BT /F1 12 Tf 72 720 Td [(H) -40 (i)] TJ ET\n",
        font_resources("F1", Object::Dictionary(helvetica())),
        &[],
    )
}

/// Synthetic Type0/CIDFontType0 — no Noto / no full CFF. Structure only.
fn add_type0_cid(doc: &mut Document, with_tounicode: bool) -> ObjectId {
    let mut sys = Dictionary::new();
    sys.set("Registry", Object::string_literal("Adobe"));
    sys.set("Ordering", Object::string_literal("Identity"));
    sys.set("Supplement", 0);

    let mut desc = Dictionary::new();
    desc.set("Type", "FontDescriptor");
    desc.set("FontName", "OffPdfCid");
    desc.set("Flags", 4);
    desc.set("FontBBox", box_obj([0, 0, 500, 700]));
    desc.set("ItalicAngle", 0);
    desc.set("Ascent", 700);
    desc.set("Descent", -200);
    desc.set("CapHeight", 700);
    desc.set("StemV", 80);
    let desc_id = doc.add_object(Object::Dictionary(desc));

    let mut cid = Dictionary::new();
    cid.set("Type", "Font");
    cid.set("Subtype", "CIDFontType0");
    cid.set("BaseFont", "OffPdfCid");
    cid.set("CIDSystemInfo", Object::Dictionary(sys));
    cid.set("FontDescriptor", desc_id);
    cid.set("DW", 500);
    let cid_id = doc.add_object(Object::Dictionary(cid));

    let mut type0 = Dictionary::new();
    type0.set("Type", "Font");
    type0.set("Subtype", "Type0");
    type0.set("BaseFont", "OffPdfCid");
    type0.set("Encoding", "Identity-H");
    type0.set("DescendantFonts", vec![Object::Reference(cid_id)]);
    if with_tounicode {
        let cmap = b"/CIDInit /ProcSet findresource begin\n\
12 dict begin\n\
begincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n\
/CMapType 2 def\n\
1 begincodespacerange\n\
<0000> <FFFF>\n\
endcodespacerange\n\
1 beginbfchar\n\
<0048> <0048>\n\
endbfchar\n\
endcmap\n\
CMapName currentdict /CMap defineresource pop\n\
end\n\
end\n";
        let tu_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), cmap.to_vec())));
        type0.set("ToUnicode", tu_id);
    }
    doc.add_object(Object::Dictionary(type0))
}

fn build_cid(with_tounicode: bool) -> Document {
    let (mut doc, pages_id) = new_doc();
    let font_id = add_type0_cid(&mut doc, with_tounicode);
    let page_id = add_page(
        &mut doc,
        pages_id,
        b"BT /C0 12 Tf 72 720 Td <0048> Tj ET\n",
        font_resources("C0", Object::Reference(font_id)),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn add_type3(doc: &mut Document) -> ObjectId {
    let proc = b"10 0 0 0 10 10 d1\n0 0 10 10 re f\n";
    let proc_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        proc.to_vec(),
    )));
    let mut char_procs = Dictionary::new();
    char_procs.set("a", proc_id);

    let mut enc = Dictionary::new();
    enc.set("Type", "Encoding");
    enc.set(
        "Differences",
        vec![Object::Integer(97), Object::Name(b"a".to_vec())],
    );

    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type3");
    font.set("FontBBox", box_obj([0, 0, 10, 10]));
    font.set(
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
    font.set("CharProcs", Object::Dictionary(char_procs));
    font.set("Encoding", Object::Dictionary(enc));
    font.set("FirstChar", 97);
    font.set("LastChar", 97);
    font.set("Widths", vec![Object::Integer(10)]);
    doc.add_object(Object::Dictionary(font))
}

fn build_type3() -> Document {
    let (mut doc, pages_id) = new_doc();
    let font_id = add_type3(&mut doc);
    let page_id = add_page(
        &mut doc,
        pages_id,
        b"BT /T3 12 Tf 72 720 Td (a) Tj ET\n",
        font_resources("T3", Object::Reference(font_id)),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn add_form(
    doc: &mut Document,
    bbox: [i64; 4],
    resources: Dictionary,
    content: &[u8],
) -> ObjectId {
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", "Form");
    d.set("BBox", box_obj(bbox));
    d.set("Resources", Object::Dictionary(resources));
    doc.add_object(Object::Stream(Stream::new(d, content.to_vec())))
}

fn build_nested_form() -> Document {
    let (mut doc, pages_id) = new_doc();
    let inner = add_form(
        &mut doc,
        [0, 0, 200, 200],
        font_resources("F1", Object::Dictionary(helvetica())),
        b"BT /F1 12 Tf 10 10 Td (Hi) Tj ET\n",
    );
    let outer = add_form(
        &mut doc,
        [0, 0, 200, 200],
        xobject_resources("Fm1", inner),
        b"/Fm1 Do\n",
    );
    let page_id = add_page(
        &mut doc,
        pages_id,
        b"/Fm0 Do\n",
        xobject_resources("Fm0", outer),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn build_text_tm(tm: &str) -> Document {
    let content = format!("BT /F1 12 Tf {tm} (Hi) Tj ET\n");
    one_page_doc(
        content.as_bytes(),
        font_resources("F1", Object::Dictionary(helvetica())),
        &[],
    )
}

/// 2×2 DeviceRGB, never a photograph.
const TINY_RGB: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];

fn add_rgb_image(doc: &mut Document, smask: Option<ObjectId>) -> ObjectId {
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", "Image");
    d.set("Width", 2);
    d.set("Height", 2);
    d.set("ColorSpace", "DeviceRGB");
    d.set("BitsPerComponent", 8);
    if let Some(s) = smask {
        d.set("SMask", s);
    }
    doc.add_object(Object::Stream(Stream::new(d, TINY_RGB.to_vec())))
}

fn paint_image() -> &'static [u8] {
    b"q 40 0 0 40 72 400 cm /Im0 Do Q\n"
}

fn build_image_unique() -> Document {
    let (mut doc, pages_id) = new_doc();
    let img = add_rgb_image(&mut doc, None);
    let page_id = add_page(
        &mut doc,
        pages_id,
        paint_image(),
        xobject_resources("Im0", img),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn build_image_reused() -> Document {
    let (mut doc, pages_id) = new_doc();
    let img = add_rgb_image(&mut doc, None);
    let p1 = add_page(
        &mut doc,
        pages_id,
        paint_image(),
        xobject_resources("Im0", img),
        &[],
    );
    let p2 = add_page(
        &mut doc,
        pages_id,
        paint_image(),
        xobject_resources("Im0", img),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![p1, p2]);
    doc
}

fn build_image_in_form() -> Document {
    let (mut doc, pages_id) = new_doc();
    let img = add_rgb_image(&mut doc, None);
    let form = add_form(
        &mut doc,
        [0, 0, 200, 200],
        xobject_resources("Im0", img),
        b"q 40 0 0 40 10 10 cm /Im0 Do Q\n",
    );
    let page_id = add_page(
        &mut doc,
        pages_id,
        b"/Fm0 Do\n",
        xobject_resources("Fm0", form),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn build_image_inline() -> Document {
    one_page_doc(
        b"q 24 0 0 12 72 400 cm\n\
BI\n\
/W 2 /H 1 /CS /DeviceRGB /BPC 8 /F /AHx\n\
ID\n\
C8101010C810>\n\
EI\n\
Q\n",
        Dictionary::new(),
        &[],
    )
}

fn build_image_mask() -> Document {
    let (mut doc, pages_id) = new_doc();
    let mut sm = Dictionary::new();
    sm.set("Type", "XObject");
    sm.set("Subtype", "Image");
    sm.set("Width", 2);
    sm.set("Height", 2);
    sm.set("ColorSpace", "DeviceGray");
    sm.set("BitsPerComponent", 8);
    let smask = doc.add_object(Object::Stream(Stream::new(sm, vec![255, 200, 180, 255])));
    let img = add_rgb_image(&mut doc, Some(smask));
    let page_id = add_page(
        &mut doc,
        pages_id,
        paint_image(),
        xobject_resources("Im0", img),
        &[],
    );
    finish_doc(&mut doc, pages_id, vec![page_id]);
    doc
}

fn build_geom_crop_offset() -> Document {
    one_page_doc(
        b"0 0 0 rg 72 72 100 100 re f\n",
        Dictionary::new(),
        &[
            (b"MediaBox", box_obj([0, 0, 612, 792])),
            (b"CropBox", box_obj([36, 48, 576, 744])),
        ],
    )
}

fn build_geom_user_unit() -> Document {
    one_page_doc(
        b"0 0 0 rg 72 72 100 100 re f\n",
        Dictionary::new(),
        &[(b"UserUnit", Object::Real(2.0))],
    )
}

fn build_geom_rotate(angle: i64) -> Document {
    one_page_doc(
        b"0 0 0 rg 72 72 100 100 re f\n",
        Dictionary::new(),
        &[(b"Rotate", Object::Integer(angle))],
    )
}

#[cfg(test)]
mod tests {
    use super::write_corpus_fixture;
    use lopdf::{Dictionary, Document, Object};
    use serde::Deserialize;
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};

    const SIZE_CAP: u64 = 2_097_152;

    const CLOSED_STRUCTURES: &[&str] = &[
        "Tj",
        "TJ",
        "CID+ToUnicode",
        "CID-no-ToUnicode",
        "text-rotated",
        "text-skewed",
        "Type3",
        "nested-form",
        "image-unique",
        "image-reused",
        "image-in-form",
        "image-inline",
        "image-mask",
        "crop-offset",
        "user-unit",
        "rotate-0",
        "rotate-90",
        "rotate-180",
        "rotate-270",
    ];

    #[derive(Debug, Deserialize)]
    struct Manifest {
        version: u32,
        license: String,
        fixtures: Vec<FixtureRow>,
    }

    #[derive(Debug, Deserialize)]
    struct FixtureRow {
        id: String,
        path: String,
        structures: Vec<String>,
        intent: String,
        #[serde(rename = "failureMode")]
        failure_mode: String,
        qpdf: String,
        geometry: Geometry,
    }

    #[derive(Debug, Deserialize)]
    struct Geometry {
        rotate: i64,
        #[serde(rename = "userUnit")]
        user_unit: f64,
        #[serde(rename = "cropOffset")]
        crop_offset: bool,
    }

    fn corpus_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("fixtures")
            .join("source-edit")
    }

    fn manifest_path() -> PathBuf {
        corpus_dir().join("manifest.json")
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

    fn load_manifest() -> (Manifest, serde_json::Value) {
        let path = manifest_path();
        assert!(
            path.is_file(),
            "CORPUS-MANIFEST: fixtures/source-edit/manifest.json must exist"
        );
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("CORPUS-MANIFEST: could not read {}: {e}", path.display())
        });
        let value: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("CORPUS-MANIFEST: manifest.json is not JSON: {e}"));
        let manifest: Manifest = serde_json::from_value(value.clone()).unwrap_or_else(|e| {
            panic!(
                "CORPUS-MANIFEST: manifest.json must have version, license, fixtures[] \
                 with id/path/structures/intent/failureMode/qpdf/geometry: {e}"
            )
        });
        (manifest, value)
    }

    fn rows_with<'a>(manifest: &'a Manifest, structure: &str) -> Vec<&'a FixtureRow> {
        manifest
            .fixtures
            .iter()
            .filter(|r| r.structures.iter().any(|s| s == structure))
            .collect()
    }

    fn require_row<'a>(manifest: &'a Manifest, structure: &str) -> &'a FixtureRow {
        let rows = rows_with(manifest, structure);
        assert!(
            !rows.is_empty(),
            "fixtures[] must include a row whose structures contains {structure:?}"
        );
        rows[0]
    }

    fn committed_pdf(row: &FixtureRow) -> PathBuf {
        let path = corpus_dir().join(&row.path);
        assert!(
            path.is_file(),
            "committed fixture {} ({}) must exist under fixtures/source-edit/",
            row.id,
            row.path
        );
        let bytes = std::fs::read(&path).unwrap_or_default();
        assert!(
            bytes.starts_with(b"%PDF"),
            "committed fixture {} is not a PDF ({} bytes)",
            row.path,
            bytes.len()
        );
        path
    }

    fn temp_dest(id: &str) -> (PathBuf, PathBuf) {
        let safe: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let root = std::env::temp_dir().join(format!(
            "offpdf-corpus-{}-{}-{}",
            safe,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&root).expect("temp corpus dir");
        let dest = root.join(format!("{safe}.pdf"));
        (root, dest)
    }

    fn regenerate(row: &FixtureRow) -> PathBuf {
        let (root, dest) = temp_dest(&row.id);
        write_corpus_fixture(&row.id, &dest).unwrap_or_else(|e| {
            panic!(
                "CORPUS-DETERMINISTIC: write_corpus_fixture({}) returned Err: {e}",
                row.id
            )
        });
        assert!(
            dest.is_file(),
            "CORPUS-DETERMINISTIC: write_corpus_fixture({}) must write dest; dest missing",
            row.id
        );
        let bytes = std::fs::read(&dest).unwrap_or_default();
        assert!(
            bytes.starts_with(b"%PDF"),
            "CORPUS-DETERMINISTIC: dest for {} is not a PDF ({} bytes)",
            row.id,
            bytes.len()
        );
        let _ = root;
        dest
    }

    fn dump_streams(path: &Path) -> String {
        let mut doc = Document::load(path).unwrap_or_else(|e| {
            panic!("load {}: {e}", path.display())
        });
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

    fn load_doc(path: &Path) -> Document {
        let mut doc = Document::load(path).unwrap_or_else(|e| {
            panic!("load {}: {e}", path.display())
        });
        let _ = doc.decompress();
        doc
    }

    fn dict_name(dict: &Dictionary, key: &[u8]) -> Option<Vec<u8>> {
        match dict.get(key).ok()? {
            Object::Name(n) => Some(n.clone()),
            _ => None,
        }
    }

    fn name_eq(got: Option<Vec<u8>>, want: &[u8]) -> bool {
        got.as_deref() == Some(want)
    }

    fn page_dicts(doc: &Document) -> Vec<Dictionary> {
        let mut pages = doc.get_pages();
        let mut keys: Vec<u32> = pages.keys().copied().collect();
        keys.sort_unstable();
        keys.into_iter()
            .filter_map(|n| {
                let id = pages.remove(&n)?;
                doc.get_dictionary(id).ok().cloned()
            })
            .collect()
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

    fn page_rotate(dict: &Dictionary) -> i64 {
        match dict.get(b"Rotate") {
            Ok(Object::Integer(i)) => *i,
            _ => 0,
        }
    }

    fn page_user_unit(dict: &Dictionary) -> f64 {
        match dict.get(b"UserUnit") {
            Ok(Object::Real(r)) => *r as f64,
            Ok(Object::Integer(i)) => *i as f64,
            _ => 1.0,
        }
    }

    fn resources_of(doc: &Document, dict: &Dictionary) -> Option<Dictionary> {
        match dict.get(b"Resources").ok()? {
            Object::Dictionary(d) => Some(d.clone()),
            Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
            _ => None,
        }
    }

    fn xobjects_of(doc: &Document, resources: &Dictionary) -> Option<Dictionary> {
        match resources.get(b"XObject").ok()? {
            Object::Dictionary(d) => Some(d.clone()),
            Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
            _ => None,
        }
    }

    fn is_op_at(blob: &str, at: usize, op: &str) -> bool {
        let bytes = blob.as_bytes();
        if at + op.len() > bytes.len() {
            return false;
        }
        if &bytes[at..at + op.len()] != op.as_bytes() {
            return false;
        }
        let before_ok = at == 0 || {
            let c = bytes[at - 1];
            c.is_ascii_whitespace() || c == b'[' || c == b']'
        };
        let after = at + op.len();
        let after_ok = after == bytes.len() || {
            let c = bytes[after];
            c.is_ascii_whitespace() || c == b'[' || c == b']'
        };
        before_ok && after_ok
    }

    fn has_operator(blob: &str, op: &str) -> bool {
        let mut i = 0;
        while let Some(rel) = blob[i..].find(op) {
            let at = i + rel;
            if is_op_at(blob, at, op) {
                return true;
            }
            i = at + 1;
        }
        false
    }

    fn strip_literals(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'(' {
                i += 1;
                while i < b.len() {
                    if b[i] == b'\\' {
                        i = (i + 2).min(b.len());
                        continue;
                    }
                    if b[i] == b')' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                out.push(' ');
            } else if b[i] == b'<' {
                i += 1;
                while i < b.len() && b[i] != b'>' {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
                out.push(' ');
            } else {
                out.push(b[i] as char);
                i += 1;
            }
        }
        out
    }

    fn has_kerned_tj(blob: &str) -> bool {
        let mut i = 0;
        while let Some(rel) = blob[i..].find("TJ") {
            let at = i + rel;
            if is_op_at(blob, at, "TJ") {
                if let Some(start) = blob[..at].rfind('[') {
                    let arr = strip_literals(&blob[start..at]);
                    if arr.split_whitespace().any(|t| t.parse::<f64>().is_ok()) {
                        return true;
                    }
                }
            }
            i = at + 1;
        }
        false
    }

    fn tm_matrices(blob: &str) -> Vec<[f64; 6]> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(rel) = blob[i..].find("Tm") {
            let at = i + rel;
            if is_op_at(blob, at, "Tm") {
                let prefix = blob[..at].trim_end();
                let mut nums = Vec::new();
                for tok in prefix.split_whitespace().rev() {
                    if let Ok(n) = tok.parse::<f64>() {
                        nums.push(n);
                        if nums.len() == 6 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if nums.len() == 6 {
                    nums.reverse();
                    out.push([nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]]);
                }
            }
            i = at + 1;
        }
        out
    }

    fn is_rotated_tm(m: [f64; 6]) -> bool {
        let [a, b, c, d, _, _] = m;
        let det = a * d - b * c;
        if det.abs() < 1e-6 {
            return false;
        }
        (b.abs() > 0.1 || c.abs() > 0.1) && (a - d).abs() < 0.25 && (b + c).abs() < 0.25
    }

    fn is_skewed_tm(m: [f64; 6]) -> bool {
        let [a, b, c, d, _, _] = m;
        if b.abs() < 0.05 && c.abs() < 0.05 {
            return false;
        }
        let det = a * d - b * c;
        if det.abs() < 1e-6 {
            return false;
        }
        !is_rotated_tm(m)
    }

    fn has_cid_font(doc: &Document) -> bool {
        for obj in doc.objects.values() {
            let dict = match obj {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                _ => continue,
            };
            let subtype = dict_name(dict, b"Subtype");
            if name_eq(subtype.clone(), b"Type0")
                || name_eq(subtype.clone(), b"CIDFontType0")
                || name_eq(subtype.clone(), b"CIDFontType2")
                || dict.get(b"CIDSystemInfo").is_ok()
            {
                return true;
            }
        }
        false
    }

    fn tounicode_usable(doc: &Document, dict: &Dictionary) -> bool {
        let obj = match dict.get(b"ToUnicode") {
            Ok(o) => o,
            Err(_) => return false,
        };
        let bytes = match obj {
            Object::Stream(s) => s.get_plain_content().unwrap_or_else(|_| s.content.clone()),
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Stream(s)) => s.get_plain_content().unwrap_or_else(|_| s.content.clone()),
                _ => return false,
            },
            _ => return false,
        };
        let text = String::from_utf8_lossy(&bytes);
        text.contains("beginbfchar")
            || text.contains("beginbfrange")
            || text.contains("begincmap")
    }

    fn cid_fonts(doc: &Document) -> Vec<Dictionary> {
        let mut out = Vec::new();
        for obj in doc.objects.values() {
            let dict = match obj {
                Object::Dictionary(d) => d.clone(),
                Object::Stream(s) => s.dict.clone(),
                _ => continue,
            };
            let subtype = dict_name(&dict, b"Subtype");
            if name_eq(subtype.clone(), b"Type0")
                || name_eq(subtype.clone(), b"CIDFontType0")
                || name_eq(subtype.clone(), b"CIDFontType2")
                || dict.get(b"CIDSystemInfo").is_ok()
            {
                out.push(dict);
            }
        }
        out
    }

    fn has_type3(doc: &Document) -> bool {
        for obj in doc.objects.values() {
            let dict = match obj {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                _ => continue,
            };
            if name_eq(dict_name(dict, b"Subtype"), b"Type3") {
                return true;
            }
        }
        false
    }

    fn form_streams(doc: &Document) -> Vec<(Dictionary, String)> {
        let mut out = Vec::new();
        for obj in doc.objects.values() {
            let Object::Stream(s) = obj else {
                continue;
            };
            if name_eq(dict_name(&s.dict, b"Subtype"), b"Form") {
                let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
                out.push((s.dict.clone(), String::from_utf8_lossy(&bytes).into_owned()));
            }
        }
        out
    }

    fn form_has_text(content: &str) -> bool {
        has_operator(content, "BT")
            || has_operator(content, "Tj")
            || has_operator(content, "TJ")
    }

    fn has_nested_form_text(doc: &Document) -> bool {
        let forms = form_streams(doc);
        if forms.is_empty() {
            return false;
        }
        for (dict, body) in &forms {
            if let Some(res) = resources_of(doc, dict) {
                if let Some(xo) = xobjects_of(doc, &res) {
                    for (_, obj) in xo.iter() {
                        let Ok(id) = obj.as_reference() else {
                            continue;
                        };
                        let Ok(inner) = doc.get_object(id) else {
                            continue;
                        };
                        if let Object::Stream(s) = inner {
                            if name_eq(dict_name(&s.dict, b"Subtype"), b"Form") {
                                let bytes =
                                    s.get_plain_content().unwrap_or_else(|_| s.content.clone());
                                let inner_body = String::from_utf8_lossy(&bytes);
                                if form_has_text(&inner_body) || form_has_text(body) {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
            if form_has_text(body) {
                return true;
            }
        }
        false
    }

    fn image_ids_on_pages(doc: &Document) -> Vec<Vec<lopdf::ObjectId>> {
        let mut per_page = Vec::new();
        for page in page_dicts(doc) {
            let mut ids = Vec::new();
            let Some(res) = resources_of(doc, &page) else {
                per_page.push(ids);
                continue;
            };
            let Some(xo) = xobjects_of(doc, &res) else {
                per_page.push(ids);
                continue;
            };
            for (_, obj) in xo.iter() {
                let Ok(id) = obj.as_reference() else {
                    continue;
                };
                if let Ok(Object::Stream(s)) = doc.get_object(id) {
                    if name_eq(dict_name(&s.dict, b"Subtype"), b"Image") {
                        ids.push(id);
                    }
                }
            }
            per_page.push(ids);
        }
        per_page
    }

    fn image_streams(doc: &Document) -> Vec<&Dictionary> {
        let mut out = Vec::new();
        for obj in doc.objects.values() {
            if let Object::Stream(s) = obj {
                if name_eq(dict_name(&s.dict, b"Subtype"), b"Image") {
                    out.push(&s.dict);
                }
            }
        }
        out
    }

    fn image_dim(dict: &Dictionary, key: &[u8]) -> Option<i64> {
        match dict.get(key).ok()? {
            Object::Integer(i) => Some(*i),
            Object::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    fn has_image_mask(dict: &Dictionary) -> bool {
        dict.get(b"SMask").is_ok()
            || dict.get(b"Mask").is_ok()
            || matches!(dict.get(b"ImageMask"), Ok(Object::Boolean(true)))
    }

    fn form_paints_image(doc: &Document) -> bool {
        for (dict, body) in form_streams(doc) {
            if has_operator(&body, "Do") {
                if let Some(res) = resources_of(doc, &dict) {
                    if let Some(xo) = xobjects_of(doc, &res) {
                        for (_, obj) in xo.iter() {
                            let Ok(id) = obj.as_reference() else {
                                continue;
                            };
                            if let Ok(Object::Stream(s)) = doc.get_object(id) {
                                if name_eq(dict_name(&s.dict, b"Subtype"), b"Image") {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn assert_no_editable_key(value: &serde_json::Value, path: &str) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(v) = map.get("editable") {
                    panic!("CORPUS-NO-EDITABLE-CLAIM: {path} must not have editable (got {v})");
                }
                for (k, v) in map {
                    assert_no_editable_key(v, &format!("{path}.{k}"));
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    assert_no_editable_key(v, &format!("{path}[{i}]"));
                }
            }
            serde_json::Value::String(s) => {
                if s.eq_ignore_ascii_case("editable") || s.eq_ignore_ascii_case("editable: true") {
                    panic!(
                        "CORPUS-NO-EDITABLE-CLAIM: {path} uses capability language {s:?}"
                    );
                }
            }
            _ => {}
        }
    }

    #[test]
    fn manifest_exists_with_required_rows_and_intent_labels() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/source-edit");
        assert!(
            !bundled.exists(),
            "CORPUS-MANIFEST: corpus must not live under src-tauri/resources/"
        );

        let (manifest, _) = load_manifest();
        assert_eq!(
            manifest.version, 1,
            "CORPUS-MANIFEST: version must be 1"
        );
        assert!(
            !manifest.license.trim().is_empty(),
            "CORPUS-MANIFEST: license must be non-empty"
        );
        assert!(
            !manifest.fixtures.is_empty(),
            "CORPUS-MANIFEST: fixtures[] must not be empty"
        );

        let mut ids = HashSet::new();
        let mut seen = HashSet::new();
        for row in &manifest.fixtures {
            assert!(
                ids.insert(row.id.clone()),
                "CORPUS-MANIFEST: duplicate fixture id {}",
                row.id
            );
            assert!(
                row.intent == "try-edit" || row.intent == "unsupported-stand-in",
                "CORPUS-MANIFEST: fixture {} intent must be try-edit or unsupported-stand-in, got {:?}",
                row.id,
                row.intent
            );
            assert!(
                !row.failure_mode.trim().is_empty(),
                "CORPUS-MANIFEST: fixture {} failureMode must be non-empty",
                row.id
            );
            assert!(
                !row.path.trim().is_empty(),
                "CORPUS-MANIFEST: fixture {} path must be non-empty",
                row.id
            );
            assert!(
                !row.structures.is_empty(),
                "CORPUS-MANIFEST: fixture {} structures[] must not be empty",
                row.id
            );
            for s in &row.structures {
                assert!(
                    CLOSED_STRUCTURES.contains(&s.as_str()),
                    "CORPUS-MANIFEST: fixture {} has unknown structure {s:?}",
                    row.id
                );
                seen.insert(s.clone());
            }
            assert!(
                matches!(row.geometry.rotate, 0 | 90 | 180 | 270),
                "CORPUS-MANIFEST: fixture {} geometry.rotate must be 0|90|180|270, got {}",
                row.id,
                row.geometry.rotate
            );
            assert!(
                row.qpdf == "must-pass" || !row.qpdf.trim().is_empty(),
                "CORPUS-MANIFEST: fixture {} qpdf must be must-pass or a documented malformed note",
                row.id
            );
        }

        for req in CLOSED_STRUCTURES {
            assert!(
                seen.contains(*req),
                "CORPUS-MANIFEST: fixtures[] must name structure {req}"
            );
        }
    }

    #[test]
    fn write_corpus_fixture_matches_committed_structure() {
        let (probe_root, probe) = temp_dest("text-tj");
        write_corpus_fixture("text-tj", &probe).unwrap_or_else(|e| {
            panic!("CORPUS-DETERMINISTIC: write_corpus_fixture(text-tj) returned Err: {e}")
        });
        assert!(
            probe.is_file(),
            "CORPUS-DETERMINISTIC: write_corpus_fixture(text-tj) must write dest; dest missing"
        );
        let probe_bytes = std::fs::read(&probe).unwrap_or_default();
        assert!(
            probe_bytes.starts_with(b"%PDF"),
            "CORPUS-DETERMINISTIC: dest for text-tj is not a PDF ({} bytes)",
            probe_bytes.len()
        );
        let _ = std::fs::remove_dir_all(&probe_root);

        let (manifest, _) = load_manifest();
        assert!(
            !manifest.fixtures.is_empty(),
            "CORPUS-DETERMINISTIC: fixtures[] must not be empty"
        );
        for row in &manifest.fixtures {
            let dest = regenerate(row);
            let committed = committed_pdf(row);
            let got = dump_streams(&dest);
            let want = dump_streams(&committed);
            assert_eq!(
                got, want,
                "CORPUS-DETERMINISTIC: regenerated {} uncompressed streams must match committed {}",
                row.id, row.path
            );
            let _ = std::fs::remove_file(&dest);
        }
    }

    #[test]
    fn must_pass_rows_survive_qpdf_check() {
        let Some(qpdf) = test_qpdf() else {
            eprintln!("skip: qpdf not available");
            return;
        };
        let (manifest, _) = load_manifest();
        let mut checked = 0usize;
        for row in &manifest.fixtures {
            if row.qpdf != "must-pass" {
                continue;
            }
            let pdf = committed_pdf(row);
            let out = std::process::Command::new(&qpdf)
                .args(["--check"])
                .arg(&pdf)
                .output()
                .expect("run qpdf --check");
            assert!(
                out.status.success(),
                "CORPUS-QPDF-CHECK: qpdf --check failed for {}: {}",
                row.path,
                String::from_utf8_lossy(&out.stderr)
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "CORPUS-QPDF-CHECK: at least one fixture must be qpdf: must-pass"
        );
    }

    #[test]
    fn source_pages_contain_tj_and_kerned_tj() {
        let (manifest, _) = load_manifest();
        let tj = require_row(&manifest, "Tj");
        let kerned = require_row(&manifest, "TJ");
        for row in [tj, kerned] {
            let _ = regenerate(row);
        }
        let tj_blob = dump_streams(&committed_pdf(tj));
        assert!(
            has_operator(&tj_blob, "Tj"),
            "CORPUS-TEXT-TJ-TJ: fixture {} must contain a Tj operator; streams={tj_blob:?}",
            tj.id
        );
        let tj_arr_blob = dump_streams(&committed_pdf(kerned));
        assert!(
            has_operator(&tj_arr_blob, "TJ"),
            "CORPUS-TEXT-TJ-TJ: fixture {} must contain a TJ operator; streams={tj_arr_blob:?}",
            kerned.id
        );
        assert!(
            has_kerned_tj(&tj_arr_blob),
            "CORPUS-TEXT-TJ-TJ: fixture {} TJ array must include a numeric kerning offset; streams={tj_arr_blob:?}",
            kerned.id
        );
    }

    #[test]
    fn cid_pair_with_and_without_tounicode() {
        let (manifest, _) = load_manifest();
        let with = require_row(&manifest, "CID+ToUnicode");
        let without = require_row(&manifest, "CID-no-ToUnicode");
        assert_ne!(
            with.path, without.path,
            "CORPUS-CID-TOUNICODE: CID+ToUnicode and CID-no-ToUnicode must be a pair of fixtures"
        );
        for row in [with, without] {
            let _ = regenerate(row);
        }

        let with_doc = load_doc(&committed_pdf(with));
        assert!(
            has_cid_font(&with_doc),
            "CORPUS-CID-TOUNICODE: fixture {} must contain a CID/Type0 font",
            with.id
        );
        let with_ok = cid_fonts(&with_doc)
            .iter()
            .any(|d| tounicode_usable(&with_doc, d));
        assert!(
            with_ok,
            "CORPUS-CID-TOUNICODE: fixture {} must have a usable ToUnicode CMap",
            with.id
        );

        let without_doc = load_doc(&committed_pdf(without));
        assert!(
            has_cid_font(&without_doc),
            "CORPUS-CID-TOUNICODE: fixture {} must contain a CID/Type0 font",
            without.id
        );
        let without_has = cid_fonts(&without_doc)
            .iter()
            .any(|d| tounicode_usable(&without_doc, d));
        assert!(
            !without_has,
            "CORPUS-CID-TOUNICODE: fixture {} must not have a usable ToUnicode CMap",
            without.id
        );
    }

    #[test]
    fn type3_nested_form_rotated_and_skewed_text() {
        let (manifest, _) = load_manifest();
        let type3 = require_row(&manifest, "Type3");
        let nested = require_row(&manifest, "nested-form");
        let rotated = require_row(&manifest, "text-rotated");
        let skewed = require_row(&manifest, "text-skewed");
        for row in [type3, nested, rotated, skewed] {
            let _ = regenerate(row);
        }

        let type3_doc = load_doc(&committed_pdf(type3));
        assert!(
            has_type3(&type3_doc),
            "CORPUS-TEXT-TYPE3-FORM-XFORM: fixture {} must contain a /Subtype /Type3 font",
            type3.id
        );

        let nested_doc = load_doc(&committed_pdf(nested));
        assert!(
            has_nested_form_text(&nested_doc),
            "CORPUS-TEXT-TYPE3-FORM-XFORM: fixture {} must contain text in a Form XObject",
            nested.id
        );

        let rot_blob = dump_streams(&committed_pdf(rotated));
        assert!(
            tm_matrices(&rot_blob).iter().copied().any(is_rotated_tm),
            "CORPUS-TEXT-TYPE3-FORM-XFORM: fixture {} must contain a rotated Tm; streams={rot_blob:?}",
            rotated.id
        );

        let skew_blob = dump_streams(&committed_pdf(skewed));
        assert!(
            tm_matrices(&skew_blob).iter().copied().any(is_skewed_tm),
            "CORPUS-TEXT-TYPE3-FORM-XFORM: fixture {} must contain a skewed Tm; streams={skew_blob:?}",
            skewed.id
        );
    }

    #[test]
    fn image_unique_reused_form_inline_and_mask() {
        let (manifest, _) = load_manifest();
        let unique = require_row(&manifest, "image-unique");
        let reused = require_row(&manifest, "image-reused");
        let in_form = require_row(&manifest, "image-in-form");
        let inline = require_row(&manifest, "image-inline");
        let mask = require_row(&manifest, "image-mask");
        for row in [unique, reused, in_form, inline, mask] {
            let _ = regenerate(row);
        }

        let unique_doc = load_doc(&committed_pdf(unique));
        let unique_pages = image_ids_on_pages(&unique_doc);
        let mut unique_hits: HashMap<lopdf::ObjectId, usize> = HashMap::new();
        for ids in &unique_pages {
            let mut page_seen = HashSet::new();
            for id in ids {
                if page_seen.insert(*id) {
                    *unique_hits.entry(*id).or_insert(0) += 1;
                }
            }
        }
        assert!(
            unique_hits.values().any(|n| *n == 1),
            "CORPUS-IMAGE-SHAPES: fixture {} must paint a unique Image XObject on one page",
            unique.id
        );
        for dict in image_streams(&unique_doc) {
            if let (Some(w), Some(h)) = (image_dim(dict, b"Width"), image_dim(dict, b"Height")) {
                assert!(
                    w <= 64 && h <= 64,
                    "CORPUS-IMAGE-SHAPES: fixture {} images must be tiny, got {w}x{h}",
                    unique.id
                );
            }
        }

        let reused_doc = load_doc(&committed_pdf(reused));
        let reused_pages = image_ids_on_pages(&reused_doc);
        assert!(
            reused_pages.len() >= 2,
            "CORPUS-IMAGE-SHAPES: fixture {} must have multiple pages so an Image XObject can be reused",
            reused.id
        );
        let mut reused_hits: HashMap<lopdf::ObjectId, usize> = HashMap::new();
        for ids in &reused_pages {
            let mut page_seen = HashSet::new();
            for id in ids {
                if page_seen.insert(*id) {
                    *reused_hits.entry(*id).or_insert(0) += 1;
                }
            }
        }
        assert!(
            reused_hits.values().any(|n| *n >= 2),
            "CORPUS-IMAGE-SHAPES: fixture {} must reuse the same Image XObject on multiple pages",
            reused.id
        );

        let form_doc = load_doc(&committed_pdf(in_form));
        assert!(
            form_paints_image(&form_doc),
            "CORPUS-IMAGE-SHAPES: fixture {} must paint an Image XObject from a Form",
            in_form.id
        );

        let inline_blob = dump_streams(&committed_pdf(inline));
        assert!(
            has_operator(&inline_blob, "BI") && has_operator(&inline_blob, "EI"),
            "CORPUS-IMAGE-SHAPES: fixture {} must contain an inline image (BI…EI); streams={inline_blob:?}",
            inline.id
        );

        let mask_doc = load_doc(&committed_pdf(mask));
        assert!(
            image_streams(&mask_doc).iter().any(|d| has_image_mask(d)),
            "CORPUS-IMAGE-SHAPES: fixture {} must contain an Image with /Mask, /SMask, or /ImageMask",
            mask.id
        );
    }

    #[test]
    fn geom_rows_name_crop_userunit_and_rotates() {
        let (manifest, _) = load_manifest();
        for structure in [
            "crop-offset",
            "user-unit",
            "rotate-0",
            "rotate-90",
            "rotate-180",
            "rotate-270",
        ] {
            let row = require_row(&manifest, structure);
            let _ = regenerate(row);
            let doc = load_doc(&committed_pdf(row));
            let pages = page_dicts(&doc);
            assert!(
                !pages.is_empty(),
                "CORPUS-GEOM: fixture {} must have a page",
                row.id
            );

            match structure {
                "crop-offset" => {
                    assert!(
                        row.geometry.crop_offset,
                        "CORPUS-GEOM: fixture {} names crop-offset so geometry.cropOffset must be true",
                        row.id
                    );
                    let hit = pages.iter().any(|p| {
                        let media = page_box(p, b"MediaBox");
                        let crop = page_box(p, b"CropBox");
                        match (media, crop) {
                            (Some(m), Some(c)) if m.len() >= 2 && c.len() >= 2 => {
                                (c[0] - m[0]).abs() > 0.5 || (c[1] - m[1]).abs() > 0.5
                            }
                            _ => false,
                        }
                    });
                    assert!(
                        hit,
                        "CORPUS-GEOM: fixture {} must have CropBox origin offset from MediaBox",
                        row.id
                    );
                }
                "user-unit" => {
                    assert!(
                        (row.geometry.user_unit - 1.0).abs() > 1e-6,
                        "CORPUS-GEOM: fixture {} names user-unit so geometry.userUnit must not be 1",
                        row.id
                    );
                    let hit = pages.iter().any(|p| {
                        (page_user_unit(p) - row.geometry.user_unit).abs() < 1e-6
                    });
                    assert!(
                        hit,
                        "CORPUS-GEOM: fixture {} must set /UserUnit {}",
                        row.id, row.geometry.user_unit
                    );
                }
                "rotate-0" | "rotate-90" | "rotate-180" | "rotate-270" => {
                    let want: i64 = structure.rsplit('-').next().unwrap().parse().unwrap();
                    assert_eq!(
                        row.geometry.rotate, want,
                        "CORPUS-GEOM: fixture {} names {structure} so geometry.rotate must be {want}",
                        row.id
                    );
                    let hit = pages.iter().any(|p| page_rotate(p) == want);
                    assert!(
                        hit,
                        "CORPUS-GEOM: fixture {} must have a page with /Rotate {want}",
                        row.id
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn committed_pdfs_total_at_most_two_mib() {
        let dir = corpus_dir();
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_pdf = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false);
                if is_pdf {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        assert!(
            total <= SIZE_CAP,
            "CORPUS-SIZE: committed fixtures/source-edit/*.pdf sum {total} exceeds {SIZE_CAP}"
        );
    }

    #[test]
    fn manifest_never_claims_editable() {
        let (manifest, value) = load_manifest();
        assert_no_editable_key(&value, "manifest");
        for row in &manifest.fixtures {
            assert!(
                row.intent != "editable" && !row.intent.eq_ignore_ascii_case("editable: true"),
                "CORPUS-NO-EDITABLE-CLAIM: fixture {} intent must not claim editable ({:?})",
                row.id,
                row.intent
            );
        }
    }

}
