//! A5 render masks (SPEC §B.15): the glyph boxes, in em, that G-RENDER excludes from its pixel
//! comparison, one per old and new glyph of an edited run.
//!
//! Per glyph, the box is the program's own outline box when the font program can outline the
//! glyph (TrueType `glyf`, CFF, OpenType; the same bounded pre-check as T2's presence proof runs
//! first), its units scale is unambiguous (`cff_scale.rs`: a CFF program's Font DICT matrices)
//! and the box lies within [−2, 3] em; otherwise it is built from the font's metrics. The
//! fallback takes its height from the
//! ascent, the descent and `/FontBBox`, but its width only from the glyph's own advance plus a
//! bounded overhang: `/FontBBox` is font-wide (Arial's reaches 2 em right of every origin), and a
//! mask that wide would hide a follower on the same line that a width-model error moved (B13).
//! Stroked text (`Tr` 1/2) grows each box by half the line width (times the miter limit for
//! miter joins) in user space.

use super::cff_scale::{em_per_unit, Host};
use crate::pdf_engine::text_edit::context::resolve;
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::fonts::glyph_budget::WorkMeter;
use crate::pdf_engine::text_edit::fonts::program::Outlines;
use crate::pdf_engine::text_edit::fonts::{number_of, Code, FontKey, FontModel};
use crate::pdf_engine::text_edit::limits::{FONT_PROGRAM_MAX_DECODED, PAGE_DECODE_BUDGET};
use crate::pdf_engine::text_edit::state::StateDigest;
use lopdf::{Dictionary, Document, Object, Stream};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use ttf_parser::{Face, GlyphId, OutlineBuilder, Rect, Tag};

/// Ascent/descent (em) when the font gives none.
const FALLBACK_ASCENT: f64 = 0.8;
const FALLBACK_DESCENT: f64 = -0.2;
/// The em box of §B.15 when nothing better is known: its height bounds every fallback mask.
const MASK_EM: [f64; 4] = [-0.1, -0.3, 1.1, 1.0];
/// Fallback margin around the advance and the ascent/descent (em).
const MASK_MARGIN_EM: f64 = 0.1;
/// Largest fallback overhang past the advance (or before the origin) of an upright glyph (em).
const UPRIGHT_REACH_EM: f64 = 0.2;
/// Largest fallback overhang of an italic glyph (em).
const ITALIC_REACH_EM: f64 = 0.35;
/// Bounds of any font or glyph box used for a mask (em). A program glyph box beyond them is not
/// taken (a wrong units scale or a hostile program: the fallback is used instead of a clamped
/// box, which could be empty or several em wide); `/FontBBox` values are clamped to them, as they
/// only set the fallback's height.
const BOX_EM: [f64; 2] = [-2.0, 3.0];
/// Largest miter factor applied to a stroked glyph's half line width.
const MITER_FACTOR_MAX: f64 = 10.0;

/// Ignores the outline; ttf-parser computes the box while it walks it.
struct NoPath;

impl OutlineBuilder for NoPath {
    fn move_to(&mut self, _x: f32, _y: f32) {}
    fn line_to(&mut self, _x: f32, _y: f32) {}
    fn quad_to(&mut self, _x1: f32, _y1: f32, _x: f32, _y: f32) {}
    fn curve_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _x: f32, _y: f32) {}
    fn close(&mut self) {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProgramKind {
    /// `FontFile2`, or `FontFile3 /OpenType`.
    Sfnt,
    /// `FontFile3 /Type1C` or `/CIDFontType0C`.
    BareCff,
}

/// What one font contributes to masks.
#[derive(Default)]
struct FontBoxes {
    /// `/FontBBox` in em (clamped).
    bbox: Option<[f64; 4]>,
    /// The decoded program, when it is one we can outline (`None` once known unusable).
    program: Option<(ProgramKind, Vec<u8>)>,
    /// Outline boxes per GID in em (`None`: the program could not outline it).
    glyphs: HashMap<u16, Option<[f64; 4]>>,
}

/// Glyph boxes of the fonts of one page plan, loaded on first use. Program decoding and outline
/// work are bounded by one page decode budget; past it every glyph uses the fallback box.
pub(crate) struct MaskFonts<'d> {
    doc: &'d Document,
    budget: DecodeBudget,
    meter: WorkMeter,
    fonts: HashMap<FontKey, FontBoxes>,
}

