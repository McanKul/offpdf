//! Bounded work for glyph-presence proofs (SPEC §A.2, §B.9.5, §B.2).
//!
//! ttf-parser's outlining only limits nesting depth (glyf `MAX_COMPONENTS` 32, CFF `STACK_LIMIT`
//! 10), so a tiny glyph whose composites or subroutines fan out runs for minutes. Before
//! ttf-parser outlines a glyph, a pre-check walks the same structure with a work budget:
//! - `GlyfGuard`: the composite tree exactly as `glyf::outline_impl` visits it (component records
//!   read as `CompositeGlyphIter` reads them); one unit per glyph visit and component record, one
//!   per contour and per point of every simple glyph visited.
//! - `CffGuard`: the Type2 charstring exactly as `cff1::_parse_char_string` runs it (operand stack
//!   values, `callsubr`/`callgsubr` with their bias, `return`, `endchar` and its `seac` form, hint
//!   masks sized by the stems seen, the width rules that decide the stem count); one unit per byte
//!   read and per FDSelect range or charset entry walked, at most `CFF_OPS_PER_GLYPH_MAX`
//!   operators. A CID-keyed glyph that calls a local subroutine is also charged its Font DICT and
//!   Private DICT bytes, which ttf-parser walks again for every such glyph; the guard itself
//!   resolves each Font DICT once (and charges that walk once). Wherever ttf-parser stops with an
//!   error the walk stops too (the glyph does not draw); argument-count errors are not checked,
//!   so the walk never does less than ttf-parser.
//!
//! A glyph may spend at most `GLYPH_WORK_MAX` units, and never more than the page budget has left;
//! over that it does not draw. Every unit is charged to a `WorkMeter` holding what is left of the
//! page decode budget, so one page's font work is bounded however many fonts and glyphs it has:
//! a walk stops as soon as it passes what the meter can still pay, a meter that runs out marks the
//! font load `PAGE_TOO_COMPLEX`, and after that no glyph is walked at all. The Type1 interpreter
//! (`type1.rs`) charges the same meter under the same rules.

use super::cff_layout::{seac_glyph, CffLayout, FdSubrs, Index, LocalSubrs};
use std::cell::OnceCell;
use std::collections::HashMap;
use ttf_parser::{loca, Face, Tag};

/// Work units one glyph proof may spend.
pub(crate) const GLYPH_WORK_MAX: u64 = 1 << 17;
/// Type2 operators one glyph may run, subroutines and `seac` components included.
const CFF_OPS_PER_GLYPH_MAX: u32 = 4_096;
/// ttf-parser's limits (`glyf::MAX_COMPONENTS`, `cff1::STACK_LIMIT`, `MAX_ARGUMENTS_STACK_LEN`).
const GLYF_DEPTH_LIMIT: u8 = 32;
const CFF_DEPTH_LIMIT: u8 = 10;
const CFF_STACK_MAX: usize = 48;

/// What is left of the page decode budget for one font load's glyph work.
#[derive(Debug)]
pub(crate) struct WorkMeter {
    remaining: u64,
    used: u64,
    exhausted: bool,
}

impl WorkMeter {
    pub fn new(allowance: usize) -> WorkMeter {
        WorkMeter {
            remaining: u64::try_from(allowance).unwrap_or(u64::MAX),
            used: 0,
            exhausted: false,
        }
    }

    /// Debits `units`; `false` (and the meter is exhausted) when they do not fit.
    pub fn charge(&mut self, units: u64) -> bool {
        if self.exhausted || units > self.remaining {
            self.exhausted = true;
            self.remaining = 0;
            return false;
        }
        self.remaining -= units;
        self.used = self.used.saturating_add(units);
        true
    }

    /// Units the next glyph may spend: `GLYPH_WORK_MAX`, or less when the page budget has less
    /// left (0 once exhausted).
    pub fn glyph_allowance(&self) -> u64 {
        if self.exhausted {
            0
        } else {
            GLYPH_WORK_MAX.min(self.remaining)
        }
    }

