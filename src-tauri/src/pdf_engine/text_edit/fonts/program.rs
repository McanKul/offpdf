//! Glyph presence in embedded TrueType, OpenType and CFF programs (SPEC §A.2, §B.9.5).
//! ttf-parser is panic-free by design; every lookup here returns `None` instead of failing.
//!
//! A glyph counts as drawn only when its outline has at least one line or curve segment (a
//! bounding box alone, e.g. from a lone `moveto`, draws nothing). ttf-parser outlines a glyph only
//! after the bounded pre-check of `glyph_budget.rs` has walked it within budget.

use super::cff_layout::CffLayout;
use super::encodings::mac_roman_code;
use super::glyph_budget::{CffGuard, GlyfGuard, WorkMeter};
use crate::pdf_engine::text_edit::reasons::TextReason;
use std::collections::{BTreeMap, HashMap, HashSet};
use ttf_parser::{cff, cmap::Subtable, glyf, Face, GlyphId, OutlineBuilder, PlatformId, Tag};

/// Counts drawing segments of an outline.
#[derive(Default)]
struct SegCount(usize);

impl OutlineBuilder for SegCount {
    fn move_to(&mut self, _x: f32, _y: f32) {}
    fn line_to(&mut self, _x: f32, _y: f32) {
        self.0 = self.0.saturating_add(1);
    }
    fn quad_to(&mut self, _x1: f32, _y1: f32, _x: f32, _y: f32) {
        self.0 = self.0.saturating_add(1);
    }
    fn curve_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _x: f32, _y: f32) {
        self.0 = self.0.saturating_add(1);
    }
    fn close(&mut self) {}
}

/// Where a program's outlines live, each with its bounded pre-check.
pub enum Outlines<'a> {
    /// `glyf` (outlined at the default instance: `gvar` deltas are zero there, so the `gvar`
    /// path ttf-parser's `Face::outline_glyph` would take draws the same segments).
    Glyf {
        guard: GlyfGuard<'a>,
        table: glyf::Table<'a>,
    },
    /// A CFF program (bare, or an OpenType `CFF ` table).
    Cff {
        guard: CffGuard<'a>,
        table: cff::Table<'a>,
    },
    /// Only a `CFF2` table: its charstrings are not pre-checked (`FONT_PROGRAM_UNSUPPORTED`).
    Cff2Only,
    /// Both a `glyf` and a `CFF `/`CFF2` table (`FONT_PROGRAM_UNSUPPORTED`): an OpenType font has
    /// one or the other, and viewers pick by the sfnt tag (FreeType, Poppler and pdf.js draw an
    /// `OTTO` program's CFF outlines), so a presence proven on `glyf` can certify a glyph the
    /// viewer draws empty (review T4 r2 M-2).
    Conflicting,
    /// A `glyf` or `CFF ` table the pre-check cannot lay out (`FONT_PROGRAM_UNREADABLE`).
    Unreadable,
    /// No outline table at all: no glyph draws.
    None,
}