fn dict_of<'d>(doc: &'d Document, obj: &'d Object) -> Option<&'d Dictionary> {
    match resolve(doc, obj)? {
        (_, Object::Dictionary(d)) => Some(d),
        _ => None,
    }
}

/// The dictionary holding the font's `/FontDescriptor`: the font's own, or for a Type0 font its
/// descendant's.
fn described_font<'d>(doc: &'d Document, key: FontKey) -> Option<&'d Dictionary> {
    let FontKey::Indirect(id) = key else {
        return None;
    };
    let font = dict_of(doc, doc.objects.get(&id)?)?;
    if font.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) != Some(&b"Type0"[..]) {
        return Some(font);
    }
    let (_, Object::Array(kids)) = resolve(doc, font.get(b"DescendantFonts").ok()?)? else {
        return None;
    };
    dict_of(doc, kids.first()?)
}

fn descriptor<'d>(doc: &'d Document, key: FontKey) -> Option<&'d Dictionary> {
    dict_of(doc, described_font(doc, key)?.get(b"FontDescriptor").ok()?)
}

/// The `/FontBBox` (em) of the font behind `key`: its descriptor's, or for a Type0 font its
/// descendant's. `None` when absent or not four finite numbers.
pub(crate) fn font_bbox(doc: &Document, key: FontKey) -> Option<[f64; 4]> {
    let (_, Object::Array(items)) = resolve(doc, descriptor(doc, key)?.get(b"FontBBox").ok()?)?
    else {
        return None;
    };
    let v: Vec<f64> = items
        .iter()
        .map(|i| resolve(doc, i).and_then(|(_, o)| number_of(o)))
        .collect::<Option<Vec<f64>>>()?;
    let [a, b, c, d] = v.as_slice() else {
        return None;
    };
    let em = |n: f64| (n / 1000.0).clamp(BOX_EM[0], BOX_EM[1]);
    Some([em(a.min(*c)), em(b.min(*d)), em(a.max(*c)), em(b.max(*d))])
}

/// The program stream of the font behind `key` when it is one ttf-parser outlines.
fn program_stream(doc: &Document, key: FontKey) -> Option<(ProgramKind, &Stream)> {
    let desc = descriptor(doc, key)?;
    let stream = |k: &[u8]| match resolve(doc, desc.get(k).ok()?)? {
        (_, Object::Stream(s)) => Some(s),
        _ => None,
    };
    if let Some(s) = stream(b"FontFile2") {
        return Some((ProgramKind::Sfnt, s));
    }
    let s = stream(b"FontFile3")?;
    match s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) {
        Some(b"OpenType") => Some((ProgramKind::Sfnt, s)),
        Some(b"Type1C" | b"CIDFontType0C") => Some((ProgramKind::BareCff, s)),
        _ => None,
    }
}

/// A glyph's outline box in em, or `None` when any edge lies outside `BOX_EM`.
fn em_rect(r: Rect, scale: f64) -> Option<[f64; 4]> {
    let b = [r.x_min, r.y_min, r.x_max, r.y_max].map(|n| f64::from(n) * scale);
    let inside = b.iter().all(|v| (BOX_EM[0]..=BOX_EM[1]).contains(v));
    (r.x_min <= r.x_max && r.y_min <= r.y_max && inside).then_some(b)
}

