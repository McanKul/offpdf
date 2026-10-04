//! A CFF program's own (built-in) encoding (SPEC §B.9.7), read by a bounded parser because
//! ttf-parser's `Encoding` is crate-private and its `glyph_index(code)` falls back to
//! StandardEncoding for codes the custom encoding lacks (`cff1.rs:967-981`) — that fallback would
//! call glyphs typeable that the font's encoding never maps.
//!
//! header → Name INDEX → Top DICT INDEX (first entry) → operator 16 (`Encoding`, default 0).
//! 0 → Standard, 1 → Expert, else an offset to format 0 (`nCodes`, `code[i]` → GID i+1) or
//! format 1 (ranges, consecutive GIDs from 1). Supplements (high bit) are ignored, so those codes
//! stay non-typeable. A CID-keyed font (`ROS`) has no encoding → `Err`. Checked slicing only.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CffEncoding {
    Standard,
    Expert,
    Custom(BTreeMap<u8, u16 /*gid*/>),
}

/// Operands kept per DICT operator (the CFF limit is 48).
const DICT_OPERANDS_MAX: usize = 48;
const OP_ENCODING: u16 = 16;
const OP_ROS: u16 = 0x0C1E; // 12 30

/// Parses the built-in encoding of a bare CFF (`FontFile3 /Type1C`) program.
pub fn cff_builtin_encoding(cff: &[u8]) -> Result<CffEncoding, ()> {
    if cff.first() != Some(&1) {
        return Err(());
    }
    let header_size = usize::from(*cff.get(2).ok_or(())?);
    let name_index = Index::parse(cff, header_size)?;
    let top_index = Index::parse(cff, name_index.end)?;
    let top_dict = top_index.entry(cff, 0)?;
    let mut encoding: i64 = 0;
    for (op, operands) in DictIter::new(top_dict) {
        let (op, operands) = (op?, operands);
        match op {
            OP_ROS => return Err(()),
            OP_ENCODING => encoding = *operands.first().ok_or(())?,
            _ => {}
        }
    }
    match encoding {
        0 => Ok(CffEncoding::Standard),
        1 => Ok(CffEncoding::Expert),
        offset => custom(cff, usize::try_from(offset).map_err(|_| ())?),
    }
}

fn custom(cff: &[u8], at: usize) -> Result<CffEncoding, ()> {
    let format = *cff.get(at).ok_or(())?;
    let count = usize::from(*cff.get(at.checked_add(1).ok_or(())?).ok_or(())?);
    let body = at.checked_add(2).ok_or(())?;
    let mut map = BTreeMap::new();
    match format & 0x7F {
        0 => {
            let codes = cff
                .get(body..body.checked_add(count).ok_or(())?)
                .ok_or(())?;
            for (i, code) in codes.iter().enumerate() {
                let gid = u16::try_from(i + 1).map_err(|_| ())?;
                map.entry(*code).or_insert(gid);
            }
        }
        1 => {
            let ranges = cff
                .get(
                    body..body
                        .checked_add(count.checked_mul(2).ok_or(())?)
                        .ok_or(())?,
                )
                .ok_or(())?;
            let mut gid: u16 = 1;
            for range in ranges.chunks_exact(2) {
                let (first, left) = match range {
                    [f, l] => (u16::from(*f), u16::from(*l)),
                    _ => return Err(()),
                };
                for code in first..=first + left {
                    let code = u8::try_from(code).map_err(|_| ())?;
                    map.entry(code).or_insert(gid);
                    gid = gid.checked_add(1).ok_or(())?;
                }
            }
        }
        _ => return Err(()),
    }
    Ok(CffEncoding::Custom(map))
}

/// A CFF INDEX: `count` (u16), `offSize`, `count + 1` offsets (1-based), data.
struct Index {
    count: usize,
    off_size: usize,
    offsets_at: usize,
    data_at: usize,
    end: usize,
}

