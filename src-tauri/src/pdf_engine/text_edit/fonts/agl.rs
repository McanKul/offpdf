//! Adobe Glyph List lookup (SPEC §A.3.1 item 2, §B.9.2): glyph name → one Unicode scalar.
//!
//! Order: the full AGL table (`agl_data.rs`, 4,495 names, exact match), then `uniXXXX` (exactly
//! one group of four hex digits), then `uXXXX`–`uXXXXXX`. Everything else — suffixed names
//! (`a.sc`), multi-group `uni` names (`uni00410042`), surrogate values — has no Unicode value.

use super::agl_data::AGL;

/// The AGL value of `name` (exact, byte-wise match), if the table lists it.
pub fn agl_value(name: &str) -> Option<u32> {
    AGL.binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes()))
        .ok()
        .and_then(|i| AGL.get(i))
        .map(|(_, v)| *v)
}

/// The Unicode scalar a glyph name stands for (AGL, then `uniXXXX`, then `uXXXX[XX]`).
pub fn glyph_name_char(name: &str) -> Option<char> {
    if let Some(v) = agl_value(name) {
        return char::from_u32(v);
    }
    if let Some(hex) = name.strip_prefix("uni") {
        // `uni` names carry exactly one 4-digit group; longer ones name sequences (ligatures).
        return if hex.len() == 4 {
            hex_scalar(hex)
        } else {
            None
        };
    }
    match name.strip_prefix('u') {
        Some(hex) if (4..=6).contains(&hex.len()) => hex_scalar(hex),
        _ => None,
    }
}

/// `hex` as a Unicode scalar; `None` for non-hex digits, surrogates and values past U+10FFFF.
fn hex_scalar(hex: &str) -> Option<char> {
    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

/// Number of names in the table (FONT-01 checks it against lopdf's 4,495).
#[cfg(test)]
pub fn agl_len() -> usize {
    AGL.len()
}

/// Whether the table is strictly sorted by name bytes (binary search precondition; FONT-01).
#[cfg(test)]
pub fn agl_is_sorted() -> bool {
    AGL.windows(2).all(|w| match w {
        [(a, _), (b, _)] => a.as_bytes() < b.as_bytes(),
        _ => true,
    })
}

/// Every AGL name of `ch`, shortest first (test fixtures name their glyphs with it).
#[cfg(test)]
pub fn names_for(ch: char) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = AGL
        .iter()
        .filter(|(_, v)| *v == u32::from(ch))
        .map(|(n, _)| *n)
        .collect();
    names.sort_by_key(|n| (n.len(), *n));
    names
}
