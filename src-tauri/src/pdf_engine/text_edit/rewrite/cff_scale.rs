//! The units-to-em scale of a CFF program's outlines, for the A5 masks (SPEC §B.15).
//!
//! ttf-parser outlines CFF glyphs in font units and reads only the Top DICT `FontMatrix`. A
//! CID-keyed program can also give each Font DICT of its FDArray a `FontMatrix`, which renderers
//! (FreeType, hence Poppler) combine with the Top DICT one (Adobe TN #5176), so the Top DICT
//! matrix alone can be off by any factor. A mask built at a wrong scale is either empty (an honest
//! edit fails) or several em wide (a moved neighbour on the same line is hidden), so a program's
//! own glyph boxes are used only when its scale is unambiguous:
//! - CID-keyed: no Font DICT has a `FontMatrix`, and the Top DICT one is absent or the default;
//! - name-keyed: the Top DICT `FontMatrix` is absent (the default) or one plain uniform scale (no
//!   skew, no offset) of at least 16 units per em; inside an OpenType font it must be the default.
//!
//! The default is 1/1000 em per unit for a bare program and 1/`unitsPerEm` inside an OpenType
//! font. Anything else, or a structure this reader cannot follow, is `None`: the masks then use
//! the bounded fallback box. Checked slicing only; no DICT longer than `DICT_LEN_MAX` is walked.

use crate::pdf_engine::text_edit::fonts::cff_layout::DICT_LEN_MAX;
use std::ops::Range;

/// Where a CFF program lives.
#[derive(Clone, Copy, Debug)]
pub(super) enum Host {
    /// `FontFile3 /Type1C` or `/CIDFontType0C`.
    Bare,
    /// The `CFF ` table of an OpenType font with this `unitsPerEm`.
    OpenType(u16),
}

const OP_FONT_MATRIX: u16 = 1207;
const OP_ROS: u16 = 1230;
const OP_FD_ARRAY: u16 = 1236;
/// Items read from one INDEX: FDSelect selects a Font DICT with one byte, and a PDF font program
/// holds one font (more is not followed).
const INDEX_ITEMS_MAX: usize = 256;
/// Operands kept per DICT operator (as ttf-parser); more makes the DICT unreadable here.
const OPERANDS_MAX: usize = 48;
/// Longest real-number operand read, in nibbles.
const REAL_NIBBLES_MAX: usize = 64;
/// Largest scale accepted: at least 16 units per em (the OpenType minimum).
const SCALE_MAX: f64 = 1.0 / 16.0;
/// Relative tolerance of "equal to the default scale" (matrices are written as decimals).
const SCALE_REL_TOL: f64 = 1e-4;

/// Em per font unit of the outlines of `cff`, when it is unambiguous (see the module doc).
pub(super) fn em_per_unit(cff: &[u8], host: Host) -> Option<f64> {
    let default = match host {
        Host::Bare => 0.001,
        Host::OpenType(upem) if upem > 0 => 1.0 / f64::from(upem),
        Host::OpenType(_) => return None,
    };
    let top = TopEntries::read(cff)?;
    let explicit = match &top.matrix {
        None => None,
        Some(m) => Some(plain_scale(m)?),
    };
    let near_default = |s: f64| (s - default).abs() <= default * SCALE_REL_TOL;
    if top.ros {
        if explicit.is_some_and(|s| !near_default(s)) {
            return None;
        }
        return (!font_dicts_have_matrix(cff, top.fd_array?)?).then_some(default);
    }
    match (explicit, host) {
        (None, _) => Some(default),
        (Some(s), Host::Bare) => (s <= SCALE_MAX).then_some(s),
        (Some(s), Host::OpenType(_)) => near_default(s).then_some(default),
    }
}

/// The scale of a `FontMatrix` that is one uniform scale with no skew or offset.
fn plain_scale(m: &[f64]) -> Option<f64> {
    let [a, b, c, d, e, f] = m else {
        return None;
    };
    let plain = *b == 0.0 && *c == 0.0 && *e == 0.0 && *f == 0.0 && *a > 0.0;
    (plain && (a - d).abs() <= a * SCALE_REL_TOL).then_some(*a)
}

