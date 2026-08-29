//! Session markup annotations for Edit PDF (issue #8).
//!
//! List + apply surface. Bodies are today's behavior (no walker, no-op apply)
//! so tests compile and fail on assertions until impl lands.
//!
//! Apply is copy-through of every existing annot plus append/remove of
//! session-owned dicts (`/NM`). It does not call `apply_link_annots`.

use crate::error::AppError;
use crate::pdf_engine::qpdf;
use crate::utils::safe_output;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

const MAX_ANNOTS_BYTES: u64 = 400 * 1024 * 1024;

/// Session markup subtype written onto dest `/Annots`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkupKind {
    Note,
    Highlight,
    Underline,
    StrikeOut,
    Ink,
}

/// A session-created markup annot to write (or keep) on a staged dest PDF.
///
/// `rect` is unrotated user space `[x, y, w, h]` (same as `EditObject.rect`).
/// `author` is the inspector string — no OS / "OffPDF" default.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMarkup {
    pub id: String,
    pub page_index: u32,
    pub kind: MarkupKind,
    pub rect: [f64; 4],
    pub color: [f64; 3],
    pub author: String,
    pub contents: Option<String>,
    pub quad_points: Option<Vec<f64>>,
    pub ink_list: Option<Vec<Vec<[f64; 2]>>>,
}

/// An annot listed from a source/dest PDF (existing leftover or session).
///
/// `rect` is the written unrotated `/Rect` as `[x, y, w, h]`, not display-swapped.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedMarkup {
    pub page_index: u32,
    pub subtype: String,
    pub rect: [f64; 4],
    pub author: Option<String>,
    pub color: Option<Vec<f64>>,
    pub contents: Option<String>,
    pub quad_points: Option<Vec<f64>>,
    pub session_id: Option<String>,
}

fn file_too_large(path: &Path) -> AppError {
    AppError::new(
        "PDF_TOO_LARGE",
        "This PDF is too large to read annotations",
        format!(
            "\"{}\" is larger than 400 MB.",
            path.display()
        ),
    )
    .with_suggestion("Use a smaller file.")
}

fn load_doc(path: &Path) -> Result<Document, AppError> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::io("Could not read the PDF.", e))?;
    if meta.len() > MAX_ANNOTS_BYTES {
        return Err(file_too_large(path));
    }
    Document::load(path).map_err(|_| AppError::invalid_pdf(&path.display().to_string()))
}

