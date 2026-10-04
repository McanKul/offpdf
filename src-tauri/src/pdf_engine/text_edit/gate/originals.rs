//! Finding an original edited part again (A3, B2): as a whole part or Form XObject (its digest),
//! or as one segment of a Form's data the way qpdf joins page parts into its overlay wrapper —
//! starting at offset 0 or right after a `\n`, and ending at the end of the data or at a `\n`.
//! The search is one rolling-hash pass per distinct part length, so a wrapper that keeps the old
//! part next to other content (a fake wrapped by a later overlay) is found without quadratic
//! work; every rolling-hash hit is confirmed by the part's FNV digest.

use crate::pdf_engine::validate_output::{content_digest, ContentDigest};

/// Polynomial base of the rolling hash (odd, so powers never collapse to 0 mod 2^64).
const BASE: u64 = 0x0000_0100_0000_01b3;
/// Rolling-hash hits at aligned positions whose digest differs, per search, before giving up
/// ("too large to verify": the caller fails closed).
const FALSE_HITS_MAX: usize = 64;

/// An original edited part: its digest (hash and length) and its rolling hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalPart {
    pub digest: ContentDigest,
    pub rolling: u64,
}

impl OriginalPart {
    pub fn of(bytes: &[u8]) -> OriginalPart {
        OriginalPart {
            digest: content_digest(bytes),
            rolling: rolling(bytes),
        }
    }
}

/// The rolling hash of `bytes` (Σ b·BASE^(n−1−i), wrapping).
pub(crate) fn rolling(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |h, b| {
        h.wrapping_mul(BASE).wrapping_add(u64::from(*b))
    })
}

fn power(mut exp: usize) -> u64 {
    let (mut base, mut out) = (BASE, 1u64);
    while exp > 0 {
        if exp & 1 == 1 {
            out = out.wrapping_mul(base);
        }
        base = base.wrapping_mul(base);
        exp >>= 1;
    }
    out
}

/// `[start, start + len)` is a qpdf-join segment of `data`.
fn aligned(data: &[u8], start: usize, len: usize) -> bool {
    let start_ok = start == 0 || start.checked_sub(1).and_then(|i| data.get(i)) == Some(&b'\n');
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let end_ok = end == data.len()
        || data.get(end) == Some(&b'\n')
        || end.checked_sub(1).and_then(|i| data.get(i)) == Some(&b'\n');
    start_ok && end_ok
}

/// The start offsets of the first `max` occurrences of `needle` in `data` (overlapping ones
/// included): one rolling-hash pass, every hash hit confirmed byte for byte, so the cost is
/// O(|data| + hits × |needle|) whatever the bytes are. `Err` after more than `FALSE_HITS_MAX`
/// hash hits that are not the needle (fail closed).
pub(crate) fn find_up_to(
    data: &[u8],
    needle: &[u8],
    max: usize,
) -> Result<Vec<usize>, &'static str> {
    let len = needle.len();
    let mut hits = Vec::new();
    if len == 0 || len > data.len() || max == 0 {
        return Ok(hits);
    }
    let target = rolling(needle);
    let top = power(len.saturating_sub(1));
    let mut h = rolling(data.get(..len).unwrap_or_default());
    let mut start = 0usize;
    let mut false_hits = 0usize;
    loop {
        if h == target {
            if data.get(start..start.saturating_add(len)) == Some(needle) {
                hits.push(start);
                if hits.len() >= max {
                    return Ok(hits);
                }
            } else {
                false_hits = false_hits.saturating_add(1);
                if false_hits > FALSE_HITS_MAX {
                    return Err("too many near matches of the expected content");
                }
            }
        }
        let end = start.saturating_add(len);
        let (Some(out), Some(next)) = (data.get(start), data.get(end)) else {
            return Ok(hits);
        };
        h = h
            .wrapping_sub(top.wrapping_mul(u64::from(*out)))
            .wrapping_mul(BASE)
            .wrapping_add(u64::from(*next));
        start = start.saturating_add(1);
    }
}

/// Whether `data` is, or holds as a qpdf-join segment, one of `originals`. `Err` when the search
/// had to give up (fail closed).
pub(crate) fn holds_original(
    data: &[u8],
    originals: &[OriginalPart],
) -> Result<bool, &'static str> {
    let mut lens: Vec<usize> = originals
        .iter()
        .map(|o| o.digest.len)
        .filter(|l| *l > 0 && *l <= data.len())
        .collect();
    lens.sort_unstable();
    lens.dedup();
    let mut false_hits = 0usize;
    for len in lens {
        let targets: Vec<&OriginalPart> =
            originals.iter().filter(|o| o.digest.len == len).collect();
        let top = power(len.saturating_sub(1));
        let mut h = rolling(data.get(..len).unwrap_or_default());
        let mut start = 0usize;
        loop {
            if targets.iter().any(|t| t.rolling == h) && aligned(data, start, len) {
                let window = data
                    .get(start..start.saturating_add(len))
                    .unwrap_or_default();
                let d = content_digest(window);
                if targets.iter().any(|t| t.digest == d) {
                    return Ok(true);
                }
                false_hits = false_hits.saturating_add(1);
                if false_hits > FALSE_HITS_MAX {
                    return Err("too many near matches of the original content");
                }
            }
            let end = start.saturating_add(len);
            let (Some(out), Some(next)) = (data.get(start), data.get(end)) else {
                break;
            };
            h = h
                .wrapping_sub(top.wrapping_mul(u64::from(*out)))
                .wrapping_mul(BASE)
                .wrapping_add(u64::from(*next));
            start = start.saturating_add(1);
        }
    }
    Ok(false)
}
