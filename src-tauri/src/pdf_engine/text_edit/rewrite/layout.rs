//! Planner steps 9–10 (SPEC §B.12): advances of the new unit list in the written numbers, the
//! column-gap absorption, the pen compensation, the f32 precision guard, the fit policy (§A.8),
//! and every expectation the re-walk checks: glyph origins, text, new ink box, caret offsets and
//! the render masks of the independent check.

use super::diff::Region;
use super::masks::{stroke_pad, MaskFonts};
use super::{problem, NewUnit, StyleTarget};
use crate::pdf_engine::text_edit::encode::num;
use crate::pdf_engine::text_edit::fonts::{Code, FontModel, TypingSurface};
use crate::pdf_engine::text_edit::geometry::{bbox_of, Matrix};
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::{
    COLUMN_GAP_EM, DRIFT_TOLERANCE_PT, F32_DRIFT_GUARD_PT, NEGLIGIBLE_KERN, OVERLAP_WARN_TOL_PT,
    SYNTH_SPACE_EM,
};
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode as P, TextWarningCode};
use crate::pdf_engine::text_edit::runs::{PageModel, TextRun, Unit};
use crate::pdf_engine::text_edit::verify::{decoded_text, TextItem};
use crate::pdf_engine::text_edit::walker::ShowRecord;
use std::sync::Arc;

const FALLBACK_ASCENT: f64 = 0.8;
const FALLBACK_DESCENT: f64 = -0.2;

/// One unit of the new run, resolved.
#[derive(Debug, Clone)]
pub(crate) enum Planned {
    Glyph {
        code: Code,
        /// Font resource name (empty for an ExtGState font).
        font_res: Vec<u8>,
        font_hash: u64,
        model: Option<Arc<FontModel>>,
        width1000: f64,
        text: String,
        /// Kept: (unit index, member, glyph index in that member's record).
        kept: Option<(usize, usize, usize)>,
    },
    Kern {
        value: f64,
        /// The number as written; `None` = the original TJ bytes at `original`.
        written: Option<String>,
        original: Option<Span>,
        synth: bool,
        /// Kept: the unit index in the run.
        kept: Option<usize>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedUnit {
    pub unit: Planned,
    pub region: Region,
}

/// Everything layout decides for one run.
pub(crate) struct Laid {
    pub units: Vec<PlannedUnit>,
    /// The compensation number as written (`None` when negligible).
    pub compensation: Option<String>,
    /// Per member (index 0 is the primary, always `None`): the absorbed member's TJ number.
    pub absorbed_numbers: Vec<Option<String>>,
    pub text: String,
    pub glyph_origins: Vec<(f64, f64)>,
    pub prefix_glyphs: usize,
    pub suffix_glyphs: usize,
    pub shift_user: (f64, f64),
    pub unshifted_from: Option<usize>,
    pub delta_pt: f64,
    pub new_rect: [f64; 4],
    pub caret_offsets: Vec<f64>,
    pub mask_boxes: Vec<[f64; 4]>,
    /// Per old glyph: the tightest box known to hold its ink (`MaskFonts::ink_box`).
    pub old_ink_boxes: Vec<[f64; 4]>,
    pub glyphs_changed: bool,
    pub warnings: Vec<TextWarningCode>,
}

fn record<'m>(model: &'m PageModel, run: &TextRun, member: usize) -> Option<&'m ShowRecord> {
    run.members
        .get(member)
        .and_then(|i| model.walk.records.get(*i))
}