    pub fn used(&self) -> usize {
        usize::try_from(self.used).unwrap_or(usize::MAX)
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted
    }
}

/// Units spent by one glyph against its allowance (`WorkMeter::glyph_allowance`).
struct Work {
    used: u64,
    cap: u64,
}

impl Work {
    fn new(meter: &WorkMeter) -> Work {
        Work {
            used: 0,
            cap: meter.glyph_allowance(),
        }
    }

    fn tick(&mut self, units: u64) -> Result<(), ()> {
        self.used = self.used.saturating_add(units);
        if self.used > self.cap {
            return Err(());
        }
        Ok(())
    }

    /// Debits the units spent (at least one) from `meter`; `false` when it cannot pay them.
    fn settle(&self, meter: &mut WorkMeter) -> bool {
        meter.charge(self.used.max(1))
    }
}

/// The glyf/loca tables of a face, read through ttf-parser's own `loca` parser.
pub(crate) struct GlyfGuard<'a> {
    loca: loca::Table<'a>,
    glyf: &'a [u8],
}

impl<'a> GlyfGuard<'a> {
    pub fn new(face: &Face<'a>) -> Option<GlyfGuard<'a>> {
        let tables = face.tables();
        let raw = face.raw_face();
        let loca = loca::Table::parse(
            tables.maxp.number_of_glyphs,
            tables.head.index_to_location_format,
            raw.table(Tag::from_bytes(b"loca"))?,
        )?;
        Some(GlyfGuard {
            loca,
            glyf: raw.table(Tag::from_bytes(b"glyf"))?,
        })
    }

    /// Whether ttf-parser may outline `gid` (within budget, and not a glyph it fails on early);
    /// the units are charged to `meter` either way. An exhausted meter walks nothing.
    pub fn check(&self, gid: u16, meter: &mut WorkMeter) -> bool {
        if meter.exhausted() {
            return false;
        }
        let mut work = Work::new(meter);
        let ok = match self.glyph_data(gid) {
            Some(data) => self.visit(data, 0, &mut work).is_ok(),
            None => false,
        };
        work.settle(meter) && ok
    }

    fn glyph_data(&self, gid: u16) -> Option<&'a [u8]> {
        self.glyf
            .get(self.loca.glyph_range(ttf_parser::GlyphId(gid))?)
    }

