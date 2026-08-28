//! PDF `/Link` annotations for Edit PDF (issue #35).
//!
//! Shared classifier + list/apply surface. Per-source dest ranges skip
//! unlistable files so a mixed workspace can still rewrite complete sources.

use crate::error::AppError;
use lopdf::{Dictionary, Document, Object, ObjectId};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Outline-sized load gate — same bound as [`super::outline`].
const MAX_LINK_BYTES: u64 = 400 * 1024 * 1024;
pub(crate) const MAX_LINKS: usize = 5000;

/// Classification of a link action (URI / in-document GoTo / leave-alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkActionClass {
    Uri,
    GoTo,
    Unsupported,
}

/// Supported session/list action: allowlisted URI or 0-based dest page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    Uri { uri: String },
    GoTo { dest_page_index: u32 },
}

/// A supported `/Link` listed from a source PDF.
///
/// `rect` is unrotated user space `[x, y, w, h]` (same as `EditObject.rect`).
#[derive(Debug, Clone, PartialEq)]
pub struct ListedLink {
    pub page_index: u32,
    pub rect: [f64; 4],
    pub action: LinkAction,
}

/// A session-owned supported link to write onto a staged dest PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionLink {
    pub page_index: u32,
    pub rect: [f64; 4],
    pub action: LinkAction,
}