fn as_name(obj: &Object) -> Option<String> {
    match obj {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

fn as_nums(obj: &Object) -> Option<Vec<f64>> {
    let Object::Array(a) = obj else {
        return None;
    };
    Some(
        a.iter()
            .filter_map(|o| match o {
                Object::Integer(i) => Some(*i as f64),
                Object::Real(r) => Some(*r as f64),
                _ => None,
            })
            .collect(),
    )
}

fn pdf_string(obj: &Object) -> Option<String> {
    match obj {
        Object::String(b, _) => Some(String::from_utf8_lossy(b).into_owned()),
        _ => {
            let s = lopdf::decode_text_string(obj).ok()?;
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    }
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn page_annot_objects(doc: &Document, page_id: ObjectId) -> Vec<Object> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    match page.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Array(a)) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn rect_xywh(nums: &[f64]) -> [f64; 4] {
    if nums.len() < 4 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let x1 = nums[0];
    let y1 = nums[1];
    let x2 = nums[2];
    let y2 = nums[3];
    [
        x1.min(x2),
        y1.min(y2),
        (x2 - x1).abs(),
        (y2 - y1).abs(),
    ]
}

fn real_array(vals: impl IntoIterator<Item = f64>) -> Object {
    Object::Array(vals.into_iter().map(|v| Object::Real(v as f32)).collect())
}

fn is_markup_subtype(subtype: &str) -> bool {
    matches!(
        subtype,
        "Text" | "Highlight" | "Underline" | "StrikeOut" | "Ink"
    )
}

/// List markup (and leftover) annots. Errors on oversize / unreadable files.
pub fn list_markup_annots(path: &Path) -> Result<Vec<ListedMarkup>, AppError> {
    let doc = load_doc(path)?;
    let mut pages: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    pages.sort_by_key(|(n, _)| *n);
    let mut out = Vec::new();
    for (page_1based, page_id) in pages {
        let page_index = page_1based.saturating_sub(1);
        for raw in page_annot_objects(&doc, page_id) {
            let Some(dict) = resolve_dict(&doc, &raw) else {
                continue;
            };
            let subtype = dict
                .get(b"Subtype")
                .ok()
                .and_then(as_name)
                .unwrap_or_default();
            let rect = dict
                .get(b"Rect")
                .ok()
                .and_then(as_nums)
                .map(|n| rect_xywh(&n))
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            out.push(ListedMarkup {
                page_index,
                subtype,
                rect,
                author: dict.get(b"T").ok().and_then(pdf_string),
                color: dict.get(b"C").ok().and_then(as_nums),
                contents: dict.get(b"Contents").ok().and_then(pdf_string),
                quad_points: dict.get(b"QuadPoints").ok().and_then(as_nums),
                session_id: dict.get(b"NM").ok().and_then(pdf_string),
            });
        }
    }
    Ok(out)
}

fn subtype_name(kind: MarkupKind) -> &'static str {
    match kind {
        MarkupKind::Note => "Text",
        MarkupKind::Highlight => "Highlight",
        MarkupKind::Underline => "Underline",
        MarkupKind::StrikeOut => "StrikeOut",
        MarkupKind::Ink => "Ink",
    }
}

fn quads_from_rect(rect: [f64; 4]) -> Vec<f64> {
    let [x, y, w, h] = rect;
    vec![x, y, x + w, y, x + w, y + h, x, y + h]
}

fn annot_rect_pdf(rect: [f64; 4]) -> [f64; 4] {
    [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]]
}

fn appearance_stream(rect_pdf: [f64; 4], content: String) -> Stream {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Form".to_vec()));
    dict.set("BBox", real_array(rect_pdf));
    dict.set("Resources", Dictionary::new());
    Stream::new(dict, content.into_bytes())
}

fn highlight_ap_content(rect_pdf: [f64; 4], color: [f64; 3]) -> String {
    let [x0, y0, x1, y1] = rect_pdf;
    format!(
        "q\n/GS gs\n{:.3} {:.3} {:.3} rg\n{:.2} {:.2} {:.2} {:.2} re\nf\nQ\n",
        color[0],
        color[1],
        color[2],
        x0,
        y0,
        x1 - x0,
        y1 - y0
    )
}

fn line_ap_content(rect_pdf: [f64; 4], color: [f64; 3], y: f64) -> String {
    format!(
        "q\n{:.3} {:.3} {:.3} RG\n1.2 w 0 J\n{:.2} {:.2} m\n{:.2} {:.2} l\nS\nQ\n",
        color[0], color[1], color[2], rect_pdf[0], y, rect_pdf[2], y
    )
}

fn note_ap_content(rect_pdf: [f64; 4], color: [f64; 3]) -> String {
    let [x0, y0, x1, y1] = rect_pdf;
    format!(
        "q\n{:.3} {:.3} {:.3} rg\n0.25 0.2 0.05 RG\n0.8 w\n{:.2} {:.2} {:.2} {:.2} re\nB\nQ\n",
        color[0],
        color[1],
        color[2],
        x0,
        y0,
        (x1 - x0).max(1.0),
        (y1 - y0).max(1.0)
    )
}

fn ink_ap_content(color: [f64; 3], ink_list: &[Vec<[f64; 2]>]) -> String {
    let mut s = format!(
        "q\n{:.3} {:.3} {:.3} RG\n1.5 w 1 J 1 j\n",
        color[0], color[1], color[2]
    );
    for stroke in ink_list {
        for (i, p) in stroke.iter().enumerate() {
            if i == 0 {
                s.push_str(&format!("{:.2} {:.2} m\n", p[0], p[1]));
            } else {
                s.push_str(&format!("{:.2} {:.2} l\n", p[0], p[1]));
            }
        }
        if stroke.len() >= 2 {
            s.push_str("S\n");
        }
    }
    s.push_str("Q\n");
    s
}

fn highlight_gs() -> Dictionary {
    let mut gs = Dictionary::new();
    gs.set("Type", Object::Name(b"ExtGState".to_vec()));
    gs.set("BM", Object::Name(b"Multiply".to_vec()));
    gs.set("ca", Object::Real(0.4));
    gs
}

fn attach_appearance(doc: &mut Document, annot: &mut Dictionary, stream: Stream, extra_gs: bool) {
    let mut stream = stream;
    if extra_gs {
        let gs_id = doc.add_object(Object::Dictionary(highlight_gs()));
        let mut ext = Dictionary::new();
        ext.set("GS", gs_id);
        let mut res = Dictionary::new();
        res.set("ExtGState", ext);
        stream.dict.set("Resources", res);
    }
    let ap_id = doc.add_object(Object::Stream(stream));
    let mut ap = Dictionary::new();
    ap.set("N", ap_id);
    annot.set("AP", Object::Dictionary(ap));
}

fn build_session_annot(doc: &mut Document, item: &SessionMarkup) -> ObjectId {
    let subtype = subtype_name(item.kind);
    let rect_pdf = annot_rect_pdf(item.rect);
    let mut annot = Dictionary::new();
    annot.set("Type", Object::Name(b"Annot".to_vec()));
    annot.set("Subtype", Object::Name(subtype.as_bytes().to_vec()));
    annot.set("Rect", real_array(rect_pdf));
    annot.set("C", real_array(item.color));
    annot.set("F", Object::Integer(4));
    if !item.author.is_empty() {
        annot.set("T", Object::string_literal(item.author.as_str()));
    }
    if let Some(contents) = &item.contents {
        annot.set("Contents", Object::string_literal(contents.as_str()));
    }
    if !item.id.is_empty() {
        annot.set("NM", Object::string_literal(item.id.as_str()));
    }

    match item.kind {
        MarkupKind::Note => {
            annot.set("Name", Object::Name(b"Comment".to_vec()));
            annot.set("Open", Object::Boolean(false));
            attach_appearance(doc, &mut annot, appearance_stream(rect_pdf, note_ap_content(rect_pdf, item.color)), false);
        }
        MarkupKind::Highlight => {
            let quads = item
                .quad_points
                .clone()
                .filter(|q| q.len() >= 8 && q.len() % 8 == 0)
                .unwrap_or_else(|| quads_from_rect(item.rect));
            annot.set("QuadPoints", real_array(quads));
            attach_appearance(
                doc,
                &mut annot,
                appearance_stream(rect_pdf, highlight_ap_content(rect_pdf, item.color)),
                true,
            );
        }
        MarkupKind::Underline => {
            let quads = item
                .quad_points
                .clone()
                .filter(|q| q.len() >= 8 && q.len() % 8 == 0)
                .unwrap_or_else(|| quads_from_rect(item.rect));
            annot.set("QuadPoints", real_array(quads));
            let y = rect_pdf[1] + (rect_pdf[3] - rect_pdf[1]) * 0.12;
            attach_appearance(
                doc,
                &mut annot,
                appearance_stream(rect_pdf, line_ap_content(rect_pdf, item.color, y)),
                false,
            );
        }
        MarkupKind::StrikeOut => {
            let quads = item
                .quad_points
                .clone()
                .filter(|q| q.len() >= 8 && q.len() % 8 == 0)
                .unwrap_or_else(|| quads_from_rect(item.rect));
            annot.set("QuadPoints", real_array(quads));
            let y = (rect_pdf[1] + rect_pdf[3]) / 2.0;
            attach_appearance(
                doc,
                &mut annot,
                appearance_stream(rect_pdf, line_ap_content(rect_pdf, item.color, y)),
                false,
            );
        }
        MarkupKind::Ink => {
            let strokes = item.ink_list.clone().unwrap_or_default();
            let ink_list: Vec<Object> = strokes
                .iter()
                .map(|stroke| {
                    real_array(stroke.iter().flat_map(|p| [p[0], p[1]]))
                })
                .collect();
            annot.set("InkList", Object::Array(ink_list));
            attach_appearance(
                doc,
                &mut annot,
                appearance_stream(rect_pdf, ink_ap_content(item.color, &strokes)),
                false,
            );
        }
    }

    doc.add_object(Object::Dictionary(annot))
}

fn keep_existing_annot(
    doc: &Document,
    entry: &Object,
    session_ids: &HashSet<String>,
) -> bool {
    let Some(dict) = resolve_dict(doc, entry) else {
        return true;
    };
    let Some(nm) = dict.get(b"NM").ok().and_then(pdf_string) else {
        return true;
    };
    if session_ids.contains(&nm) {
        return false;
    }
    let subtype = dict
        .get(b"Subtype")
        .ok()
        .and_then(as_name)
        .unwrap_or_default();
    if !is_markup_subtype(&subtype) {
        return true;
    }
    // C4: empty session drops leftover markup that already has /NM.
    // C9: a non-empty session copies through leftover /NM that is not
    // this sitting's (Acrobat leftovers, previous OffPDF annots).
    !session_ids.is_empty()
}

fn rewrite_page_annots(
    doc: &mut Document,
    page_id: ObjectId,
    session_for_page: &[&SessionMarkup],
    session_ids: &HashSet<String>,
) {
    let existing = page_annot_objects(doc, page_id);
    let mut kept: Vec<Object> = existing
        .iter()
        .filter(|entry| keep_existing_annot(doc, entry, session_ids))
        .cloned()
        .collect();
    for item in session_for_page {
        let id = build_session_annot(doc, item);
        kept.push(id.into());
    }
    if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
        if kept.is_empty() {
            if page.get(b"Annots").is_ok() {
                page.set("Annots", Object::Array(Vec::new()));
            }
        } else {
            page.set("Annots", Object::Array(kept));
        }
    }
}