impl Index {
    fn parse(cff: &[u8], at: usize) -> Result<Index, ()> {
        let count = usize::from(u16::from_be_bytes([
            *cff.get(at).ok_or(())?,
            *cff.get(at.checked_add(1).ok_or(())?).ok_or(())?,
        ]));
        let after_count = at.checked_add(2).ok_or(())?;
        if count == 0 {
            return Ok(Index {
                count,
                off_size: 1,
                offsets_at: after_count,
                data_at: after_count,
                end: after_count,
            });
        }
        let off_size = usize::from(*cff.get(after_count).ok_or(())?);
        if !(1..=4).contains(&off_size) {
            return Err(());
        }
        let offsets_at = after_count.checked_add(1).ok_or(())?;
        let data_at = offsets_at
            .checked_add((count + 1).checked_mul(off_size).ok_or(())?)
            .ok_or(())?;
        let mut index = Index {
            count,
            off_size,
            offsets_at,
            data_at,
            end: 0,
        };
        let last = index.offset(cff, count)?;
        index.end = data_at
            .checked_add(last.checked_sub(1).ok_or(())?)
            .ok_or(())?;
        if index.end > cff.len() {
            return Err(());
        }
        Ok(index)
    }

    fn offset(&self, cff: &[u8], i: usize) -> Result<usize, ()> {
        let at = self
            .offsets_at
            .checked_add(i.checked_mul(self.off_size).ok_or(())?)
            .ok_or(())?;
        let bytes = cff
            .get(at..at.checked_add(self.off_size).ok_or(())?)
            .ok_or(())?;
        Ok(bytes
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b)))
    }

    fn entry<'a>(&self, cff: &'a [u8], i: usize) -> Result<&'a [u8], ()> {
        if i >= self.count {
            return Err(());
        }
        let start = self.offset(cff, i)?.checked_sub(1).ok_or(())?;
        let end = self.offset(cff, i + 1)?.checked_sub(1).ok_or(())?;
        if end < start {
            return Err(());
        }
        cff.get(
            self.data_at.checked_add(start).ok_or(())?..self.data_at.checked_add(end).ok_or(())?,
        )
        .ok_or(())
    }
}

/// Iterates `(operator, integer operands)` of a DICT; reals are skipped as operands (kept as 0).
struct DictIter<'a> {
    data: &'a [u8],
    pos: usize,
    failed: bool,
}

impl<'a> DictIter<'a> {
    fn new(data: &'a [u8]) -> Self {
        DictIter {
            data,
            pos: 0,
            failed: false,
        }
    }

    fn byte(&mut self) -> Result<u8, ()> {
        let b = *self.data.get(self.pos).ok_or(())?;
        self.pos += 1;
        Ok(b)
    }

    fn next_entry(&mut self) -> Result<(u16, Vec<i64>), ()> {
        let mut operands = Vec::new();
        loop {
            let b0 = self.byte()?;
            let value = match b0 {
                0..=21 => {
                    let op = if b0 == 12 {
                        0x0C00 | u16::from(self.byte()?)
                    } else {
                        u16::from(b0)
                    };
                    return Ok((op, operands));
                }
                28 => i64::from(i16::from_be_bytes([self.byte()?, self.byte()?])),
                29 => i64::from(i32::from_be_bytes([
                    self.byte()?,
                    self.byte()?,
                    self.byte()?,
                    self.byte()?,
                ])),
                30 => {
                    // real: nibbles up to and including an 0xF nibble
                    loop {
                        let b = self.byte()?;
                        if b & 0x0F == 0x0F || b >> 4 == 0x0F {
                            break;
                        }
                    }
                    0
                }
                32..=246 => i64::from(b0) - 139,
                247..=250 => (i64::from(b0) - 247) * 256 + i64::from(self.byte()?) + 108,
                251..=254 => -(i64::from(b0) - 251) * 256 - i64::from(self.byte()?) - 108,
                _ => return Err(()),
            };
            if operands.len() >= DICT_OPERANDS_MAX {
                return Err(());
            }
            operands.push(value);
        }
    }
}

impl Iterator for DictIter<'_> {
    type Item = (Result<u16, ()>, Vec<i64>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.pos >= self.data.len() {
            return None;
        }
        match self.next_entry() {
            Ok((op, operands)) => Some((Ok(op), operands)),
            Err(()) => {
                self.failed = true;
                Some((Err(()), Vec::new()))
            }
        }
    }
}