/// IPC DTO for `list_pdf_links` (paths in, JSON out).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfLinkDto {
    pub page_index: u32,
    pub rect: PdfRectDto,
    pub action: PdfLinkActionDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRectDto {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PdfLinkActionDto {
    Uri {
        uri: String,
    },
    Goto {
        #[serde(rename = "destPageIndex")]
        dest_page_index: u32,
    },
}

/// Write allowlist: `https` / `http` / `mailto` only (frozen #35 policy).
pub fn uri_is_allowed(uri: &str) -> bool {
    let Some(scheme) = uri_scheme(uri) else {
        return false;
    };
    matches!(scheme, "https" | "http" | "mailto")
}

/// Classify a PDF link action.
///
/// `s` is the `/S` name (`URI`, `GoTo`, `Launch`, `GoToR`, `JavaScript`, …).
/// `uri` is `/URI` when `s` is `URI`. `dest_is_named` is true when `/D` is a
/// name or string (named dest — unsupported, leave).
pub fn classify_link_action(s: &str, uri: Option<&str>, dest_is_named: bool) -> LinkActionClass {
    if s.eq_ignore_ascii_case("URI") {
        return match uri {
            Some(u) if uri_is_allowed(u) => LinkActionClass::Uri,
            _ => LinkActionClass::Unsupported,
        };
    }
    if s.eq_ignore_ascii_case("GoTo") {
        return if dest_is_named {
            LinkActionClass::Unsupported
        } else {
            LinkActionClass::GoTo
        };
    }
    LinkActionClass::Unsupported
}

/// List supported URI / in-document GoTo `/Link` annots.
///
/// Rects are the written unrotated `/Rect` values as `[x, y, w, h]`, not
/// display-swapped for `/Rotate`. Oversize files, more than [`MAX_LINKS`]
/// supported links, a missing path, or an unreadable PDF return an actionable
/// [`AppError`] — never a silent empty or truncated `Ok`.
pub fn list_link_annots(path: &Path) -> Result<Vec<ListedLink>, AppError> {
    let meta = std::fs::metadata(path).map_err(|e| {
        AppError::io("Could not read the file.", e)
            .with_suggestion("Check that the PDF still exists and try again.")
    })?;
    if meta.len() > MAX_LINK_BYTES {
        return Err(file_too_large_for_links());
    }
    let path_str = path.to_string_lossy();
    let doc = Document::load(path)
        .map_err(|e| AppError::invalid_pdf(&path_str).with_details(format!("lopdf: {e}")))?;
    let page_index_of = page_index_map(&doc);
    let mut out = Vec::new();
    let pages = doc.get_pages();
    let mut nums: Vec<u32> = pages.keys().copied().collect();
    nums.sort_unstable();
    for num in nums {
        let Some(&page_id) = pages.get(&num) else {
            continue;
        };
        for raw in page_annot_objects(&doc, page_id) {
            let Some(listed) = listed_from_annot(&doc, &raw, num.saturating_sub(1), &page_index_of)
            else {
                continue;
            };
            if out.len() >= MAX_LINKS {
                return Err(too_many_links());
            }
            out.push(listed);
        }
    }
    Ok(out)
}

/// Replace the dest page's supported-Link set with `links`.
///
/// Copy through every non-Link annot and every unsupported Link. Reject
/// writing a non-allowlisted URI with an actionable [`AppError`].
pub fn apply_link_annots(staged: &Path, links: &[SessionLink]) -> Result<(), AppError> {
    apply_link_annots_impl(staged, links, None)
}

/// True when dest will still have any annot after apply (leftover ∪ added).
pub fn expected_dest_has_annots(staged: &Path, links: &[SessionLink]) -> Result<bool, AppError> {
    if !links.is_empty() {
        return Ok(true);
    }
    dest_has_leftover_annots(staged)
}

/// True when any dest annot is a supported URI/GoTo Link (apply would rewrite).
pub fn dest_has_supported_links(staged: &Path) -> Result<bool, AppError> {
    let doc = Document::load(staged)
        .map_err(|e| AppError::engine_failed(format!("Could not read the staged PDF: {e}")))?;
    let page_index_of = page_index_map(&doc);
    for id in doc.get_pages().values() {
        for raw in page_annot_objects(&doc, *id) {
            if annot_is_supported_link(&doc, &raw, &page_index_of) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Whether Save should rewrite dest supported `/Link`s.
///
/// Incomplete hydrate + empty session must not wipe dest. Complete empty
/// still deletes (L7).
pub fn should_rewrite_supported_links(complete: bool, session_len: usize, dest_has: bool) -> bool {
    if !complete {
        return false;
    }
    if session_len > 0 {
        return true;
    }
    dest_has
}

/// Dest 0-based page ranges Save should rewrite for this assemble.
///
/// Each complete source's dest pages. Incomplete sources are skipped so
/// their assembled annots stay.
pub fn dest_ranges_to_rewrite(
    groups: &[(&str, u32)],
    incomplete_paths: &[&str],
) -> Vec<std::ops::Range<u32>> {
    let mut start = 0u32;
    let mut out = Vec::with_capacity(groups.len());
    for &(path, n) in groups {
        let end = start.saturating_add(n);
        let incomplete = incomplete_paths.iter().any(|&p| p == path);
        if !incomplete && n > 0 {
            out.push(start..end);
        }
        start = end;
    }
    out
}

/// Apply session links, limited to `dest_pages` (0-based dest indexes).
///
/// An empty page list is a no-op. Pages not in `dest_pages` are left
/// unchanged.
pub fn apply_link_annots_for_pages(
    staged: &Path,
    links: &[SessionLink],
    dest_pages: &[u32],
) -> Result<(), AppError> {
    if dest_pages.is_empty() {
        return Ok(());
    }
    apply_link_annots_impl(staged, links, Some(dest_pages))
}

fn apply_link_annots_impl(
    staged: &Path,
    links: &[SessionLink],
    dest_pages: Option<&[u32]>,
) -> Result<(), AppError> {
    for link in links {
        if let LinkAction::Uri { uri } = &link.action {
            if !uri_is_allowed(uri) {
                return Err(unsafe_uri_error(uri));
            }
        }
    }

    let mut doc = Document::load(staged)
        .map_err(|e| AppError::engine_failed(format!("Could not read the staged PDF: {e}")))?;
    let pages = doc.get_pages();
    let page_index_of = page_index_map(&doc);
    let mut page_ids: Vec<(u32, ObjectId)> = pages.iter().map(|(&n, &id)| (n, id)).collect();
    page_ids.sort_by_key(|(n, _)| *n);

    let mut by_page: HashMap<u32, Vec<&SessionLink>> = HashMap::new();
    for link in links {
        if (link.page_index as usize) >= page_ids.len() {
            return Err(AppError::new(
                "BAD_EDIT",
                "Link page is out of range",
                format!(
                    "A link points at page {}, but this PDF has {} page{}.",
                    link.page_index + 1,
                    page_ids.len(),
                    if page_ids.len() == 1 { "" } else { "s" }
                ),
            )
            .with_suggestion("Pick a page that exists in the document."));
        }
        by_page.entry(link.page_index).or_default().push(link);
    }

    let dest_filter: Option<HashSet<u32>> = dest_pages.map(|p| p.iter().copied().collect());

    for (num, page_id) in &page_ids {
        let page_index = num.saturating_sub(1);
        if dest_filter
            .as_ref()
            .is_some_and(|set| !set.contains(&page_index))
        {
            continue;
        }
        let existing = page_annot_objects(&doc, *page_id);
        let mut kept: Vec<Object> = Vec::new();
        let mut session: Vec<&SessionLink> = by_page.get(&page_index).cloned().unwrap_or_default();
        for raw in existing {
            if let Some(listed) = listed_from_annot(&doc, &raw, page_index, &page_index_of) {
                if let Some(i) = session
                    .iter()
                    .position(|l| session_matches_listed(l, &listed))
                {
                    session.remove(i);
                    kept.push(raw);
                }
                continue;
            }
            kept.push(raw);
        }
        for link in session {
            let annot_id = add_session_link(&mut doc, link, &page_ids)?;
            kept.push(annot_id.into());
        }
        set_page_annots(&mut doc, *page_id, kept)?;
    }

    doc.save(staged)
        .map_err(|e| AppError::io("Could not write link annotations.", e))?;
    Ok(())
}

/// Command helper: size-gated list as JSON DTOs.
pub fn list_pdf_links_cmd(path: &str) -> Result<Vec<PdfLinkDto>, AppError> {
    let listed = list_link_annots(Path::new(path))?;
    Ok(listed.into_iter().map(listed_to_dto).collect())
}

pub fn unsafe_uri_error(uri: &str) -> AppError {
    AppError::new(
        "UNSAFE_URI",
        "This link is not allowed",
        format!("OffPDF cannot write a link to \"{uri}\"."),
    )
    .with_suggestion("Use an https, http, or mailto address.")
}

fn file_too_large_for_links() -> AppError {
    AppError::new(
        "FILE_TOO_LARGE",
        "File too large to read links",
        "Reading links needs the document loaded into memory, and this file is over 400 MB.",
    )
    .with_suggestion("Save stamps only, or split the PDF into smaller documents.")
}

fn too_many_links() -> AppError {
    AppError::new(
        "TOO_MANY_LINKS",
        "Too many links to edit",
        "This PDF has more than 5,000 supported links, so OffPDF cannot load them all.",
    )
    .with_suggestion("Save stamps only, or split the PDF into smaller documents.")
}

fn rects_close(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.5)
}

fn session_matches_listed(link: &SessionLink, listed: &ListedLink) -> bool {
    rects_close(link.rect, listed.rect) && link.action == listed.action
}

fn listed_to_dto(link: ListedLink) -> PdfLinkDto {
    PdfLinkDto {
        page_index: link.page_index,
        rect: PdfRectDto {
            x: link.rect[0],
            y: link.rect[1],
            w: link.rect[2],
            h: link.rect[3],
        },
        action: match link.action {
            LinkAction::Uri { uri } => PdfLinkActionDto::Uri { uri },
            LinkAction::GoTo { dest_page_index } => PdfLinkActionDto::Goto { dest_page_index },
        },
    }
}

fn uri_scheme(uri: &str) -> Option<&str> {
    let s = uri.trim();
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    if scheme.is_empty() {
        return None;
    }
    let ok = scheme.bytes().enumerate().all(|(i, b)| {
        if i == 0 {
            b.is_ascii_alphabetic()
        } else {
            b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.'
        }
    });
    if !ok {
        return None;
    }
    // Lowercase compare without allocating when already ASCII-lower.
    // Callers only match a tiny allowlist; a small String is fine.
    None.or_else(|| {
        let lower = scheme.to_ascii_lowercase();
        // Leak-free: compare via a local then map back is awkward; return owned via match in caller.
        // Re-parse by returning a static when it matches.
        match lower.as_str() {
            "https" => Some("https"),
            "http" => Some("http"),
            "mailto" => Some("mailto"),
            "javascript" => Some("javascript"),
            "file" => Some("file"),
            "data" => Some("data"),
            "ftp" => Some("ftp"),
            "vbscript" => Some("vbscript"),
            _ => Some("other"),
        }
    })
}

fn dest_has_leftover_annots(staged: &Path) -> Result<bool, AppError> {
    let doc = Document::load(staged)
        .map_err(|e| AppError::engine_failed(format!("Could not read the staged PDF: {e}")))?;
    let page_index_of = page_index_map(&doc);
    for id in doc.get_pages().values() {
        for raw in page_annot_objects(&doc, *id) {
            if !annot_is_supported_link(&doc, &raw, &page_index_of) {
                // Any leftover object (non-Link or unsupported Link). Empty Annots
                // arrays still count as leftover only if they hold an object.
                if resolve_dict(&doc, &raw).is_some() {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn page_index_map(doc: &Document) -> HashMap<ObjectId, u32> {
    let mut map = HashMap::new();
    for (num, id) in doc.get_pages() {
        map.insert(id, num.saturating_sub(1));
    }
    map
}

fn page_annot_objects(doc: &Document, page_id: ObjectId) -> Vec<Object> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    match page.get(b"Annots") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(r)) => doc
            .get_object(*r)
            .ok()
            .and_then(|o| o.as_array().ok())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn resolve_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn as_name(obj: &Object) -> Option<String> {
    match obj {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
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

fn dest_is_named(doc: &Document, dest: &Object) -> bool {
    match dest {
        Object::Array(_) => false,
        Object::Name(_) | Object::String(_, _) => true,
        Object::Reference(r) => match doc.get_object(*r) {
            Ok(Object::Array(_)) => false,
            Ok(Object::Name(_) | Object::String(_, _)) => true,
            _ => true,
        },
        _ => true,
    }
}

fn dest_page_index(
    doc: &Document,
    dest: &Object,
    page_index_of: &HashMap<ObjectId, u32>,
) -> Option<u32> {
    let arr = match dest {
        Object::Array(a) => a,
        Object::Reference(r) => doc.get_object(*r).ok()?.as_array().ok()?,
        _ => return None,
    };
    let page_id = arr.first()?.as_reference().ok()?;
    page_index_of.get(&page_id).copied()
}

struct ParsedLink<'a> {
    class: LinkActionClass,
    uri: Option<String>,
    dest_page_index: Option<u32>,
    rect: [f64; 4],
    _annot: &'a Dictionary,
}

fn parse_link_annot<'a>(
    doc: &'a Document,
    raw: &'a Object,
    page_index_of: &HashMap<ObjectId, u32>,
) -> Option<ParsedLink<'a>> {
    let annot = resolve_dict(doc, raw)?;
    let subtype = annot.get(b"Subtype").ok().and_then(as_name)?;
    if !subtype.eq_ignore_ascii_case("Link") {
        return None;
    }
    let rect = annot
        .get(b"Rect")
        .ok()
        .and_then(as_nums)
        .and_then(|v| {
            (v.len() == 4).then_some({
                let x0 = v[0];
                let y0 = v[1];
                let x1 = v[2];
                let y1 = v[3];
                [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()]
            })
        })
        .unwrap_or([0.0, 0.0, 0.0, 0.0]);

    if let Some(action_obj) = annot.get(b"A").ok() {
        let action = resolve_dict(doc, action_obj)?;
        let s = action.get(b"S").ok().and_then(as_name).unwrap_or_default();
        let uri = action.get(b"URI").ok().and_then(pdf_string);
        let dest = action.get(b"D").ok();
        let named = dest.map(|d| dest_is_named(doc, d)).unwrap_or(false);
        let class = classify_link_action(&s, uri.as_deref(), named);
        let dest_page = dest.and_then(|d| dest_page_index(doc, d, page_index_of));
        return Some(ParsedLink {
            class,
            uri,
            dest_page_index: dest_page,
            rect,
            _annot: annot,
        });
    }

    if let Some(dest) = annot.get(b"Dest").ok() {
        let named = dest_is_named(doc, dest);
        let class = classify_link_action("GoTo", None, named);
        let dest_page = dest_page_index(doc, dest, page_index_of);
        return Some(ParsedLink {
            class,
            uri: None,
            dest_page_index: dest_page,
            rect,
            _annot: annot,
        });
    }

    Some(ParsedLink {
        class: LinkActionClass::Unsupported,
        uri: None,
        dest_page_index: None,
        rect,
        _annot: annot,
    })
}

fn listed_from_annot(
    doc: &Document,
    raw: &Object,
    page_index: u32,
    page_index_of: &HashMap<ObjectId, u32>,
) -> Option<ListedLink> {
    let parsed = parse_link_annot(doc, raw, page_index_of)?;
    match parsed.class {
        LinkActionClass::Uri => {
            let uri = parsed.uri?;
            Some(ListedLink {
                page_index,
                rect: parsed.rect,
                action: LinkAction::Uri { uri },
            })
        }
        LinkActionClass::GoTo => {
            let dest_page_index = parsed.dest_page_index?;
            Some(ListedLink {
                page_index,
                rect: parsed.rect,
                action: LinkAction::GoTo { dest_page_index },
            })
        }
        LinkActionClass::Unsupported => None,
    }
}

fn annot_is_supported_link(
    doc: &Document,
    raw: &Object,
    page_index_of: &HashMap<ObjectId, u32>,
) -> bool {
    // Same set list_link_annots hydrates: allowlisted URI or resolvable
    // in-document GoTo. Unresolvable /D arrays stay leftover and copy through.
    listed_from_annot(doc, raw, 0, page_index_of).is_some()
}

fn add_session_link(
    doc: &mut Document,
    link: &SessionLink,
    page_ids: &[(u32, ObjectId)],
) -> Result<ObjectId, AppError> {
    let [x, y, w, h] = link.rect;
    let rect = Object::Array(vec![
        Object::Real(x as f32),
        Object::Real(y as f32),
        Object::Real((x + w) as f32),
        Object::Real((y + h) as f32),
    ]);
    let mut action = Dictionary::new();
    match &link.action {
        LinkAction::Uri { uri } => {
            action.set("S", "URI");
            action.set("URI", Object::string_literal(uri.as_str()));
        }
        LinkAction::GoTo { dest_page_index } => {
            let dest_1 = dest_page_index + 1;
            let dest_id = page_ids
                .iter()
                .find(|(n, _)| *n == dest_1)
                .map(|(_, id)| *id)
                .ok_or_else(|| {
                    AppError::new(
                        "BAD_EDIT",
                        "Link destination is out of range",
                        format!(
                            "A link points at page {dest_1}, but this PDF has {} page{}.",
                            page_ids.len(),
                            if page_ids.len() == 1 { "" } else { "s" }
                        ),
                    )
                    .with_suggestion("Pick a page that exists in the document.")
                })?;
            action.set("S", "GoTo");
            action.set(
                "D",
                Object::Array(vec![dest_id.into(), Object::Name(b"Fit".to_vec())]),
            );
        }
    }
    let mut annot = Dictionary::new();
    annot.set("Type", "Annot");
    annot.set("Subtype", "Link");
    annot.set("Rect", rect);
    annot.set(
        "Border",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(0),
        ]),
    );
    annot.set("A", Object::Dictionary(action));
    Ok(doc.add_object(Object::Dictionary(annot)))
}

fn set_page_annots(
    doc: &mut Document,
    page_id: ObjectId,
    annots: Vec<Object>,
) -> Result<(), AppError> {
    let page = doc
        .get_object_mut(page_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| AppError::engine_failed(format!("Could not update page annotations: {e}")))?;
    if annots.is_empty() {
        page.remove(b"Annots");
    } else {
        page.set("Annots", Object::Array(annots));
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
                "offpdf-links-{}-{}-{}",
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

        fn push_annot(&mut self, page_index: usize, dict: Dictionary) {
            let annot_id = self.doc.add_object(Object::Dictionary(dict));
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
            arr.push(annot_id.into());
            if let Ok(Object::Dictionary(page)) = self.doc.get_object_mut(page_id) {
                page.set("Annots", Object::Array(arr));
            }
        }

        fn add_link_uri(&mut self, page_index: usize, rect: [i64; 4], uri: &str) {
            let mut action = Dictionary::new();
            action.set("S", name("URI"));
            action.set("URI", Object::string_literal(uri));
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Link");
            annot.set("Rect", box_obj(rect));
            annot.set("A", Object::Dictionary(action));
            self.push_annot(page_index, annot);
        }

        fn add_link_goto(&mut self, page_index: usize, rect: [i64; 4], dest_page: usize) {
            let dest_id = self.page_ids[dest_page];
            let mut action = Dictionary::new();
            action.set("S", name("GoTo"));
            action.set("D", Object::Array(vec![dest_id.into(), name("Fit")]));
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Link");
            annot.set("Rect", box_obj(rect));
            annot.set("A", Object::Dictionary(action));
            self.push_annot(page_index, annot);
        }

        fn add_highlight(&mut self, page_index: usize, rect: [i64; 4], contents: &str) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Highlight");
            annot.set("Rect", box_obj(rect));
            annot.set("Contents", Object::string_literal(contents));
            self.push_annot(page_index, annot);
        }

        fn add_n_uri_links(&mut self, page_index: usize, n: usize, uri: &str) {
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
            for i in 0..n {
                let x = (i as i64) % 500;
                let y = (i as i64) / 500;
                let mut action = Dictionary::new();
                action.set("S", name("URI"));
                action.set("URI", Object::string_literal(uri));
                let mut annot = Dictionary::new();
                annot.set("Type", "Annot");
                annot.set("Subtype", "Link");
                annot.set("Rect", box_obj([x, y, x + 10, y + 10]));
                annot.set("A", Object::Dictionary(action));
                let annot_id = self.doc.add_object(Object::Dictionary(annot));
                arr.push(annot_id.into());
            }
            if let Ok(Object::Dictionary(page)) = self.doc.get_object_mut(page_id) {
                page.set("Annots", Object::Array(arr));
            }
        }

        fn add_link_uri_with_extra_keys(&mut self, page_index: usize, rect: [i64; 4], uri: &str) {
            let mut action = Dictionary::new();
            action.set("S", name("URI"));
            action.set("URI", Object::string_literal(uri));
            let mut ap = Dictionary::new();
            ap.set("N", name("Off"));
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Link");
            annot.set("Rect", box_obj(rect));
            annot.set("A", Object::Dictionary(action));
            annot.set(
                "Border",
                Object::Array(vec![
                    Object::Integer(2),
                    Object::Integer(1),
                    Object::Integer(3),
                ]),
            );
            annot.set("H", name("I"));
            annot.set("F", Object::Integer(4));
            annot.set("AP", Object::Dictionary(ap));
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
            annot.set("Contents", Object::string_literal("keep-contents"));
            self.push_annot(page_index, annot);
        }

        fn save(&mut self, path: &Path) {
            self.doc.save(path).expect("write link fixture");
        }
    }

    struct InspectedAnnot {
        subtype: String,
        rect: [f64; 4],
        action_s: Option<String>,
        uri: Option<String>,
        dest_page_index: Option<u32>,
        contents: Option<String>,
        has_annots_key: bool,
        has_border: bool,
        has_h: bool,
        has_f: bool,
        has_ap: bool,
        has_quad_points: bool,
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

    fn page_index_of(doc: &Document, id: ObjectId) -> Option<u32> {
        doc.get_pages()
            .iter()
            .find_map(|(num, pid)| (*pid == id).then_some(*num - 1))
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

    fn inspect_page_annots(path: &Path, page_1based: u32) -> Vec<InspectedAnnot> {
        let doc = Document::load(path).expect("load dest");
        let Some(id) = doc.get_pages().get(&page_1based).copied() else {
            return Vec::new();
        };
        let Ok(page) = doc.get_dictionary(id) else {
            return Vec::new();
        };
        let has_annots_key = page.get(b"Annots").is_ok();
        let annot_objs: Vec<Object> = match page.get(b"Annots") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => doc
                .get_object(*r)
                .ok()
                .and_then(|o| o.as_array().ok())
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        annot_objs
            .iter()
            .filter_map(|obj| {
                let annot = resolve_dict(&doc, obj)?;
                let subtype = annot
                    .get(b"Subtype")
                    .ok()
                    .and_then(as_name)
                    .unwrap_or_default();
                let rect = annot
                    .get(b"Rect")
                    .ok()
                    .and_then(as_nums)
                    .and_then(|v| (v.len() == 4).then_some([v[0], v[1], v[2], v[3]]))
                    .unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let action = annot.get(b"A").ok().and_then(|a| resolve_dict(&doc, a));
                let action_s = action.and_then(|a| a.get(b"S").ok().and_then(as_name));
                let uri = action.and_then(|a| a.get(b"URI").ok().and_then(pdf_string));
                let dest_page_index = action.and_then(|a| {
                    let dest = a.get(b"D").ok()?;
                    let arr = match dest {
                        Object::Array(v) => v,
                        Object::Reference(r) => doc.get_object(*r).ok()?.as_array().ok()?,
                        _ => return None,
                    };
                    let first = arr.first()?;
                    let page_id = first.as_reference().ok()?;
                    page_index_of(&doc, page_id)
                });
                let contents = annot.get(b"Contents").ok().and_then(pdf_string);
                Some(InspectedAnnot {
                    subtype,
                    rect,
                    action_s,
                    uri,
                    dest_page_index,
                    contents,
                    has_annots_key,
                    has_border: annot.get(b"Border").is_ok(),
                    has_h: annot.get(b"H").is_ok(),
                    has_f: annot.get(b"F").is_ok(),
                    has_ap: annot.get(b"AP").is_ok(),
                    has_quad_points: annot.get(b"QuadPoints").is_ok(),
                })
            })
            .collect()
    }

    fn rects_near(a: [f64; 4], b: [f64; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 0.5)
    }

    fn xywh_to_pdf(r: [f64; 4]) -> [f64; 4] {
        [r[0], r[1], r[0] + r[2], r[1] + r[3]]
    }

    fn assert_actionable(err: &AppError, what: &str) {
        assert!(
            !err.title.trim().is_empty(),
            "{what}: AppError must have a title; got {err:?}"
        );
        assert!(
            !err.message.trim().is_empty(),
            "{what}: AppError must have a message; got {err:?}"
        );
        assert!(
            err.suggestion
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            "{what}: AppError must have a suggestion; got {err:?}"
        );
    }

    const URI_RECT_XYWH: [f64; 4] = [100.0, 200.0, 80.0, 40.0];
    const GOTO_RECT_XYWH: [f64; 4] = [200.0, 300.0, 80.0, 60.0];
    const URI_RECT_PDF: [i64; 4] = [100, 200, 180, 240];
    const GOTO_RECT_PDF: [i64; 4] = [200, 300, 280, 360];
    const HIGHLIGHT_RECT_PDF: [i64; 4] = [50, 50, 150, 80];

    // --- L3 allowlist / classifier -----------------------------------------

    #[test]
    fn uri_allowlist_https_http_mailto_only() {
        let allowed = [
            "https://example.com/a",
            "http://example.com/b",
            "mailto:user@example.com",
        ];
        let rejected = [
            "javascript:alert(1)",
            "file:///tmp/x",
            "data:text/html,hi",
            "ftp://example.com/f",
            "vbscript:msgbox",
            "myapp:custom",
        ];
        for uri in allowed {
            assert!(uri_is_allowed(uri), "L3: {uri} must be allowed");
        }
        for uri in rejected {
            assert!(!uri_is_allowed(uri), "L3: {uri} must be rejected");
        }
    }

    #[test]
    fn classify_allowlisted_uri_and_explicit_goto_are_supported() {
        assert_eq!(
            classify_link_action("URI", Some("https://example.com"), false),
            LinkActionClass::Uri,
            "L3: https URI must be Uri"
        );
        assert_eq!(
            classify_link_action("URI", Some("http://example.com"), false),
            LinkActionClass::Uri,
            "L3: http URI must be Uri"
        );
        assert_eq!(
            classify_link_action("URI", Some("mailto:a@b.com"), false),
            LinkActionClass::Uri,
            "L3: mailto URI must be Uri"
        );
        assert_eq!(
            classify_link_action("GoTo", None, false),
            LinkActionClass::GoTo,
            "L3: explicit GoTo must be GoTo"
        );
    }

    #[test]
    fn classify_javascript_file_data_uri_are_unsupported() {
        for uri in ["javascript:alert(1)", "file:///tmp/x", "data:text/plain,x"] {
            assert_eq!(
                classify_link_action("URI", Some(uri), false),
                LinkActionClass::Unsupported,
                "L3: classify {uri} must be Unsupported"
            );
        }
    }

    #[test]
    fn classify_launch_gotor_js_named_dest_are_unsupported() {
        assert_eq!(
            classify_link_action("Launch", None, false),
            LinkActionClass::Unsupported,
            "L3: classify Launch must be Unsupported"
        );
        assert_eq!(
            classify_link_action("GoToR", None, false),
            LinkActionClass::Unsupported,
            "L3: classify GoToR must be Unsupported"
        );
        assert_eq!(
            classify_link_action("JavaScript", None, false),
            LinkActionClass::Unsupported,
            "L3: classify JavaScript action must be Unsupported"
        );
        assert_eq!(
            classify_link_action("GoTo", None, true),
            LinkActionClass::Unsupported,
            "L3: classify named dest must be Unsupported"
        );
    }

    #[test]
    fn apply_rejected_uri_is_app_error_and_leaves_dest() {
        let scratch = Scratch::new("l3-write");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);
        let before = std::fs::read(&dest).unwrap();

        for uri in [
            "javascript:alert(1)",
            "file:///tmp/x",
            "data:text/html,hi",
            "ftp://example.com/f",
            "vbscript:msgbox",
            "myapp:custom",
        ] {
            std::fs::write(&dest, &before).unwrap();
            let links = [SessionLink {
                page_index: 0,
                rect: URI_RECT_XYWH,
                action: LinkAction::Uri {
                    uri: uri.to_string(),
                },
            }];
            let err = apply_link_annots(&dest, &links)
                .expect_err(&format!("L3: writing {uri} must be AppError"));
            assert_actionable(&err, &format!("L3 {uri}"));
            assert_eq!(
                std::fs::read(&dest).unwrap(),
                before,
                "L3: dest bytes must stay unchanged after rejected {uri}"
            );
        }
    }

    // --- L1 list -----------------------------------------------------------

    #[test]
    fn list_uri_and_goto_rects_are_unrotated_on_rotate_90_crop() {
        let scratch = Scratch::new("l1");
        let src = scratch.file("src.pdf");
        let mut pdf = PdfFix::new(1, 90, Some([72, 72, 540, 720]));
        pdf.add_link_uri(0, URI_RECT_PDF, "https://example.com/uri");
        pdf.add_link_goto(0, GOTO_RECT_PDF, 0);
        pdf.save(&src);

        let listed = list_link_annots(&src).expect("L1: list_link_annots");
        assert!(
            !listed.is_empty(),
            "L1: list_link_annots must return the URI and GoTo on a Rotate 90 Crop≠Media page; got empty"
        );

        let uri = listed.iter().find(
            |l| matches!(&l.action, LinkAction::Uri { uri } if uri == "https://example.com/uri"),
        );
        let goto = listed
            .iter()
            .find(|l| matches!(&l.action, LinkAction::GoTo { dest_page_index: 0 }));
        let uri = uri.expect("L1: listed set must include the URI link");
        let goto = goto.expect("L1: listed set must include the GoTo link");
        assert_eq!(uri.page_index, 0);
        assert_eq!(goto.page_index, 0);
        assert!(
            rects_near(uri.rect, URI_RECT_XYWH),
            "L1: listed URI rect must be unrotated /Rect as [x,y,w,h], not display-swapped; got {:?}",
            uri.rect
        );
        assert!(
            rects_near(goto.rect, GOTO_RECT_XYWH),
            "L1: listed GoTo rect must be unrotated /Rect as [x,y,w,h], not display-swapped; got {:?}",
            goto.rect
        );
    }

    // --- L2 apply add ------------------------------------------------------

    #[test]
    fn apply_writes_uri_and_goto_link_annots() {
        let scratch = Scratch::new("l2");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(2, 0, None).save(&dest);

        let links = [
            SessionLink {
                page_index: 0,
                rect: URI_RECT_XYWH,
                action: LinkAction::Uri {
                    uri: "https://example.com/new".into(),
                },
            },
            SessionLink {
                page_index: 0,
                rect: GOTO_RECT_XYWH,
                action: LinkAction::GoTo { dest_page_index: 1 },
            },
        ];
        apply_link_annots(&dest, &links).expect("L2: apply_link_annots");

        let annots = inspect_page_annots(&dest, 1);
        let uri = annots.iter().find(|a| {
            a.subtype == "Link"
                && a.action_s.as_deref() == Some("URI")
                && a.uri.as_deref() == Some("https://example.com/new")
        });
        let goto = annots.iter().find(|a| {
            a.subtype == "Link"
                && a.action_s.as_deref() == Some("GoTo")
                && a.dest_page_index == Some(1)
        });
        assert!(
            uri.is_some(),
            "L2: dest page /Annots must have /Subtype /Link /A /S /URI; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.action_s, &a.uri))
                .collect::<Vec<_>>()
        );
        assert!(
            goto.is_some(),
            "L2: dest page /Annots must have /Subtype /Link /A /S /GoTo dest page 1; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.action_s, a.dest_page_index))
                .collect::<Vec<_>>()
        );
        let uri = uri.unwrap();
        let goto = goto.unwrap();
        assert!(
            rects_near(uri.rect, xywh_to_pdf(URI_RECT_XYWH)),
            "L2: URI /Rect must be unrotated [x y x+w y+h]; got {:?}",
            uri.rect
        );
        assert!(
            rects_near(goto.rect, xywh_to_pdf(GOTO_RECT_XYWH)),
            "L2: GoTo /Rect must be unrotated [x y x+w y+h]; got {:?}",
            goto.rect
        );
    }

    // --- L4 survival (keepGreen-after-impl if no-op leaves dest == copy) ----

    #[test]
    fn apply_kept_uri_copies_through_highlight() {
        let scratch = Scratch::new("l4");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://keep.example/");
        pdf.add_highlight(0, HIGHLIGHT_RECT_PDF, "keep-me");
        pdf.save(&dest);

        let links = [SessionLink {
            page_index: 0,
            rect: URI_RECT_XYWH,
            action: LinkAction::Uri {
                uri: "https://keep.example/".into(),
            },
        }];
        apply_link_annots(&dest, &links).expect("L4: apply_link_annots");

        let annots = inspect_page_annots(&dest, 1);
        assert!(
            annots.iter().any(|a| {
                a.subtype == "Link"
                    && a.action_s.as_deref() == Some("URI")
                    && a.uri.as_deref() == Some("https://keep.example/")
            }),
            "L4: dest must still have the session URI; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.uri))
                .collect::<Vec<_>>()
        );
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("keep-me")),
            "L4: dest must copy through the non-Link Highlight; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.contents))
                .collect::<Vec<_>>()
        );
    }

    // --- L6 first Annots key -----------------------------------------------

    #[test]
    fn apply_first_link_creates_annots_key() {
        let scratch = Scratch::new("l6");
        let dest = scratch.file("dest.pdf");
        PdfFix::new(1, 0, None).save(&dest);
        assert!(
            !page_has_annots_key(&dest, 1),
            "fixture must start with no /Annots"
        );

        let links = [SessionLink {
            page_index: 0,
            rect: URI_RECT_XYWH,
            action: LinkAction::Uri {
                uri: "https://example.com/first".into(),
            },
        }];
        apply_link_annots(&dest, &links).expect("L6: apply_link_annots");
        assert!(
            page_has_annots_key(&dest, 1),
            "L6: apply of the first link onto a file with no /Annots must create dest /Annots"
        );
        let annots = inspect_page_annots(&dest, 1);
        assert!(
            annots.iter().any(|a| {
                a.has_annots_key
                    && a.subtype == "Link"
                    && a.action_s.as_deref() == Some("URI")
                    && a.uri.as_deref() == Some("https://example.com/first")
            }),
            "L6: dest /Annots must contain the new URI Link; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.uri))
                .collect::<Vec<_>>()
        );
    }

    // --- L7 delete one supported, keep non-Link ----------------------------

    #[test]
    fn apply_deletes_one_uri_and_keeps_highlight() {
        let scratch = Scratch::new("l7");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://keep.example/");
        pdf.add_link_uri(0, GOTO_RECT_PDF, "https://drop.example/");
        pdf.add_highlight(0, HIGHLIGHT_RECT_PDF, "keep-me");
        pdf.save(&dest);

        let links = [SessionLink {
            page_index: 0,
            rect: URI_RECT_XYWH,
            action: LinkAction::Uri {
                uri: "https://keep.example/".into(),
            },
        }];
        apply_link_annots(&dest, &links).expect("L7: apply_link_annots");

        let annots = inspect_page_annots(&dest, 1);
        let uris: Vec<&str> = annots
            .iter()
            .filter(|a| a.subtype == "Link" && a.action_s.as_deref() == Some("URI"))
            .filter_map(|a| a.uri.as_deref())
            .collect();
        assert!(
            uris.contains(&"https://keep.example/"),
            "L7: dest must keep the remaining URI; got {uris:?}"
        );
        assert!(
            !uris.contains(&"https://drop.example/"),
            "L7: dest must drop the deleted URI; still has {uris:?}"
        );
        assert_eq!(
            uris.len(),
            1,
            "L7: dest must have exactly one URI Link; got {uris:?}"
        );
        assert!(
            annots
                .iter()
                .any(|a| a.subtype == "Highlight" && a.contents.as_deref() == Some("keep-me")),
            "L7: dest must keep the non-Link Highlight; got {:?}",
            annots
                .iter()
                .map(|a| (&a.subtype, &a.contents))
                .collect::<Vec<_>>()
        );
    }

    // --- H1 oversize -------------------------------------------------------

    #[test]
    fn list_oversize_file_is_app_error() {
        let scratch = Scratch::new("h1");
        let path = scratch.file("huge.pdf");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_LINK_BYTES + 1).unwrap();
        drop(f);

        let err = list_link_annots(&path).expect_err(
            "H1: list_link_annots on set_len(MAX_LINK_BYTES+1) must be AppError, not Ok([])",
        );
        assert_actionable(&err, "H1");
    }

    // --- H2-list count cap -------------------------------------------------

    #[test]
    fn list_over_max_links_is_app_error() {
        let scratch = Scratch::new("h2-list");
        let src = scratch.file("many.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_n_uri_links(0, MAX_LINKS + 1, "https://example.com/cap");
        pdf.save(&src);

        match list_link_annots(&src) {
            Ok(listed) => panic!(
                "H2-list: 5,001 supported URI /Link annots must be Err, not Ok of length {}",
                listed.len()
            ),
            Err(err) => assert_actionable(&err, "H2-list"),
        }
    }

    // --- H2-save incomplete empty must not wipe ----------------------------

    #[test]
    fn incomplete_empty_session_does_not_rewrite_supported_links() {
        assert!(
            !should_rewrite_supported_links(false, 0, true),
            "H2-save: incomplete hydrate + empty session must not rewrite dest supported links"
        );
        assert!(
            should_rewrite_supported_links(true, 0, true),
            "H2-save: complete hydrate + empty session must still rewrite (L7 delete-all)"
        );
    }

    #[test]
    fn incomplete_empty_session_does_not_wipe_dest_supported_links() {
        let scratch = Scratch::new("h2-save");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://a.example/");
        pdf.add_link_uri(0, GOTO_RECT_PDF, "https://b.example/");
        pdf.add_link_uri(0, HIGHLIGHT_RECT_PDF, "https://c.example/");
        pdf.save(&dest);

        if should_rewrite_supported_links(false, 0, true) {
            apply_link_annots(&dest, &[]).expect("H2-save: apply empty");
        }

        let annots = inspect_page_annots(&dest, 1);
        let uris: Vec<&str> = annots
            .iter()
            .filter(|a| a.subtype == "Link" && a.action_s.as_deref() == Some("URI"))
            .filter_map(|a| a.uri.as_deref())
            .collect();
        assert_eq!(
            uris.len(),
            3,
            "H2-save: incomplete hydrate + empty session must not wipe dest supported links; got {uris:?}"
        );
        assert!(uris.contains(&"https://a.example/"));
        assert!(uris.contains(&"https://b.example/"));
        assert!(uris.contains(&"https://c.example/"));
    }

    #[test]
    fn complete_empty_session_still_deletes_dest_supported_links() {
        assert!(
            should_rewrite_supported_links(true, 0, true),
            "H2-save: complete empty must still rewrite (L7)"
        );
        let scratch = Scratch::new("h2-save-l7");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://drop.example/");
        pdf.save(&dest);
        apply_link_annots(&dest, &[]).expect("H2-save L7 contrast: apply empty");
        let annots = inspect_page_annots(&dest, 1);
        let uris: Vec<&str> = annots
            .iter()
            .filter(|a| a.subtype == "Link" && a.action_s.as_deref() == Some("URI"))
            .filter_map(|a| a.uri.as_deref())
            .collect();
        assert!(
            uris.is_empty(),
            "H2-save: complete empty session must still delete dest supported links; got {uris:?}"
        );
    }

    // --- H3 no-op keeps extra dest keys ------------------------------------

    #[test]
    fn apply_noop_keeps_extra_link_dict_keys() {
        let scratch = Scratch::new("h3");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(1, 0, None);
        pdf.add_link_uri_with_extra_keys(0, URI_RECT_PDF, "https://keep.example/");
        pdf.save(&dest);

        let links = [SessionLink {
            page_index: 0,
            rect: URI_RECT_XYWH,
            action: LinkAction::Uri {
                uri: "https://keep.example/".into(),
            },
        }];
        apply_link_annots(&dest, &links).expect("H3: apply_link_annots no-op");

        let annots = inspect_page_annots(&dest, 1);
        let uri = annots.iter().find(|a| {
            a.subtype == "Link"
                && a.action_s.as_deref() == Some("URI")
                && a.uri.as_deref() == Some("https://keep.example/")
        });
        let uri = uri.expect("H3: dest must still have the session URI");
        assert!(
            uri.has_border,
            "H3: dest URI must keep /Border; keys gone after rewrite"
        );
        assert!(
            uri.has_h,
            "H3: dest URI must keep /H; keys gone after rewrite"
        );
        assert!(
            uri.has_f,
            "H3: dest URI must keep /F; keys gone after rewrite"
        );
        assert!(
            uri.has_ap,
            "H3: dest URI must keep /AP; keys gone after rewrite"
        );
        assert!(
            uri.has_quad_points,
            "H3: dest URI must keep /QuadPoints; keys gone after rewrite"
        );
        assert_eq!(
            uri.contents.as_deref(),
            Some("keep-contents"),
            "H3: dest URI must keep /Contents; keys gone after rewrite"
        );
    }

    // --- H4 list Err -------------------------------------------------------

    #[test]
    fn list_missing_path_is_app_error() {
        let scratch = Scratch::new("h4-missing");
        let path = scratch.file("missing.pdf");
        let err =
            list_link_annots(&path).expect_err("H4: missing path must be AppError, not Ok([])");
        assert_actionable(&err, "H4 missing path");
    }

    #[test]
    fn list_invalid_pdf_is_app_error() {
        let scratch = Scratch::new("h4-invalid");
        let path = scratch.file("not.pdf");
        std::fs::write(&path, b"not a pdf").unwrap();
        let err = list_link_annots(&path).expect_err("H4: load fail must be AppError, not Ok([])");
        assert_actionable(&err, "H4 load fail");
    }

    // --- H7 per-source rewrite ---------------------------------------------

    fn page_uris(path: &Path, page_1based: u32) -> Vec<String> {
        inspect_page_annots(path, page_1based)
            .into_iter()
            .filter(|a| a.subtype == "Link" && a.action_s.as_deref() == Some("URI"))
            .filter_map(|a| a.uri)
            .collect()
    }

    #[test]
    fn incomplete_a_session_on_b_rewrites_b_range_not_a() {
        let groups = [("/a.pdf", 1u32), ("/b.pdf", 1)];
        let ranges = dest_ranges_to_rewrite(&groups, &["/a.pdf"]);
        assert_eq!(
            ranges,
            vec![1..2],
            "H7-range: incomplete A + session links only on B must rewrite B dest pages, not A; got {ranges:?}"
        );
        assert!(
            !ranges.iter().any(|r| r.contains(&0)),
            "H7-range: must not rewrite A's dest page 0; got {ranges:?}"
        );
    }

    #[test]
    fn mixed_source_b_uri_change_applied_a_unchanged() {
        let scratch = Scratch::new("h7-edit");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(2, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://a.example/keep");
        pdf.add_link_uri(1, GOTO_RECT_PDF, "https://b.example/old");
        pdf.save(&dest);

        let session = [SessionLink {
            page_index: 1,
            rect: GOTO_RECT_XYWH,
            action: LinkAction::Uri {
                uri: "https://b.example/new".into(),
            },
        }];
        let ranges = dest_ranges_to_rewrite(&[("/a.pdf", 1), ("/b.pdf", 1)], &["/a.pdf"]);
        let pages: Vec<u32> = ranges.into_iter().flatten().collect();
        apply_link_annots_for_pages(&dest, &session, &pages).expect("H7-edit apply");

        let a_uris = page_uris(&dest, 1);
        assert_eq!(
            a_uris,
            vec!["https://a.example/keep".to_string()],
            "H7-edit: A dest URI must stay unchanged; got {a_uris:?}"
        );
        let b_uris = page_uris(&dest, 2);
        assert_eq!(
            b_uris,
            vec!["https://b.example/new".to_string()],
            "H7-edit: B dest URI must be the session URI; got {b_uris:?}"
        );
    }

    #[test]
    fn mixed_source_b_links_removed_a_remain() {
        let scratch = Scratch::new("h7-delete");
        let dest = scratch.file("dest.pdf");
        let mut pdf = PdfFix::new(2, 0, None);
        pdf.add_link_uri(0, URI_RECT_PDF, "https://a.example/keep");
        pdf.add_link_uri(1, GOTO_RECT_PDF, "https://b.example/old");
        pdf.save(&dest);

        let ranges = dest_ranges_to_rewrite(&[("/a.pdf", 1), ("/b.pdf", 1)], &["/a.pdf"]);
        let pages: Vec<u32> = ranges.into_iter().flatten().collect();
        apply_link_annots_for_pages(&dest, &[], &pages).expect("H7-delete apply");

        let a_uris = page_uris(&dest, 1);
        assert_eq!(
            a_uris,
            vec!["https://a.example/keep".to_string()],
            "H7-delete: A dest links must remain; got {a_uris:?}"
        );
        let b_uris = page_uris(&dest, 2);
        assert!(
            b_uris.is_empty(),
            "H7-delete: B dest supported links must be gone; got {b_uris:?}"
        );
    }
}
