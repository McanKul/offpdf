//! Inline images (SPEC §A.4, decision D13): the `BI … ID … EI` dictionary and the proven end
//! of the payload (`/L`, unfiltered size, Flate end, ASCII terminators), or the pdf.js-style
//! heuristic that marks every following op `after_unproven_inline_image`.

use super::{
    is_delimiter, is_whitespace, lex_inner, malformed, values::Builder, InlineImage, InlineProof,
    LexError, LexLimits, Operand, Raw, Reader,
};
use crate::pdf_engine::text_edit::decode::{self, DecodeError};
use crate::pdf_engine::text_edit::limits;

/// Reads an inline image after `BI` (dict, `ID`, payload, `EI`) and proves the payload end.
pub(super) fn inline_image(
    r: &mut Reader<'_>,
    lim: &LexLimits,
    bi_end: usize,
) -> Result<InlineImage, LexError> {
    let mut dict: Vec<(Vec<u8>, Operand)> = Vec::new();
    loop {
        let raw = r.next()?.ok_or(LexError::Malformed {
            at: bi_end,
            what: "unterminated inline image",
        })?;
        let key = match raw {
            Raw::Word(span) if r.b.get(span.clone()) == Some(&b"ID"[..]) => break,
            Raw::Operand(Operand::Name { bytes, .. }) => bytes,
            other => return malformed(other.start(), "inline image dictionary"),
        };
        let first = r.next()?.ok_or(LexError::Malformed {
            at: bi_end,
            what: "unterminated inline image",
        })?;
        let value = read_value(r, first, lim)?;
        if dict.len() >= lim.array_items_max {
            return Err(LexError::TooComplex {
                what: "inline image dictionary",
            });
        }
        dict.push((key, value));
    }
    let id_end = r.pos;
    if !r.at(id_end).is_some_and(is_whitespace) {
        return malformed(id_end, "no whitespace after ID");
    }
    // The separator is one whitespace byte; CR LF counts as two only when the one-byte reading
    // cannot be proven and the two-byte one can.
    let one = id_end + 1;
    let mut found = prove_end(r, &dict, one, lim)?.map(|f| (one, f));
    if found.is_none() && r.at(id_end) == Some(b'\r') && r.at(one) == Some(b'\n') {
        found = prove_end(r, &dict, one + 1, lim)?.map(|f| (one + 1, f));
    }
    let (data_start, data_end, ei, proof) = match found {
        Some((start, (end, ei, proof))) => (start, end, ei, proof),
        None => {
            let (end, ei) = heuristic_end(r.b, one, lim)?;
            (one, end, ei, InlineProof::Heuristic)
        }
    };
    r.pos = ei + 2;
    dict.shrink_to_fit();
    Ok(InlineImage {
        dict,
        data: data_start..data_end,
        proof,
    })
}

/// The ends of the inline images of one buffer, read in order with the lexer's own proofs (the
/// content-part boundary scan, `content::joins`). One reader serves the whole buffer, so the
/// ASCII-terminator searches are remembered across images as in a lex.
pub(crate) struct InlineEnds<'a> {
    r: Reader<'a>,
    lim: LexLimits,
}

impl<'a> InlineEnds<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        let lim = LexLimits::page();
        InlineEnds {
            r: Reader::new(b, true, false, &lim),
            lim,
        }
    }

    /// The offset just past the `EI` of the inline image whose `BI` ends at `bi_end`. `None` when
    /// the image does not end in the buffer, is malformed or over a limit, or when the bytes a
    /// reader looks at after the `EI` reach the end of the buffer, whatever proof found it
    /// (review-verify MEDIUM-A): pdf.js 4.10 takes an `EI` only when the 15 bytes after it are
    /// ASCII and lex to a known operator (`findDefaultInlineStreamEnd`, its default for every
    /// filter but DCT and the ASCII ones), and our heuristic looks `INLINE_HEURISTIC_WINDOW`
    /// (≥ 15) bytes ahead, so bytes after the buffer — the next part, shifted by qpdf's `\n` —
    /// could change which `EI` ends the image.
    pub(crate) fn end(&mut self, bi_end: usize) -> Option<usize> {
        self.r.pos = bi_end;
        self.r.nodes = 0;
        inline_image(&mut self.r, &self.lim, bi_end).ok()?;
        let end = self.r.pos;
        let window_end = end.saturating_add(limits::INLINE_HEURISTIC_WINDOW);
        (window_end < self.r.b.len()).then_some(end)
    }
}

fn read_value(r: &mut Reader<'_>, first: Raw, lim: &LexLimits) -> Result<Operand, LexError> {
    let mut builder = Builder::new(lim);
    let mut raw = first;
    loop {
        if let Raw::Word(span) = &raw {
            return malformed(span.start, "inline image value");
        }
        if let Some(v) = builder.feed(raw)? {
            return Ok(v);
        }
        raw = r.next()?.ok_or(LexError::Malformed {
            at: r.pos,
            what: "unterminated inline image",
        })?;
    }
}

