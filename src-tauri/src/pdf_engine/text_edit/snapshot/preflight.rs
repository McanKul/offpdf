//! Raw preflight (SPEC §B.4): bounds what lopdf 0.34's parser does before it runs. lopdf
//! allocates from xref-stream fields without checks and recurses once per array/dictionary level
//! and once per stream `/Length` reference it resolves, on rayon worker stacks (2 MiB), so every
//! input that would make it allocate or recurse without bound is refused here:
//! - the cross-reference chain (≤ `MAX_XREF_CHAIN` hops incl. `/XRefStm`), each xref stream's
//!   decoded size, `/W`, `/Index` and PNG-predictor row (`/DecodeParms`);
//! - every object lopdf could parse, and every `/Length` chain (`objects.rs`).
//!
//! The chain must be the one lopdf reads (review-T1 round 2, HIGH-3): a dictionary key that
//! occurs twice counts with its **last** value (lopdf's `Dictionary::set`), a table's trailer is
//! the one lopdf's grammar reaches (comments skipped, not the first `trailer` bytes), and
//! `/XRefStm` is followed from xref-stream trailers too.

use super::objects;
use crate::error::AppError;
use crate::pdf_engine::text_edit::decode::{self, DecodeError};
use crate::pdf_engine::text_edit::lexer::{self, Operand, Token};
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons;

/// Runs every preflight check on the bytes of one snapshot (also driven by the preflight fuzz).
pub(crate) fn preflight(bytes: &[u8]) -> Result<(), AppError> {
    check_xref_chain(bytes)?;
    objects::check_objects(bytes)
}

pub(super) fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut i = from;
    while let Some(rel) = hay.get(i..)?.iter().position(|c| *c == first) {
        let p = i + rel;
        if hay.get(p..p.checked_add(needle.len())?) == Some(needle) {
            return Some(p);
        }
        i = p + 1;
    }
    None
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(|c| lexer::is_whitespace(*c)) {
        i += 1;
    }
    i
}

fn parse_uint(b: &[u8], i: usize) -> Option<(u64, usize)> {
    let digits = b
        .get(i..)?
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digits == 0 || digits > 19 {
        return None;
    }
    let text = std::str::from_utf8(b.get(i..i + digits)?).ok()?;
    Some((text.parse().ok()?, i + digits))
}

fn find_startxref(b: &[u8]) -> Option<usize> {
    let tail_start = b.len().saturating_sub(limits::XREF_TAIL_SEARCH_BYTES);
    let tail = b.get(tail_start..)?;
    let rel = tail.windows(9).rposition(|w| w == b"startxref")?;
    let (off, _) = parse_uint(b, skip_ws(b, tail_start + rel + 9))?;
    usize::try_from(off).ok().filter(|o| *o < b.len())
}

/// Minimal value tree of a scanned dictionary (only what the preflight reads). `Num` is a number
/// written as an integer (lopdf's `Integer`), `Real` one written with a `.`.
#[derive(Debug)]
enum PVal {
    Num(f64),
    Real(f64),
    Name(Vec<u8>),
    Array(Vec<PVal>),
    Dict(Vec<(Vec<u8>, PVal)>),
    Other,
}

fn tokens_to_value(b: &[u8], tokens: Vec<Token>) -> Option<PVal> {
    let mut stack: Vec<(bool, Vec<PVal>)> = Vec::new();
    let mut done = None;
    for t in tokens {
        let v = match t {
            Token::DictOpen(_) => {
                stack.push((true, Vec::new()));
                continue;
            }
            Token::ArrayOpen(_) => {
                stack.push((false, Vec::new()));
                continue;
            }
            Token::DictClose(_) | Token::ArrayClose(_) => {
                let (is_dict, items) = stack.pop()?;
                if !is_dict {
                    PVal::Array(items)
                } else {
                    let mut entries = Vec::new();
                    let mut it = items.into_iter();
                    while let Some(k) = it.next() {
                        let PVal::Name(k) = k else { return None };
                        entries.push((k, it.next()?));
                    }
                    PVal::Dict(entries)
                }
            }
            Token::Keyword { bytes, .. } if bytes == b"R" => {
                let items = &mut stack.last_mut()?.1;
                let (Some(PVal::Num(_)), Some(PVal::Num(_))) = (items.pop(), items.pop()) else {
                    return None;
                };
                PVal::Other
            }
            Token::Keyword { .. } => PVal::Other,
            Token::Operand(Operand::Number { value, span }) => {
                if b.get(span).is_some_and(|text| text.contains(&b'.')) {
                    PVal::Real(value)
                } else {
                    PVal::Num(value)
                }
            }
            Token::Operand(Operand::Name { bytes, .. }) => PVal::Name(bytes),
            Token::Operand(_) => PVal::Other,
        };
        match stack.last_mut() {
            Some((_, items)) => items.push(v),
            None => done = Some(v),
        }
    }
    done
}

