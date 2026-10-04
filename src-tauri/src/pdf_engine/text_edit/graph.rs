//! Canonical, object-id-free digest of a whole document (SPEC §B.16.1, D28): Phase A check A2
//! proves that qpdf's output equals qpdf's input everywhere except the edited content streams.
//!
//! Traversal: BFS over canonical indices — 0 = trailer `/Root`, 1 = trailer `/Info` when present;
//! every reference becomes its canonical index (assigned on first sight). Direct values serialise
//! recursively with dictionary keys sorted bytewise, names and strings by their decoded bytes,
//! integers as i64 and reals by their f32 bits. qpdf's known normalisations are applied on both
//! sides: dictionary entries whose value is null (or a reference to nothing) are absent, a dangling
//! reference in an array is `null`, and `/Root /Extensions` (shallow) and its `/ADBE` (deep) are
//! serialised inline. Streams: the node is the dictionary without `/Length /Filter /DecodeParms
//! /DL`; the data key is `Replaced` for an edited part (its expected decoded bytes), `Plain` for an
//! unfiltered stream and `Raw` (filter signature + raw bytes) for a filtered one, which qpdf
//! (`--decode-level=none`) must keep byte for byte. The filter signature is serialised through the
//! traversal, so objects reached only through `/DecodeParms` (`/JBIG2Globals`) are compared too.
//! An edited stream reached from more than one place is never `Replaced` (it is shared: the edit
//! would change the other place too).