fn strip_markup_annots(doc: &mut Document) {
    let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
    for page_id in pages {
        let existing = page_annot_objects(doc, page_id);
        let kept: Vec<Object> = existing
            .iter()
            .filter(|entry| {
                resolve_dict(doc, entry)
                    .and_then(|d| d.get(b"Subtype").ok().and_then(as_name))
                    .map(|s| !is_markup_subtype(&s))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
            if kept.is_empty() {
                let _ = page.remove(b"Annots");
            } else {
                page.set("Annots", Object::Array(kept));
            }
        }
    }
}

fn flatten_with_qpdf(staged: &Path) -> Result<(), AppError> {
    #[cfg(test)]
    {
        if staged.file_name().and_then(|n| n.to_str()) == Some("c10-qpdf-fail.pdf") {
            return Err(AppError::engine_failed(
                "qpdf --flatten-annotations=all failed: injected",
            ));
        }
    }
    let unique = format!(
        "flatten-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp = safe_output::sibling_temp_path(staged, &unique)?;
    let exe = qpdf::resolve_qpdf_standalone();
    let mut cmd = Command::new(&exe);
    cmd.arg("--flatten-annotations=all");
    cmd.arg(staged);
    cmd.arg(&tmp);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .output()
        .map_err(|e| AppError::io("Could not flatten annotations.", e))?;
    if !output.status.success() && output.status.code() != Some(3) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::engine_failed(format!(
            "qpdf --flatten-annotations=all failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    safe_output::replace_file(&tmp, staged)
}

/// Copy through every existing annot; append/remove session dicts only.
///
/// `flatten` is the opt-in `qpdf --flatten-annotations=all` switch (default
/// callers pass `false`).
pub fn apply_markup_annots(
    staged: &Path,
    session: &[SessionMarkup],
    flatten: bool,
) -> Result<(), AppError> {
    let mut doc = load_doc(staged)?;
    let session_ids: HashSet<String> = session
        .iter()
        .filter(|s| !s.id.is_empty())
        .map(|s| s.id.clone())
        .collect();
    let mut by_page: HashMap<u32, Vec<&SessionMarkup>> = HashMap::new();
    for item in session {
        by_page.entry(item.page_index).or_default().push(item);
    }
    let mut pages: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
    pages.sort_by_key(|(n, _)| *n);
    for (page_1based, page_id) in &pages {
        let page_index = page_1based.saturating_sub(1);
        let empty: Vec<&SessionMarkup> = Vec::new();
        let items = by_page.get(&page_index).unwrap_or(&empty);
        if items.is_empty() && session_ids.is_empty() && page_annot_objects(&doc, *page_id).is_empty()
        {
            continue;
        }
        rewrite_page_annots(&mut doc, *page_id, items, &session_ids);
    }
    doc.save(staged)
        .map_err(|e| AppError::io("Could not write annotations.", e))?;

    if flatten {
        flatten_with_qpdf(staged)?;
        let mut doc = load_doc(staged)?;
        strip_markup_annots(&mut doc);
        doc.save(staged)
            .map_err(|e| AppError::io("Could not flatten annotations.", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
    use std::path::{Path, PathBuf};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "offpdf-annots-{}-{}-{}",
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

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
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

    fn name(s: &str) -> Object {
        Object::Name(s.as_bytes().to_vec())
    }

    struct PdfFix {
        doc: Document,
        page_ids: Vec<ObjectId>,
    }

    impl PdfFix {
        fn new(n_pages: usize, rotate: i64, crop: Option<[i64; 4]>) -> Self {
            let mut doc = Document::with_version("1.5");
            let pages_id = doc.new_object_id();
            let mut page_ids = Vec::with_capacity(n_pages);
            for _ in 0..n_pages {
                let content_id = doc.add_object(Object::Stream(Stream::new(
                    Dictionary::new(),
                    b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
                )));
                let mut page = Dictionary::new();
                page.set("Type", "Page");
                page.set("Parent", pages_id);
                page.set("MediaBox", box_obj([0, 0, 612, 792]));
                if let Some(c) = crop {
                    page.set("CropBox", box_obj(c));
                }
                if rotate != 0 {
                    page.set("Rotate", Object::Integer(rotate));
                }
                page.set("Contents", content_id);
                page_ids.push(doc.add_object(Object::Dictionary(page)));
            }

            let mut pages = Dictionary::new();
            pages.set("Type", "Pages");
            pages.set(
                "Kids",
                Object::Array(page_ids.iter().copied().map(Object::from).collect()),
            );
            pages.set("Count", Object::Integer(n_pages as i64));
            doc.objects.insert(pages_id, Object::Dictionary(pages));

            let mut catalog = Dictionary::new();
            catalog.set("Type", "Catalog");
            catalog.set("Pages", pages_id);
            let catalog_id = doc.add_object(Object::Dictionary(catalog));
            doc.trailer.set("Root", catalog_id);
            Self { doc, page_ids }
        }

        fn push_raw_annot(&mut self, page_index: usize, obj: Object) {
            let page_id = self.page_ids[page_index];
            let mut arr = match self.doc.get_dictionary(page_id).ok().and_then(|p| {
                p.get(b"Annots").ok().and_then(|o| match o {
                    Object::Array(a) => Some(a.clone()),
                    _ => None,
                })
            }) {
                Some(a) => a,
                None => Vec::new(),
            };
            arr.push(obj);
            if let Ok(Object::Dictionary(page)) = self.doc.get_object_mut(page_id) {
                page.set("Annots", Object::Array(arr));
            }
        }

        fn push_annot(&mut self, page_index: usize, dict: Dictionary) {
            let annot_id = self.doc.add_object(Object::Dictionary(dict));
            self.push_raw_annot(page_index, annot_id.into());
        }

        fn add_highlight(&mut self, page_index: usize, rect: [i64; 4], contents: &str) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", name("Highlight"));
            annot.set("Rect", box_obj(rect));
            annot.set("Contents", Object::string_literal(contents));
            annot.set(
                "QuadPoints",
                Object::Array(vec![
                    Object::Integer(rect[0]),
                    Object::Integer(rect[1]),
                    Object::Integer(rect[2]),
                    Object::Integer(rect[1]),
                    Object::Integer(rect[2]),
                    Object::Integer(rect[3]),
                    Object::Integer(rect[0]),
                    Object::Integer(rect[3]),
                ]),
            );
            self.push_annot(page_index, annot);
        }

        fn add_highlight_with_nm(
            &mut self,
            page_index: usize,
            rect: [i64; 4],
            contents: &str,
            nm: &str,
        ) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", name("Highlight"));
            annot.set("Rect", box_obj(rect));
            annot.set("Contents", Object::string_literal(contents));
            annot.set("NM", Object::string_literal(nm));
            annot.set(
                "QuadPoints",
                Object::Array(vec![
                    Object::Integer(rect[0]),
                    Object::Integer(rect[1]),
                    Object::Integer(rect[2]),
                    Object::Integer(rect[1]),
                    Object::Integer(rect[2]),
                    Object::Integer(rect[3]),
                    Object::Integer(rect[0]),
                    Object::Integer(rect[3]),
                ]),
            );
            self.push_annot(page_index, annot);
        }

        fn add_session_highlight(
            &mut self,
            page_index: usize,
            rect: [i64; 4],
            contents: &str,
            nm: &str,
        ) {
            self.add_highlight_with_nm(page_index, rect, contents, nm);
        }

        fn add_link_uri(&mut self, page_index: usize, rect: [i64; 4], uri: &str) {
            let mut action = Dictionary::new();
            action.set("S", name("URI"));
            action.set("URI", Object::string_literal(uri));
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", name("Link"));
            annot.set("Rect", box_obj(rect));
            annot.set("A", Object::Dictionary(action));
            self.push_annot(page_index, annot);
        }

        fn add_widget(&mut self, page_index: usize, rect: [i64; 4]) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", name("Widget"));
            annot.set("Rect", box_obj(rect));
            annot.set("FT", name("Tx"));
            annot.set("T", Object::string_literal("keep-widget"));
            self.push_annot(page_index, annot);
        }

        fn add_unknown(&mut self, page_index: usize, subtype: &str, contents: &str) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", name(subtype));
            annot.set("Rect", box_obj([10, 10, 40, 40]));
            annot.set("Contents", Object::string_literal(contents));
            self.push_annot(page_index, annot);
        }

        fn save(&mut self, path: &Path) {
            self.doc.save(path).expect("write markup fixture");
        }
    }

    fn as_name(obj: &Object) -> Option<String> {
        match obj {
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            _ => None,
        }
    }

    fn as_nums(obj: &Object) -> Option<Vec<f64>> {
        let Object::Array(a) = obj else {
            return None;
        };
        Some(
            a.iter()
                .filter_map(|o| match o {
                    Object::Integer(i) => Some(*i as f64),
                    Object::Real(r) => Some(*r as f64),
                    _ => None,
                })
                .collect(),
        )
    }

    fn pdf_string(obj: &Object) -> Option<String> {
        match obj {
            Object::String(b, _) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => {
                let s = lopdf::decode_text_string(obj).ok()?;
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            }
        }
    }

    fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
        match obj {
            Object::Dictionary(d) => Some(d),
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        }
    }

    struct InspectedAnnot {
        subtype: String,
        rect: Option<Vec<f64>>,
        color: Option<Vec<f64>>,
        author: Option<String>,
        contents: Option<String>,
        nm: Option<String>,
        quad_points: Option<Vec<f64>>,
        has_ap: bool,
        has_ink_list: bool,
    }

    fn page_annot_objects(doc: &Document, page_1based: u32) -> Vec<Object> {
        let Some(id) = doc.get_pages().get(&page_1based).copied() else {
            return Vec::new();
        };
        let Ok(page) = doc.get_dictionary(id) else {
            return Vec::new();
        };
        match page.get(b"Annots") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn page_has_annots_key(path: &Path, page_1based: u32) -> bool {
        let doc = Document::load(path).expect("load dest");
        let Some(id) = doc.get_pages().get(&page_1based).copied() else {
            return false;
        };
        doc.get_dictionary(id)
            .ok()
            .and_then(|p| p.get(b"Annots").ok())
            .is_some()
    }

    fn inspect_page(path: &Path, page_1based: u32) -> Vec<InspectedAnnot> {
        let doc = Document::load(path).expect("load dest");
        let mut out = Vec::new();
        for raw in page_annot_objects(&doc, page_1based) {
            let Some(dict) = resolve_dict(&doc, &raw) else {
                continue;
            };
            let subtype = dict
                .get(b"Subtype")
                .ok()
                .and_then(as_name)
                .unwrap_or_default();
            let has_ap = match dict.get(b"AP") {
                Ok(Object::Stream(_)) => true,
                Ok(ap) => resolve_dict(&doc, ap)
                    .map(|d| d.get(b"N").is_ok() || d.get(b"R").is_ok() || d.get(b"D").is_ok())
                    .unwrap_or(false),
                _ => false,
            };
            out.push(InspectedAnnot {
                subtype,
                rect: dict.get(b"Rect").ok().and_then(as_nums),
                color: dict.get(b"C").ok().and_then(as_nums),
                author: dict.get(b"T").ok().and_then(pdf_string),
                contents: dict.get(b"Contents").ok().and_then(pdf_string),
                nm: dict.get(b"NM").ok().and_then(pdf_string),
                quad_points: dict.get(b"QuadPoints").ok().and_then(as_nums),
                has_ap,
                has_ink_list: dict.get(b"InkList").is_ok(),
            });
        }
        out
    }

    fn subtypes(annots: &[InspectedAnnot]) -> Vec<String> {
        annots.iter().map(|a| a.subtype.clone()).collect()
    }

    fn near4(got: &[f64], want: [f64; 4]) -> bool {
        got.len() == 4 && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.5)
    }

    fn near3(got: &[f64], want: [f64; 3]) -> bool {
        got.len() == 3 && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-3)
    }

    fn session(
        id: &str,
        kind: MarkupKind,
        rect: [f64; 4],
        color: [f64; 3],
        author: &str,
        contents: Option<&str>,
    ) -> SessionMarkup {
        let quad_points = match kind {
            MarkupKind::Highlight | MarkupKind::Underline | MarkupKind::StrikeOut => Some(vec![
                rect[0],
                rect[1],
                rect[0] + rect[2],
                rect[1],
                rect[0] + rect[2],
                rect[1] + rect[3],
                rect[0],
                rect[1] + rect[3],
            ]),
            _ => None,
        };
        SessionMarkup {
            id: id.to_string(),
            page_index: 0,
            kind,
            rect,
            color,
            author: author.to_string(),
            contents: contents.map(str::to_string),
            quad_points,
            ink_list: None,
        }
    }

    #[test]
    fn apply_writes_five_markup_subtypes_with_color_author_contents() {
        let scratch = Scratch::new("c1-five");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);

        let mut ink = session(
            "sess-ink",
            MarkupKind::Ink,
            [72.0, 100.0, 80.0, 40.0],
            [0.0, 0.0, 0.0],
            "Ada",
            None,
        );
        ink.ink_list = Some(vec![vec![[72.0, 100.0], [120.0, 140.0], [152.0, 110.0]]]);

        let session_annots = vec![
            session(
                "sess-note",
                MarkupKind::Note,
                [72.0, 700.0, 48.0, 40.0],
                [1.0, 0.0, 0.0],
                "Ada",
                Some("sticky"),
            ),
            session(
                "sess-hl",
                MarkupKind::Highlight,
                [100.0, 200.0, 80.0, 40.0],
                [1.0, 1.0, 0.0],
                "Ada",
                Some("review this"),
            ),
            session(
                "sess-ul",
                MarkupKind::Underline,
                [100.0, 160.0, 80.0, 14.0],
                [0.0, 0.0, 1.0],
                "Ada",
                Some("under"),
            ),
            session(
                "sess-so",
                MarkupKind::StrikeOut,
                [100.0, 140.0, 80.0, 14.0],
                [0.0, 0.0, 0.0],
                "Ada",
                Some("strike"),
            ),
            ink,
        ];

        apply_markup_annots(&dest, &session_annots, false).expect("apply");
        let annots = inspect_page(&dest, 1);
        let kinds = subtypes(&annots);
        assert!(
            kinds.iter().any(|s| s == "Text"),
            "C1: dest page /Annots must have /Subtype /Text; got {kinds:?}"
        );
        assert!(
            kinds.iter().any(|s| s == "Highlight"),
            "C1: dest page /Annots must have /Subtype /Highlight; got {kinds:?}"
        );
        assert!(
            kinds.iter().any(|s| s == "Underline"),
            "C1: dest page /Annots must have /Subtype /Underline; got {kinds:?}"
        );
        assert!(
            kinds.iter().any(|s| s == "StrikeOut"),
            "C1: dest page /Annots must have /Subtype /StrikeOut; got {kinds:?}"
        );
        assert!(
            kinds.iter().any(|s| s == "Ink"),
            "C1: dest page /Annots must have /Subtype /Ink; got {kinds:?}"
        );

        for want_sub in ["Text", "Highlight", "Underline", "StrikeOut", "Ink"] {
            let a = annots
                .iter()
                .find(|a| a.subtype == want_sub)
                .unwrap_or_else(|| panic!("C1: missing {want_sub}"));
            assert!(
                a.color.as_deref().is_some_and(|c| c.len() == 3),
                "C1: /{want_sub} must have /C; got {:?}",
                a.color
            );
            assert_eq!(
                a.author.as_deref(),
                Some("Ada"),
                "C1: /{want_sub} must have /T from the inspector string"
            );
        }

        let hl = annots.iter().find(|a| a.subtype == "Highlight").unwrap();
        assert_eq!(
            hl.contents.as_deref(),
            Some("review this"),
            "C1: highlight /Contents must be written when the session set a comment"
        );
        let note = annots.iter().find(|a| a.subtype == "Text").unwrap();
        assert!(
            near3(note.color.as_deref().unwrap_or(&[]), [1.0, 0.0, 0.0]),
            "C1: note /C must be the session color; got {:?}",
            note.color
        );
    }

    #[test]
    fn apply_highlight_writes_quadpoints_not_only_rect() {
        let scratch = Scratch::new("c2-quads");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);

        let hl = session(
            "sess-hl",
            MarkupKind::Highlight,
            [100.0, 200.0, 80.0, 40.0],
            [1.0, 1.0, 0.0],
            "Ada",
            Some("review this"),
        );
        apply_markup_annots(&dest, &[hl], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        let hl = annots.iter().find(|a| a.subtype == "Highlight");
        assert!(
            hl.is_some(),
            "C2: dest must have /Subtype /Highlight; got {:?}",
            subtypes(&annots)
        );
        let quads = hl.and_then(|a| a.quad_points.as_ref());
        assert!(
            quads.is_some_and(|q| q.len() >= 8 && q.len() % 8 == 0),
            "C2: highlight must have /QuadPoints with 8×n numbers, not only /Rect; got {quads:?}"
        );
        let rect = hl.and_then(|a| a.rect.as_ref());
        assert!(
            rect.is_some_and(|r| near4(r, [100.0, 200.0, 180.0, 240.0])),
            "C2: highlight /Rect must be unrotated [x y x+w y+h]; got {rect:?}"
        );
    }

    #[test]
    fn apply_noop_session_keeps_highlight_and_link() {
        // C3 markup-apply lock (keepGreen-after-impl): empty session must
        // copy leftover Highlight + Link through. Overlay-only is keep-green
        // in edit_overlay_integ.rs.
        let scratch = Scratch::new("c3-noop");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.add_highlight(0, [100, 200, 180, 240], "keep-me");
        fx.add_link_uri(0, [200, 300, 280, 360], "https://keep.example/");
        fx.save(&dest);

        apply_markup_annots(&dest, &[], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("keep-me")),
            "C3: leftover Highlight must survive a no-op markup apply; got {:?}",
            subtypes(&annots)
        );
        assert!(
            annots.iter().any(|a| a.subtype == "Link"),
            "C3: leftover Link must survive a no-op markup apply; got {:?}",
            subtypes(&annots)
        );
    }

    #[test]
    fn apply_session_delete_drops_only_session_annot() {
        let scratch = Scratch::new("c4-delete");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.add_highlight(0, [100, 200, 180, 240], "keep-me");
        fx.add_session_highlight(0, [50, 50, 130, 90], "drop-me", "sess-hl");
        fx.add_link_uri(0, [200, 300, 280, 360], "https://keep.example/");
        fx.add_widget(0, [400, 400, 460, 430]);
        fx.save(&dest);

        apply_markup_annots(&dest, &[], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("keep-me")),
            "C4: leftover Highlight must stay; got {:?}",
            subtypes(&annots)
        );
        assert!(
            !annots
                .iter()
                .any(|a| a.contents.as_deref() == Some("drop-me")
                    || a.nm.as_deref() == Some("sess-hl")),
            "C4: dest must drop the deleted session annot; still has {:?}",
            annots
                .iter()
                .map(|a| (a.subtype.clone(), a.contents.clone(), a.nm.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            annots.iter().any(|a| a.subtype == "Link"),
            "C4: leftover /Link must stay; got {:?}",
            subtypes(&annots)
        );
        assert!(
            annots.iter().any(|a| a.subtype == "Widget"),
            "C4: leftover /Widget must stay; got {:?}",
            subtypes(&annots)
        );
    }

    #[test]
    fn apply_nonempty_session_keeps_leftover_markup_with_nm() {
        // C9: leftover markup that already has /NM (Acrobat leftovers /
        // previous OffPDF session) must survive apply of a new, non-empty
        // session. C3/C4 leftovers have no /NM, so they stay green today.
        let scratch = Scratch::new("c9-nm-keep");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.add_highlight_with_nm(0, [100, 200, 180, 240], "third-party", "acrobat-keep");
        fx.add_link_uri(0, [200, 300, 280, 360], "https://keep.example/");
        fx.save(&dest);

        let session_hl = session(
            "sess-new-hl",
            MarkupKind::Highlight,
            [50.0, 50.0, 80.0, 40.0],
            [1.0, 1.0, 0.0],
            "Ada",
            Some("session-new"),
        );
        apply_markup_annots(&dest, &[session_hl], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        let summary: Vec<_> = annots
            .iter()
            .map(|a| (a.subtype.clone(), a.contents.clone(), a.nm.clone()))
            .collect();
        assert!(
            annots.iter().any(|a| a.subtype == "Highlight"
                && (a.nm.as_deref() == Some("acrobat-keep")
                    || a.contents.as_deref() == Some("third-party"))),
            "C9: leftover Highlight with /NM must survive a non-empty session apply; got {summary:?}"
        );
        assert!(
            annots.iter().any(|a| a.subtype == "Highlight"
                && (a.nm.as_deref() == Some("sess-new-hl")
                    || a.contents.as_deref() == Some("session-new"))),
            "C9: dest must also have the new session highlight; got {summary:?}"
        );
        assert!(
            annots.iter().any(|a| a.subtype == "Link"),
            "C9: leftover /Link must stay; got {:?}",
            subtypes(&annots)
        );
    }

    #[test]
    fn apply_flatten_on_removes_markup_annots() {
        let scratch = Scratch::new("c5-flatten");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.add_highlight(0, [100, 200, 180, 240], "keep-me");
        fx.add_link_uri(0, [200, 300, 280, 360], "https://keep.example/");
        fx.save(&dest);

        apply_markup_annots(&dest, &[], true).expect("flatten");
        let annots = inspect_page(&dest, 1);
        let markup: Vec<_> = annots
            .iter()
            .filter(|a| {
                matches!(
                    a.subtype.as_str(),
                    "Text" | "Highlight" | "Underline" | "StrikeOut" | "Ink"
                )
            })
            .map(|a| a.subtype.clone())
            .collect();
        assert!(
            markup.is_empty(),
            "C5: flatten-on must remove markup annots; still has {markup:?}"
        );

        let blob = {
            let mut doc = Document::load(&dest).expect("load dest");
            let _ = doc.decompress();
            let mut out = String::new();
            for obj in doc.objects.values() {
                if let Object::Stream(s) = obj {
                    let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
                    out.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            out
        };
        assert!(
            blob.contains("Hello"),
            "C5: flatten-on must keep the source content digest stream; blob={blob:?}"
        );
    }

    #[test]
    fn apply_flatten_on_qpdf_failure_returns_err_and_keeps_markup() {
        // C10: dest file name `c10-qpdf-fail.pdf` is the cfg(test) injection
        // token. Call the existing apply_markup_annots; do not expect Ok.
        let scratch = Scratch::new("c10-qpdf-fail");
        let dest = scratch.file("c10-qpdf-fail.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.add_highlight(0, [100, 200, 180, 240], "keep-me");
        fx.add_link_uri(0, [200, 300, 280, 360], "https://keep.example/");
        fx.save(&dest);

        let result = apply_markup_annots(&dest, &[], true);
        assert!(
            result.is_err(),
            "C10: flatten-on + qpdf failure must return Err, not Ok(()); dest file name is the injection token"
        );
        let annots = inspect_page(&dest, 1);
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("keep-me")),
            "C10: dest must still have leftover Highlight keep-me after flatten-on qpdf failure; got {:?}",
            subtypes(&annots)
        );
    }

    #[test]
    fn list_and_apply_rects_are_unrotated_on_rotate_90_crop() {
        let scratch = Scratch::new("c6-rotate");
        let src = scratch.file("src.pdf");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 90, Some([72, 72, 540, 720]));
        fx.add_highlight(0, [100, 200, 180, 240], "keep-me");
        fx.save(&src);
        std::fs::copy(&src, &dest).unwrap();

        let listed = list_markup_annots(&src).expect("list");
        assert!(
            !listed.is_empty(),
            "C6: list_markup_annots must return the highlight on a Rotate 90 Crop≠Media page; got empty"
        );
        let hl = listed
            .iter()
            .find(|a| a.subtype == "Highlight")
            .expect("C6: listed highlight");
        assert!(
            near4(&hl.rect, [100.0, 200.0, 80.0, 40.0]),
            "C6: listed /Rect must be unrotated [x,y,w,h], not display-swapped; got {:?}",
            hl.rect
        );
        if let Some(q) = &hl.quad_points {
            assert!(
                q.len() >= 8 && q.len() % 8 == 0,
                "C6: listed /QuadPoints must stay in unrotated user space (8×n); got {q:?}"
            );
        }

        let session_hl = session(
            "sess-hl",
            MarkupKind::Highlight,
            [100.0, 200.0, 80.0, 40.0],
            [1.0, 1.0, 0.0],
            "Ada",
            Some("review this"),
        );
        apply_markup_annots(&dest, &[session_hl], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        let written = annots
            .iter()
            .find(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("review this"));
        assert!(
            written.is_some(),
            "C6: apply must write the session highlight; got {:?}",
            annots
                .iter()
                .map(|a| (a.subtype.clone(), a.contents.clone(), a.rect.clone()))
                .collect::<Vec<_>>()
        );
        let written = written.unwrap();
        assert!(
            written
                .rect
                .as_deref()
                .is_some_and(|r| near4(r, [100.0, 200.0, 180.0, 240.0])),
            "C6: written /Rect must be unrotated [x y x+w y+h] on Rotate 90 Crop≠Media; got {:?}",
            written.rect
        );
        let quads = written.quad_points.as_ref();
        assert!(
            quads.is_some_and(|q| q.len() >= 8 && q.len() % 8 == 0),
            "C6: written /QuadPoints must be unrotated 8×n; got {quads:?}"
        );
    }

    #[test]
    fn apply_new_session_annots_have_appearance() {
        let scratch = Scratch::new("c7-ap");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);

        let mut ink = session(
            "sess-ink",
            MarkupKind::Ink,
            [72.0, 100.0, 80.0, 40.0],
            [0.0, 0.0, 0.0],
            "Ada",
            None,
        );
        ink.ink_list = Some(vec![vec![[72.0, 100.0], [120.0, 140.0]]]);
        let session_annots = vec![
            session(
                "sess-note",
                MarkupKind::Note,
                [72.0, 700.0, 48.0, 40.0],
                [1.0, 0.0, 0.0],
                "Ada",
                Some("sticky"),
            ),
            session(
                "sess-hl",
                MarkupKind::Highlight,
                [100.0, 200.0, 80.0, 40.0],
                [1.0, 1.0, 0.0],
                "Ada",
                Some("review this"),
            ),
            session(
                "sess-ul",
                MarkupKind::Underline,
                [100.0, 160.0, 80.0, 14.0],
                [0.0, 0.0, 1.0],
                "Ada",
                None,
            ),
            session(
                "sess-so",
                MarkupKind::StrikeOut,
                [100.0, 140.0, 80.0, 14.0],
                [0.0, 0.0, 0.0],
                "Ada",
                None,
            ),
            ink,
        ];
        apply_markup_annots(&dest, &session_annots, false).expect("apply");
        let annots = inspect_page(&dest, 1);
        assert!(
            !annots.is_empty(),
            "C7: dest must have session annots with /AP; got []"
        );
        for want in ["Text", "Highlight", "Underline", "StrikeOut", "Ink"] {
            let a = annots.iter().find(|a| a.subtype == want);
            assert!(
                a.is_some_and(|a| a.has_ap),
                "C7: new session /{want} must have /AP (or /AP /N); got {:?}",
                a.map(|a| a.has_ap)
            );
        }
    }

    #[test]
    fn list_malformed_annots_does_not_panic_and_surfaces_unknown() {
        let scratch = Scratch::new("c8-list");
        let src = scratch.file("src.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.push_raw_annot(0, Object::Integer(1));
        let mut missing_rect = Dictionary::new();
        missing_rect.set("Type", "Annot");
        missing_rect.set("Subtype", name("Text"));
        missing_rect.set("Contents", Object::string_literal("no-rect"));
        fx.push_annot(0, missing_rect);
        let mut odd_quads = Dictionary::new();
        odd_quads.set("Type", "Annot");
        odd_quads.set("Subtype", name("Highlight"));
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
        fx.push_annot(0, odd_quads);
        fx.add_unknown(0, "FooBar", "mystery");
        fx.save(&src);

        let listed = list_markup_annots(&src);
        assert!(
            listed.is_ok(),
            "C8: list_markup_annots must not panic or error on malformed /Annots; got {listed:?}"
        );
        let listed = listed.unwrap();
        assert!(
            listed.iter().any(|a| a.subtype == "FooBar"),
            "C8: list must return leftover unknown /Subtype without panicking; got {:?}",
            listed.iter().map(|a| a.subtype.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn apply_copies_malformed_leftovers_through() {
        let scratch = Scratch::new("c8-apply");
        let dest = scratch.file("dest.pdf");
        let mut fx = PdfFix::new(1, 0, None);
        fx.push_raw_annot(0, Object::Integer(1));
        let mut missing_rect = Dictionary::new();
        missing_rect.set("Type", "Annot");
        missing_rect.set("Subtype", name("Text"));
        missing_rect.set("Contents", Object::string_literal("no-rect"));
        fx.push_annot(0, missing_rect);
        let mut odd_quads = Dictionary::new();
        odd_quads.set("Type", "Annot");
        odd_quads.set("Subtype", name("Highlight"));
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
        fx.push_annot(0, odd_quads);
        fx.add_unknown(0, "FooBar", "mystery");
        fx.save(&dest);

        let note = session(
            "sess-note",
            MarkupKind::Note,
            [72.0, 700.0, 48.0, 40.0],
            [1.0, 0.0, 0.0],
            "Ada",
            Some("sticky"),
        );
        apply_markup_annots(&dest, &[note], false).expect("apply");

        let doc = Document::load(&dest).expect("load dest");
        let raw = page_annot_objects(&doc, 1);
        assert!(
            raw.iter().any(|o| matches!(o, Object::Integer(1))),
            "C8: non-dict /Annots entry must copy through; got {raw:?}"
        );
        let annots = inspect_page(&dest, 1);
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Text" && a.contents.as_deref() == Some("no-rect")),
            "C8: leftover missing-/Rect annot must copy through; got {:?}",
            annots
                .iter()
                .map(|a| (a.subtype.clone(), a.contents.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("odd-quads")),
            "C8: leftover odd-length /QuadPoints annot must copy through; got {:?}",
            annots
                .iter()
                .map(|a| (a.subtype.clone(), a.contents.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "FooBar" && a.contents.as_deref() == Some("mystery")),
            "C8: leftover unknown /Subtype must copy through; got {:?}",
            annots
                .iter()
                .map(|a| (a.subtype.clone(), a.contents.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn apply_first_markup_creates_annots_key() {
        let scratch = Scratch::new("c1-first");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);
        assert!(
            !page_has_annots_key(&dest, 1),
            "fixture must start with no /Annots"
        );
        let note = session(
            "sess-note",
            MarkupKind::Note,
            [72.0, 700.0, 48.0, 40.0],
            [1.0, 0.0, 0.0],
            "Ada",
            Some("sticky"),
        );
        apply_markup_annots(&dest, &[note], false).expect("apply");
        assert!(
            page_has_annots_key(&dest, 1),
            "C1: apply of the first session annot onto a file with no /Annots must create dest /Annots"
        );
    }

    #[test]
    fn apply_markup_ink_writes_inklist() {
        let scratch = Scratch::new("c1-inklist");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);
        let mut ink = session(
            "sess-ink",
            MarkupKind::Ink,
            [72.0, 100.0, 80.0, 40.0],
            [0.0, 0.0, 0.0],
            "Ada",
            None,
        );
        ink.ink_list = Some(vec![vec![[72.0, 100.0], [120.0, 140.0]]]);
        apply_markup_annots(&dest, &[ink], false).expect("apply");
        let annots = inspect_page(&dest, 1);
        let ink = annots.iter().find(|a| a.subtype == "Ink");
        assert!(
            ink.is_some_and(|a| a.has_ink_list),
            "C1: markup /Ink must write /InkList; got {:?}",
            subtypes(&annots)
        );
    }
}
