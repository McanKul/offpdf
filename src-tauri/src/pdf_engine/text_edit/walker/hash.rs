//! Deep, object-id-free hashes of XObjects, colour spaces, shadings and ExtGState values (SPEC
//! §B.10 `PaintKind`, D32): dictionaries with sorted keys, references followed (cycles marked by
//! their position on the path, dangling references hashed as `null` like qpdf's
//! `fixDanglingReferences`), streams by their decoded bytes under the walk's hash budget, or by
//! raw bytes + `/Filter` + `/DecodeParms` when they cannot (or no longer can) be decoded.
//!
//! The decode/raw decision depends only on the decoded size and the budget left, which are the
//! same in the source and in qpdf's output (where unfiltered streams come back Flate-compressed),
//! so equal content hashes equal on both sides. `/Parent`, `/PieceInfo` and `/Metadata` are
//! skipped (they never change what is painted and can reach the whole document).
//!
//! Bounded per walk, whatever the page repeats: every hash is memoised (indirect objects by id,
//! direct values by address), all hashes of a walk share one step count (`HASH_STEPS_MAX`), the
//! decode budget is charged for a failed decode as if it ran to its cap, and raw bytes are
//! charged too (`Walker::hash_raw_left`). Exhaustion refuses the page `PAGE_TOO_COMPLEX`. Each
//! memo entry is charged to the page-model budget (`budget::map_entry`); a stream decoded for its
//! hash is not (one at a time, ≤ `STREAM_MAX_DECODED`, freed at once), so the decode/raw decision
//! never depends on what else the page holds.

use super::budget::map_entry;
use super::{Stop, Walker};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget, DecodeError};
use crate::pdf_engine::text_edit::limits::{PAGE_DECODE_BUDGET, STREAM_MAX_DECODED};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::snapshot::fnv1a_extend;
use lopdf::{Dictionary, Object, ObjectId, Stream};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// Objects visited by all the deep hashes of one walk.
const HASH_STEPS_MAX: usize = 1_000_000;
/// Nesting depth of one hash (references included).
const HASH_DEPTH_MAX: usize = 64;
const SKIPPED_KEYS: [&[u8]; 3] = [b"Parent", b"PieceInfo", b"Metadata"];
const STREAM_KEYS: [&[u8]; 4] = [b"Length", b"Filter", b"DecodeParms", b"DL"];

struct Hasher<'w, 'a> {
    w: &'w mut Walker<'a>,
    path: Vec<ObjectId>,
}

fn budget_exhausted() -> Stop {
    Stop::new(TextReason::PageTooComplex, "object hash budget")
}

impl<'a> Walker<'a> {
    /// Deep hash of a resource value reached through the reference `id` (its object), or of the
    /// direct value `obj` itself; memoised per walk either way, so an op that repeats (`sh`,
    /// `gs`, `cs`, `scn`) pays for its resource once.
    pub(crate) fn entry_hash(
        &mut self,
        id: Option<ObjectId>,
        obj: &'a Object,
    ) -> Result<u64, Stop> {
        if let Some(id) = id {
            return self.stream_hash(id);
        }
        let address = obj as *const Object as usize;
        if let Some(h) = self.direct_hashes.get(&address) {
            return Ok(*h);
        }
        let mut h = Hasher {
            w: self,
            path: Vec::new(),
        };
        let mut out = FNV_OFFSET;
        h.object(obj, 0, &mut out)?;
        self.mem.scratch(map_entry::<usize, u64>())?;
        self.direct_hashes.insert(address, out);
        Ok(out)
    }

    /// Deep hash of the object `id` (memoised per walk).
    pub(crate) fn stream_hash(&mut self, id: ObjectId) -> Result<u64, Stop> {
        if let Some(h) = self.hashes.get(&id) {
            return Ok(*h);
        }
        let doc = self.doc;
        let h = match doc.objects.get(&id) {
            Some(obj) => {
                let mut hasher = Hasher {
                    w: self,
                    path: vec![id],
                };
                let mut out = FNV_OFFSET;
                hasher.object(obj, 0, &mut out)?;
                out
            }
            None => fnv1a_extend(FNV_OFFSET, b"n"),
        };
        self.mem.scratch(map_entry::<ObjectId, u64>())?;
        self.hashes.insert(id, h);
        Ok(h)
    }
}