    fn visit(&self, data: &[u8], depth: u8, work: &mut Work) -> Result<(), ()> {
        if depth >= GLYF_DEPTH_LIMIT {
            return Err(());
        }
        work.tick(1)?;
        let contours = data
            .get(..2)
            .and_then(|b| Some(i16::from_be_bytes([*b.first()?, *b.get(1)?])))
            .ok_or(())?;
        if contours == 0 {
            return Ok(());
        }
        let tail = data.get(10..).ok_or(())?;
        if contours > 0 {
            let n = usize::from(contours.unsigned_abs());
            let last_at = (n - 1) * 2;
            let last = tail
                .get(last_at..last_at + 2)
                .and_then(|b| Some(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
                .ok_or(())?;
            let points = last.checked_add(1).ok_or(())?;
            return work.tick(n as u64 + u64::from(points));
        }
        // Composite records, read as `CompositeGlyphIter` reads them; a record cut short ends
        // the list (it is not an error there).
        let mut pos = 0usize;
        loop {
            let Some((flags, component, next)) = component_record(tail, pos) else {
                return Ok(());
            };
            work.tick(1)?;
            if let Some(data) = self.glyph_data(component) {
                self.visit(data, depth + 1, work)?;
            }
            if flags & 0x0020 == 0 {
                return Ok(());
            }
            pos = next;
        }
    }
}

/// One composite record at `pos`: flags, component GID and the offset after it. Arguments are
/// read only with ARGS_ARE_XY_VALUES (ttf-parser's reading), then the scale fields.
fn component_record(tail: &[u8], pos: usize) -> Option<(u16, u16, usize)> {
    let word = |at: usize| -> Option<u16> {
        let b = tail.get(at..at.checked_add(2)?)?;
        Some(u16::from_be_bytes([*b.first()?, *b.get(1)?]))
    };
    let flags = word(pos)?;
    let gid = word(pos.checked_add(2)?)?;
    let mut at = pos.checked_add(4)?;
    if flags & 0x0002 != 0 {
        at = at.checked_add(if flags & 0x0001 != 0 { 4 } else { 2 })?;
    }
    if flags & 0x0080 != 0 {
        at = at.checked_add(8)?;
    } else if flags & 0x0040 != 0 {
        at = at.checked_add(4)?;
    } else if flags & 0x0008 != 0 {
        at = at.checked_add(2)?;
    }
    if at > tail.len() {
        return None;
    }
    Some((flags, gid, at))
}

/// FDSelect selects Font DICTs by a `u8` index.
const FONT_DICTS_MAX: usize = 256;

/// A CFF program's layout, its SID → first GID map (for `seac`) and, for a CID-keyed font, each
/// Font DICT's local subroutines once resolved.
pub(crate) struct CffGuard<'a> {
    layout: CffLayout<'a>,
    seac: HashMap<u16, u16>,
    fd_subrs: Vec<OnceCell<FdSubrs<'a>>>,
}

impl<'a> CffGuard<'a> {
    /// `None` when the program cannot be laid out, or disagrees with ttf-parser's glyph count.
    pub fn new(data: &'a [u8], glyphs: u16) -> Option<CffGuard<'a>> {
        let layout = CffLayout::parse(data)?;
        if layout.number_of_glyphs != glyphs {
            return None;
        }
        let mut seac = HashMap::new();
        if let Some(sids) = layout.gid_to_sid() {
            for (gid, sid) in sids.iter().enumerate().skip(1) {
                if let (Some(sid), Ok(gid)) = (sid, u16::try_from(gid)) {
                    seac.entry(*sid).or_insert(gid);
                }
            }
        }
        let fds = if layout.is_cid() { FONT_DICTS_MAX } else { 0 };
        Some(CffGuard {
            layout,
            seac,
            fd_subrs: (0..fds).map(|_| OnceCell::new()).collect(),
        })
    }

    /// The local subroutines `glyph` runs and the units finding them costs: the FDSelect ranges
    /// walked; for a CID-keyed font also the Font and Private DICT bytes ttf-parser walks for
    /// this glyph, plus this guard's own walk of them the first time that Font DICT is used.
    fn local_subrs(&self, glyph: u16) -> (Option<Index<'a>>, u64) {
        let (fd, walked) = match self.layout.local_subrs(glyph) {
            LocalSubrs::Font(index) => return (Some(index), 0),
            LocalSubrs::FontDict(fd, walked) => (fd, walked),
        };
        let Some((fd, cell)) = fd.and_then(|fd| Some((fd, self.fd_subrs.get(usize::from(fd))?)))
        else {
            return (None, walked);
        };
        let (found, first_walk) = match cell.get() {
            Some(found) => (*found, 0),
            None => {
                let found = self.layout.fd_local_subrs(fd);
                let _ = cell.set(found); // empty until now: never refused
                (found, found.dict_bytes)
            }
        };
        let units = walked
            .saturating_add(found.dict_bytes)
            .saturating_add(first_walk);
        (found.subrs, units)
    }

    pub fn layout(&self) -> &CffLayout<'a> {
        &self.layout
    }

