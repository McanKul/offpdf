//! Page content and ownership (SPEC §B.7): a page's `/Contents` parts decoded under budget and
//! joined with one `\n` between parts (all spans are offsets into the joined buffer), qpdf's
//! join rule for its overlay wrapper, and the reference/`/Kids` counts that decide whether a
//! content part belongs to this page alone.

mod joins;

pub(crate) use joins::{check_part_joins, first_unsafe_boundary};

use crate::pdf_engine::text_edit::decode::{self, DecodeBudget};
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::snapshot::fnv1a_extend;
use crate::pdf_engine::validate_output::{content_digest, ContentDigest};
use lopdf::{Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PART_KEYS: [&[u8]; 4] = [b"Length", b"Filter", b"DecodeParms", b"DL"];
const REF_DEPTH_MAX: usize = 64;

#[derive(Debug, Clone)]
pub struct ContentPart {
    pub stream_id: ObjectId,
    pub start: usize,
    pub len: usize,
    pub digest: ContentDigest,
}

#[derive(Debug, Clone)]
pub struct PageContent {
    pub page_id: ObjectId,
    pub parts: Vec<ContentPart>,
    pub joined: Vec<u8>,
    pub contents_array: Option<ObjectId>,
}

impl PageContent {
    /// (part index, local span) or None when the span covers a separator.
    pub fn locate(&self, span: &Span) -> Option<(usize, Span)> {
        self.parts.iter().enumerate().find_map(|(i, p)| {
            let end = p.start.checked_add(p.len)?;
            (span.start >= p.start && span.end <= end && span.start <= span.end)
                .then(|| (i, span.start - p.start..span.end - p.start))
        })
    }

    pub fn part_bytes(&self, i: usize) -> &[u8] {
        self.parts
            .get(i)
            .and_then(|p| self.joined.get(p.start..p.start.checked_add(p.len)?))
            .unwrap_or_default()
    }

    /// Digest of the parts concatenated WITHOUT separator (= #34 `get_page_content` semantics).
    pub fn concat_digest(&self) -> ContentDigest {
        let mut hash = FNV_OFFSET;
        let mut len = 0usize;
        for i in 0..self.parts.len() {
            let bytes = self.part_bytes(i);
            hash = fnv1a_extend(hash, bytes);
            len = len.saturating_add(bytes.len());
        }
        ContentDigest { hash, len }
    }

    /// The same page with some parts' decoded bytes replaced (unknown indices are ignored).
    pub fn with_replaced_parts(&self, replaced: &[(usize, Vec<u8>)]) -> PageContent {
        let parts: Vec<(ObjectId, &[u8])> = (0..self.parts.len())
            .filter_map(|i| {
                let id = self.parts.get(i)?.stream_id;
                let bytes = replaced
                    .iter()
                    .rev()
                    .find(|(j, _)| *j == i)
                    .map(|(_, b)| b.as_slice());
                Some((id, bytes.unwrap_or_else(|| self.part_bytes(i))))
            })
            .collect();
        assemble(self.page_id, &parts, self.contents_array)
    }
}

fn assemble(
    page_id: ObjectId,
    parts: &[(ObjectId, &[u8])],
    contents_array: Option<ObjectId>,
) -> PageContent {
    let total: usize =
        parts.iter().map(|(_, b)| b.len()).sum::<usize>() + parts.len().saturating_sub(1);
    let mut joined = Vec::with_capacity(total);
    let mut out = Vec::with_capacity(parts.len());
    for (i, (stream_id, bytes)) in parts.iter().enumerate() {
        if i > 0 {
            joined.push(b'\n');
        }
        out.push(ContentPart {
            stream_id: *stream_id,
            start: joined.len(),
            len: bytes.len(),
            digest: content_digest(bytes),
        });
        joined.extend_from_slice(bytes);
    }
    PageContent {
        page_id,
        parts: out,
        joined,
        contents_array,
    }
}

/// Reads and decodes a page's `/Contents` (missing or null = empty page). Direct streams,
/// unresolvable references and non-stream parts are `MALFORMED_CONTENT`; more than
/// `PAGE_PARTS_MAX` parts or too much decoded data `PAGE_TOO_COMPLEX`; part dictionaries with
/// keys other than `/Length /Filter /DecodeParms /DL` `UNSUPPORTED_FILTER`.
pub fn page_content(
    doc: &Document,
    page_id: ObjectId,
    budget: &mut DecodeBudget,
) -> Result<PageContent, TextReason> {
    let Some(Object::Dictionary(page)) = doc.objects.get(&page_id) else {
        return Err(TextReason::MalformedContent);
    };
    let refs_of = |items: &[Object]| -> Result<Vec<ObjectId>, TextReason> {
        if items.len() > limits::PAGE_PARTS_MAX {
            return Err(TextReason::PageTooComplex);
        }
        items
            .iter()
            .map(|o| o.as_reference().map_err(|_| TextReason::MalformedContent))
            .collect()
    };
    let (ids, contents_array) = match page.get(b"Contents").ok() {
        None | Some(Object::Null) => (Vec::new(), None),
        Some(Object::Reference(id)) => match doc.objects.get(id) {
            Some(Object::Stream(_)) => (vec![*id], None),
            Some(Object::Array(items)) => (refs_of(items)?, Some(*id)),
            _ => return Err(TextReason::MalformedContent),
        },
        Some(Object::Array(items)) => (refs_of(items)?, None),
        Some(_) => return Err(TextReason::MalformedContent),
    };
    let mut decoded: Vec<(ObjectId, Vec<u8>)> = Vec::with_capacity(ids.len());
    let mut page_total = 0usize;
    for id in ids {
        let Some(Object::Stream(stream)) = doc.objects.get(&id) else {
            return Err(TextReason::MalformedContent);
        };
        if stream
            .dict
            .iter()
            .any(|(k, _)| !PART_KEYS.contains(&k.as_slice()))
        {
            return Err(TextReason::UnsupportedFilter);
        }
        let page_left = limits::PAGE_CONTENT_MAX_DECODED.saturating_sub(page_total);
        let cap = limits::STREAM_MAX_DECODED.min(page_left);
        let data = decode::decode_stream(stream, cap, budget).map_err(|e| e.page_reason())?;
        page_total = page_total.saturating_add(data.len());
        decoded.push((id, data));
    }
    let parts: Vec<(ObjectId, &[u8])> = decoded.iter().map(|(id, d)| (*id, d.as_slice())).collect();
    Ok(assemble(page_id, &parts, contents_array))
}

/// qpdf's rule when it turns a page's parts into one Form stream (overlay wrapper, P-OV), as
/// probed on qpdf 12.3.2: before each part a `\n` is written when the previous step left the
/// output not ending in `\n` (a part ending in `\n` needs none; an empty part leaves the
/// state "needs a newline" unless a `\n` was just written for it). Pinned by CON-10 and APP-07.
pub fn qpdf_join(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len() + 1).sum());
    let mut need_newline = false;
    for part in parts {
        if need_newline {
            out.push(b'\n');
        }
        let last = match part.last() {
            Some(c) => Some(*c),
            None if need_newline => Some(b'\n'),
            None => None,
        };
        out.extend_from_slice(part);
        need_newline = last != Some(b'\n');
    }
    out
}