/// Outline boxes (em) of `gids` in `program`; a glyph the pre-check or ttf-parser refuses, or an
/// unusable program, gives `None`.
fn outline_boxes(
    kind: ProgramKind,
    data: &[u8],
    gids: &[u16],
    meter: &mut WorkMeter,
) -> Vec<(u16, Option<[f64; 4]>)> {
    let none = || gids.iter().map(|g| (*g, None)).collect();
    let face;
    let (outlines, scale) = match kind {
        ProgramKind::Sfnt => {
            let Ok(f) = Face::parse(data, 0) else {
                return none();
            };
            face = f;
            let upem = face.units_per_em();
            let outlines = Outlines::of_face(&face);
            // A `CFF ` table carries its own matrices; `glyf` units are 1/unitsPerEm.
            let scale = match &outlines {
                Outlines::Cff { .. } => face
                    .raw_face()
                    .table(Tag::from_bytes(b"CFF "))
                    .and_then(|cff| em_per_unit(cff, Host::OpenType(upem))),
                _ => (upem > 0).then(|| 1.0 / f64::from(upem)),
            };
            let Some(scale) = scale else {
                return none();
            };
            (outlines, scale)
        }
        ProgramKind::BareCff => {
            let (Some(outlines), Some(scale)) =
                (Outlines::of_cff(data), em_per_unit(data, Host::Bare))
            else {
                return none();
            };
            (outlines, scale)
        }
    };
    gids.iter()
        .map(|&gid| {
            let rect = match &outlines {
                Outlines::Glyf { guard, table } if guard.check(gid, meter) => {
                    table.outline(GlyphId(gid), &mut NoPath)
                }
                Outlines::Cff { guard, table } if guard.check(gid, meter) => {
                    table.outline(GlyphId(gid), &mut NoPath).ok()
                }
                _ => None,
            };
            (gid, rect.and_then(|r| em_rect(r, scale)))
        })
        .collect()
}

/// The entry of `key`, created on first use (its `/FontBBox`, and its program decoded under
/// `budget` when ttf-parser can outline it).
fn font_entry<'f>(
    doc: &Document,
    budget: &mut DecodeBudget,
    fonts: &'f mut HashMap<FontKey, FontBoxes>,
    key: FontKey,
) -> &'f mut FontBoxes {
    fonts.entry(key).or_insert_with(|| FontBoxes {
        bbox: font_bbox(doc, key),
        program: program_stream(doc, key).and_then(|(kind, s)| {
            decode_stream(s, FONT_PROGRAM_MAX_DECODED, budget)
                .ok()
                .map(|data| (kind, data))
        }),
        glyphs: HashMap::new(),
    })
}

/// The fallback box (em) of a glyph of advance `w0` (§B.15 "else `/FontBBox`, else the em
/// box", with the horizontal reach bounded, see the module doc).
fn fallback_box(model: Option<&FontModel>, bbox: Option<[f64; 4]>, w0: f64) -> [f64; 4] {
    let (ascent, descent) = model.map_or((FALLBACK_ASCENT, FALLBACK_DESCENT), |m| {
        (m.ascent, m.descent)
    });
    let italic = model.is_some_and(|m| m.italic);
    let max_reach = if italic {
        ITALIC_REACH_EM
    } else {
        UPRIGHT_REACH_EM
    };
    let unknown = if italic { max_reach } else { MASK_MARGIN_EM };
    let (left, right) = match bbox {
        Some(b) => (
            (-b[0]).clamp(MASK_MARGIN_EM, max_reach),
            (b[2] - w0.max(0.0)).clamp(MASK_MARGIN_EM, max_reach),
        ),
        None => (unknown, unknown),
    };
    let mut y = [
        MASK_EM[1].min(descent - MASK_MARGIN_EM),
        MASK_EM[3].max(ascent + MASK_MARGIN_EM),
    ];
    if let Some(b) = bbox {
        y = [y[0].min(b[1]), y[1].max(b[3])];
    }
    let y = [y[0].max(BOX_EM[0]), y[1].min(BOX_EM[1])];
    [w0.min(0.0) - left, y[0], w0.max(0.0) + right, y[1]]
}

/// Half the line width of stroked text (`Tr` 1, 2) in user space, times the miter limit for
/// miter joins (≤ `MITER_FACTOR_MAX`); 0 for filled text.
pub(crate) fn stroke_pad(state: &StateDigest) -> f64 {
    if !matches!(state.text.tr, 1 | 2) {
        return 0.0;
    }
    let gs = &state.gs;
    // The CTM's largest singular value: how far a user-space line width reaches on the page.
    let c = state.ctm;
    let sum = c[0] * c[0] + c[1] * c[1] + c[2] * c[2] + c[3] * c[3];
    let det = c[0] * c[3] - c[1] * c[2];
    let scale = ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt();
    let miter = if gs.line_join == 0 {
        gs.miter_limit.clamp(1.0, MITER_FACTOR_MAX)
    } else {
        1.0
    };
    let pad = gs.line_width.abs() / 2.0 * miter * scale;
    if pad.is_finite() {
        pad
    } else {
        0.0
    }
}