/// The value lopdf keeps for `key`: the last occurrence (`Dictionary::set` replaces).
fn dict_get<'a>(d: &'a [(Vec<u8>, PVal)], key: &[u8]) -> Option<&'a PVal> {
    d.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn dict_uint(d: &[(Vec<u8>, PVal)], key: &[u8]) -> Option<usize> {
    match dict_get(d, key)? {
        PVal::Num(v) if *v >= 0.0 && v.fract() == 0.0 && *v < 1e15 => Some(*v as usize),
        _ => None,
    }
}

fn scan_dict(b: &[u8], at: usize) -> Result<(Vec<(Vec<u8>, PVal)>, usize), AppError> {
    let (tokens, end) =
        lexer::scan_dict_at(b, at, limits::PREFLIGHT_DICT_TOKENS_MAX).map_err(|e| match e {
            lexer::LexError::TooComplex { what } => reasons::file_too_complex(what),
            other => reasons::invalid_xref(&other.to_string()),
        })?;
    match tokens_to_value(b, tokens) {
        Some(PVal::Dict(d)) => Ok((d, end)),
        _ => Err(reasons::invalid_xref(&format!(
            "unreadable dictionary at byte {at}"
        ))),
    }
}

fn check_xref_chain(b: &[u8]) -> Result<(), AppError> {
    let start = find_startxref(b).ok_or_else(|| reasons::invalid_xref("no startxref"))?;
    let mut queue = vec![start];
    let mut visited = std::collections::HashSet::new();
    while let Some(off) = queue.pop() {
        if !visited.insert(off) {
            continue;
        }
        if visited.len() > limits::MAX_XREF_CHAIN {
            return Err(reasons::file_too_complex("cross-reference chain too long"));
        }
        let p = skip_ws(b, off);
        let dict = if b.get(p..p + 4) == Some(&b"xref"[..]) {
            let t = table_trailer(b, p + 4).ok_or_else(|| reasons::invalid_xref("no trailer"))?;
            scan_dict(b, t + 7)?.0
        } else {
            check_xref_stream(b, p)?
        };
        // lopdf reads `/XRefStm` from its newest trailer of either kind; every trailer counts here.
        if let Some(stm) = dict_uint(&dict, b"XRefStm") {
            queue.push(stm);
        }
        if dict_get(&dict, b"Prev").is_some() {
            let prev =
                dict_uint(&dict, b"Prev").ok_or_else(|| reasons::invalid_xref("bad /Prev"))?;
            queue.push(prev);
        }
    }
    Ok(())
}

/// Where lopdf's `xref` parser (sections of digits, spaces, `n`/`f` and EOLs) and the `space`
/// after it (whitespace and `%` comments) leave off: the `trailer` keyword it parses next. Every
/// byte lopdf consumes there is skipped here with the same comment boundaries, so when lopdf
/// reads a trailer it is this one; a `trailer` inside a comment is never taken.
fn table_trailer(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    loop {
        match *b.get(i)? {
            c if c.is_ascii_digit() || c == b'n' || c == b'f' || lexer::is_whitespace(c) => i += 1,
            b'%' => {
                i += b
                    .get(i..)?
                    .iter()
                    .take_while(|c| !matches!(c, b'\r' | b'\n'))
                    .count();
            }
            _ => break,
        }
    }
    b.get(i..)?.starts_with(b"trailer").then_some(i)
}

/// `N G obj << /Type /XRef … >> stream … endstream`: bounds the decoded size, `/W` and `/Index`.
fn check_xref_stream(b: &[u8], p: usize) -> Result<Vec<(Vec<u8>, PVal)>, AppError> {
    let bad = |what: &str| reasons::invalid_xref(&format!("{what} at byte {p}"));
    let (_, i) = parse_uint(b, p).ok_or_else(|| bad("no xref"))?;
    let (_, i) = parse_uint(b, skip_ws(b, i)).ok_or_else(|| bad("no xref"))?;
    let i = skip_ws(b, i);
    if b.get(i..i + 3) != Some(&b"obj"[..]) {
        return Err(bad("no xref"));
    }
    let (dict, end) = scan_dict(b, i + 3)?;
    if !matches!(dict_get(&dict, b"Type"), Some(PVal::Name(n)) if n == b"XRef") {
        return Err(bad("not an xref stream"));
    }
    let s = skip_ws(b, end);
    if b.get(s..s + 6) != Some(&b"stream"[..]) {
        return Err(bad("xref stream without data"));
    }
    let mut data_start = s + 6;
    if b.get(data_start) == Some(&b'\r') {
        data_start += 1;
    }
    if b.get(data_start) == Some(&b'\n') {
        data_start += 1;
    }
    let by_length = dict_uint(&dict, b"Length")
        .and_then(|l| data_start.checked_add(l))
        .filter(|e| {
            b.get(skip_ws(b, *e)..)
                .is_some_and(|rest| rest.starts_with(b"endstream"))
        });
    let data_end = match by_length {
        Some(e) => e,
        None => {
            find_from(b, data_start, b"endstream").ok_or_else(|| bad("unterminated xref stream"))?
        }
    };
    let payload = b.get(data_start..data_end).unwrap_or_default();
    decode_xref_payload(&dict, payload)?;
    check_xref_fields(&dict, p)?;
    Ok(dict)
}