/// Resolves the new unit list against the run, its records and the typing surface.
fn resolve(
    model: &PageModel,
    run: &TextRun,
    surface: &TypingSurface,
    new_units: &[(NewUnit, Region)],
) -> Result<Vec<PlannedUnit>, EditProblem> {
    let internal = || problem(P::EditVerifyFailed, "unit out of range");
    let mut out = Vec::with_capacity(new_units.len());
    for (nu, region) in new_units {
        let unit = match nu {
            NewUnit::Kept(i) => match run.units.get(*i).ok_or_else(internal)? {
                Unit::Glyph {
                    member,
                    rec,
                    glyph,
                    code,
                    font_res,
                    font_hash,
                    text,
                    width1000,
                    ..
                } => Planned::Glyph {
                    code: *code,
                    font_res: font_res.clone().unwrap_or_default(),
                    font_hash: *font_hash,
                    model: model.walk.records.get(*rec).and_then(|r| r.font.clone()),
                    width1000: *width1000,
                    text: text.clone(),
                    kept: Some((*i, *member, *glyph)),
                },
                Unit::Kern {
                    value,
                    src,
                    synth_space,
                } => match src {
                    crate::pdf_engine::text_edit::runs::KernSrc::Tj { span } => Planned::Kern {
                        value: *value,
                        written: None,
                        original: Some(span.clone()),
                        synth: *synth_space,
                        kept: Some(*i),
                    },
                    crate::pdf_engine::text_edit::runs::KernSrc::Gap => {
                        let (s, v) = num(*value)?;
                        Planned::Kern {
                            value: v,
                            written: Some(s),
                            original: None,
                            synth: *synth_space,
                            kept: Some(*i),
                        }
                    }
                },
            },
            NewUnit::Code { font, code } => {
                let (name, m) = surface.fonts.get(*font).ok_or_else(internal)?;
                let text = m
                    .text(*code)
                    .map(str::to_string)
                    .ok_or_else(|| problem(P::EditVerifyFailed, "encoded code has no text"))?;
                Planned::Glyph {
                    code: *code,
                    font_res: name.clone(),
                    font_hash: m.content_hash,
                    model: Some(Arc::clone(m)),
                    width1000: m.width(*code),
                    text,
                    kept: None,
                }
            }
            NewUnit::KernSpace(v) => {
                let (s, v) = num(*v)?;
                Planned::Kern {
                    value: v,
                    written: Some(s),
                    original: None,
                    synth: true,
                    kept: None,
                }
            }
        };
        out.push(PlannedUnit {
            unit,
            region: *region,
        });
    }
    Ok(out)
}

/// Text-space advance (Th excluded) of a unit with size `tf` and character spacing `tc`.
fn advance(u: &Planned, tf: f64, tc: f64, tw: f64) -> f64 {
    match u {
        Planned::Glyph {
            code, width1000, ..
        } => {
            let word = code.len == 1 && code.value == 32;
            width1000 / 1000.0 * tf + tc + if word { tw } else { 0.0 }
        }
        Planned::Kern { value, .. } => -value / 1000.0 * tf,
    }
}

/// The original advance of a run unit (the run's own size and spacing).
fn original_advance(u: &Unit, run: &TextRun) -> f64 {
    match u {
        Unit::Glyph {
            code, width1000, ..
        } => {
            let word = code.len == 1 && code.value == 32;
            width1000 / 1000.0 * run.tfs + run.tc + if word { run.tw } else { 0.0 }
        }
        Unit::Kern { value, .. } => -value / 1000.0 * run.tfs,
    }
}

/// `|f32(v) − v|` scaled to points by `scale` stays within the guard (§B.12 step 9).
fn f32_ok(v: f64, scale: f64) -> bool {
    let err = (f64::from(v as f32) - v).abs();
    err * scale.abs() <= F32_DRIFT_GUARD_PT
}

fn em_box(origin: (f64, f64), l: &Matrix, x: [f64; 2], y: [f64; 2]) -> [f64; 4] {
    let mut pts = Vec::with_capacity(4);
    for gx in x {
        for gy in y {
            pts.push((
                origin.0 + gx * l[0] + gy * l[2],
                origin.1 + gx * l[1] + gy * l[3],
            ));
        }
    }
    bbox_of(&pts).unwrap_or([origin.0, origin.1, origin.0, origin.1])
}

fn metrics(model: Option<&Arc<FontModel>>) -> (f64, f64) {
    model.map_or((FALLBACK_ASCENT, FALLBACK_DESCENT), |m| {
        (m.ascent, m.descent)
    })
}

/// `b` (user space) grown by `pad` on every side.
fn grow(b: [f64; 4], pad: f64) -> [f64; 4] {
    [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]
}

/// The glyph-space → user linear map of `rec` scaled to size `tf`.
fn scaled_linear(rec: &ShowRecord, tf: f64) -> Matrix {
    let t = rec.text_to_user;
    let k = if rec.before.text.tfs != 0.0 {
        tf / rec.before.text.tfs
    } else {
        1.0
    };
    [t[0] * k, t[1] * k, t[2] * k, t[3] * k, 0.0, 0.0]
}

