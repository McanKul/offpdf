//! Embedded Type1 programs (`FontFile`, SPEC §A.2, §B.9.5): the clear-text built-in `/Encoding`,
//! the eexec layer and a bounded charstring interpreter that proves a glyph draws something.
//!
//! - `Length1` bytes of clear text, then `Length2` bytes of eexec data (PFB segment headers do not
//!   exist inside a PDF `FontFile`). A missing or out-of-range `Length2` is `Err`.
//! - eexec: hex form when the first 4 bytes (after optional whitespace) are hex digits — decoded
//!   whitespace-tolerantly first; then r = 55665, c1 = 52845, c2 = 22719, and the first 4
//!   plaintext bytes are always dropped.
//! - `/Subrs` and `/CharStrings` binary entries follow `RD`/`-|` after one byte; charstrings use
//!   r = 4330 and drop `/lenIV` bytes (default 4; `-1` = not encrypted). Every entry is decrypted
//!   in place once, when the program is parsed (the entries never overlap), so a `callsubr` costs
//!   no decryption; an entry longer than 65,535 bytes (the Type1 charstring limit) never runs.
//! - Interpreter: numbers, `hsbw`, `sbw`, path operators, `closepath`, `callsubr`/`return`
//!   (depth ≤ 10), `seac` (exactly its 5 operands; present when both StandardEncoding components
//!   are), `endchar`, plus the operators that never draw (`hstem`, `vstem`, `hstem3`, `vstem3`,
//!   `dotsection`, `div`, `callothersubr`, `pop`, `setcurrentpoint`) so hinted real fonts can be
//!   proven; ≤ 4,096 operations and ≤ `GLYPH_WORK_MAX` tokens per glyph (its `seac` components
//!   included, and never more than the load's `WorkMeter` can still pay), charged to that meter.
//!   Anything else leaves that glyph unproven (never a font refusal).

use super::encodings::standard_name;
use super::glyph_budget::WorkMeter;
use crate::pdf_engine::text_edit::lexer::{scan_tokens, Operand, ScanMode, Token};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

const EEXEC_R: u16 = 55665;
const CHARSTRING_R: u16 = 4330;
const C1: u16 = 52845;
const C2: u16 = 22719;
const OPS_PER_GLYPH_MAX: usize = 4_096;
const SUBR_DEPTH_MAX: usize = 10;
const STACK_MAX: usize = 48;
const ENTRIES_MAX: usize = 65_536;
/// Tokens scanned from the clear text (a real one has a few hundred; a custom `/Encoding` array
/// needs 4 per code). Each token is ~56 bytes while the encoding is read.
const CLEAR_TOKENS_MAX: usize = 1 << 16;
/// Longest charstring or subroutine the Type1 format allows (Adobe Type 1 Font Format, App. B).
const CHARSTRING_BYTES_MAX: usize = 65_535;

/// The program's own encoding (clear text), used when the PDF gives no `/Encoding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type1Encoding {
    Standard,
    Custom(BTreeMap<u8, String>),
}

/// What the interpreter proved about one glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphProof {
    /// `hsbw`/`sbw`, then at least one drawing operator before `endchar`.
    Drawn,
    /// `hsbw`/`sbw` … `endchar` with nothing drawn (whitespace glyphs), and its advance width.
    Blank {
        width: f64,
    },
    Unproven,
}

pub struct Type1Program {
    pub builtin: Option<Type1Encoding>,
    /// The decrypted private part, with every `/Subrs` and `/CharStrings` entry decrypted in
    /// place; the ranges below are their plaintext (`lenIV` bytes skipped).
    private: Vec<u8>,
    charstrings: BTreeMap<String, Range<usize>>,
    subrs: BTreeMap<usize, Range<usize>>,
}