impl<'a> Outlines<'a> {
    pub fn of_face(face: &Face<'a>) -> Outlines<'a> {
        let tables = face.tables();
        let has = |tag: &[u8; 4]| face.raw_face().table(Tag::from_bytes(tag)).is_some();
        if has(b"glyf") && (has(b"CFF ") || has(b"CFF2")) {
            return Outlines::Conflicting;
        }
        if let Some(table) = tables.glyf {
            return match GlyfGuard::new(face) {
                Some(guard) => Outlines::Glyf { guard, table },
                None => Outlines::Unreadable,
            };
        }
        if let Some(table) = tables.cff {
            return Self::of_cff_table(table, face.raw_face().table(Tag::from_bytes(b"CFF ")));
        }
        if tables.cff2.is_some() {
            return Outlines::Cff2Only;
        }
        Outlines::None
    }

    /// A bare CFF program (`FontFile3 /Type1C`, `/CIDFontType0C`).
    pub fn of_cff(data: &'a [u8]) -> Option<Outlines<'a>> {
        let table = cff::Table::parse(data)?;
        Some(Self::of_cff_table(table, Some(data)))
    }

    fn of_cff_table(table: cff::Table<'a>, data: Option<&'a [u8]>) -> Outlines<'a> {
        match data.and_then(|d| CffGuard::new(d, table.number_of_glyphs())) {
            Some(guard) => Outlines::Cff { guard, table },
            None => Outlines::Unreadable,
        }
    }

    /// The font refusal this outline source implies, if any.
    pub fn refusal(&self) -> Option<TextReason> {
        match self {
            Outlines::Cff2Only | Outlines::Conflicting => Some(TextReason::FontProgramUnsupported),
            Outlines::Unreadable => Some(TextReason::FontProgramUnreadable),
            Outlines::Glyf { .. } | Outlines::Cff { .. } | Outlines::None => None,
        }
    }

    /// The CFF layout, when the outlines are CFF.
    pub fn cff_layout(&self) -> Option<&CffLayout<'a>> {
        match self {
            Outlines::Cff { guard, .. } => Some(guard.layout()),
            _ => None,
        }
    }

    /// Whether `gid` draws at least one segment. The pre-check runs first and charges `meter`;
    /// ttf-parser outlines only a glyph that passed it.
    pub fn drawn(&self, gid: u16, meter: &mut WorkMeter) -> bool {
        let mut count = SegCount::default();
        match self {
            Outlines::Glyf { guard, table } => {
                guard.check(gid, meter)
                    && table.outline(GlyphId(gid), &mut count).is_some()
                    && count.0 > 0
            }
            Outlines::Cff { guard, table } => {
                guard.check(gid, meter)
                    && table.outline(GlyphId(gid), &mut count).is_ok()
                    && count.0 > 0
            }
            Outlines::Cff2Only | Outlines::Conflicting | Outlines::Unreadable | Outlines::None => {
                false
            }
        }
    }
}

/// Outline results per GID for one font load: each glyph is outlined at most once, however
/// many codes map to it.
#[derive(Default)]
pub struct OutlineMemo(HashMap<u16, bool>);

impl OutlineMemo {
    pub fn get(&mut self, gid: u16, outline: impl FnOnce() -> bool) -> bool {
        *self.0.entry(gid).or_insert_with(outline)
    }
}

/// The outcome of resolving a code through every applicable strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GidLookup {
    /// No strategy produced a glyph.
    None,
    /// Every strategy that produced a glyph produced this one.
    Agree(u16),
    /// Two strategies produced different glyphs (never drawable).
    Disagree,
}

impl GidLookup {
    fn add(self, gid: Option<GlyphId>) -> GidLookup {
        match (self, gid) {
            (_, None) => self,
            (GidLookup::None, Some(g)) => GidLookup::Agree(g.0),
            (GidLookup::Agree(a), Some(g)) if a == g.0 => self,
            _ => GidLookup::Disagree,
        }
    }
}

fn subtable<'a>(face: &Face<'a>, platform: PlatformId, encoding: u16) -> Option<Subtable<'a>> {
    face.tables()
        .cmap?
        .subtables
        .into_iter()
        .find(|s| s.platform_id == platform && s.encoding_id == encoding)
}

/// Glyph name → GID through a `post` format 2 table, built once per face in O(glyphs).
/// ttf-parser's own `glyph_index_by_name` scans the 258 Macintosh names and every glyph index
/// per call (and `glyph_name` walks the custom names), which a 256-code font load repeats.
/// Same answers for conforming tables: a name used through a standard Macintosh index maps to
/// the first such glyph; a custom name to the first glyph using its first position.
pub struct PostNames {
    by_name: HashMap<String, u16>,
}

impl PostNames {
    pub fn new(face: &Face<'_>) -> PostNames {
        let mut by_name = HashMap::new();
        if let Some(data) = face.raw_face().table(Tag::from_bytes(b"post")) {
            Self::fill(face, data, &mut by_name);
        }
        PostNames { by_name }
    }