use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget, DecodeError};
use crate::pdf_engine::text_edit::limits::GRAPH_DIRECT_DEPTH_MAX;
use crate::pdf_engine::text_edit::snapshot::fnv1a_extend;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const STREAM_KEYS: [&[u8]; 4] = [b"Length", b"Filter", b"DecodeParms", b"DL"];
/// Objects processed between two looks at the cancel flag.
const CANCEL_EVERY: usize = 4_096;
/// Longest path kept for a mismatch message.
const PATH_MAX_CHARS: usize = 512;
/// The `/Root` of a trailer that has none.
static NULL_OBJECT: Object = Object::Null;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataKey {
    None,
    Raw {
        filter_sig: u64,
        raw: u64,
    },
    /// `len`: the decoded length (the after side never decodes more than that).
    Plain {
        hash: u64,
        len: u64,
    },
    Replaced {
        expected: u64,
        len: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEntry {
    /// Hash of the canonical serialisation (references as canonical indices).
    pub node: u64,
    pub data: DataKey,
}

#[derive(Debug, Clone, Default)]
pub struct GraphDigest {
    pub entries: Vec<GraphEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphMismatch {
    pub index: usize,
    pub path: String,
    /// node|data|missing|extra|budget|cancelled
    pub what: &'static str,
}

struct Hasher(u64);

impl Hasher {
    fn new() -> Hasher {
        Hasher(FNV_OFFSET)
    }
    fn tag(&mut self, t: u8) {
        self.0 = fnv1a_extend(self.0, &[t]);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0 = fnv1a_extend(self.0, &(b.len() as u64).to_le_bytes());
        self.0 = fnv1a_extend(self.0, b);
    }
    fn u64(&mut self, v: u64) {
        self.0 = fnv1a_extend(self.0, &v.to_le_bytes());
    }
}

fn hash_bytes(b: &[u8]) -> u64 {
    fnv1a_extend(FNV_OFFSET, b)
}

/// How references inside a value are serialised.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inline {
    /// As canonical indices.
    No,
    /// Every reference resolved inline (qpdf `makeDirect`).
    Deep,
}

/// A canonical traversal of one document.
struct Traversal<'a> {
    doc: &'a Document,
    index: HashMap<ObjectId, usize>,
    /// Per canonical index: the object (or `None` for a direct trailer `/Info`).
    queue: VecDeque<usize>,
    targets: Vec<Target<'a>>,
    paths: Vec<String>,
    incoming: Vec<u32>,
    parent: Vec<Option<usize>>,
}

#[derive(Clone, Copy)]
enum Target<'a> {
    Ref(ObjectId),
    Direct(&'a Object),
}

fn null_like(doc: &Document, obj: &Object) -> bool {
    match obj {
        Object::Null => true,
        Object::Reference(id) => matches!(doc.objects.get(id), None | Some(Object::Null)),
        _ => false,
    }
}

fn join_path(base: &str, part: &str) -> String {
    let mut p = String::with_capacity(base.len() + part.len());
    p.push_str(base);
    p.push_str(part);
    if p.chars().count() > PATH_MAX_CHARS {
        p = p.chars().take(PATH_MAX_CHARS).collect::<String>() + "…";
    }
    p
}

/// Only containers and references can lead to a new canonical index (whose path is kept).
fn needs_path(obj: &Object) -> bool {
    matches!(
        obj,
        Object::Reference(_) | Object::Array(_) | Object::Dictionary(_) | Object::Stream(_)
    )
}

fn key_path(key: &[u8]) -> String {
    format!("/{}", String::from_utf8_lossy(key))
}

impl<'a> Traversal<'a> {
    fn new(doc: &'a Document) -> Traversal<'a> {
        let mut t = Traversal {
            doc,
            index: HashMap::new(),
            queue: VecDeque::new(),
            targets: Vec::new(),
            paths: Vec::new(),
            incoming: Vec::new(),
            parent: Vec::new(),
        };
        let seed = |t: &mut Traversal<'a>, key: &[u8], path: &str| {
            if let Ok(v) = doc.trailer.get(key) {
                let target = match v {
                    Object::Reference(id) => Target::Ref(*id),
                    other => Target::Direct(other),
                };
                t.push(target, path.to_string(), None);
            }
        };
        seed(&mut t, b"Root", "/Root");
        if t.targets.is_empty() {
            t.push(Target::Direct(&NULL_OBJECT), "/Root".to_string(), None);
        }
        seed(&mut t, b"Info", "/Info");
        t
    }

    fn push(&mut self, target: Target<'a>, path: String, parent: Option<usize>) -> usize {
        let i = self.targets.len();
        if let Target::Ref(id) = target {
            self.index.insert(id, i);
        }
        self.targets.push(target);
        self.paths.push(path);
        self.incoming.push(0);
        self.parent.push(parent);
        self.queue.push_back(i);
        i
    }

    /// The canonical index of `id` (assigned on first sight), counting the reference.
    fn index_of(&mut self, id: ObjectId, path: &str, from: usize) -> usize {
        let i = match self.index.get(&id) {
            Some(i) => *i,
            None => self.push(Target::Ref(id), path.to_string(), Some(from)),
        };
        if let Some(c) = self.incoming.get_mut(i) {
            *c = c.saturating_add(1);
        }
        i
    }

    fn ser(
        &mut self,
        h: &mut Hasher,
        obj: &'a Object,
        depth: usize,
        path: &str,
        from: usize,
        inline: Inline,
    ) -> Result<(), &'static str> {
        if depth > GRAPH_DIRECT_DEPTH_MAX {
            return Err("budget");
        }
        match obj {
            Object::Null => h.tag(b'n'),
            Object::Boolean(b) => {
                h.tag(b'b');
                h.tag(u8::from(*b));
            }
            Object::Integer(i) => {
                h.tag(b'i');
                h.u64(*i as u64);
            }
            Object::Real(r) => {
                h.tag(b'r');
                h.u64(u64::from(r.to_bits()));
            }
            Object::Name(n) => {
                h.tag(b'N');
                h.bytes(n);
            }
            Object::String(s, _) => {
                h.tag(b'S');
                h.bytes(s);
            }
            Object::Array(items) => {
                h.tag(b'[');
                h.u64(items.len() as u64);
                for (k, item) in items.iter().enumerate() {
                    if null_like(self.doc, item) {
                        h.tag(b'n');
                    } else if needs_path(item) {
                        let p = join_path(path, &format!("[{k}]"));
                        self.ser(h, item, depth + 1, &p, from, inline)?;
                    } else {
                        self.ser(h, item, depth + 1, "", from, inline)?;
                    }
                }
            }
            Object::Dictionary(d) => self.ser_dict(h, d, &[], depth, path, from, inline, false)?,
            Object::Stream(s) => {
                h.tag(b'X');
                self.ser_dict(h, &s.dict, &STREAM_KEYS, depth, path, from, inline, false)?;
            }
            Object::Reference(id) => match self.doc.objects.get(id) {
                None | Some(Object::Null) => h.tag(b'n'),
                Some(target) if inline == Inline::Deep => {
                    if matches!(target, Object::Stream(_)) {
                        return Err("node");
                    }
                    self.ser(h, target, depth + 1, path, from, inline)?;
                }
                Some(_) => {
                    let i = self.index_of(*id, path, from);
                    h.tag(b'R');
                    h.u64(i as u64);
                }
            },
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn ser_dict(
        &mut self,
        h: &mut Hasher,
        d: &'a Dictionary,
        skip: &[&[u8]],
        depth: usize,
        path: &str,
        from: usize,
        inline: Inline,
        catalog: bool,
    ) -> Result<(), &'static str> {
        let mut entries: Vec<(&'a Vec<u8>, &'a Object)> = d
            .iter()
            .filter(|(k, v)| !skip.contains(&k.as_slice()) && !null_like(self.doc, v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        h.tag(b'<');
        h.u64(entries.len() as u64);
        for (k, v) in entries {
            h.bytes(k);
            let p = if needs_path(v) {
                join_path(path, &key_path(k))
            } else {
                String::new()
            };
            if catalog && k.as_slice() == b"Extensions" {
                self.ser_extensions(h, v, depth + 1, &p, from)?;
            } else {
                self.ser(h, v, depth + 1, &p, from, inline)?;
            }
        }
        Ok(())
    }

    /// `/Root /Extensions`: the dictionary itself inline (qpdf makes it direct), and its `/ADBE`
    /// entry deeply inline when it is a reference (qpdf `makeDirect`).
    fn ser_extensions(
        &mut self,
        h: &mut Hasher,
        v: &'a Object,
        depth: usize,
        path: &str,
        from: usize,
    ) -> Result<(), &'static str> {
        let resolved = match v {
            Object::Reference(id) => match self.doc.objects.get(id) {
                Some(o) => o,
                None => {
                    h.tag(b'n');
                    return Ok(());
                }
            },
            other => other,
        };
        let Object::Dictionary(ext) = resolved else {
            return self.ser(h, resolved, depth, path, from, Inline::No);
        };
        let mut entries: Vec<(&'a Vec<u8>, &'a Object)> = ext
            .iter()
            .filter(|(_, v)| !null_like(self.doc, v))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        h.tag(b'<');
        h.u64(entries.len() as u64);
        for (k, v) in entries {
            h.bytes(k);
            let p = join_path(path, &key_path(k));
            let inline = if k.as_slice() == b"ADBE" && matches!(v, Object::Reference(_)) {
                Inline::Deep
            } else {
                Inline::No
            };
            self.ser(h, v, depth + 1, &p, from, inline)?;
        }
        Ok(())
    }

    /// The node hash of canonical index `i` and, for a stream, the stream.
    fn node(&mut self, i: usize) -> Result<(u64, Option<&'a Stream>), &'static str> {
        let target = self.targets.get(i).copied();
        let path = self.paths.get(i).cloned().unwrap_or_default();
        let mut h = Hasher::new();
        let obj: &'a Object = match target {
            Some(Target::Ref(id)) => match self.doc.objects.get(&id) {
                Some(o) => o,
                None => {
                    h.tag(b'n');
                    return Ok((h.0, None));
                }
            },
            Some(Target::Direct(o)) => o,
            None => return Err("missing"),
        };
        match obj {
            Object::Stream(s) => {
                h.tag(b'T');
                self.ser_dict(
                    &mut h,
                    &s.dict,
                    &STREAM_KEYS,
                    0,
                    &path,
                    i,
                    Inline::No,
                    false,
                )?;
                Ok((h.0, Some(s)))
            }
            Object::Dictionary(d) => {
                self.ser_dict(&mut h, d, &[], 0, &path, i, Inline::No, i == 0)?;
                Ok((h.0, None))
            }
            other => {
                self.ser(&mut h, other, 0, &path, i, Inline::No)?;
                Ok((h.0, None))
            }
        }
    }

    fn id_of(&self, i: usize) -> Option<ObjectId> {
        match self.targets.get(i) {
            Some(Target::Ref(id)) => Some(*id),
            _ => None,
        }
    }
}

/// Signature of a stream whose filter signature cannot be part of the traversal any more.
const DETACHED_SIG: u64 = 0;

impl<'a> Traversal<'a> {
    /// `/Filter` and `/DecodeParms` of stream `i` as one signature, serialised through this
    /// traversal: a reference among them (an indirect parameter dictionary, a JBIG2 image's
    /// `/JBIG2Globals` stream) gets its canonical index and is compared like any other object.
    fn filter_sig(&mut self, i: usize, s: &'a Stream) -> Result<u64, &'static str> {
        let path = self.paths.get(i).cloned().unwrap_or_default();
        let mut h = Hasher::new();
        for key in [&b"Filter"[..], b"DecodeParms"] {
            h.bytes(key);
            match s.dict.get(key) {
                Ok(v) if !null_like(self.doc, v) => {
                    let p = if needs_path(v) {
                        join_path(&path, &key_path(key))
                    } else {
                        String::new()
                    };
                    self.ser(&mut h, v, 1, &p, i, Inline::No)?;
                }
                _ => h.tag(b'-'),
            }
        }
        Ok(h.0)
    }

    /// The data key of an unedited stream `i`: `Plain` when unfiltered (a `/DecodeParms` without
    /// `/Filter` has no effect and is not followed), else `Raw` with its filter signature.
    fn original_key(&mut self, i: usize, s: &'a Stream) -> Result<DataKey, &'static str> {
        if unfiltered(s) {
            return Ok(plain_key(s));
        }
        Ok(DataKey::Raw {
            filter_sig: self.filter_sig(i, s)?,
            raw: hash_bytes(&s.content),
        })
    }
}

fn plain_key(s: &Stream) -> DataKey {
    DataKey::Plain {
        hash: hash_bytes(&s.content),
        len: s.content.len() as u64,
    }
}

fn unfiltered(stream: &Stream) -> bool {
    match stream.dict.get(b"Filter") {
        Err(_) | Ok(Object::Null) => true,
        Ok(Object::Array(items)) => items.is_empty(),
        Ok(_) => false,
    }
}

/// The key of a shared edited stream, decided after the traversal: its original data, so any
/// change fails. qpdf writes the edit into it unfiltered, so a filtered original can never match
/// on the after side and its filter signature is not needed (`DETACHED_SIG`).
fn shared_key(s: &Stream) -> DataKey {
    if unfiltered(s) {
        plain_key(s)
    } else {
        DataKey::Raw {
            filter_sig: DETACHED_SIG,
            raw: hash_bytes(&s.content),
        }
    }
}

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Canonical traversal of `doc` from trailer `/Root` then `/Info`. `replaced` maps the ids of the
/// edited content streams **in this document** to their expected decoded bytes.
pub fn graph_digest(
    doc: &Document,
    replaced: &HashMap<ObjectId, Vec<u8>>,
    cancel: Option<&AtomicBool>,
) -> Result<GraphDigest, GraphMismatch> {
    let mut t = Traversal::new(doc);
    let mut entries: Vec<GraphEntry> = Vec::new();
    let mut done = 0usize;
    while let Some(i) = t.queue.pop_front() {
        done += 1;
        if done % CANCEL_EVERY == 0 && cancelled(cancel) {
            return Err(mismatch(&t, i, "cancelled"));
        }
        let (node, stream) = t.node(i).map_err(|what| mismatch(&t, i, what))?;
        let data = match stream {
            None => DataKey::None,
            Some(s) => match t.id_of(i).and_then(|id| replaced.get(&id)) {
                Some(bytes) => DataKey::Replaced {
                    expected: hash_bytes(bytes),
                    len: bytes.len() as u64,
                },
                None => t.original_key(i, s).map_err(|what| mismatch(&t, i, what))?,
            },
        };
        entries.push(GraphEntry { node, data });
    }
    // A replaced stream (or its `/Contents` array) reached from more than one place is shared:
    // keep its original data so any change to it fails.
    for (i, e) in entries.iter_mut().enumerate() {
        if !matches!(e.data, DataKey::Replaced { .. }) {
            continue;
        }
        let parent_shared = t.parent.get(i).copied().flatten().is_some_and(|p| {
            let array = |id: &ObjectId| matches!(doc.objects.get(id), Some(Object::Array(_)));
            matches!(t.targets.get(p), Some(Target::Ref(id)) if array(id))
                && t.incoming.get(p).copied().unwrap_or(0) > 1
        });
        let shared = t.incoming.get(i).copied().unwrap_or(0) > 1 || parent_shared;
        if shared {
            if let Some(Object::Stream(s)) = t.id_of(i).and_then(|id| doc.objects.get(&id)) {
                e.data = shared_key(s);
            }
        }
    }
    Ok(GraphDigest { entries })
}

fn mismatch(t: &Traversal<'_>, i: usize, what: &'static str) -> GraphMismatch {
    GraphMismatch {
        index: i,
        path: t.paths.get(i).cloned().unwrap_or_default(),
        what,
    }
}

/// Same traversal over `doc`, compared entry by entry with `before`; a stream is decoded only when
/// `before` holds `Plain` or `Replaced` for that entry (never beyond its expected length). The
/// first mismatch is returned with its canonical path in this document.
pub fn graph_matches(
    doc: &Document,
    before: &GraphDigest,
    budget: &mut DecodeBudget,
    cancel: Option<&AtomicBool>,
) -> Result<(), GraphMismatch> {
    let mut t = Traversal::new(doc);
    let mut done = 0usize;
    while let Some(i) = t.queue.pop_front() {
        done += 1;
        if done % CANCEL_EVERY == 0 && cancelled(cancel) {
            return Err(mismatch(&t, i, "cancelled"));
        }
        let Some(want) = before.entries.get(i) else {
            return Err(mismatch(&t, i, "extra"));
        };
        let (node, stream) = t.node(i).map_err(|what| mismatch(&t, i, what))?;
        if node != want.node {
            return Err(mismatch(&t, i, "node"));
        }
        let ok = match (&want.data, stream) {
            (DataKey::None, None) => true,
            (DataKey::None, Some(_)) | (_, None) => false,
            (
                DataKey::Raw {
                    filter_sig: sig,
                    raw,
                },
                Some(s),
            ) => {
                !unfiltered(s)
                    && hash_bytes(&s.content) == *raw
                    && t.filter_sig(i, s).map_err(|what| mismatch(&t, i, what))? == *sig
            }
            (DataKey::Plain { hash, len }, Some(s))
            | (
                DataKey::Replaced {
                    expected: hash,
                    len,
                },
                Some(s),
            ) => {
                let cap = usize::try_from(*len).unwrap_or(usize::MAX);
                match decode_stream(s, cap, budget) {
                    Ok(data) => hash_bytes(&data) == *hash && data.len() as u64 == *len,
                    Err(DecodeError::TooLarge) if budget.remaining() < cap => {
                        return Err(mismatch(&t, i, "budget"))
                    }
                    Err(_) => false,
                }
            }
        };
        if !ok {
            return Err(mismatch(&t, i, "data"));
        }
    }
    if t.targets.len() < before.entries.len() {
        return Err(GraphMismatch {
            index: t.targets.len(),
            path: String::new(),
            what: "missing",
        });
    }
    Ok(())
}