/// The Top DICT entries this reader needs (later entries win, as in ttf-parser and FreeType).
struct TopEntries {
    ros: bool,
    matrix: Option<Vec<f64>>,
    fd_array: Option<usize>,
}

impl TopEntries {
    /// The first Top DICT of `cff` (header, Name INDEX, Top DICT INDEX).
    fn read(cff: &[u8]) -> Option<TopEntries> {
        if *cff.first()? != 1 {
            return None;
        }
        let header = usize::from(*cff.get(2)?);
        if header < 4 {
            return None;
        }
        let (_, after_names) = index_items(cff, header)?;
        let (tops, _) = index_items(cff, after_names)?;
        let dict = cff.get(tops.first()?.clone())?;
        let mut top = TopEntries {
            ros: false,
            matrix: None,
            fd_array: None,
        };
        walk_dict(dict, |op, operands| {
            match op {
                OP_ROS => top.ros = true,
                OP_FONT_MATRIX => top.matrix = Some(operands.to_vec()),
                OP_FD_ARRAY => top.fd_array = Some(offset_of(operands)?),
                _ => {}
            }
            Some(())
        })?;
        Some(top)
    }
}

/// A DICT offset operand: exactly one non-negative integer.
fn offset_of(operands: &[f64]) -> Option<usize> {
    let [v] = operands else {
        return None;
    };
    (v.fract() == 0.0 && *v >= 0.0 && *v <= f64::from(u32::MAX)).then(|| *v as usize)
}

/// Whether any Font DICT of the FDArray at `at` has a `FontMatrix` (`None`: unreadable).
fn font_dicts_have_matrix(cff: &[u8], at: usize) -> Option<bool> {
    let (dicts, _) = index_items(cff, at)?;
    let mut found = false;
    for range in dicts {
        walk_dict(cff.get(range)?, |op, _| {
            found |= op == OP_FONT_MATRIX;
            Some(())
        })?;
    }
    Some(found)
}

/// The item ranges (absolute) of the INDEX at `at` and the offset after it. `None` for a malformed
/// INDEX or one of more than `INDEX_ITEMS_MAX` items.
fn index_items(cff: &[u8], at: usize) -> Option<(Vec<Range<usize>>, usize)> {
    let count = usize::from(u16::from_be_bytes([
        *cff.get(at)?,
        *cff.get(at.checked_add(1)?)?,
    ]));
    let pos = at.checked_add(2)?;
    if count == 0 {
        return Some((Vec::new(), pos));
    }
    if count > INDEX_ITEMS_MAX {
        return None;
    }
    let off_size = usize::from(*cff.get(pos)?);
    if !(1..=4).contains(&off_size) {
        return None;
    }
    let offsets_at = pos.checked_add(1)?;
    let offsets_len = count.checked_add(1)?.checked_mul(off_size)?;
    let offsets = cff.get(offsets_at..offsets_at.checked_add(offsets_len)?)?;
    // Offsets are 1-based from the byte before the data.
    let base = offsets_at.checked_add(offsets_len)?.checked_sub(1)?;
    let offset = |i: usize| -> Option<usize> {
        let start = i.checked_mul(off_size)?;
        let raw = offsets
            .get(start..start.checked_add(off_size)?)?
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (raw >= 1).then_some(raw)
    };
    let mut items = Vec::with_capacity(count);
    let mut start = offset(0)?;
    for i in 1..=count {
        let end = offset(i)?;
        if end < start {
            return None;
        }
        items.push(base.checked_add(start)?..base.checked_add(end)?);
        start = end;
    }
    let end = base.checked_add(start)?;
    (end <= cff.len()).then_some((items, end))
}

