//! Fakes the publish gate must reject (SPEC §E.5, T4): cover-and-overlay (a new content stream
//! over the old text, or a real `qpdf --overlay`), a raster of the page, annotations, re-attached
//! old content, catalog and font changes, legacy-filter tampering, and writer bugs (wrong bytes,
//! wrong target). Built only with this test kit and real qpdf (`--overlay`, `--update-from-json`):
//! no production writer is used, so a production bug cannot make a fake pass by construction.

use lopdf::{Dictionary, Document, Object, ObjectId};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::process::Command;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A PDF name as qpdf JSON writes it (`/Name`, `#xx` for anything unusual).
fn json_name(n: &[u8]) -> String {
    let mut s = String::from("/");
    for &b in n {
        if b.is_ascii_alphanumeric() || b"-_.+*".contains(&b) {
            s.push(char::from(b));
        } else {
            s.push_str(&format!("#{b:02x}"));
        }
    }
    s
}

/// A lopdf value in qpdf JSON v2 form.
pub fn json_value(obj: &Object) -> Value {
    match obj {
        Object::Null => Value::Null,
        Object::Boolean(b) => json!(b),
        Object::Integer(i) => json!(i),
        Object::Real(r) => json!(f64::from(*r)),
        Object::Name(n) => json!(json_name(n)),
        Object::String(s, _) => json!(format!(
            "b:{}",
            s.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )),
        Object::Array(items) => Value::Array(items.iter().map(json_value).collect()),
        Object::Dictionary(d) => json_dict(d),
        Object::Stream(s) => json_dict(&s.dict),
        Object::Reference((n, g)) => json!(format!("{n} {g} R")),
    }
}

pub fn json_dict(d: &Dictionary) -> Value {
    let mut m = Map::new();
    for (k, v) in d.iter() {
        m.insert(json_name(k), json_value(v));
    }
    Value::Object(m)
}

pub fn stream_obj(dict: Value, data: &[u8]) -> Value {
    json!({ "stream": { "dict": dict, "data": base64(data) } })
}

pub fn value_obj(v: Value) -> Value {
    json!({ "value": v })
}

pub fn obj_key(id: ObjectId) -> String {
    format!("obj:{} {} R", id.0, id.1)
}

/// The file parsed by lopdf (test code may load freely).
pub fn load(path: &Path) -> Document {
    Document::load(path).unwrap_or_else(|e| panic!("fake input {}: {e}", path.display()))
}

/// The page object ids of `doc`, in order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