pub struct RefCounts {
    counts: HashMap<ObjectId, u32>,
    truncated: bool, // a value nested deeper than 64 levels was not counted
}

impl RefCounts {
    /// Every Reference in every object (and in the trailer), iterative, depth ≤ 64.
    pub fn of(doc: &Document) -> RefCounts {
        let mut counts: HashMap<ObjectId, u32> = HashMap::new();
        let mut truncated = false;
        let mut stack: Vec<(&Object, usize)> = doc.objects.values().map(|o| (o, 0)).collect();
        stack.extend(doc.trailer.iter().map(|(_, v)| (v, 1)));
        while let Some((obj, depth)) = stack.pop() {
            let container = matches!(
                obj,
                Object::Array(_) | Object::Dictionary(_) | Object::Stream(_)
            );
            if container && depth >= REF_DEPTH_MAX {
                truncated = true;
                continue;
            }
            match obj {
                Object::Reference(id) => {
                    let c = counts.entry(*id).or_insert(0);
                    *c = c.saturating_add(1);
                }
                Object::Array(items) => stack.extend(items.iter().map(|c| (c, depth + 1))),
                Object::Dictionary(d) => stack.extend(d.iter().map(|(_, v)| (v, depth + 1))),
                Object::Stream(s) => stack.extend(s.dict.iter().map(|(_, v)| (v, depth + 1))),
                _ => {}
            }
        }
        RefCounts { counts, truncated }
    }