/// Calls `f(operator, operands)` for every entry of a DICT (two-byte operators as `1200 + b1`).
/// `None` when the DICT is longer than `DICT_LEN_MAX`, malformed, or `f` returns `None`.
fn walk_dict(dict: &[u8], mut f: impl FnMut(u16, &[f64]) -> Option<()>) -> Option<()> {
    if dict.len() > DICT_LEN_MAX {
        return None;
    }
    let byte = |at: usize| dict.get(at).copied();
    let mut operands: Vec<f64> = Vec::new();
    let mut pos = 0usize;
    while let Some(b0) = byte(pos) {
        pos = pos.checked_add(1)?;
        let value = match b0 {
            12 => {
                let b1 = byte(pos)?;
                pos = pos.checked_add(1)?;
                f(1200 + u16::from(b1), &operands)?;
                operands.clear();
                continue;
            }
            0..=27 | 31 | 255 => {
                f(u16::from(b0), &operands)?;
                operands.clear();
                continue;
            }
            28 => {
                let v = i16::from_be_bytes([byte(pos)?, byte(pos.checked_add(1)?)?]);
                pos = pos.checked_add(2)?;
                f64::from(v)
            }
            29 => {
                let at = |k: usize| byte(pos.checked_add(k)?);
                let v = i32::from_be_bytes([at(0)?, at(1)?, at(2)?, at(3)?]);
                pos = pos.checked_add(4)?;
                f64::from(v)
            }
            30 => {
                let (v, next) = real(dict, pos)?;
                pos = next;
                v
            }
            32..=246 => f64::from(b0) - 139.0,
            247..=250 => {
                let b1 = byte(pos)?;
                pos = pos.checked_add(1)?;
                (f64::from(b0) - 247.0) * 256.0 + f64::from(b1) + 108.0
            }
            251..=254 => {
                let b1 = byte(pos)?;
                pos = pos.checked_add(1)?;
                -(f64::from(b0) - 251.0) * 256.0 - f64::from(b1) - 108.0
            }
        };
        if operands.len() >= OPERANDS_MAX {
            return None;
        }
        operands.push(value);
    }
    Some(())
}