/// Column-gap absorption (§B.12 step 9): the first kept-suffix kern of at least one em takes up
/// the width change, provided it still reads as a space. Returns the planned index it adjusted.
fn absorb_column_gap(
    units: &mut [PlannedUnit],
    run: &TextRun,
    target: &StyleTarget,
) -> Result<Option<usize>, EditProblem> {
    if target.face_changed {
        return Ok(None);
    }
    let (tf, tc, tw) = (target.tfs, target.tc, run.tw);
    let found = units.iter().position(|p| {
        let gap = |value: f64| -value / 1000.0 >= COLUMN_GAP_EM;
        p.region == Region::Suffix
            && matches!(p.unit, Planned::Kern { value, kept: Some(_), .. } if gap(value))
    });
    let Some(idx) = found else {
        return Ok(None);
    };
    let (value, kept) = match units.get(idx).map(|p| &p.unit) {
        Some(Planned::Kern {
            value,
            kept: Some(k),
            ..
        }) => (*value, *k),
        _ => return Ok(None),
    };
    let orig_before: f64 = run
        .units
        .get(..kept)
        .unwrap_or_default()
        .iter()
        .map(|u| original_advance(u, run))
        .sum();
    let new_before: f64 = units
        .get(..idx)
        .unwrap_or_default()
        .iter()
        .map(|p| advance(&p.unit, tf, tc, tw))
        .sum();
    if tf == 0.0 {
        return Ok(None);
    }
    let adjusted = (new_before - orig_before) * 1000.0 / tf + value * run.tfs / tf;
    let (s, v) = num(adjusted)?;
    if -v / 1000.0 < SYNTH_SPACE_EM {
        return Ok(None);
    }
    if let Some(p) = units.get_mut(idx) {
        if let Planned::Kern {
            value,
            written,
            original,
            ..
        } = &mut p.unit
        {
            *value = v;
            *written = Some(s);
            *original = None;
        }
    }
    Ok(Some(idx))
}