fn run_qpdf(qpdf: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let out = Command::new(qpdf)
        .args(args)
        .output()
        .map_err(|e| format!("qpdf: {e}"))?;
    match out.status.code() {
        Some(0) | Some(3) => Ok(()),
        c => Err(format!(
            "qpdf exited {c:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}

/// `qpdf input output --decode-level=none --update-from-json=…` with `objects`.
pub fn update(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    objects: Map<String, Value>,
    max_id: u32,
) -> Result<(), String> {
    let doc = json!({ "qpdf": [
        { "jsonversion": 2, "pushedinheritedpageresources": false, "calledgetallpages": false, "maxobjectid": max_id },
        Value::Object(objects),
    ]});
    let json_path = output.with_extension("fake.json");
    std::fs::write(
        &json_path,
        serde_json::to_vec(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut arg = std::ffi::OsString::from("--update-from-json=");
    arg.push(json_path.as_os_str());
    let r = run_qpdf(
        qpdf,
        &[
            input.as_os_str(),
            output.as_os_str(),
            std::ffi::OsStr::new("--decode-level=none"),
            arg.as_os_str(),
        ],
    );
    let _ = std::fs::remove_file(&json_path);
    r
}

/// Replaces the decoded data of streams (a writer that wrote other bytes than planned).
pub fn replace_streams(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    streams: &[(ObjectId, Vec<u8>)],
) -> Result<(), String> {
    let doc = load(input);
    let mut m = Map::new();
    for (id, data) in streams {
        m.insert(obj_key(*id), stream_obj(json!({}), data));
    }
    update(qpdf, input, output, m, doc.max_id)
}

/// Writes `raw` as the already-filtered data of stream `id` with dictionary `dict`.
pub fn raw_stream(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    id: ObjectId,
    dict: Value,
    raw: &[u8],
) -> Result<(), String> {
    let doc = load(input);
    let mut m = Map::new();
    m.insert(obj_key(id), stream_obj(dict, raw));
    update(qpdf, input, output, m, doc.max_id)
}

/// Replaces stream `id`'s dictionary, keeping its data.
pub fn stream_dict(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    id: ObjectId,
    dict: Value,
) -> Result<(), String> {
    let doc = load(input);
    let mut m = Map::new();
    m.insert(obj_key(id), json!({ "stream": { "dict": dict } }));
    update(qpdf, input, output, m, doc.max_id)
}

/// Replaces object `id` with a (non-stream) value.
pub fn set_value(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    id: ObjectId,
    value: Value,
) -> Result<(), String> {
    let doc = load(input);
    let mut m = Map::new();
    m.insert(obj_key(id), value_obj(value));
    update(qpdf, input, output, m, doc.max_id)
}

fn page_dict(doc: &Document, page: ObjectId) -> Dictionary {
    doc.get_dictionary(page).cloned().unwrap_or_default()
}

fn contents_refs(d: &Dictionary) -> Vec<Value> {
    match d.get(b"Contents") {
        Ok(Object::Reference(r)) => vec![json_value(&Object::Reference(*r))],
        Ok(Object::Array(items)) => items.iter().map(json_value).collect(),
        _ => Vec::new(),
    }
}

/// Adds content streams before and after the page's parts (as flattening passes do).
pub fn add_parts(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    before: &[&[u8]],
    after: &[&[u8]],
) -> Result<(), String> {
    let doc = load(input);
    let page = *page_ids(&doc).get(page_index).ok_or("page")?;
    let mut d = json_dict(&page_dict(&doc, page));
    let mut next = doc.max_id + 1;
    let mut m = Map::new();
    let mut new_ref = |data: &[u8], m: &mut Map<String, Value>| {
        let id = (next, 0);
        next += 1;
        m.insert(obj_key(id), stream_obj(json!({}), data));
        json!(format!("{} 0 R", id.0))
    };
    let mut parts: Vec<Value> = before.iter().map(|p| new_ref(p, &mut m)).collect();
    parts.extend(contents_refs(&page_dict(&doc, page)));
    parts.extend(after.iter().map(|p| new_ref(p, &mut m)));
    if let Some(o) = d.as_object_mut() {
        o.insert("/Contents".into(), Value::Array(parts));
    }
    m.insert(obj_key(page), value_obj(d));
    update(qpdf, input, output, m, doc.max_id)
}

/// GATE-18 cover-and-overlay F1: the original parts stay; a new content stream paints a white box
/// over `cover` (`[x, y, w, h]`) and draws `text` with the page font `font` at its corner.
pub fn cover_and_overlay_f1(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    cover: [f64; 4],
    font: &str,
    text: &str,
) -> Result<(), String> {
    let [x, y, w, h] = cover;
    let fake = format!(
        "q 1 g {x} {y} {w} {h} re f Q BT /{font} 12 Tf {x} {} Td ({text}) Tj ET",
        y + 2.0
    );
    add_parts(qpdf, input, output, page_index, &[], &[fake.as_bytes()])
}

/// A one-page PDF (Helvetica `/F1`) drawing a white box over `cover` and `text` in it.
pub fn cover_page(media: [f64; 4], cover: [f64; 4], text: &str) -> Vec<u8> {
    use super::producers::{DocBuilder, PageSpec, HELVETICA};
    let [x, y, w, h] = cover;
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let content = format!(
        "q 1 g {x} {y} {w} {h} re f Q BT /F1 12 Tf {x} {} Td ({text}) Tj ET",
        y + 2.0
    );
    let media = format!("[{} {} {} {}]", media[0], media[1], media[2], media[3]);
    d.page(
        PageSpec::new(content.as_bytes(), &format!("/Font << /F1 {f} 0 R >>")).media(Some(&media)),
    );
    d.build()
}

/// GATE-19 cover-and-overlay F2: a real `qpdf --overlay` of a white box + text page onto page
/// `page_1` (1-based).
pub fn overlay_f2(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_1: u32,
    media: [f64; 4],
    cover: [f64; 4],
    text: &str,
) -> Result<(), String> {
    let cover_pdf = output.with_extension("cover.pdf");
    std::fs::write(&cover_pdf, cover_page(media, cover, text)).map_err(|e| e.to_string())?;
    overlay(qpdf, input, output, &cover_pdf, Some(page_1))
}

/// `qpdf input --overlay overlay [--to=n] -- output` (every page when `to` is `None`).
pub fn overlay(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    overlay_pdf: &Path,
    to: Option<u32>,
) -> Result<(), String> {
    let to_arg = to.map(|n| format!("--to={n}"));
    let mut args: Vec<&std::ffi::OsStr> = vec![
        input.as_os_str(),
        std::ffi::OsStr::new("--overlay"),
        overlay_pdf.as_os_str(),
    ];
    if let Some(t) = &to_arg {
        args.push(std::ffi::OsStr::new(t));
    }
    args.push(std::ffi::OsStr::new("--"));
    args.push(output.as_os_str());
    run_qpdf(qpdf, &args)
}

/// `qpdf --empty --pages a.pdf b.pdf … -- output` (assembly with other files).
pub fn assemble(qpdf: &Path, inputs: &[&Path], output: &Path) -> Result<(), String> {
    let mut args: Vec<&std::ffi::OsStr> = vec![
        std::ffi::OsStr::new("--empty"),
        std::ffi::OsStr::new("--pages"),
    ];
    for i in inputs {
        args.push(i.as_os_str());
    }
    args.push(std::ffi::OsStr::new("--"));
    args.push(output.as_os_str());
    run_qpdf(qpdf, &args)
}

/// Reads a binary PPM (P6) written by pdftoppm.
fn read_ppm(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    let mut at = 0usize;
    while fields.len() < 4 {
        while data.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let start = at;
        while data.get(at).is_some_and(|c| !c.is_ascii_whitespace()) {
            at += 1;
        }
        fields.push(String::from_utf8_lossy(&data[start..at]).into_owned());
    }
    let w: u32 = fields[1].parse().map_err(|_| "ppm width")?;
    let h: u32 = fields[2].parse().map_err(|_| "ppm height")?;
    Ok((w, h, data[at + 1..].to_vec()))
}

/// GATE-20 raster fake: the page content replaced by `q W 0 0 H X Y cm /Im0 Do Q` drawing a
/// pdftoppm render of `rendered` (the honestly edited page) over the media box.
pub fn raster_fake(
    qpdf: &Path,
    pdftoppm: &Path,
    rendered: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
) -> Result<(), String> {
    let prefix = output.with_extension("render");
    let page = (page_index + 1).to_string();
    let status = Command::new(pdftoppm)
        .args(["-r", "72", "-f", &page, "-l", &page, "-singlefile"])
        .arg(rendered)
        .arg(&prefix)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("pdftoppm".into());
    }
    let mut ppm = prefix.into_os_string();
    ppm.push(".ppm");
    let ppm = std::path::PathBuf::from(ppm);
    let (w, h, rgb) = read_ppm(&ppm)?;
    let _ = std::fs::remove_file(&ppm);
    let doc = load(input);
    let page_id = *page_ids(&doc).get(page_index).ok_or("page")?;
    let media = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|d| d.get(b"MediaBox").ok())
        .and_then(|m| m.as_array().ok())
        .map(|a| {
            a.iter()
                .filter_map(|o| o.as_float().ok())
                .collect::<Vec<f32>>()
        })
        .unwrap_or_else(|| vec![0.0, 0.0, 612.0, 792.0]);
    let (x0, y0, x1, y1) = (media[0], media[1], media[2], media[3]);
    let image = (doc.max_id + 1, 0);
    let content = (doc.max_id + 2, 0);
    let mut m = Map::new();
    m.insert(
        obj_key(image),
        stream_obj(
            json!({ "/Type": "/XObject", "/Subtype": "/Image", "/Width": w, "/Height": h,
                    "/ColorSpace": "/DeviceRGB", "/BitsPerComponent": 8 }),
            &rgb,
        ),
    );
    let draw = format!("q {} 0 0 {} {x0} {y0} cm /Im0 Do Q", x1 - x0, y1 - y0);
    m.insert(obj_key(content), stream_obj(json!({}), draw.as_bytes()));
    let mut d = json_dict(&page_dict(&doc, page_id));
    if let Some(o) = d.as_object_mut() {
        o.insert("/Contents".into(), json!(format!("{} 0 R", content.0)));
        o.insert(
            "/Resources".into(),
            json!({ "/XObject": { "/Im0": format!("{} 0 R", image.0) } }),
        );
    }
    m.insert(obj_key(page_id), value_obj(d));
    update(qpdf, input, output, m, doc.max_id)
}

/// Adds an annotation dictionary (`annot`, with `/P` set here) to page `page_index`.
fn add_annotation(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    mut annot: Map<String, Value>,
    appearance: Option<&[u8]>,
) -> Result<(), String> {
    let doc = load(input);
    let page = *page_ids(&doc).get(page_index).ok_or("page")?;
    let annot_id = (doc.max_id + 1, 0);
    let mut m = Map::new();
    annot.insert("/P".into(), json!(format!("{} {} R", page.0, page.1)));
    if let Some(ap) = appearance {
        let ap_id = (doc.max_id + 2, 0);
        let rect = annot
            .get("/Rect")
            .cloned()
            .unwrap_or(json!([0, 0, 100, 20]));
        m.insert(
            obj_key(ap_id),
            stream_obj(
                json!({ "/Type": "/XObject", "/Subtype": "/Form", "/BBox": rect }),
                ap,
            ),
        );
        annot.insert("/AP".into(), json!({ "/N": format!("{} 0 R", ap_id.0) }));
    }
    m.insert(obj_key(annot_id), value_obj(Value::Object(annot)));
    let mut d = json_dict(&page_dict(&doc, page));
    if let Some(o) = d.as_object_mut() {
        let mut annots = match o.get("/Annots") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        annots.push(json!(format!("{} 0 R", annot_id.0)));
        o.insert("/Annots".into(), Value::Array(annots));
    }
    m.insert(obj_key(page), value_obj(d));
    update(qpdf, input, output, m, doc.max_id)
}

/// GATE-22: a FreeText annotation showing `text` over `rect` (`[x0, y0, x1, y1]`).
pub fn freetext(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    rect: [f64; 4],
    text: &str,
) -> Result<(), String> {
    let mut a = Map::new();
    a.insert("/Type".into(), json!("/Annot"));
    a.insert("/Subtype".into(), json!("/FreeText"));
    a.insert("/Rect".into(), json!(rect));
    a.insert(
        "/Contents".into(),
        json!(format!(
            "b:{}",
            text.bytes().map(|b| format!("{b:02x}")).collect::<String>()
        )),
    );
    a.insert("/DA".into(), json!("u:/Helv 12 Tf 0 g"));
    let ap = format!(
        "q 1 g 0 0 {} {} re f Q",
        rect[2] - rect[0],
        rect[3] - rect[1]
    );
    add_annotation(qpdf, input, output, page_index, a, Some(ap.as_bytes()))
}

/// GATE-26: a Link annotation on page `page_index`.
pub fn link(qpdf: &Path, input: &Path, output: &Path, page_index: usize) -> Result<(), String> {
    let mut a = Map::new();
    a.insert("/Type".into(), json!("/Annot"));
    a.insert("/Subtype".into(), json!("/Link"));
    a.insert("/Rect".into(), json!([72, 72, 144, 96]));
    a.insert("/Border".into(), json!([0, 0, 0]));
    a.insert(
        "/A".into(),
        json!({ "/S": "/URI", "/URI": "u:https://example.invalid/" }),
    );
    add_annotation(qpdf, input, output, page_index, a, None)
}

/// GATE-24: `old` (the original content) re-attached as an unused Form XObject `/Old`.
pub fn reattach_old(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    old: &[u8],
) -> Result<(), String> {
    let doc = load(input);
    let page = *page_ids(&doc).get(page_index).ok_or("page")?;
    let form = (doc.max_id + 1, 0);
    let mut m = Map::new();
    m.insert(
        obj_key(form),
        stream_obj(
            json!({ "/Type": "/XObject", "/Subtype": "/Form", "/BBox": [0, 0, 612, 792] }),
            old,
        ),
    );
    let pd = page_dict(&doc, page);
    let mut d = json_dict(&pd);
    let mut resources = match pd.get(b"Resources") {
        Ok(Object::Reference(r)) => doc
            .get_dictionary(*r)
            .map(json_dict)
            .unwrap_or_else(|_| json!({})),
        Ok(Object::Dictionary(r)) => json_dict(r),
        _ => json!({}),
    };
    if let Some(r) = resources.as_object_mut() {
        r.insert(
            "/XObject".into(),
            json!({ "/Old": format!("{} 0 R", form.0) }),
        );
    }
    if let Some(o) = d.as_object_mut() {
        o.insert("/Resources".into(), resources);
    }
    m.insert(obj_key(page), value_obj(d));
    update(qpdf, input, output, m, doc.max_id)
}

/// The catalog id of `doc`.
pub fn catalog_id(doc: &Document) -> ObjectId {
    doc.trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .unwrap_or((1, 0))
}

/// GATE-27: the first group of `/OCProperties /D /ON` moved to `/OFF` (the catalog rewritten).
pub fn ocg_off(qpdf: &Path, input: &Path, output: &Path, ocg: ObjectId) -> Result<(), String> {
    let doc = load(input);
    let cat_id = catalog_id(&doc);
    let mut cat = json_dict(&doc.get_dictionary(cat_id).cloned().unwrap_or_default());
    let r = json!(format!("{} {} R", ocg.0, ocg.1));
    if let Some(props) = cat.get_mut("/OCProperties").and_then(Value::as_object_mut) {
        if let Some(d) = props.get_mut("/D").and_then(Value::as_object_mut) {
            let mut off = match d.get("/OFF") {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            off.push(r.clone());
            d.insert("/OFF".into(), Value::Array(off));
            if let Some(Value::Array(on)) = d.get_mut("/ON") {
                on.retain(|v| *v != r);
            }
        }
    }
    let mut m = Map::new();
    m.insert(obj_key(cat_id), value_obj(cat));
    update(qpdf, input, output, m, doc.max_id)
}

/// GATE-28: entry `index` of font `font`'s `/Widths` set to `value`.
pub fn widths_changed(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    font: ObjectId,
    index: usize,
    value: f64,
) -> Result<(), String> {
    let doc = load(input);
    let mut d = json_dict(&doc.get_dictionary(font).cloned().unwrap_or_default());
    if let Some(Value::Array(w)) = d.get_mut("/Widths") {
        if let Some(slot) = w.get_mut(index) {
            *slot = json!(value);
        }
    }
    let mut m = Map::new();
    m.insert(obj_key(font), value_obj(d));
    update(qpdf, input, output, m, doc.max_id)
}

/// APP-04 (probe P4 shape): an update aimed at the page object instead of its content stream.
pub fn mistargeted(
    qpdf: &Path,
    input: &Path,
    output: &Path,
    page_index: usize,
    data: &[u8],
) -> Result<(), String> {
    let doc = load(input);
    let page = *page_ids(&doc).get(page_index).ok_or("page")?;
    let mut m = Map::new();
    m.insert(obj_key(page), stream_obj(json!({}), data));
    update(qpdf, input, output, m, doc.max_id)
}

/// Review H-1: `program`, a CID-keyed program from `testkit::cff::CffBuilder`, with `font_matrix`
/// (DICT operand bytes) as the `FontMatrix` of its one Font DICT. A new FDArray INDEX is appended
/// and the Top DICT's fixed-width `FDArray` offset (`29 <i32> 12 36`) is patched to point at it.
pub fn cff_with_font_dict_matrix(program: &[u8], font_matrix: &[u8]) -> Vec<u8> {
    use crate::pdf_engine::text_edit::testkit::cff::index;
    let p = program;
    let at = p
        .windows(7)
        .position(|w| w[0] == 29 && w[5] == 12 && w[6] == 36)
        .expect("an FDArray entry");
    let old = u32::from_be_bytes([p[at + 1], p[at + 2], p[at + 3], p[at + 4]]) as usize;
    // The builder's FDArray: count 1, offSize 4, offsets 1 and 1 + len, then the Font DICT.
    assert_eq!(&p[old..old + 7], &[0, 1, 4, 0, 0, 0, 1], "one Font DICT");
    let end = u32::from_be_bytes([p[old + 7], p[old + 8], p[old + 9], p[old + 10]]);
    let dict = &p[old + 11..old + 10 + end as usize];
    let mut font_dict = font_matrix.to_vec();
    font_dict.extend([12, 7]);
    font_dict.extend_from_slice(dict);
    let mut out = p.to_vec();
    let new_at = u32::try_from(out.len()).expect("small program");
    out[at + 1..at + 5].copy_from_slice(&new_at.to_be_bytes());
    out.extend(index(&[font_dict]));
    out
}
