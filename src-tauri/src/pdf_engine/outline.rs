//! Extract a PDF's bookmarks/outline tree (flattened). Uses lopdf, which loads
//! the whole file, so it's gated by size — large files return an empty list
//! rather than risk memory. Named destinations are skipped (page = None).

use crate::error::AppError;
use crate::models::OutlineItem;
use lopdf::{Document, Object, ObjectId};
use std::collections::HashMap;

/// Outline is only parsed for files up to this size (keeps memory bounded).
const MAX_OUTLINE_BYTES: u64 = 400 * 1024 * 1024;
const MAX_ITEMS: usize = 5000;

fn decode_title(obj: &Object) -> String {
    lopdf::decode_text_string(obj).unwrap_or_default().trim().to_string()
}

/// Resolve an outline item's destination to a 1-based page number, if possible.
fn dest_page(doc: &Document, item: &lopdf::Dictionary, page_no: &HashMap<ObjectId, u32>) -> Option<u32> {
    // /Dest directly, or /A action dict with /D.
    let dest = item
        .get(b"Dest")
        .ok()
        .cloned()
        .or_else(|| {
            item.get(b"A")
                .ok()
                .and_then(|a| a.as_reference().ok().and_then(|id| doc.get_dictionary(id).ok()).or_else(|| a.as_dict().ok()))
                .and_then(|action| action.get(b"D").ok().cloned())
        })?;

    // dest may be a Reference to an array, or an array directly.
    let arr = match &dest {
        Object::Array(a) => a.clone(),
        Object::Reference(r) => doc.get_object(*r).ok()?.as_array().ok()?.clone(),
        _ => return None, // Name/String = named destination — skipped
    };
    let first = arr.first()?;
    let page_id = first.as_reference().ok()?;
    page_no.get(&page_id).copied()
}

fn walk(
    doc: &Document,
    start: ObjectId,
    level: u32,
    page_no: &HashMap<ObjectId, u32>,
    out: &mut Vec<OutlineItem>,
) {
    let mut cur = Some(start);
    let mut guard = 0;
    while let Some(id) = cur {
        guard += 1;
        if guard > MAX_ITEMS || out.len() >= MAX_ITEMS {
            break;
        }
        let Ok(dict) = doc.get_dictionary(id) else { break };
        let title = dict.get(b"Title").ok().map(decode_title).unwrap_or_default();
        if !title.is_empty() {
            out.push(OutlineItem { title, page: dest_page(doc, dict, page_no), level });
        }
        if let Ok(first) = dict.get(b"First").and_then(|o| o.as_reference()) {
            walk(doc, first, level + 1, page_no, out);
        }
        cur = dict.get(b"Next").and_then(|o| o.as_reference()).ok();
    }
}

pub fn extract(path: &str) -> Result<Vec<OutlineItem>, AppError> {
    let meta = std::fs::metadata(path).map_err(|e| AppError::io("Could not read the file.", e))?;
    if meta.len() > MAX_OUTLINE_BYTES {
        return Ok(vec![]);
    }
    let doc = match Document::load(path) {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    let mut page_no: HashMap<ObjectId, u32> = HashMap::new();
    for (num, id) in doc.get_pages() {
        page_no.insert(id, num);
    }

    let root_id = match doc.trailer.get(b"Root").and_then(|o| o.as_reference()) {
        Ok(id) => id,
        Err(_) => return Ok(vec![]),
    };
    let Ok(catalog) = doc.get_dictionary(root_id) else { return Ok(vec![]) };
    let outlines_id = match catalog.get(b"Outlines").and_then(|o| o.as_reference()) {
        Ok(id) => id,
        Err(_) => return Ok(vec![]),
    };
    let Ok(outlines) = doc.get_dictionary(outlines_id) else { return Ok(vec![]) };

    let mut out = Vec::new();
    if let Ok(first) = outlines.get(b"First").and_then(|o| o.as_reference()) {
        walk(&doc, first, 0, &page_no, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::decode_title;
    use lopdf::{Object, StringFormat};

    #[test]
    fn pdf_doc_encoding_special_bytes() {
        let per_mille = Object::String(vec![0x8B], StringFormat::Literal);
        assert_eq!(decode_title(&per_mille), "‰");
        let euro = Object::String(vec![0xA0], StringFormat::Literal);
        assert_eq!(decode_title(&euro), "€");
    }

    #[test]
    fn utf16be_title_decodes() {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "Başlık".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        let title = Object::String(bytes, StringFormat::Literal);
        assert_eq!(decode_title(&title), "Başlık");
    }
}