/// Steps 9–10 for one run.
pub(super) fn lay_out(
    masks: &mut MaskFonts<'_>,
    model: &PageModel,
    run: &TextRun,
    surface: &TypingSurface,
    target: &StyleTarget,
    new_units: &[(NewUnit, Region)],
) -> Result<Laid, EditProblem> {
    let primary =
        record(model, run, 0).ok_or_else(|| problem(P::EditVerifyFailed, "run without members"))?;
    let mut units = resolve(model, run, surface, new_units)?;
    let absorbed_at = absorb_column_gap(&mut units, run, target)?;
    let (tf, tc, tw, th, ttux) = (target.tfs, target.tc, run.tw, run.th, run.text_to_user_x);
    if tf == 0.0 || run.tfs == 0.0 {
        return Err(problem(P::EditVerifyFailed, "zero size"));
    }
    let precision = || problem(P::EditVerifyFailed, "number precision");
    // Positions (text space, Th included) and the total advance.
    let mut x = Vec::with_capacity(units.len() + 1);
    let mut at = 0.0f64;
    for p in &units {
        x.push(at);
        at += advance(&p.unit, tf, tc, tw) * th;
    }
    let advance_new: f64 = units.iter().map(|p| advance(&p.unit, tf, tc, tw)).sum();
    let n_c = (advance_new - primary.advance_ts) * 1000.0 / tf;
    let compensation = if n_c.abs() < NEGLIGIBLE_KERN {
        None
    } else {
        Some(num(n_c)?)
    };
    let kern_scale = tf * th * ttux / 1000.0;
    let mut glyph_count = 0usize;
    let mut em_sum = 0.0f64;
    for p in &units {
        match &p.unit {
            Planned::Kern {
                value,
                written: Some(_),
                ..
            } => {
                if !f32_ok(*value, kern_scale) {
                    return Err(precision());
                }
                em_sum += value.abs() / 1000.0;
            }
            Planned::Kern { value, .. } => em_sum += value.abs() / 1000.0,
            Planned::Glyph { width1000, .. } => {
                glyph_count += 1;
                em_sum += width1000.abs() / 1000.0;
            }
        }
    }
    if compensation
        .as_ref()
        .is_some_and(|(_, v)| !f32_ok(*v, kern_scale))
    {
        return Err(precision());
    }
    if target.tc_changed && !f32_ok(tc, glyph_count as f64 * th * ttux) {
        return Err(precision());
    }
    if target.size_changed && !f32_ok(tf, em_sum * th * ttux) {
        return Err(precision());
    }
    let mut absorbed_numbers = vec![None];
    for m in 1..run.members.len() {
        let rec = record(model, run, m)
            .ok_or_else(|| problem(P::EditVerifyFailed, "member out of range"))?;
        let tfs_j = rec.before.text.tfs;
        if tfs_j == 0.0 {
            return Err(precision());
        }
        let n_j = -rec.advance_ts * 1000.0 / tfs_j;
        if n_j.abs() < NEGLIGIBLE_KERN {
            absorbed_numbers.push(None);
        } else {
            let (s, v) = num(n_j)?;
            if !f32_ok(v, tfs_j * rec.before.text.th * ttux / 1000.0) {
                return Err(precision());
            }
            absorbed_numbers.push(Some(s));
        }
    }
    // Glyph expectations.
    let origin = run.origin;
    let step = (run.dir.0 * ttux, run.dir.1 * ttux);
    let l_new = scaled_linear(primary, tf);
    let mut glyph_origins = Vec::new();
    let mut prefix_glyphs = 0usize;
    let mut suffix_glyphs = 0usize;
    let mut shift_user = None;
    let mut unshifted_from = None;
    let mut new_extent = 0.0f64;
    let mut ink: Vec<(f64, f64)> = Vec::new();
    let mut mask_boxes = Vec::new();
    let new_glyphs = units.iter().filter_map(|p| match &p.unit {
        Planned::Glyph {
            model: Some(m),
            code,
            ..
        } => Some((m, *code)),
        _ => None,
    });
    let old_glyphs = (0..run.members.len())
        .filter_map(|m| record(model, run, m))
        .flat_map(|rec| {
            rec.font
                .iter()
                .flat_map(move |f| rec.glyphs.iter().map(move |g| (f, g.code)))
        });
    masks.prepare(new_glyphs.chain(old_glyphs));
    let new_pad = stroke_pad(&primary.before);
    for (k, p) in units.iter().enumerate() {
        let xk = x.get(k).copied().unwrap_or(0.0);
        let Planned::Glyph {
            code,
            width1000,
            model: font,
            kept,
            ..
        } = &p.unit
        else {
            continue;
        };
        let o = (origin.0 + xk * step.0, origin.1 + xk * step.1);
        let w0 = width1000 / 1000.0;
        if absorbed_at.is_some_and(|a| k > a) && unshifted_from.is_none() {
            unshifted_from = Some(glyph_origins.len());
        }
        match p.region {
            Region::Prefix => prefix_glyphs += 1,
            Region::Suffix => {
                suffix_glyphs += 1;
                if shift_user.is_none() {
                    if let Some((_, member, gi)) = kept {
                        if let Some(g) = record(model, run, *member).and_then(|r| r.glyphs.get(*gi))
                        {
                            shift_user = Some((o.0 - g.origin.0, o.1 - g.origin.1));
                        }
                    }
                }
            }
            Region::Middle => {}
        }
        glyph_origins.push(o);
        new_extent = new_extent.max((xk + w0 * tf * th) * ttux);
        let (asc, desc) = metrics(font.as_ref());
        let b = em_box(o, &l_new, [0.0, w0], [desc, asc]);
        ink.extend([(b[0], b[1]), (b[2], b[3])]);
        let gb = masks.glyph_box(font.as_ref(), *code, w0);
        let b = em_box(o, &l_new, [gb[0], gb[2]], [gb[1], gb[3]]);
        mask_boxes.push(grow(b, new_pad));
    }
    // Old glyphs of every member are masked too.
    let mut old_ink_boxes = Vec::new();
    for m in 0..run.members.len() {
        let Some(rec) = record(model, run, m) else {
            continue;
        };
        let l = scaled_linear(rec, rec.before.text.tfs);
        let pad = stroke_pad(&rec.before);
        for g in &rec.glyphs {
            let (font, w0) = (rec.font.as_ref(), g.width1000 / 1000.0);
            let gb = masks.glyph_box(font, g.code, w0);
            let b = em_box(g.origin, &l, [gb[0], gb[2]], [gb[1], gb[3]]);
            mask_boxes.push(grow(b, pad));
            let ib = masks.ink_box(font, g.code, w0);
            let b = em_box(g.origin, &l, [ib[0], ib[2]], [ib[1], ib[3]]);
            old_ink_boxes.push(grow(b, pad));
        }
    }
    let (asc, desc) = metrics(primary.font.as_ref());
    if ink.is_empty() {
        let b = em_box(origin, &l_new, [0.0, 0.0], [desc, asc]);
        ink.extend([(b[0], b[1]), (b[2], b[3])]);
    }
    let rect = bbox_of(&ink).unwrap_or([origin.0, origin.1, origin.0, origin.1]);
    let new_rect = [rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1]];
    // Fit (§A.8).
    if new_extent > run.visible_extent.max(run.original_extent) + DRIFT_TOLERANCE_PT {
        return Err(problem(
            P::TextOutsideVisibleArea,
            format!(
                "extent {new_extent:.3} pt > visible {:.3} pt",
                run.visible_extent
            ),
        ));
    }
    let warnings = run
        .next_obstacle
        .filter(|o| new_extent > o + OVERLAP_WARN_TOL_PT)
        .map(|_| TextWarningCode::NextTextOverlap)
        .into_iter()
        .collect();
    let (text, caret_offsets) = text_and_carets(&units, &x, tf, tc, tw, th, ttux);
    let items = units.iter().map(|p| match &p.unit {
        Planned::Glyph { text, .. } => TextItem::Glyph(text.as_str()),
        Planned::Kern { value, .. } => TextItem::Kern(-value / 1000.0),
    });
    if decoded_text(items) != text {
        return Err(problem(
            P::EditVerifyFailed,
            "the new line would not read back as typed",
        ));
    }
    Ok(Laid {
        glyphs_changed: glyphs_changed(run, &units),
        units,
        compensation: compensation.map(|(s, _)| s),
        absorbed_numbers,
        text,
        glyph_origins,
        prefix_glyphs,
        suffix_glyphs,
        shift_user: shift_user.unwrap_or((0.0, 0.0)),
        unshifted_from,
        delta_pt: new_extent - run.original_extent,
        new_rect,
        caret_offsets,
        mask_boxes,
        old_ink_boxes,
        warnings,
    })
}