impl Walker<'_> {
    /// Runs `f` with deep-hash state of its own (fresh budgets, step count and memos) and then
    /// restores the walk's: a check outside the page content (the wrapper Form's `/Group` in
    /// `Wrapped` mode) leaves the content's hashes exactly as a walk without that check computes
    /// them, so a large `/Group` cannot push a content image onto its raw-bytes hash.
    pub(crate) fn with_own_hash_state<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, Stop>,
    ) -> Result<T, Stop> {
        let fresh_raw = self.ctx.snap.file_len();
        let saved = (
            std::mem::replace(&mut self.hash_budget, DecodeBudget::new(PAGE_DECODE_BUDGET)),
            std::mem::take(&mut self.hash_steps),
            std::mem::replace(&mut self.hash_raw_left, fresh_raw),
            std::mem::take(&mut self.hashes),
            std::mem::take(&mut self.direct_hashes),
        );
        let out = f(self);
        (
            self.hash_budget,
            self.hash_steps,
            self.hash_raw_left,
            self.hashes,
            self.direct_hashes,
        ) = saved;
        out
    }
}

fn feed(out: &mut u64, bytes: &[u8]) {
    *out = fnv1a_extend(*out, bytes);
}

fn feed_len(out: &mut u64, tag: &[u8], len: usize) {
    feed(out, tag);
    feed(out, &(len as u64).to_le_bytes());
}

