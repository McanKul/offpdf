//! Font identity across files (SPEC §B.9.3, D32): `content_hash`, an FNV-1a over a canonical
//! serialisation of the resolved font dictionary tree. Object ids never enter it, so the same font
//! hashes alike in the source, the edited copy, the preview and the final file even though qpdf
//! renumbers every object.
//!
//! Streams the font parsers read (font programs, ToUnicode, CIDToGIDMap) are decoded once per
//! load and shared with the parsers. Every other stream (Type3 `/CharProcs`, `/Resources`
//! XObjects, embedded CMaps) is decoded only for the hash, and at most
//! `HASH_OTHER_DECODED_MAX` bytes of them per font: past that, such a stream hashes as its
//! dictionary and a fixed marker (the same in every copy of the file, since decoded sizes do not
//! change), so a large Type3 font costs the page budget a bounded amount.

use super::{number_of, parsed_role, role_cap, Loader};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeError};
use crate::pdf_engine::text_edit::snapshot::{fnv1a_extend, fnv1a_u64};
use lopdf::{Dictionary, Object, ObjectId, Stream};

/// Nested references followed by `content_hash` (§B.9.3).
const HASH_REF_DEPTH_MAX: usize = 16;
/// Values visited by one `content_hash` (bounds DAG blow-up; the marker keeps it deterministic).
const HASH_STEPS_MAX: usize = 1_000_000;
/// Nested values (direct and through references) one `content_hash` descends into: the
/// recursion depth, so the stack stays bounded whatever the dictionary tree looks like.
const HASH_NESTING_MAX: usize = 256;
/// Decoded bytes of streams no font parser reads that one `content_hash` may hash.
const HASH_OTHER_DECODED_MAX: usize = 4 << 20;

struct Hasher {
    state: u64,
    steps: usize,
    depth: usize,
    /// Decode cap for the next referenced stream: that of the dictionary key leading to it.
    cap: usize,
    /// Whether that key names a stream a font parser reads (decoded once, shared).
    parsed: bool,
    /// What is left of `HASH_OTHER_DECODED_MAX`.
    other_left: usize,
}

impl Hasher {
    fn feed(&mut self, bytes: &[u8]) {
        self.state = fnv1a_extend(self.state, bytes);
    }

    fn feed_len(&mut self, tag: u8, len: usize) {
        self.feed(&[tag]);
        self.feed(&(len as u64).to_le_bytes());
    }
}

/// FNV-1a over a canonical serialisation of the resolved dict tree: keys sorted, numbers as
/// f64, string format ignored, references resolved (depth ≤ 16, cycles marked), streams by their
/// dictionary (minus `/Length`, `/Filter`, `/DecodeParms`, `/DL`) and decoded bytes — or raw
/// bytes plus filter when not decodable. Object ids never enter the hash (D32).
pub(super) fn content_hash<'a>(loader: &mut Loader<'a, '_>, dict: &'a Dictionary) -> u64 {
    let mut h = Hasher {
        state: fnv1a_u64(b"offpdf-font-v1"),
        steps: 0,
        depth: 0,
        cap: role_cap(b""),
        parsed: false,
        other_left: HASH_OTHER_DECODED_MAX,
    };
    let mut path = Vec::new();
    hash_dict(loader, dict, &mut h, &mut path, false);
    h.state
}