fn dict_get<'d>(dict: &'d [(Vec<u8>, Operand)], keys: &[&[u8]]) -> Option<&'d Operand> {
    dict.iter()
        .find(|(k, _)| keys.contains(&k.as_slice()))
        .map(|(_, v)| v)
}

fn dict_uint(dict: &[(Vec<u8>, Operand)], keys: &[&[u8]]) -> Option<usize> {
    let v = dict_get(dict, keys)?.as_number()?;
    (v >= 0.0 && v.fract() == 0.0 && v <= limits::NUMBER_ABS_MAX).then_some(v as usize)
}

/// Filters of an inline image, abbreviations kept as written.
fn image_filters(dict: &[(Vec<u8>, Operand)]) -> Option<Vec<Vec<u8>>> {
    match dict_get(dict, &[b"F", b"Filter"]) {
        None => Some(Vec::new()),
        Some(Operand::Name { bytes, .. }) => Some(vec![bytes.clone()]),
        Some(Operand::Array { items, .. }) => items
            .iter()
            .map(|i| i.as_name().map(<[u8]>::to_vec))
            .collect(),
        Some(_) => None,
    }
}

/// `EI` after optional whitespace from `pos`, followed by whitespace, a delimiter or EOF.
fn ei_after(b: &[u8], pos: usize) -> Option<usize> {
    let mut q = pos;
    while b.get(q).is_some_and(|c| is_whitespace(*c)) {
        q += 1;
    }
    let ok = b.get(q..q.checked_add(2)?) == Some(&b"EI"[..])
        && b.get(q + 2)
            .is_none_or(|c| is_whitespace(*c) || is_delimiter(*c));
    ok.then_some(q)
}

/// Tries the four proofs of §A.4 in order; `Ok(None)` when none holds.
fn prove_end(
    r: &mut Reader<'_>,
    dict: &[(Vec<u8>, Operand)],
    start: usize,
    lim: &LexLimits,
) -> Result<Option<(usize, usize, InlineProof)>, LexError> {
    let b = r.b;
    let too_big = LexError::TooComplex {
        what: "inline image size",
    };
    let check = |end: usize| -> Option<(usize, usize)> { ei_after(b, end).map(|ei| (end, ei)) };
    if let Some(len) = dict_uint(dict, &[b"L", b"Length"]) {
        if len > lim.inline_image_max {
            return Err(too_big);
        }
        if let Some((end, ei)) = start.checked_add(len).and_then(check) {
            return Ok(Some((end, ei, InlineProof::LengthKey)));
        }
    }
    let Some(filters) = image_filters(dict) else {
        return Ok(None);
    };
    if filters.is_empty() {
        if let Some(size) = unfiltered_size(dict) {
            if size > lim.inline_image_max as u64 {
                return Err(too_big);
            }
            let end = start.checked_add(size as usize).and_then(check);
            if let Some((end, ei)) = end {
                return Ok(Some((end, ei, InlineProof::UnfilteredSize)));
            }
        }
        return Ok(None);
    }
    let payload = b.get(start..).unwrap_or_default();
    match filters.first().map(Vec::as_slice) {
        Some(b"Fl" | b"FlateDecode") if filters.len() == 1 => {
            match decode::inflate_end(payload, lim.inline_image_max) {
                Ok(consumed) => {
                    let end = start.checked_add(consumed).ok_or(LexError::TooComplex {
                        what: "inline image size",
                    })?;
                    for candidate in [end.checked_add(4), Some(end)].into_iter().flatten() {
                        if candidate <= b.len() {
                            if let Some((e, ei)) = check(candidate) {
                                return Ok(Some((e, ei, InlineProof::FlateEnd)));
                            }
                        }
                    }
                    Ok(None)
                }
                Err(DecodeError::TooLarge) => Err(too_big),
                Err(_) => Ok(None),
            }
        }
        Some(b"AHx" | b"ASCIIHexDecode") => Ok(find_terminator(r, HEX_END, start, lim)
            .and_then(check)
            .map(|(e, ei)| (e, ei, InlineProof::AsciiEnd))),
        Some(b"A85" | b"ASCII85Decode") => Ok(find_terminator(r, A85_END, start, lim)
            .and_then(check)
            .map(|(e, ei)| (e, ei, InlineProof::AsciiEnd))),
        _ => Ok(None),
    }
}

/// The ASCIIHex and ASCII85 end markers, with their slot in `Reader::terminators`.
const HEX_END: (usize, &[u8]) = (0, b">");
const A85_END: (usize, &[u8]) = (1, b"~>");