impl<'d> MaskFonts<'d> {
    pub(crate) fn new(doc: &'d Document) -> MaskFonts<'d> {
        MaskFonts {
            doc,
            budget: DecodeBudget::new(PAGE_DECODE_BUDGET),
            meter: WorkMeter::new(PAGE_DECODE_BUDGET),
            fonts: HashMap::new(),
        }
    }

    /// Outlines, once per font and GID, every glyph of `glyphs` whose font program ttf-parser
    /// can draw (fonts in first-seen order, so the budget is spent deterministically).
    pub(crate) fn prepare<'m>(
        &mut self,
        glyphs: impl IntoIterator<Item = (&'m Arc<FontModel>, Code)>,
    ) {
        let mut order: Vec<FontKey> = Vec::new();
        let mut wanted: HashMap<FontKey, BTreeSet<u16>> = HashMap::new();
        for (model, code) in glyphs {
            let Some(gid) = model.info(code).and_then(|i| i.gid) else {
                continue;
            };
            let gids = wanted.entry(model.key).or_insert_with(|| {
                order.push(model.key);
                BTreeSet::new()
            });
            gids.insert(gid);
        }
        for key in order {
            let Some(gids) = wanted.remove(&key) else {
                continue;
            };
            let font = font_entry(self.doc, &mut self.budget, &mut self.fonts, key);
            let gids: Vec<u16> = gids
                .into_iter()
                .filter(|g| !font.glyphs.contains_key(g))
                .collect();
            let Some((kind, data)) = font.program.as_ref() else {
                continue;
            };
            if gids.is_empty() {
                continue;
            }
            for (gid, b) in outline_boxes(*kind, data, &gids, &mut self.meter) {
                font.glyphs.insert(gid, b);
            }
        }
    }

    /// The mask box (em, `[x0, y0, x1, y1]`) of `code` drawn with `font`, advance `w0` em: the
    /// program's outline box when `prepare` found one, else the bounded fallback.
    pub(crate) fn glyph_box(
        &mut self,
        font: Option<&Arc<FontModel>>,
        code: Code,
        w0: f64,
    ) -> [f64; 4] {
        self.boxes(font, code, w0).0
    }

    /// The tightest box (em) known to hold the glyph's ink: the program's outline box when
    /// `prepare` found one, else the fallback narrowed to the advance (`advance_box`). A5 checks
    /// the pixels of the glyphs no edit changes outside it (review-verify HIGH-A): the fallback's
    /// margin would hide a narrow follower drawn right after a removed glyph.
    pub(crate) fn ink_box(
        &mut self,
        font: Option<&Arc<FontModel>>,
        code: Code,
        w0: f64,
    ) -> [f64; 4] {
        self.boxes(font, code, w0).1
    }

    /// (`glyph_box`, `ink_box`).
    fn boxes(
        &mut self,
        font: Option<&Arc<FontModel>>,
        code: Code,
        w0: f64,
    ) -> ([f64; 4], [f64; 4]) {
        let Some(model) = font else {
            let b = fallback_box(None, None, w0);
            return (b, advance_box(b, None, w0));
        };
        let gid = model.info(code).and_then(|i| i.gid);
        let entry = font_entry(self.doc, &mut self.budget, &mut self.fonts, model.key);
        match gid.and_then(|g| entry.glyphs.get(&g).copied().flatten()) {
            Some(b) => (b, b),
            None => {
                let b = fallback_box(Some(model), entry.bbox, w0);
                (b, advance_box(b, Some(model), w0))
            }
        }
    }
}

/// The fallback box `b` (em) narrowed to the advance `w0`, wider by `ITALIC_REACH_EM` on each side
/// in an italic font; its height stays the fallback's.
fn advance_box(b: [f64; 4], model: Option<&FontModel>, w0: f64) -> [f64; 4] {
    let reach = if model.is_some_and(|m| m.italic) {
        ITALIC_REACH_EM
    } else {
        0.0
    };
    [w0.min(0.0) - reach, b[1], w0.max(0.0) + reach, b[3]]
}