/// A DICT real number (nibbles after byte 30, from `pos`) and the offset after it.
fn real(data: &[u8], mut pos: usize) -> Option<(f64, usize)> {
    let mut text = String::new();
    loop {
        let b = *data.get(pos)?;
        pos = pos.checked_add(1)?;
        for nibble in [b >> 4, b & 0xF] {
            match nibble {
                0..=9 => text.push(char::from(b'0' + nibble)),
                0xA => text.push('.'),
                0xB => text.push('E'),
                0xC => text.push_str("E-"),
                0xE => text.push('-'),
                0xF => {
                    let v = text.parse::<f64>().ok().filter(|v| v.is_finite())?;
                    return Some((v, pos));
                }
                _ => return None,
            }
        }
        if text.len() > REAL_NIBBLES_MAX {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{em_per_unit, Host};
    use crate::pdf_engine::text_edit::testkit::cff::{box_charstring, CffBuilder};
    use crate::pdf_engine::text_edit::testkit::fakes::cff_with_font_dict_matrix;

    /// `FontMatrix` operands: `[1 0 0 1 0 0]` (integers) and `[s 0 0 s 0 0]` for a real `s`.
    const IDENTITY: [u8; 6] = [140, 139, 139, 140, 139, 139];
    const MILLI: [u8; 12] = [
        30, 0x0a, 0x00, 0x1f, 139, 139, 30, 0x0a, 0x00, 0x1f, 139, 139,
    ];
    const HALF_MILLI: [u8; 14] = [
        30, 0x0a, 0x00, 0x05, 0xff, 139, 139, 30, 0x0a, 0x00, 0x05, 0xff, 139, 139,
    ];

    fn matrix_op(operands: &[u8]) -> Vec<u8> {
        let mut v = operands.to_vec();
        v.extend([12, 7]);
        v
    }

    fn cid(top: Option<&[u8]>, fd: Option<&[u8]>) -> Vec<u8> {
        let mut b = CffBuilder::new("ABCDEF+Box").raw_cid_glyph(1, box_charstring());
        if let Some(t) = top {
            b.top_padding = matrix_op(t);
        }
        let cff = b.build();
        fd.map_or(cff.clone(), |m| cff_with_font_dict_matrix(&cff, m))
    }

    fn named(top: Option<&[u8]>) -> Vec<u8> {
        let mut b = CffBuilder::new("ABCDEF+Box").glyph("A", true);
        if let Some(t) = top {
            b.top_padding = matrix_op(t);
        }
        b.build()
    }

    #[test]
    fn cid_keyed_programs_use_their_boxes_only_without_font_dict_matrices() {
        assert_eq!(em_per_unit(&cid(None, None), Host::Bare), Some(0.001));
        assert_eq!(
            em_per_unit(&cid(Some(&MILLI), None), Host::Bare),
            Some(0.001),
            "an explicit default Top matrix"
        );
        assert_eq!(
            em_per_unit(&cid(Some(&IDENTITY), Some(&MILLI)), Host::Bare),
            None,
            "Top [1 0 0 1 0 0] + Font DICT [0.001 …] (ttf-parser would scale by 1)"
        );
        assert_eq!(
            em_per_unit(&cid(None, Some(&HALF_MILLI)), Host::Bare),
            None,
            "Font DICT [0.0005 …] under the default Top (ttf-parser would scale by 0.001)"
        );
        assert_eq!(
            em_per_unit(&cid(Some(&HALF_MILLI), None), Host::Bare),
            None,
            "a non-default Top matrix of a CID-keyed program"
        );
        assert_eq!(
            em_per_unit(&cid(None, None), Host::OpenType(1000)),
            Some(0.001)
        );
    }

    #[test]
    fn name_keyed_programs_need_one_plain_scale() {
        assert_eq!(em_per_unit(&named(None), Host::Bare), Some(0.001));
        assert_eq!(
            em_per_unit(&named(Some(&HALF_MILLI)), Host::Bare),
            Some(0.0005)
        );
        assert_eq!(
            em_per_unit(&named(Some(&IDENTITY)), Host::Bare),
            None,
            "one unit per em"
        );
        let skewed = [
            30, 0x0a, 0x00, 0x1f, 139, 30, 0x0a, 0x00, 0x2f, 30, 0x0a, 0x00, 0x1f, 139, 139,
        ];
        assert_eq!(em_per_unit(&named(Some(&skewed)), Host::Bare), None, "skew");
        let offset = [
            30, 0x0a, 0x00, 0x1f, 139, 139, 30, 0x0a, 0x00, 0x1f, 140, 139,
        ];
        assert_eq!(
            em_per_unit(&named(Some(&offset)), Host::Bare),
            None,
            "offset"
        );
        assert_eq!(
            em_per_unit(&named(Some(&[140, 139])), Host::Bare),
            None,
            "2 operands"
        );
        // Inside OpenType the scale is 1/unitsPerEm; a Top matrix must agree with it.
        assert_eq!(
            em_per_unit(&named(None), Host::OpenType(2048)),
            Some(1.0 / 2048.0)
        );
        assert_eq!(
            em_per_unit(&named(Some(&MILLI)), Host::OpenType(1000)),
            Some(0.001)
        );
        assert_eq!(
            em_per_unit(&named(Some(&MILLI)), Host::OpenType(2048)),
            None
        );
        assert_eq!(em_per_unit(&named(None), Host::OpenType(0)), None);
    }

    #[test]
    fn unreadable_programs_have_no_scale() {
        let good = cid(None, None);
        assert_eq!(em_per_unit(&good, Host::Bare), Some(0.001));
        // Cut inside the header, the Name INDEX, the Top DICT INDEX, or before the FDArray.
        for n in [0, 1, 3, 4, 10, 40, good.len() / 2] {
            assert_eq!(
                em_per_unit(&good[..n], Host::Bare),
                None,
                "truncated at {n}"
            );
        }
        let mut wrong_version = good.clone();
        wrong_version[0] = 2;
        assert_eq!(em_per_unit(&wrong_version, Host::Bare), None);
        let mut short_header = good;
        short_header[2] = 3;
        assert_eq!(em_per_unit(&short_header, Host::Bare), None);
        // A real number without an end nibble, and a reserved nibble.
        assert_eq!(
            em_per_unit(&named(Some(&[30, 0x0a, 0x00, 0x11])), Host::Bare),
            None
        );
        assert_eq!(
            em_per_unit(
                &named(Some(&[30, 0xd1, 0xff, 139, 139, 139, 139, 139])),
                Host::Bare
            ),
            None
        );
    }
}