fn hash_dict<'a>(
    loader: &mut Loader<'a, '_>,
    dict: &'a Dictionary,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
    stream_dict: bool,
) {
    let mut entries: Vec<(&'a Vec<u8>, &'a Object)> = dict
        .iter()
        .filter(|(k, _)| {
            !stream_dict || !matches!(k.as_slice(), b"Length" | b"Filter" | b"DecodeParms" | b"DL")
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    h.feed_len(b'd', entries.len());
    for (k, v) in entries {
        h.feed_len(b'k', k.len());
        h.feed(k);
        h.cap = role_cap(k);
        h.parsed = parsed_role(k);
        hash_obj(loader, v, h, path);
    }
}

fn hash_obj<'a>(
    loader: &mut Loader<'a, '_>,
    obj: &'a Object,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
) {
    h.steps += 1;
    if h.steps > HASH_STEPS_MAX || h.depth >= HASH_NESTING_MAX {
        h.feed(b"X");
        return;
    }
    h.depth += 1;
    hash_value(loader, obj, h, path);
    h.depth -= 1;
}

fn hash_value<'a>(
    loader: &mut Loader<'a, '_>,
    obj: &'a Object,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
) {
    match obj {
        Object::Null => h.feed(b"z"),
        Object::Boolean(b) => h.feed(if *b { b"b1" } else { b"b0" }),
        Object::Integer(_) | Object::Real(_) => {
            h.feed(b"f");
            let v = number_of(obj).unwrap_or(f64::NAN);
            h.feed(&v.to_bits().to_le_bytes());
        }
        Object::Name(n) => {
            h.feed_len(b'n', n.len());
            h.feed(n);
        }
        Object::String(s, _) => {
            h.feed_len(b's', s.len());
            h.feed(s);
        }
        Object::Array(items) => {
            h.feed_len(b'a', items.len());
            for item in items {
                hash_obj(loader, item, h, path);
            }
        }
        Object::Dictionary(d) => hash_dict(loader, d, h, path, false),
        Object::Stream(s) => {
            // Streams are only reachable through references; a bare one hashes as its raw form.
            hash_dict(loader, &s.dict, h, path, true);
            hash_raw_stream(loader, s, h, path);
        }
        Object::Reference(id) => hash_reference(loader, *id, h, path),
    }
}

fn hash_reference(
    loader: &mut Loader<'_, '_>,
    id: ObjectId,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
) {
    if path.len() >= HASH_REF_DEPTH_MAX {
        h.feed(b"D");
        return;
    }
    if path.contains(&id) {
        h.feed(b"C");
        return;
    }
    let Some((target_id, target)) = loader.resolve_id(id) else {
        h.feed(b"U");
        return;
    };
    path.push(id);
    let (cap, parsed) = (h.cap, h.parsed);
    match target {
        Object::Stream(s) if parsed => {
            hash_dict(loader, &s.dict, h, path, true);
            match loader.stream_bytes(target_id, s, cap) {
                Ok(bytes) => {
                    h.feed_len(b'B', bytes.len());
                    h.feed(&bytes);
                }
                Err(_) => hash_raw_stream(loader, s, h, path),
            }
        }
        Object::Stream(s) => {
            hash_dict(loader, &s.dict, h, path, true);
            hash_other_stream(loader, s, h, path);
        }
        other => hash_obj(loader, other, h, path),
    }
    path.pop();
}

/// A stream only the hash reads: decoded (not kept) while `HASH_OTHER_DECODED_MAX` lasts, else
/// the marker `L`; raw bytes plus filter when it cannot be decoded at all.
fn hash_other_stream<'a>(
    loader: &mut Loader<'a, '_>,
    s: &'a Stream,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
) {
    if h.other_left == 0 {
        h.feed(b"L");
        return;
    }
    let before = loader.budget.remaining();
    match decode_stream(s, h.other_left, loader.budget) {
        Ok(bytes) => {
            h.other_left = h.other_left.saturating_sub(bytes.len());
            h.feed_len(b'B', bytes.len());
            h.feed(&bytes);
        }
        Err(DecodeError::TooLarge) => {
            if before < h.other_left {
                loader.budget_hit = true; // the page budget ran out, not this hash's share
            }
            h.other_left = 0;
            h.feed(b"L");
        }
        Err(_) => hash_raw_stream(loader, s, h, path),
    }
}

fn hash_raw_stream<'a>(
    loader: &mut Loader<'a, '_>,
    s: &'a Stream,
    h: &mut Hasher,
    path: &mut Vec<ObjectId>,
) {
    h.feed_len(b'R', s.content.len());
    h.feed(&s.content);
    for key in [&b"Filter"[..], b"DecodeParms"] {
        if let Ok(v) = s.dict.get(key) {
            h.feed(key);
            hash_obj(loader, v, h, path);
        }
    }
}