    pub fn count(&self, id: ObjectId) -> u32 {
        self.counts.get(&id).copied().unwrap_or(0)
    }
}

/// Occurrences of each page object id across all `/Kids` arrays of the page tree (own iterative walk from
/// the catalog `/Pages`, depth ≤ PAGE_TREE_DEPTH_MAX, visited set; a cycle or a non-dictionary kid ⇒ Err).
pub struct KidsCounts {
    counts: HashMap<ObjectId, u32>,
}

impl KidsCounts {
    pub fn of(doc: &Document) -> Result<KidsCounts, TextReason> {
        let bad = TextReason::MalformedContent;
        let root = doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Pages").ok())
            .and_then(|o| o.as_reference().ok())
            .ok_or(bad)?;
        let mut counts: HashMap<ObjectId, u32> = HashMap::new();
        let mut visited: HashSet<ObjectId> = HashSet::from([root]);
        let mut stack = vec![(root, 0usize)];
        while let Some((node_id, depth)) = stack.pop() {
            let Some(Object::Dictionary(node)) = doc.objects.get(&node_id) else {
                return Err(bad);
            };
            let kids = match node.get(b"Kids").map_err(|_| bad)? {
                Object::Array(items) => items,
                Object::Reference(id) => match doc.objects.get(id) {
                    Some(Object::Array(items)) => items,
                    _ => return Err(bad),
                },
                _ => return Err(bad),
            };
            for kid in kids {
                let kid_id = kid.as_reference().map_err(|_| bad)?;
                let c = counts.entry(kid_id).or_insert(0);
                *c = c.saturating_add(1);
                let Some(Object::Dictionary(kid_dict)) = doc.objects.get(&kid_id) else {
                    return Err(bad);
                };
                if kid_dict.has(b"Kids") || kid_dict.type_is(b"Pages") {
                    if depth + 1 > limits::PAGE_TREE_DEPTH_MAX || !visited.insert(kid_id) {
                        return Err(bad);
                    }
                    stack.push((kid_id, depth + 1));
                }
            }
        }
        Ok(KidsCounts { counts })
    }

    pub fn count(&self, id: ObjectId) -> u32 {
        self.counts.get(&id).copied().unwrap_or(0)
    }
}

/// True only if: the page id occurs exactly once across all `/Kids` arrays **and** once in `get_pages()`
/// (references from StructElem `/Pg`, annotation `/P`, outline/link `/Dest`, named destinations and
/// `/OpenAction` are legitimate and ignored); `/Contents` is a stream reference, a direct array, or an array
/// object whose `RefCounts` count is 1; the part stream's `RefCounts` count is exactly 1 (a content stream can
/// only legitimately be referenced from `/Contents`); the part's dict has no `/F`.
pub fn part_exclusive(
    doc: &Document,
    refs: &RefCounts,
    kids: &KidsCounts,
    content: &PageContent,
    part: usize,
) -> bool {
    let Some(p) = content.parts.get(part) else {
        return false;
    };
    if refs.truncated || kids.count(content.page_id) != 1 {
        return false;
    }
    if doc
        .get_pages()
        .values()
        .filter(|id| **id == content.page_id)
        .count()
        != 1
    {
        return false;
    }
    if content
        .contents_array
        .is_some_and(|arr| refs.count(arr) != 1)
    {
        return false;
    }
    let has_f = match doc.objects.get(&p.stream_id) {
        Some(Object::Stream(s)) => s.dict.has(b"F"),
        _ => true,
    };
    refs.count(p.stream_id) == 1 && !has_f
}
