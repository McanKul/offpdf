//! Bounded stream decoding (SPEC §B.5): Flate (no predictor), ASCIIHex, ASCII85 and chains of
//! up to four of them. Every decoder is capped while it runs; there is no path that returns the
//! raw (still encoded) bytes of a filtered stream.

use crate::pdf_engine::text_edit::limits::INFLATE_STEP_BYTES;
use crate::pdf_engine::text_edit::reasons::TextReason;
use flate2::{Decompress, FlushDecompress, Status};
use lopdf::{Dictionary, Object, Stream};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    UnsupportedFilter(String),
    Corrupt(&'static str),
    TooLarge,
}

impl DecodeError {
    /// UnsupportedFilter → UNSUPPORTED_FILTER, Corrupt → MALFORMED_CONTENT, TooLarge → PAGE_TOO_COMPLEX.
    pub fn page_reason(&self) -> TextReason {
        match self {
            DecodeError::UnsupportedFilter(_) => TextReason::UnsupportedFilter,
            DecodeError::Corrupt(_) => TextReason::MalformedContent,
            DecodeError::TooLarge => TextReason::PageTooComplex,
        }
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::UnsupportedFilter(name) => write!(f, "unsupported filter: {name}"),
            DecodeError::Corrupt(what) => write!(f, "corrupt stream data: {what}"),
            DecodeError::TooLarge => write!(f, "decoded data too large"),
        }
    }
}

/// A decode budget shared by several streams (e.g. one page walk).
#[derive(Debug, Clone)]
pub struct DecodeBudget {
    remaining: usize,
}

impl DecodeBudget {
    pub fn new(total: usize) -> Self {
        DecodeBudget { remaining: total }
    }

    pub fn remaining(&self) -> usize {
        self.remaining
    }

    /// Debits `n` bytes; `TooLarge` when the budget cannot cover them (nothing is debited then).
    pub fn take(&mut self, n: usize) -> Result<(), DecodeError> {
        self.remaining = self.remaining.checked_sub(n).ok_or(DecodeError::TooLarge)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Flate,
    AsciiHex,
    Ascii85,
}

/// Decodes `stream` with at most `cap` output bytes (and at most `budget.remaining()`), then
/// debits the budget with the decoded length.
pub fn decode_stream(
    stream: &Stream,
    cap: usize,
    budget: &mut DecodeBudget,
) -> Result<Vec<u8>, DecodeError> {
    let filters = filters_of(&stream.dict)?;
    let cap = cap.min(budget.remaining());
    let mut data: Cow<'_, [u8]> = Cow::Borrowed(stream.content.as_slice());
    for filter in filters {
        let decoded = match filter {
            Filter::Flate => inflate_capped(&data, cap)?,
            Filter::AsciiHex => ascii_hex_decode(&data, cap)?,
            Filter::Ascii85 => ascii85_decode(&data, cap)?,
        };
        data = Cow::Owned(decoded);
    }
    if data.len() > cap {
        return Err(DecodeError::TooLarge);
    }
    budget.take(data.len())?;
    Ok(data.into_owned())
}

fn filters_of(dict: &Dictionary) -> Result<Vec<Filter>, DecodeError> {
    for key in [&b"F"[..], b"FFilter", b"FDecodeParms"] {
        if dict.has(key) {
            return Err(DecodeError::UnsupportedFilter("external file (/F)".into()));
        }
    }
    let names: Vec<&[u8]> = match dict.get(b"Filter").ok() {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Name(n)) => vec![n.as_slice()],
        Some(Object::Array(items)) => {
            if items.len() > 4 {
                return Err(DecodeError::UnsupportedFilter("more than 4 filters".into()));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Object::Name(n) => out.push(n.as_slice()),
                    _ => return Err(DecodeError::UnsupportedFilter("malformed /Filter".into())),
                }
            }
            out
        }
        Some(_) => return Err(DecodeError::UnsupportedFilter("malformed /Filter".into())),
    };
    check_decode_parms(dict.get(b"DecodeParms").ok())?;
    names.into_iter().map(filter_from_name).collect()
}