/// Parses a decoded `FontFile` stream with its `Length1`/`Length2`.
pub fn parse_type1(data: &[u8], length1: usize, length2: usize) -> Result<Type1Program, ()> {
    let end = length1.checked_add(length2).ok_or(())?;
    let clear = data.get(..length1).ok_or(())?;
    let mut encrypted = data.get(length1..end).ok_or(())?;
    if length2 == 0 {
        return Err(());
    }
    // `Length1` that stops right after `eexec` leaves the end-of-line in the encrypted part.
    if clear.ends_with(b"eexec") {
        while let Some((first, rest)) = encrypted.split_first() {
            if !matches!(first, b'\r' | b'\n' | b' ' | b'\t') {
                break;
            }
            encrypted = rest;
        }
    }
    let cipher = if is_hex_form(encrypted) {
        hex_decode(encrypted)
    } else {
        encrypted.to_vec()
    };
    let plain = decrypt(&cipher, EEXEC_R);
    let private = plain.get(4..).ok_or(())?.to_vec();
    let mut program = Type1Program {
        builtin: builtin_encoding(clear),
        private,
        charstrings: BTreeMap::new(),
        subrs: BTreeMap::new(),
    };
    program.index_private()?;
    Ok(program)
}

fn is_hex_form(data: &[u8]) -> bool {
    let start = data
        .iter()
        .position(|b| !matches!(b, b'\r' | b'\n' | b' ' | b'\t'))
        .unwrap_or(data.len());
    data.get(start..start.saturating_add(4))
        .is_some_and(|four| four.len() == 4 && four.iter().all(u8::is_ascii_hexdigit))
}

/// Whitespace-tolerant hex decoding; stops at the first other byte (the `cleartomark` trailer).
fn hex_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for b in data {
        let nibble = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            b'\r' | b'\n' | b' ' | b'\t' | b'\x0c' | 0 => continue,
            _ => break,
        };
        match high.take() {
            Some(h) => out.push((h << 4) | nibble),
            None => high = Some(nibble),
        }
    }
    out
}

/// Type1 decryption (eexec r = 55665, charstrings r = 4330).
fn decrypt(cipher: &[u8], r: u16) -> Vec<u8> {
    let mut plain = cipher.to_vec();
    decrypt_in_place(&mut plain, r);
    plain
}

fn decrypt_in_place(data: &mut [u8], r: u16) {
    let mut r = r;
    for byte in data.iter_mut() {
        let c = *byte;
        *byte = c ^ (r >> 8) as u8;
        r = (u16::from(c).wrapping_add(r))
            .wrapping_mul(C1)
            .wrapping_add(C2);
    }
}