/// The end of an ASCII payload starting at `start`: just after the first terminator, when that
/// keeps the payload within `inline_image_max` (a longer one is no proof, as for the other
/// proofs). Each search is remembered per reader and reused while it still answers (inline images
/// are read in order), so every byte of a content is searched at most once per terminator however
/// many images lack one: one search per image over the rest of the content was quadratic
/// (review T3-budget MEDIUM-4, 8,000 images took 19 s).
fn find_terminator(
    r: &mut Reader<'_>,
    (slot, needle): (usize, &[u8]),
    start: usize,
    lim: &LexLimits,
) -> Option<usize> {
    let remembered = r.terminators.get(slot).copied().flatten();
    let hit = match remembered {
        Some((from, None)) if from <= start => None,
        Some((from, Some(at))) if from <= start && at >= start => Some(at),
        _ => {
            let at =
                r.b.get(start..)
                    .and_then(|rest| find(rest, needle))
                    .map(|p| start + p);
            if let Some(memo) = r.terminators.get_mut(slot) {
                *memo = Some((start, at));
            }
            at
        }
    };
    let end = hit?.checked_add(needle.len())?;
    (end.saturating_sub(start) <= lim.inline_image_max).then_some(end)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `ceil(W·BPC·ncomp/8)·H` for an unfiltered image whose colour space is known.
fn unfiltered_size(dict: &[(Vec<u8>, Operand)]) -> Option<u64> {
    let w = dict_uint(dict, &[b"W", b"Width"])? as u64;
    let h = dict_uint(dict, &[b"H", b"Height"])? as u64;
    let mask = matches!(
        dict_get(dict, &[b"IM", b"ImageMask"]),
        Some(Operand::Bool { value: true, .. })
    );
    let (bpc, ncomp) = if mask {
        (1u64, 1u64)
    } else {
        let bpc = dict_uint(dict, &[b"BPC", b"BitsPerComponent"])? as u64;
        if !matches!(bpc, 1 | 2 | 4 | 8 | 16) {
            return None;
        }
        let ncomp = match dict_get(dict, &[b"CS", b"ColorSpace"])? {
            Operand::Name { bytes, .. } => match bytes.as_slice() {
                b"G" | b"DeviceGray" | b"I" | b"Indexed" => 1,
                b"RGB" | b"DeviceRGB" => 3,
                b"CMYK" | b"DeviceCMYK" => 4,
                _ => return None,
            },
            Operand::Array { items, .. } => match items.first().and_then(Operand::as_name) {
                Some(b"I" | b"Indexed") => 1,
                _ => return None,
            },
            _ => return None,
        };
        (bpc, ncomp)
    };
    let row_bits = w.checked_mul(bpc)?.checked_mul(ncomp)?;
    row_bits.checked_add(7).map(|x| x / 8)?.checked_mul(h)
}

/// pdf.js-style fallback: the first `EI` preceded by whitespace and followed by whitespace/EOF
/// after which the next `INLINE_HEURISTIC_WINDOW` bytes lex as valid operators. At most
/// `INLINE_HEURISTIC_CANDIDATES_MAX` such candidates are lexed (each lex reads a window).
fn heuristic_end(b: &[u8], start: usize, lim: &LexLimits) -> Result<(usize, usize), LexError> {
    let limit = start.saturating_add(lim.inline_image_max);
    let mut q = start;
    let mut candidates = 0usize;
    while let Some(rel) = b.get(q..).and_then(|rest| find(rest, b"EI")) {
        let p = q + rel;
        if p > limit {
            return Err(LexError::TooComplex {
                what: "inline image size",
            });
        }
        let before = p
            .checked_sub(1)
            .and_then(|i| b.get(i))
            .is_some_and(|c| is_whitespace(*c));
        let after = b.get(p + 2).is_none_or(|c| is_whitespace(*c));
        if before && after {
            if candidates >= limits::INLINE_HEURISTIC_CANDIDATES_MAX {
                return Err(LexError::TooComplex {
                    what: "inline image end candidates",
                });
            }
            candidates += 1;
            if tail_lexes(b, p + 2, lim) {
                return Ok((p.saturating_sub(1).max(start), p));
            }
        }
        q = p + 1;
    }
    malformed(start, "inline image without an end")
}

/// The bytes after a candidate `EI` must lex as operators with valid arity. A window cut
/// before EOF is shortened to its last whitespace and must then contain at least one op.
fn tail_lexes(b: &[u8], from: usize, lim: &LexLimits) -> bool {
    let end = from
        .saturating_add(limits::INLINE_HEURISTIC_WINDOW)
        .min(b.len());
    let Some(window) = b.get(from..end) else {
        return true;
    };
    let truncated = end < b.len();
    let window = match window.iter().rposition(|c| is_whitespace(*c)) {
        Some(cut) if truncated => window.get(..cut).unwrap_or_default(),
        _ => window,
    };
    match lex_inner(window, lim, None, true) {
        Ok(ops) => !truncated || !ops.is_empty(),
        Err(_) => false,
    }
}