fn filter_from_name(name: &[u8]) -> Result<Filter, DecodeError> {
    match name {
        b"FlateDecode" | b"Fl" => Ok(Filter::Flate),
        b"ASCIIHexDecode" | b"AHx" => Ok(Filter::AsciiHex),
        b"ASCII85Decode" | b"A85" => Ok(Filter::Ascii85),
        other => Err(DecodeError::UnsupportedFilter(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

/// `/DecodeParms` may only carry parameters that leave the data unchanged (`/Predictor` 1 or absent).
fn check_decode_parms(parms: Option<&Object>) -> Result<(), DecodeError> {
    let check_dict = |d: &Dictionary| -> Result<(), DecodeError> {
        match d.get(b"Predictor").ok() {
            None | Some(Object::Integer(1)) => Ok(()),
            Some(_) => Err(DecodeError::UnsupportedFilter("predictor".into())),
        }
    };
    match parms {
        None | Some(Object::Null) => Ok(()),
        Some(Object::Dictionary(d)) => check_dict(d),
        Some(Object::Array(items)) => {
            for item in items {
                match item {
                    Object::Null => {}
                    Object::Dictionary(d) => check_dict(d)?,
                    _ => {
                        return Err(DecodeError::UnsupportedFilter(
                            "malformed /DecodeParms".into(),
                        ))
                    }
                }
            }
            Ok(())
        }
        Some(_) => Err(DecodeError::UnsupportedFilter(
            "malformed /DecodeParms".into(),
        )),
    }
}

/// Splits off and checks the 2-byte zlib header (CM = 8, CINFO ≤ 7, FCHECK, FDICT = 0).
fn zlib_body(data: &[u8]) -> Result<&[u8], DecodeError> {
    let (cmf, flg) = match (data.first(), data.get(1)) {
        (Some(c), Some(f)) => (*c, *f),
        _ => return Err(DecodeError::Corrupt("truncated flate")),
    };
    let check = (u16::from(cmf) * 256 + u16::from(flg)) % 31;
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || check != 0 || flg & 0x20 != 0 {
        return Err(DecodeError::Corrupt("bad zlib header"));
    }
    Ok(data.get(2..).unwrap_or_default())
}

/// Raw inflate of `body` in fixed output steps; every produced chunk goes to `sink` only after
/// the running total was checked against `cap`. Returns (total output, consumed input) at
/// `StreamEnd`; input ending before it is `Corrupt("truncated flate")`.
fn inflate_run(
    body: &[u8],
    cap: usize,
    mut sink: impl FnMut(&[u8]),
) -> Result<(usize, usize), DecodeError> {
    let mut d = Decompress::new(false);
    let mut step = vec![0u8; INFLATE_STEP_BYTES];
    loop {
        let in_before = usize::try_from(d.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let out_before = usize::try_from(d.total_out()).map_err(|_| DecodeError::TooLarge)?;
        let input = body.get(in_before..).unwrap_or_default();
        let status = d
            .decompress(input, &mut step, FlushDecompress::None)
            .map_err(|_| DecodeError::Corrupt("flate data error"))?;
        let in_after = usize::try_from(d.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let out_after = usize::try_from(d.total_out()).map_err(|_| DecodeError::TooLarge)?;
        let produced = out_after
            .checked_sub(out_before)
            .ok_or(DecodeError::Corrupt("flate state"))?;
        if out_after > cap {
            return Err(DecodeError::TooLarge);
        }
        if produced > 0 {
            sink(step.get(..produced).unwrap_or_default());
        }
        match status {
            Status::StreamEnd => return Ok((out_after, in_after)),
            Status::Ok | Status::BufError => {
                if produced == 0 && in_after == in_before {
                    return Err(DecodeError::Corrupt("truncated flate"));
                }
            }
        }
    }
}

/// Zlib-wrapped Flate with the output capped at `cap` while inflating. Peak allocation is at
/// most `cap` + one 64 KiB step: a first pass only measures, the second fills an exact buffer.
/// The Adler-32 trailer and any trailing bytes are ignored (as pdf.js and qpdf do).
pub fn inflate_capped(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let body = zlib_body(data)?;
    let (len, _) = inflate_run(body, cap, |_| {})?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| DecodeError::TooLarge)?;
    inflate_run(body, len, |chunk| out.extend_from_slice(chunk))?;
    Ok(out)
}

/// Number of input bytes (zlib header included, Adler-32 trailer excluded) a capped inflate
/// consumes before `StreamEnd`. Used to prove the end of a Flate inline image.
pub fn inflate_end(data: &[u8], cap: usize) -> Result<usize, DecodeError> {
    let body = zlib_body(data)?;
    let (_, consumed) = inflate_run(body, cap, |_| {})?;
    consumed.checked_add(2).ok_or(DecodeError::TooLarge)
}

fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// ASCIIHexDecode: whitespace ignored, `>` ends the data, an odd final digit is padded with 0.
pub fn ascii_hex_decode(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::new();
    out.try_reserve_exact(
        (data.len() / 2)
            .saturating_add(1)
            .min(cap.saturating_add(1)),
    )
    .map_err(|_| DecodeError::TooLarge)?;
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        if is_pdf_whitespace(b) {
            continue;
        }
        let v = hex_value(b).ok_or(DecodeError::Corrupt("bad hex digit"))?;
        match high.take() {
            None => high = Some(v),
            Some(h) => {
                if out.len() >= cap {
                    return Err(DecodeError::TooLarge);
                }
                out.push((h << 4) | v);
            }
        }
    }
    if let Some(h) = high {
        if out.len() >= cap {
            return Err(DecodeError::TooLarge);
        }
        out.push(h << 4);
    }
    Ok(out)
}

/// ASCII85Decode: optional leading `<~`, whitespace ignored, `z` for four zero bytes, `~>` ends.
pub fn ascii85_decode(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    let mut out = Vec::new();
    out.try_reserve_exact((body.len() / 5 * 4 + 4).min(cap.saturating_add(4)))
        .map_err(|_| DecodeError::TooLarge)?;
    let mut group = [0u8; 5];
    let mut count = 0usize;
    let mut iter = body.iter().copied().peekable();
    let push = |out: &mut Vec<u8>, bytes: &[u8]| -> Result<(), DecodeError> {
        if out.len().saturating_add(bytes.len()) > cap {
            return Err(DecodeError::TooLarge);
        }
        out.extend_from_slice(bytes);
        Ok(())
    };
    while let Some(b) = iter.next() {
        match b {
            _ if is_pdf_whitespace(b) => {}
            b'~' => {
                if iter.peek() == Some(&b'>') {
                    break;
                }
                return Err(DecodeError::Corrupt("bad ASCII85 end"));
            }
            b'z' if count == 0 => push(&mut out, &[0, 0, 0, 0])?,
            b'!'..=b'u' => {
                if let Some(slot) = group.get_mut(count) {
                    *slot = b - b'!';
                }
                count += 1;
                if count == 5 {
                    push(&mut out, &a85_group(&group)?)?;
                    count = 0;
                }
            }
            _ => return Err(DecodeError::Corrupt("bad ASCII85 character")),
        }
    }
    match count {
        0 => {}
        1 => return Err(DecodeError::Corrupt("bad ASCII85 final group")),
        n => {
            for slot in group.iter_mut().skip(n) {
                *slot = 84;
            }
            let bytes = a85_group(&group)?;
            push(&mut out, bytes.get(..n - 1).unwrap_or_default())?;
        }
    }
    Ok(out)
}

fn a85_group(digits: &[u8; 5]) -> Result<[u8; 4], DecodeError> {
    let value = digits.iter().fold(0u64, |acc, d| acc * 85 + u64::from(*d));
    let value = u32::try_from(value).map_err(|_| DecodeError::Corrupt("ASCII85 group overflow"))?;
    Ok(value.to_be_bytes())
}