/// The new text (synthetic spaces as " ") and its chars + 1 caret offsets along `dir`.
fn text_and_carets(
    units: &[PlannedUnit],
    x: &[f64],
    tf: f64,
    tc: f64,
    tw: f64,
    th: f64,
    ttux: f64,
) -> (String, Vec<f64>) {
    let mut text = String::new();
    let mut offsets = Vec::new();
    let mut end = 0.0f64;
    for (k, p) in units.iter().enumerate() {
        let start = x.get(k).copied().unwrap_or(0.0) * ttux;
        let adv = advance(&p.unit, tf, tc, tw) * th * ttux;
        match &p.unit {
            Planned::Glyph { text: t, .. } => {
                let n = t.chars().count().max(1);
                for j in 0..t.chars().count() {
                    offsets.push(start + adv * j as f64 / n as f64);
                }
                text.push_str(t);
                end = start + adv;
            }
            Planned::Kern { synth: true, .. } => {
                offsets.push(start);
                text.push(' ');
                end = start + adv;
            }
            Planned::Kern { .. } => {}
        }
    }
    offsets.push(end);
    let mut max = f64::NEG_INFINITY;
    for o in &mut offsets {
        max = max.max(*o);
        *o = max;
    }
    (text, offsets)
}

/// Whether the non-space glyphs (font and code) differ between the run and the new units.
fn glyphs_changed(run: &TextRun, units: &[PlannedUnit]) -> bool {
    let space = |t: &str| matches!(t, " " | "\u{a0}");
    let old = run.units.iter().filter_map(|u| match u {
        Unit::Glyph {
            code,
            font_hash,
            text,
            ..
        } if !space(text) => Some((*font_hash, *code)),
        _ => None,
    });
    let new = units.iter().filter_map(|p| match &p.unit {
        Planned::Glyph {
            code,
            font_hash,
            text,
            ..
        } if !space(text) => Some((*font_hash, *code)),
        _ => None,
    });
    !old.eq(new)
}
