//! Bounded ToUnicode CMap parser (SPEC §B.9.4), on top of `lexer::scan_tokens` in CMap mode.
//!
//! Sections: `begincodespacerange` (1–4-byte ranges), `beginbfchar`, `beginbfrange` (incrementing
//! form: the destination's last UTF-16 unit is incremented and the range stops on overflow past
//! 0xFFFF; array form). Destinations are UTF-16BE (surrogate pairs allowed; a lone surrogate, an
//! odd or empty destination maps the code to "undecodable"), at most 512 bytes. At most
//! `CMAP_MAPPINGS_MAX` mappings, whose destinations total at most `TOUNICODE_MAX_DECODED` bytes
//! (checked before a range is expanded, so a few bytes of CMap cannot expand into megabytes of
//! text). `usecmap` marks every code this CMap does not map itself as undecodable. Any syntax
//! error is `Err(())`.
//!
//! Codes are keyed by their numeric value, whatever the length of the source string, as pdf.js
//! (`CMap.mapOne`) and Poppler (`CharCodeToUnicode::parseCMap1`) key them: `<0041>` maps code
//! 0x41 of a simple font, `<41>` code 0x0041 of a Type0 font. A later mapping of a code wins.

use super::Code;
use crate::pdf_engine::text_edit::lexer::{scan_tokens, Operand, ScanMode, Token};
use crate::pdf_engine::text_edit::limits::{CMAP_MAPPINGS_MAX, TOUNICODE_MAX_DECODED};
use std::collections::BTreeMap;

/// Destinations longer than this are a syntax error.
const DESTINATION_MAX_BYTES: usize = 512;
/// Tokens scanned from one ToUnicode stream (each ~56 bytes while it is parsed). Real CMaps need
/// 2 (`bfchar`), 3 (`bfrange`) or 5 (a one-destination `bfrange` array) per mapping, so within
/// `TOUNICODE_MAX_DECODED` bytes the densest real form (23-byte array lines) stays under 460,000.
const TOKENS_MAX: usize = 4 * CMAP_MAPPINGS_MAX;

/// What a ToUnicode CMap says about one code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup<'a> {
    /// The CMap does not map the code (and has no `usecmap`).
    Absent,
    /// Mapped, but to nothing usable (lone surrogate, odd/empty destination), or possibly mapped
    /// by a `usecmap` parent this parser does not load.
    Undecodable,
    Text(&'a str),
}

#[derive(Debug, Clone, Default)]
pub struct ToUnicode {
    map: BTreeMap<u32, Option<String>>,
    uses_parent: bool,
}

impl ToUnicode {
    /// What the CMap says about the code with numeric value `code`.
    pub fn lookup(&self, code: u32) -> Lookup<'_> {
        match self.map.get(&code) {
            Some(Some(text)) => Lookup::Text(text),
            Some(None) => Lookup::Undecodable,
            None if self.uses_parent => Lookup::Undecodable,
            None => Lookup::Absent,
        }
    }

    /// The value of every code with an explicit mapping, ascending.
    pub fn codes(&self) -> impl Iterator<Item = u32> + '_ {
        self.map.keys().copied()
    }
}

/// Parses a decoded ToUnicode stream.
pub fn parse_tounicode(bytes: &[u8]) -> Result<ToUnicode, ()> {
    let tokens = scan_tokens(bytes, ScanMode::CMap, TOKENS_MAX).map_err(|_| ())?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        out: ToUnicode::default(),
        count: 0,
        dest_bytes: 0,
    };
    while let Some(token) = parser.next() {
        match keyword(token) {
            Some(b"begincodespacerange") => parser.codespace()?,
            Some(b"beginbfchar") => parser.bfchar()?,
            Some(b"beginbfrange") => parser.bfrange()?,
            Some(b"usecmap") => parser.out.uses_parent = true,
            Some(b"begincidrange" | b"begincidchar" | b"beginnotdefrange" | b"beginnotdefchar") => {
                parser.skip_section()?
            }
            _ => {}
        }
    }
    Ok(parser.out)
}

fn keyword(token: &Token) -> Option<&[u8]> {
    match token {
        Token::Keyword { bytes, .. } => Some(bytes),
        _ => None,
    }
}

fn string(token: &Token) -> Option<&[u8]> {
    match token {
        Token::Operand(Operand::Str { bytes, .. }) => Some(bytes),
        _ => None,
    }
}

/// A source code: 1–4 bytes, big-endian.
fn code_of(bytes: &[u8]) -> Result<Code, ()> {
    if bytes.is_empty() || bytes.len() > 4 {
        return Err(());
    }
    let value = bytes.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
    let len = u8::try_from(bytes.len()).map_err(|_| ())?;
    Ok(Code { value, len })
}

/// UTF-16BE units of a destination (`None` for an odd or empty one).
fn units(dst: &[u8]) -> Result<Option<Vec<u16>>, ()> {
    if dst.len() > DESTINATION_MAX_BYTES {
        return Err(());
    }
    if dst.is_empty() || dst.len() % 2 != 0 {
        return Ok(None);
    }
    Ok(Some(
        dst.chunks_exact(2)
            .map(|p| match p {
                [hi, lo] => u16::from_be_bytes([*hi, *lo]),
                _ => 0,
            })
            .collect(),
    ))
}