    fn fill(face: &Face<'_>, data: &[u8], by_name: &mut HashMap<String, u16>) -> Option<()> {
        if data.get(..4)? != [0, 2, 0, 0] {
            return None; // only version 2.0 carries names
        }
        let count = usize::from(u16::from_be_bytes([*data.get(32)?, *data.get(33)?]));
        let names_at = 34usize.checked_add(count.checked_mul(2)?)?;
        let indexes: Vec<u16> = data
            .get(34..names_at)?
            .chunks_exact(2)
            .map(|p| {
                u16::from_be_bytes([
                    p.first().copied().unwrap_or(0),
                    p.get(1).copied().unwrap_or(0),
                ])
            })
            .collect();
        let mut first_gid_of_index: HashMap<u16, u16> = HashMap::new();
        for (gid, index) in indexes.iter().enumerate() {
            let gid = u16::try_from(gid).ok()?;
            first_gid_of_index.entry(*index).or_insert(gid);
            if *index < 258 {
                if let Some(name) = face.glyph_name(GlyphId(gid)) {
                    by_name.entry(name.to_string()).or_insert(gid);
                }
            }
        }
        // Custom names: Pascal strings; the list ends at an empty, truncated or non-UTF-8 one.
        let mut seen: HashSet<String> = HashSet::new();
        let mut at = names_at;
        let mut position: u16 = 0;
        while let Some(len) = data.get(at).map(|l| usize::from(*l)) {
            let name = data.get(at + 1..at + 1 + len).filter(|_| len > 0)?;
            let name = std::str::from_utf8(name).ok()?;
            at += 1 + len;
            if seen.insert(name.to_string()) && !by_name.contains_key(name) {
                let index = 258u16.checked_add(position)?;
                if let Some(gid) = first_gid_of_index.get(&index) {
                    by_name.insert(name.to_string(), *gid);
                }
            }
            position = position.checked_add(1)?;
        }
        Some(())
    }

    pub fn gid(&self, name: &str) -> Option<GlyphId> {
        self.by_name.get(name).map(|g| GlyphId(*g))
    }
}

/// The lookup tables of one TrueType/OpenType face, found once per font load.
pub struct TrueTypeLookup<'f, 'a> {
    pub face: &'f Face<'a>,
    post: PostNames,
    /// OpenType-CFF: the CFF glyph names.
    cff_names: Option<CffNames>,
    win_unicode: Option<Subtable<'a>>,
    win_symbol: Option<Subtable<'a>>,
    mac_roman: Option<Subtable<'a>>,
}

impl<'f, 'a> TrueTypeLookup<'f, 'a> {
    pub fn new(face: &'f Face<'a>, outlines: &Outlines<'a>) -> Self {
        let cff_names = match (outlines.cff_layout(), face.tables().cff) {
            (Some(layout), Some(table)) => Some(CffNames::new(layout, &table)),
            _ => None,
        };
        TrueTypeLookup {
            face,
            post: PostNames::new(face),
            cff_names,
            win_unicode: subtable(face, PlatformId::Windows, 1),
            win_symbol: subtable(face, PlatformId::Windows, 0),
            mac_roman: subtable(face, PlatformId::Macintosh, 0),
        }
    }

    /// Non-symbolic TrueType (PDF 32000-1 §9.6.6.4): glyph name → Unicode (AGL) → (3,1) cmap;
    /// name → MacRoman code → (1,0) cmap; `post` (and, for OpenType-CFF, CFF) glyph names; the
    /// name's AGL character already resolved by the caller.
    pub fn by_name_char(&self, name: &str, name_char: Option<char>) -> GidLookup {
        let mut out = GidLookup::None;
        if name == ".notdef" {
            return out;
        }
        if let (Some(ch), Some(table)) = (name_char, self.win_unicode.as_ref()) {
            out = out.add(table.glyph_index(u32::from(ch)));
        }
        if let (Some(code), Some(table)) = (mac_roman_code(name), self.mac_roman.as_ref()) {
            out = out.add(table.glyph_index(u32::from(code)));
        }
        let by_name = self.post.gid(name).or_else(|| {
            self.cff_names
                .as_ref()
                .and_then(|names| names.gid(name))
                .map(GlyphId)
        });
        out.add(by_name)
    }