/// `/Encoding StandardEncoding def` or `/Encoding 256 array … dup <code> /<name> put … def`.
fn builtin_encoding(clear: &[u8]) -> Option<Type1Encoding> {
    let tokens = scan_tokens(clear, ScanMode::Type1Clear, CLEAR_TOKENS_MAX).ok()?;
    let start = tokens.iter().position(|t| {
        matches!(t, Token::Operand(Operand::Name { bytes, .. }) if bytes.as_slice() == b"Encoding")
    })?;
    let rest = tokens.get(start + 1..)?;
    if let Some(Token::Keyword { bytes, .. }) = rest.first() {
        if bytes.as_slice() == b"StandardEncoding" {
            return Some(Type1Encoding::Standard);
        }
    }
    let mut map = BTreeMap::new();
    for (i, token) in rest.iter().enumerate() {
        if matches!(token, Token::Keyword { bytes, .. } if bytes.as_slice() == b"def") {
            break;
        }
        let window = (
            rest.get(i),
            rest.get(i + 1),
            rest.get(i + 2),
            rest.get(i + 3),
        );
        if let (
            Some(Token::Keyword { bytes: dup, .. }),
            Some(Token::Operand(Operand::Number { value, .. })),
            Some(Token::Operand(Operand::Name { bytes: name, .. })),
            Some(Token::Keyword { bytes: put, .. }),
        ) = window
        {
            let code =
                (*value >= 0.0 && *value <= 255.0 && value.fract() == 0.0).then_some(*value as u8);
            if let (b"dup", b"put", Some(code), Ok(name)) = (
                dup.as_slice(),
                put.as_slice(),
                code,
                std::str::from_utf8(name),
            ) {
                map.insert(code, name.to_string());
            }
        }
    }
    Some(Type1Encoding::Custom(map))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Name(Vec<u8>),
    Int(i64),
    Other,
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    None,
    Subrs,
    CharStrings,
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl Type1Program {
    /// Indexes `/lenIV`, `/Subrs` and `/CharStrings` of the decrypted private part, then
    /// decrypts every entry in place.
    fn index_private(&mut self) -> Result<(), ()> {
        let p = &self.private;
        let (mut pos, mut section) = (0usize, Section::None);
        let (mut prev2, mut prev1) = (Tok::Other, Tok::Other);
        let mut subrs = BTreeMap::new();
        let mut charstrings = BTreeMap::new();
        let mut len_iv: i64 = 4;
        while let Some(&c) = p.get(pos) {
            if is_ws(c) {
                pos += 1;
                continue;
            }
            let start = pos;
            let tok = match c {
                b'%' => {
                    while p.get(pos).is_some_and(|b| *b != b'\n' && *b != b'\r') {
                        pos += 1;
                    }
                    continue;
                }
                b'/' => {
                    pos += 1;
                    while p.get(pos).is_some_and(|b| !is_ws(*b) && !is_delim(*b)) {
                        pos += 1;
                    }
                    let name = p.get(start + 1..pos).ok_or(())?.to_vec();
                    match name.as_slice() {
                        b"Subrs" => section = Section::Subrs,
                        b"CharStrings" => section = Section::CharStrings,
                        _ => {}
                    }
                    Tok::Name(name)
                }
                b'(' => {
                    pos = skip_literal(p, pos);
                    Tok::Other
                }
                b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b')' => {
                    pos += 1;
                    Tok::Other
                }
                _ => {
                    while p.get(pos).is_some_and(|b| !is_ws(*b) && !is_delim(*b)) {
                        pos += 1;
                    }
                    let word = p.get(start..pos).ok_or(())?;
                    if word == b"RD" || word == b"-|" {
                        let Tok::Int(n) = prev1 else {
                            return Err(());
                        };
                        let len = usize::try_from(n).map_err(|_| ())?;
                        let data_start = pos.checked_add(1).ok_or(())?;
                        let data_end = data_start.checked_add(len).ok_or(())?;
                        if data_end > p.len() {
                            return Err(());
                        }
                        match (section, &prev2) {
                            (Section::Subrs, Tok::Int(idx)) => {
                                let idx = usize::try_from(*idx).map_err(|_| ())?;
                                subrs.insert(idx, data_start..data_end);
                            }
                            (Section::CharStrings, Tok::Name(name)) => {
                                if let Ok(name) = String::from_utf8(name.clone()) {
                                    charstrings.insert(name, data_start..data_end);
                                }
                            }
                            _ => {}
                        }
                        if subrs.len() > ENTRIES_MAX || charstrings.len() > ENTRIES_MAX {
                            return Err(());
                        }
                        pos = data_end;
                        prev2 = Tok::Other;
                        prev1 = Tok::Other;
                        continue;
                    }
                    match std::str::from_utf8(word)
                        .ok()
                        .and_then(|w| w.parse::<i64>().ok())
                    {
                        Some(v) => Tok::Int(v),
                        None => Tok::Other,
                    }
                }
            };
            if let (Tok::Name(n), Tok::Int(v)) = (&prev1, &tok) {
                if n.as_slice() == b"lenIV" {
                    len_iv = *v;
                }
            }
            prev2 = std::mem::replace(&mut prev1, tok);
        }
        // Entries never overlap (each `RD` skips its own bytes), so each is decrypted once.
        // `lenIV` −1 (any negative value) means the charstrings are not encrypted.
        let skip = usize::try_from(len_iv).ok();
        let private = &mut self.private;
        let mut plain = |range: Range<usize>| -> Range<usize> {
            let Some(skip) = skip else {
                return range;
            };
            if let Some(bytes) = private.get_mut(range.clone()) {
                decrypt_in_place(bytes, CHARSTRING_R);
            }
            range.start.saturating_add(skip).min(range.end)..range.end
        };
        self.subrs = subrs.into_iter().map(|(k, r)| (k, plain(r))).collect();
        self.charstrings = charstrings
            .into_iter()
            .map(|(k, r)| (k, plain(r)))
            .collect();
        Ok(())
    }

    /// Whether `/CharStrings` has an entry for `name` (`.notdef` never counts).
    pub fn has_charstring(&self, name: &str) -> bool {
        name != ".notdef" && self.charstrings.contains_key(name)
    }

    /// The plaintext of an entry (`None` past the Type1 charstring length limit).
    fn code(&self, range: &Range<usize>) -> Option<&[u8]> {
        if range.len() > CHARSTRING_BYTES_MAX {
            return None;
        }
        self.private.get(range.clone())
    }

    /// Runs the bounded interpreter on `name`'s charstring; the tokens it reads are charged to
    /// `meter` (a meter that cannot cover them leaves the glyph unproven). The run stops as soon
    /// as it passes what the meter can still pay; an exhausted meter runs nothing.
    pub fn proof(&self, name: &str, meter: &mut WorkMeter) -> GlyphProof {
        if meter.exhausted() {
            return GlyphProof::Unproven;
        }
        let mut tokens = 0u64;
        let proof = self.proof_at(name, true, &mut tokens, meter.glyph_allowance());
        if meter.charge(tokens.max(1)) {
            proof
        } else {
            GlyphProof::Unproven
        }
    }

    /// `limit`: the tokens the glyph may read, its `seac` components included.
    fn proof_at(&self, name: &str, allow_seac: bool, tokens: &mut u64, limit: u64) -> GlyphProof {
        if !self.has_charstring(name) {
            return GlyphProof::Unproven;
        }
        let Some(code) = self.charstrings.get(name).and_then(|r| self.code(r)) else {
            return GlyphProof::Unproven;
        };
        let mut st = Interp {
            tokens: *tokens,
            limit,
            ..Interp::default()
        };
        let flow = self.run(code, &mut st, 0);
        *tokens = st.tokens;
        if !matches!(flow, Ok(Flow::End)) {
            return GlyphProof::Unproven;
        }
        let Some(width) = st.width else {
            return GlyphProof::Unproven;
        };
        if let Some((base, accent)) = st.seac {
            if !allow_seac {
                return GlyphProof::Unproven;
            }
            let mut component = |c: u8| {
                standard_name(c).map_or(GlyphProof::Unproven, |n| {
                    self.proof_at(n, false, tokens, limit)
                })
            };
            return match (component(base), component(accent)) {
                (GlyphProof::Drawn, GlyphProof::Drawn) => GlyphProof::Drawn,
                _ => GlyphProof::Unproven,
            };
        }
        if st.drawn {
            GlyphProof::Drawn
        } else {
            GlyphProof::Blank { width }
        }
    }

    fn run(&self, code: &[u8], st: &mut Interp, depth: usize) -> Result<Flow, ()> {
        let mut pos = 0usize;
        while let Some(&v) = code.get(pos) {
            pos += 1;
            st.tokens = st.tokens.saturating_add(1);
            if st.tokens > st.limit {
                return Err(());
            }
            if v >= 32 {
                let value = match v {
                    32..=246 => i32::from(v) - 139,
                    247..=250 => {
                        let w = i32::from(*code.get(pos).ok_or(())?);
                        pos += 1;
                        (i32::from(v) - 247) * 256 + w + 108
                    }
                    251..=254 => {
                        let w = i32::from(*code.get(pos).ok_or(())?);
                        pos += 1;
                        -(i32::from(v) - 251) * 256 - w - 108
                    }
                    _ => {
                        let b = code.get(pos..pos.checked_add(4).ok_or(())?).ok_or(())?;
                        pos += 4;
                        i32::from_be_bytes([
                            *b.first().ok_or(())?,
                            *b.get(1).ok_or(())?,
                            *b.get(2).ok_or(())?,
                            *b.get(3).ok_or(())?,
                        ])
                    }
                };
                st.push(f64::from(value))?;
                continue;
            }
            st.ops += 1;
            if st.ops > OPS_PER_GLYPH_MAX {
                return Err(());
            }
            let op = if v == 12 {
                let e = *code.get(pos).ok_or(())?;
                pos += 1;
                0x0C00 | u16::from(e)
            } else {
                u16::from(v)
            };
            if st.width.is_none() && op != 13 && op != 0x0C07 {
                return Err(()); // a charstring starts with hsbw or sbw
            }
            match op {
                13 => {
                    st.need(2)?;
                    st.width = st.stack.get(1).copied();
                    st.stack.clear();
                }
                0x0C07 => {
                    st.need(4)?;
                    st.width = st.stack.get(2).copied();
                    st.stack.clear();
                }
                // rlineto, rmoveto, hstem, vstem, setcurrentpoint
                5 | 21 | 1 | 3 | 0x0C21 => st.consume(2)?,
                6 | 7 | 22 | 4 => st.consume(1)?, // hlineto/vlineto/hmoveto/vmoveto
                8 | 0x0C01 | 0x0C02 => st.consume(6)?, // rrcurveto/vstem3/hstem3
                30 | 31 => st.consume(4)?,        // vhcurveto/hvcurveto
                9 | 0x0C00 => st.stack.clear(),   // closepath/dotsection
                10 => {
                    let index = st.pop()?;
                    if depth >= SUBR_DEPTH_MAX || index < 0.0 || index.fract() != 0.0 {
                        return Err(());
                    }
                    let range = self.subrs.get(&(index as usize)).ok_or(())?;
                    let sub = self.code(range).ok_or(())?;
                    if let Flow::End = self.run(sub, st, depth + 1)? {
                        return Ok(Flow::End);
                    }
                }
                11 => return Ok(Flow::Return),
                14 => return Ok(Flow::End),
                0x0C06 => {
                    // asb adx ady bchar achar seac: exactly five operands.
                    if st.stack.len() != 5 {
                        return Err(());
                    }
                    let code_of = |x: Option<&f64>| -> Result<u8, ()> {
                        let x = *x.ok_or(())?;
                        (x >= 0.0 && x <= 255.0 && x.fract() == 0.0)
                            .then_some(x as u8)
                            .ok_or(())
                    };
                    st.seac = Some((code_of(st.stack.get(3))?, code_of(st.stack.get(4))?));
                    return Ok(Flow::End);
                }
                0x0C0C => {
                    let b = st.pop()?;
                    let a = st.pop()?;
                    if b == 0.0 {
                        return Err(());
                    }
                    st.push(a / b)?;
                }
                0x0C10 => {
                    let _othersubr = st.pop()?;
                    let n = st.pop()?;
                    if n < 0.0 || n.fract() != 0.0 || n as usize > st.stack.len() {
                        return Err(());
                    }
                    let keep = st.stack.len() - n as usize;
                    let args = st.stack.split_off(keep);
                    st.ps.extend(args);
                    if st.ps.len() > STACK_MAX {
                        return Err(());
                    }
                }
                0x0C11 => {
                    let v = st.ps.pop().ok_or(())?;
                    st.push(v)?;
                }
                _ => return Err(()),
            }
            if matches!(op, 5 | 6 | 7 | 8 | 30 | 31) {
                st.drawn = true;
            }
        }
        Err(()) // ran off the end without endchar/return
    }
}

enum Flow {
    Return,
    End,
}

#[derive(Default)]
struct Interp {
    stack: Vec<f64>,
    ps: Vec<f64>,
    ops: usize,
    /// Tokens read for the glyph so far (its `seac` components included), and how many it may.
    tokens: u64,
    limit: u64,
    width: Option<f64>,
    drawn: bool,
    seac: Option<(u8, u8)>,
}

impl Interp {
    fn push(&mut self, v: f64) -> Result<(), ()> {
        if self.stack.len() >= STACK_MAX {
            return Err(());
        }
        self.stack.push(v);
        Ok(())
    }

    fn pop(&mut self) -> Result<f64, ()> {
        self.stack.pop().ok_or(())
    }

    fn need(&self, n: usize) -> Result<(), ()> {
        if self.stack.len() < n {
            return Err(());
        }
        Ok(())
    }

    fn consume(&mut self, n: usize) -> Result<(), ()> {
        self.need(n)?;
        self.stack.clear();
        Ok(())
    }
}

/// Skips a balanced literal string starting at `start` (`(`); returns the offset after it.
fn skip_literal(p: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut pos = start;
    while let Some(&b) = p.get(pos) {
        pos += 1;
        match b {
            b'\\' => pos += 1,
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return pos;
                }
            }
            _ => {}
        }
    }
    pos
}

/// The names of a FontDescriptor `/CharSet` string (`(/a/b/c)`).
pub fn charset_names(charset: &[u8]) -> HashSet<String> {
    charset
        .split(|b| *b == b'/')
        .map(|n| String::from_utf8_lossy(n).trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}