fn decode_xref_payload(dict: &[(Vec<u8>, PVal)], payload: &[u8]) -> Result<(), AppError> {
    let names: Vec<&[u8]> = match dict_get(dict, b"Filter") {
        None => Vec::new(),
        Some(PVal::Name(n)) => vec![n.as_slice()],
        Some(PVal::Array(items)) if items.len() <= 4 => items
            .iter()
            .map(|i| match i {
                PVal::Name(n) => Ok(n.as_slice()),
                _ => Err(reasons::file_too_complex("xref stream filter")),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(reasons::file_too_complex("xref stream filter")),
    };
    let cap = limits::XREF_STREAM_MAX_DECODED;
    let mut data = std::borrow::Cow::Borrowed(payload);
    for name in names {
        let out = match name {
            b"FlateDecode" | b"Fl" => decode::inflate_capped(&data, cap),
            b"ASCIIHexDecode" | b"AHx" => decode::ascii_hex_decode(&data, cap),
            b"ASCII85Decode" | b"A85" => decode::ascii85_decode(&data, cap),
            _ => return Err(reasons::file_too_complex("xref stream filter")),
        };
        data = std::borrow::Cow::Owned(out.map_err(|e| match e {
            DecodeError::TooLarge => reasons::file_too_complex("xref stream too large"),
            other => reasons::invalid_xref(&format!("xref stream data: {other}")),
        })?);
    }
    if data.len() > cap {
        return Err(reasons::file_too_complex("xref stream too large"));
    }
    Ok(())
}

/// `/W`: three integers 0..=8 with at least one byte per row; `/Index` (or `[0 /Size]`):
/// non-negative pairs whose counts sum (checked) to at most MAX_OBJECTS (lopdf loops over every
/// count); a PNG predictor row of at most `XREF_PREDICTOR_ROW_MAX` bytes (lopdf allocates it).
/// Malformed fields → `INVALID_PDF`; over a budget → `FILE_TOO_COMPLEX`. Array items must be
/// written as integers: lopdf rejects a `/W` with a real and replaces such an `/Index` by
/// `[0 /Size]`, which this check would not have bounded.
fn check_xref_fields(dict: &[(Vec<u8>, PVal)], p: usize) -> Result<(), AppError> {
    let bad = |what: &str| reasons::invalid_xref(&format!("{what} at byte {p}"));
    let too_large = |what: &str| reasons::file_too_complex(&format!("{what} at byte {p}"));
    let ints = |v: Option<&PVal>| -> Option<Vec<i64>> {
        match v? {
            PVal::Array(items) => items
                .iter()
                .map(|i| match i {
                    PVal::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => Some(*n as i64),
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    };
    let w = ints(dict_get(dict, b"W")).ok_or_else(|| bad("xref stream /W"))?;
    if w.len() != 3
        || w.iter()
            .any(|x| !(0..=limits::XREF_STREAM_FIELD_WIDTH_MAX).contains(x))
    {
        return Err(bad("xref stream /W"));
    }
    let row: i64 = w.iter().sum();
    if row == 0 {
        return Err(bad("xref stream /W"));
    }
    let index = match dict_get(dict, b"Index") {
        Some(v) => ints(Some(v)).ok_or_else(|| bad("xref stream /Index"))?,
        None => vec![
            0,
            dict_uint(dict, b"Size").ok_or_else(|| bad("xref stream /Size"))? as i64,
        ],
    };
    if index.len() % 2 != 0 || index.iter().any(|x| *x < 0) {
        return Err(bad("xref stream /Index"));
    }
    let count = index
        .iter()
        .skip(1)
        .step_by(2)
        .try_fold(0i64, |sum, n| sum.checked_add(*n));
    if !count.is_some_and(|c| c <= limits::MAX_OBJECTS as i64) {
        return Err(too_large("xref stream lists too many objects"));
    }
    if !predictor_row_ok(dict) {
        return Err(too_large("xref stream predictor row too large"));
    }
    Ok(())
}

/// lopdf 0.34 (`object.rs` `decompress_predictor`): with a direct `/DecodeParms` dictionary whose
/// `/Predictor` is 10..=15 it allocates a row of `max(1, Columns) × max(1, Colors) ×
/// max(8, BitsPerComponent) / 8` bytes (and multiplies them unchecked). Non-integer values fall
/// back to lopdf's defaults there; here every number counts, so the bound is never lower.
fn predictor_row_ok(dict: &[(Vec<u8>, PVal)]) -> bool {
    let Some(PVal::Dict(parms)) = dict_get(dict, b"DecodeParms") else {
        return true;
    };
    let num = |key: &[u8], default: f64| match dict_get(parms, key) {
        Some(PVal::Num(n) | PVal::Real(n)) => *n,
        _ => default,
    };
    if !(10.0..=15.0).contains(&num(b"Predictor", 1.0)) {
        return true;
    }
    let row = num(b"Columns", 1.0).max(1.0)
        * num(b"Colors", 1.0).max(1.0)
        * num(b"BitsPerComponent", 8.0).max(8.0)
        / 8.0;
    row.is_finite() && row <= limits::XREF_PREDICTOR_ROW_MAX as f64
}