    /// Symbolic TrueType: (3,0) at `code`, `0xF000+code`, `0xF100+code`, `0xF200+code`; (1,0)
    /// at `code`.
    pub fn symbolic(&self, code: u8) -> GidLookup {
        let mut out = GidLookup::None;
        let code = u32::from(code);
        if let Some(table) = self.win_symbol.as_ref() {
            for base in [0, 0xF000, 0xF100, 0xF200] {
                out = out.add(table.glyph_index(base + code));
            }
        }
        if let Some(table) = self.mac_roman.as_ref() {
            out = out.add(table.glyph_index(code));
        }
        out
    }
}

/// Glyph name → first GID of a name-keyed CFF, built once per font load in O(glyphs) from the
/// charset walked once (ttf-parser's `glyph_name` walks a format 1/2 charset per call, and
/// `glyph_index_by_name` scans the 391 standard strings per call). Same answers as ttf-parser's
/// `glyph_name` for every GID. CID-keyed fonts have no names.
pub struct CffNames {
    by_name: HashMap<String, u16>,
}

impl CffNames {
    pub fn new(layout: &CffLayout<'_>, table: &cff::Table<'_>) -> CffNames {
        let mut by_name = HashMap::new();
        if layout.is_cid() {
            return CffNames { by_name };
        }
        match layout.gid_to_sid() {
            Some(sids) => {
                for (gid, sid) in sids.iter().enumerate() {
                    let name = sid.and_then(|s| layout.sid_name(s));
                    if let (Some(name), Ok(gid)) = (name, u16::try_from(gid)) {
                        by_name.entry(name.to_string()).or_insert(gid);
                    }
                }
            }
            // The predefined Expert charsets: ttf-parser's lookups are O(1) per glyph there.
            None => {
                for gid in 0..table.number_of_glyphs() {
                    if let Some(name) = table.glyph_name(GlyphId(gid)) {
                        by_name.entry(name.to_string()).or_insert(gid);
                    }
                }
            }
        }
        CffNames { by_name }
    }

    pub fn gid(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }
}

/// A CID-keyed CFF's CID → GID map (the inverse of its charset, first GID per CID), or `None`
/// for a name-keyed CFF (where GID = CID).
pub fn cff_cid_to_gid(layout: &CffLayout<'_>) -> Option<BTreeMap<u16, u16>> {
    if !layout.is_cid() {
        return None;
    }
    let mut map = BTreeMap::new();
    for (gid, cid) in layout.gid_to_sid()?.iter().enumerate() {
        if let (Some(cid), Ok(gid)) = (cid, u16::try_from(gid)) {
            map.entry(*cid).or_insert(gid);
        }
    }
    Some(map)
}

/// Font-wide vertical metrics of a TrueType/OpenType program: (ascent, descent) in em, from
/// `hhea` (OS/2 typographic values when `hhea` is empty).
pub fn face_metrics(face: &Face<'_>) -> Option<(f64, f64)> {
    let upem = f64::from(face.units_per_em());
    if upem <= 0.0 {
        return None;
    }
    let (asc, desc) = match (face.ascender(), face.descender()) {
        (0, 0) => (face.typographic_ascender()?, face.typographic_descender()?),
        pair => pair,
    };
    Some((f64::from(asc) / upem, f64::from(desc) / upem))
}

#[cfg(test)]
impl TrueTypeLookup<'_, '_> {
    /// `by_name_char` with the name's AGL character looked up here.
    pub fn by_name(&self, name: &str) -> GidLookup {
        self.by_name_char(name, super::encodings::glyph_name_char(name))
    }
}