impl<'a> Hasher<'_, 'a> {
    fn object(&mut self, obj: &'a Object, depth: usize, out: &mut u64) -> Result<(), Stop> {
        self.w.hash_steps = self.w.hash_steps.saturating_add(1);
        if self.w.hash_steps > steps_max() || depth > HASH_DEPTH_MAX {
            return Err(budget_exhausted());
        }
        match obj {
            Object::Null => feed(out, b"n"),
            Object::Boolean(b) => feed(out, if *b { b"bt" } else { b"bf" }),
            Object::Integer(i) => {
                feed(out, b"i");
                feed(out, &i.to_le_bytes());
            }
            Object::Real(r) => {
                feed(out, b"r");
                feed(out, &r.to_bits().to_le_bytes());
            }
            Object::Name(n) => {
                feed_len(out, b"N", n.len());
                feed(out, n);
            }
            Object::String(s, _) => {
                feed_len(out, b"S", s.len());
                feed(out, s);
            }
            Object::Array(items) => {
                feed_len(out, b"A", items.len());
                for item in items {
                    self.object(item, depth + 1, out)?;
                }
            }
            Object::Dictionary(d) => self.dict(d, &[], depth, out)?,
            Object::Stream(s) => self.stream(s, depth, out)?,
            Object::Reference(id) => self.reference(*id, depth, out)?,
        }
        Ok(())
    }

    fn reference(&mut self, id: ObjectId, depth: usize, out: &mut u64) -> Result<(), Stop> {
        if let Some(pos) = self.path.iter().position(|p| *p == id) {
            feed_len(out, b"C", pos);
            return Ok(());
        }
        if let Some(h) = self.w.hashes.get(&id) {
            feed(out, b"H");
            feed(out, &h.to_le_bytes());
            return Ok(());
        }
        let doc = self.w.doc;
        let Some(target) = doc.objects.get(&id) else {
            feed(out, b"n");
            return Ok(());
        };
        self.path.push(id);
        let mut sub = FNV_OFFSET;
        self.object(target, depth + 1, &mut sub)?;
        self.path.pop();
        self.w.mem.scratch(map_entry::<ObjectId, u64>())?;
        self.w.hashes.insert(id, sub);
        feed(out, b"H");
        feed(out, &sub.to_le_bytes());
        Ok(())
    }

    fn dict(
        &mut self,
        d: &'a Dictionary,
        also_skip: &[&[u8]],
        depth: usize,
        out: &mut u64,
    ) -> Result<(), Stop> {
        let mut entries: Vec<(&'a Vec<u8>, &'a Object)> = d
            .iter()
            .filter(|(k, _)| {
                !SKIPPED_KEYS.contains(&k.as_slice()) && !also_skip.contains(&k.as_slice())
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        feed_len(out, b"D", entries.len());
        for (k, v) in entries {
            feed_len(out, b"K", k.len());
            feed(out, k);
            self.object(v, depth + 1, out)?;
        }
        Ok(())
    }

    fn stream(&mut self, s: &'a Stream, depth: usize, out: &mut u64) -> Result<(), Stop> {
        feed(out, b"T");
        self.dict(&s.dict, &STREAM_KEYS, depth, out)?;
        let budget = &mut self.w.hash_budget;
        let cap = STREAM_MAX_DECODED.min(budget.remaining());
        let decoded: Option<std::borrow::Cow<'a, [u8]>> = if s.dict.has(b"Filter") {
            #[cfg(test)]
            tests::note_decode_attempt(cap);
            match decode_stream(s, cap, budget) {
                Ok(data) => Some(std::borrow::Cow::Owned(data)),
                // Rejected before any byte was decoded.
                Err(DecodeError::UnsupportedFilter(_)) => None,
                // It may have produced up to `cap` bytes before failing: charge them all, so a
                // stream that inflates past the cap is not inflated again for free.
                Err(DecodeError::TooLarge | DecodeError::Corrupt(_)) => {
                    *budget = DecodeBudget::new(budget.remaining().saturating_sub(cap));
                    None
                }
            }
        } else if s.content.len() <= cap && budget.take(s.content.len()).is_ok() {
            Some(std::borrow::Cow::Borrowed(s.content.as_slice()))
        } else {
            None
        };
        match decoded {
            Some(data) => {
                feed_len(out, b"P", data.len());
                feed(out, &data);
            }
            None => {
                self.w.hash_raw_left = self
                    .w
                    .hash_raw_left
                    .checked_sub(s.content.len())
                    .ok_or_else(budget_exhausted)?;
                feed(out, b"R");
                for key in [&b"Filter"[..], b"DecodeParms"] {
                    match s.dict.get(key).ok() {
                        Some(v) => self.object(v, depth + 1, out)?,
                        None => feed(out, b"-"),
                    }
                }
                feed_len(out, b"B", s.content.len());
                feed(out, &s.content);
            }
        }
        Ok(())
    }
}

/// `HASH_STEPS_MAX`, or the test override of the calling thread.
fn steps_max() -> usize {
    #[cfg(test)]
    if let Some(n) = tests::STEPS_OVERRIDE.with(|c| c.get()) {
        return n;
    }
    HASH_STEPS_MAX
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;

    thread_local! {
        pub(super) static STEPS_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
        static DECODE_ATTEMPTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    }

    /// Test seam: override `HASH_STEPS_MAX` on the calling thread (`None` restores it).
    pub(crate) fn set_steps_override(n: Option<usize>) {
        STEPS_OVERRIDE.with(|c| c.set(n));
    }

    pub(super) fn note_decode_attempt(cap: usize) {
        DECODE_ATTEMPTS.with(|c| {
            let (n, bytes) = c.get();
            c.set((n.saturating_add(1), bytes.saturating_add(cap)))
        });
    }

    /// `(attempts, Σ caps)`: the filtered streams the deep hashes tried to decode on the calling
    /// thread since the last call, and the most output those attempts were allowed to produce.
    pub(crate) fn take_decode_attempts() -> (usize, usize) {
        DECODE_ATTEMPTS.with(|c| c.replace((0, 0)))
    }
}