    /// Whether ttf-parser may outline `gid` (within budget, ends in `endchar`, no error on the
    /// way); the units are charged to `meter` either way. An exhausted meter walks nothing.
    pub fn check(&self, gid: u16, meter: &mut WorkMeter) -> bool {
        if meter.exhausted() {
            return false;
        }
        let mut scan = Type2 {
            guard: self,
            glyph: gid,
            local: None,
            local_resolved: false,
            stack: Vec::with_capacity(CFF_STACK_MAX),
            width: false,
            stems: 0,
            has_endchar: false,
            has_seac: false,
            ops: 0,
            work: Work::new(meter),
        };
        let ok = match self.layout.char_strings.get(u32::from(gid)) {
            Some(code) => scan.run(code, 0).is_ok() && scan.has_endchar,
            None => false,
        };
        scan.work.settle(meter) && ok
    }
}

/// One Type2 charstring walk (the state `CharStringParserContext` + `CharStringParser` keep that
/// decides which bytes run next).
struct Type2<'g, 'a> {
    guard: &'g CffGuard<'a>,
    glyph: u16,
    local: Option<Index<'a>>,
    local_resolved: bool,
    stack: Vec<f32>,
    width: bool,
    stems: u32,
    has_endchar: bool,
    has_seac: bool,
    ops: u32,
    work: Work,
}

impl<'a> Type2<'_, 'a> {
    fn push(&mut self, v: f32) -> Result<(), ()> {
        if self.stack.len() >= CFF_STACK_MAX {
            return Err(());
        }
        self.stack.push(v);
        Ok(())
    }

    fn operator(&mut self) -> Result<(), ()> {
        self.ops += 1;
        if self.ops > CFF_OPS_PER_GLYPH_MAX {
            return Err(());
        }
        Ok(())
    }

    /// `width` rule of the moveto operators: an extra leading argument is the width.
    fn moveto(&mut self, args: usize) {
        if self.stack.len() == args + 1 {
            self.width = true;
        }
        self.stack.clear();
    }

    fn run(&mut self, code: &'a [u8], depth: u8) -> Result<(), ()> {
        let byte = |at: usize| code.get(at).copied().ok_or(());
        let mut pos = 0usize;
        while let Some(&op) = code.get(pos) {
            pos += 1;
            self.work.tick(1)?;
            match op {
                0 | 2 | 9 | 13 | 15 | 16 | 17 => return Err(()),
                1 | 3 | 18 | 23 => {
                    self.operator()?;
                    let mut len = self.stack.len();
                    if len % 2 == 1 && !self.width {
                        self.width = true;
                        len -= 1;
                    }
                    self.stems = self.stems.saturating_add((len / 2) as u32);
                    self.stack.clear();
                }
                4 | 22 => {
                    self.operator()?;
                    self.moveto(1);
                }
                21 => {
                    self.operator()?;
                    self.moveto(2);
                }
                5..=8 | 24..=27 | 30 | 31 => {
                    self.operator()?;
                    self.stack.clear();
                }
                10 | 29 => {
                    self.operator()?;
                    self.call(op == 29, depth)?;
                    if self.has_endchar && !self.has_seac {
                        return if pos < code.len() { Err(()) } else { Ok(()) };
                    }
                }
                11 => {
                    self.operator()?;
                    return Ok(());
                }
                12 => {
                    self.operator()?;
                    let op2 = byte(pos)?;
                    pos += 1;
                    self.work.tick(1)?;
                    if !(34..=37).contains(&op2) {
                        return Err(()); // only the flex operators are supported there
                    }
                    self.stack.clear();
                }
                14 => {
                    self.operator()?;
                    self.endchar(depth)?;
                    if pos < code.len() {
                        return Err(());
                    }
                    self.has_endchar = true;
                    return Ok(());
                }
                19 | 20 => {
                    self.operator()?;
                    let mut len = self.stack.len();
                    self.stack.clear();
                    if len % 2 == 1 {
                        len -= 1;
                        self.width = true;
                    }
                    self.stems = self.stems.saturating_add((len / 2) as u32);
                    let mask = usize::try_from((u64::from(self.stems) + 7) >> 3).map_err(|_| ())?;
                    pos = pos.saturating_add(mask);
                }
                28 => {
                    let v = i16::from_be_bytes([byte(pos)?, byte(pos + 1)?]);
                    pos += 2;
                    self.work.tick(2)?;
                    self.push(f32::from(v))?;
                }
                32..=246 => self.push(f32::from(i16::from(op) - 139))?,
                247..=250 => {
                    let b1 = i16::from(byte(pos)?);
                    pos += 1;
                    self.work.tick(1)?;
                    self.push(f32::from((i16::from(op) - 247) * 256 + b1 + 108))?;
                }
                251..=254 => {
                    let b1 = i16::from(byte(pos)?);
                    pos += 1;
                    self.work.tick(1)?;
                    self.push(f32::from(-(i16::from(op) - 251) * 256 - b1 - 108))?;
                }
                255 => {
                    let v = i32::from_be_bytes([
                        byte(pos)?,
                        byte(pos + 1)?,
                        byte(pos + 2)?,
                        byte(pos + 3)?,
                    ]);
                    pos += 4;
                    self.work.tick(4)?;
                    self.push(v as f32 / 65536.0)?;
                }
            }
        }
        Ok(())
    }