/// Decodes UTF-16 units; a lone surrogate makes the whole destination undecodable.
fn text_of(units: &[u16]) -> Option<String> {
    char::decode_utf16(units.iter().copied())
        .collect::<Result<String, _>>()
        .ok()
}

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
    out: ToUnicode,
    count: usize,
    /// Destination bytes of every mapping so far (a range counts its destination per code).
    dest_bytes: usize,
}

impl<'t> Parser<'t> {
    fn next(&mut self) -> Option<&'t Token> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    fn take_string(&mut self) -> Result<&'t [u8], ()> {
        self.next().and_then(string).ok_or(())
    }

    /// True (and consumed) when the next token is the keyword `end`.
    fn at_end(&mut self, end: &[u8]) -> Result<bool, ()> {
        let token = self.tokens.get(self.pos).ok_or(())?;
        if keyword(token) == Some(end) {
            self.pos += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn insert(&mut self, code: Code, text: Option<String>) -> Result<(), ()> {
        self.count = self.count.checked_add(1).ok_or(())?;
        if self.count > CMAP_MAPPINGS_MAX {
            return Err(());
        }
        self.out.map.insert(code.value, text);
        Ok(())
    }

    /// Debits `mappings` destinations of `dest_len` bytes (before any of them is built).
    fn reserve_text(&mut self, mappings: u32, dest_len: usize) -> Result<(), ()> {
        let bytes = usize::try_from(mappings)
            .map_err(|_| ())?
            .checked_mul(dest_len)
            .ok_or(())?;
        self.dest_bytes = self.dest_bytes.checked_add(bytes).ok_or(())?;
        if self.dest_bytes > TOUNICODE_MAX_DECODED {
            return Err(());
        }
        Ok(())
    }

    fn codespace(&mut self) -> Result<(), ()> {
        while !self.at_end(b"endcodespacerange")? {
            let lo = self.take_string()?;
            let hi = self.take_string()?;
            if lo.is_empty() || lo.len() > 4 || lo.len() != hi.len() {
                return Err(());
            }
        }
        Ok(())
    }

    fn bfchar(&mut self) -> Result<(), ()> {
        while !self.at_end(b"endbfchar")? {
            let code = code_of(self.take_string()?)?;
            let dst = self.take_string()?;
            self.reserve_text(1, dst.len())?;
            let text = units(dst)?.and_then(|u| text_of(&u));
            self.insert(code, text)?;
        }
        Ok(())
    }

    fn bfrange(&mut self) -> Result<(), ()> {
        while !self.at_end(b"endbfrange")? {
            let lo = code_of(self.take_string()?)?;
            let hi = code_of(self.take_string()?)?;
            if lo.len != hi.len || lo.value > hi.value {
                return Err(());
            }
            let count = hi.value - lo.value;
            let total = self
                .count
                .checked_add(usize::try_from(count).map_err(|_| ())?)
                .ok_or(())?;
            if total >= CMAP_MAPPINGS_MAX {
                return Err(());
            }
            match self.next().ok_or(())? {
                Token::ArrayOpen(_) => self.range_array(lo, count)?,
                token => {
                    let dst = string(token).ok_or(())?;
                    self.reserve_text(count.checked_add(1).ok_or(())?, dst.len())?;
                    self.range_increment(lo, count, units(dst)?)?;
                }
            }
        }
        Ok(())
    }

    fn range_increment(&mut self, lo: Code, count: u32, base: Option<Vec<u16>>) -> Result<(), ()> {
        for k in 0..=count {
            let code = Code {
                value: lo.value + k,
                len: lo.len,
            };
            let Some(units) = base.as_ref() else {
                self.insert(code, None)?;
                continue;
            };
            let (last, head) = units.split_last().ok_or(())?;
            let Some(next) = u32::from(*last)
                .checked_add(k)
                .and_then(|v| u16::try_from(v).ok())
            else {
                break; // overflow past 0xFFFF stops the range
            };
            let mut shifted = head.to_vec();
            shifted.push(next);
            self.insert(code, text_of(&shifted))?;
        }
        Ok(())
    }

    fn range_array(&mut self, lo: Code, count: u32) -> Result<(), ()> {
        let mut k: u32 = 0;
        loop {
            let token = self.next().ok_or(())?;
            if matches!(token, Token::ArrayClose(_)) {
                return Ok(());
            }
            let dst = string(token).ok_or(())?;
            self.reserve_text(1, dst.len())?;
            let text = units(dst)?.and_then(|u| text_of(&u));
            if k <= count {
                let code = Code {
                    value: lo.value + k,
                    len: lo.len,
                };
                self.insert(code, text)?;
            }
            k = k.saturating_add(1);
        }
    }

    /// CID/notdef sections do not belong in a ToUnicode CMap: their content maps nothing here.
    fn skip_section(&mut self) -> Result<(), ()> {
        loop {
            let token = self.next().ok_or(())?;
            if keyword(token).is_some_and(|k| k.starts_with(b"end")) {
                return Ok(());
            }
        }
    }
}