    /// `callsubr` (local) or `callgsubr` (global): pops the biased index and runs the subroutine.
    fn call(&mut self, global: bool, depth: u8) -> Result<(), ()> {
        if self.stack.is_empty() || depth == CFF_DEPTH_LIMIT {
            return Err(());
        }
        let subrs = if global {
            self.guard.layout.global_subrs
        } else {
            if !self.local_resolved {
                let (local, units) = self.guard.local_subrs(self.glyph);
                self.work.tick(units)?;
                self.local = local;
                self.local_resolved = true;
            }
            self.local.ok_or(())?
        };
        let index = self.stack.pop().ok_or(())?;
        let sub = subrs.get(subr_index(index, subrs.len())?).ok_or(())?;
        self.run(sub, depth + 1)
    }

    /// `endchar`: with 4 arguments (5 before the width) it is `seac`, which runs the base and
    /// accent glyphs' charstrings (`seac_code_to_glyph_id` through the charset).
    fn endchar(&mut self, depth: u8) -> Result<(), ()> {
        let len = self.stack.len();
        if len == 4 || (!self.width && len == 5) {
            let code_of = |v: Option<f32>| -> Option<u8> {
                let v = v?;
                (v >= i32::MIN as f32 && v < i32::MAX as f32)
                    .then(|| u8::try_from(v as i32).ok())
                    .flatten()
            };
            let layout = &self.guard.layout;
            let walk = layout.charset_walk();
            let accent = code_of(self.stack.pop())
                .and_then(|c| seac_glyph(layout.charset, &self.guard.seac, c))
                .ok_or(())?;
            self.work.tick(walk)?;
            let base = code_of(self.stack.pop())
                .and_then(|c| seac_glyph(layout.charset, &self.guard.seac, c))
                .ok_or(())?;
            self.work.tick(walk)?;
            self.stack.truncate(self.stack.len().saturating_sub(2)); // dy dx
            if !self.width && !self.stack.is_empty() {
                self.stack.pop();
                self.width = true;
            }
            self.has_seac = true;
            if depth == CFF_DEPTH_LIMIT {
                return Err(());
            }
            let base = layout.char_strings.get(u32::from(base)).ok_or(())?;
            self.run(base, depth + 1)?;
            let accent = layout.char_strings.get(u32::from(accent)).ok_or(())?;
            self.run(accent, depth + 1)?;
        } else if len == 1 && !self.width {
            self.stack.pop();
            self.width = true;
        }
        Ok(())
    }
}

/// `conv_subroutine_index` with `calc_subroutine_bias(count)`.
fn subr_index(index: f32, count: u32) -> Result<u32, ()> {
    let bias: i32 = if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    };
    if !(index >= i32::MIN as f32 && index < i32::MAX as f32) {
        return Err(());
    }
    let index = (index as i32).checked_add(bias).ok_or(())?;
    u32::try_from(index).map_err(|_| ())
}
