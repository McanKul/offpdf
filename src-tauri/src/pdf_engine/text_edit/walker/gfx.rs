//! Graphics-state operators of the walker: `cm`, line parameters, ExtGState, paths and clips,
//! colour, marked content (SPEC §B.10 "operators modelled").
//!
//! A `gs` costs O(its dictionary): ExtGState dictionaries hold at most `EXTGSTATE_KEYS_MAX` keys
//! and at most `GS_OTHER_MAX` unmodelled keys may be in force at once (real producers use a
//! handful of each), so a page that repeats `gs` cannot multiply a large dictionary.
//!
//! A state operator that sets a value already in force keeps the shared allocation in force
//! (`set_bytes`, `set_dash`, `GsEffects::set_others`), so re-applying one ExtGState, `ri` or `d`
//! per op leaves the digest shared (`Exec::digest`) instead of one new digest per op. Unmodelled
//! ExtGState keys are at most `EXTGSTATE_KEY_BYTES_MAX` long, and re-applying the dictionary
//! that put the keys in force costs O(1) (`Walker::gs_last`), so a `gs` never compares long keys
//! over and over (review T3 r3 MEDIUM-2). The key lists a walk keeps are charged to its budget.

use super::budget::{arc_slice, map_entry};
use super::{Exec, Frame, Others, PaintKind, PaintRecord, Stop, Walked, Walker};
use crate::pdf_engine::text_edit::context::{resolve, Lookup};
use crate::pdf_engine::text_edit::fonts::{number_of, FontKey};
use crate::pdf_engine::text_edit::geometry::{
    apply, axis_aligned, bbox_of, intersect, is_finite, mul, Matrix,
};
use crate::pdf_engine::text_edit::lexer::{Op, Operand, Operator};
use crate::pdf_engine::text_edit::limits::{EXTGSTATE_KEY_BYTES_MAX, MARKED_DEPTH_MAX};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::{
    color_effect, Bytes, ClipState, ColorSpaceKind, FontUse, GsEffects, MarkedEntry, OcState, Paint,
};
use lopdf::{Dictionary, Object, ObjectId};
use std::mem::size_of;
use std::sync::Arc;

/// Keys of one ExtGState dictionary (ISO 32000-2 defines 29).
const EXTGSTATE_KEYS_MAX: usize = 64;
/// Unmodelled ExtGState keys in force at once (`GsEffects::other`).
const GS_OTHER_MAX: usize = 64;
/// Entries of an ExtGState `/D` dash array that are modelled; a longer one is recorded through
/// its deep hash like an unmodelled key.
const GS_DASH_ITEMS_MAX: usize = 64;

pub(super) fn num(op: &Op, i: usize) -> Result<f64, Stop> {
    op.operands
        .get(i)
        .and_then(Operand::as_number)
        .filter(|v| v.is_finite())
        .ok_or_else(|| Stop::malformed(format!("operand at byte {}", op.op_span.start)))
}

pub(super) fn six(op: &Op) -> Result<Matrix, Stop> {
    Ok([
        num(op, 0)?,
        num(op, 1)?,
        num(op, 2)?,
        num(op, 3)?,
        num(op, 4)?,
        num(op, 5)?,
    ])
}

pub(super) fn finite(m: &Matrix, op: &Op) -> Walked {
    if is_finite(m) {
        Ok(())
    } else {
        Err(Stop::malformed(format!(
            "non-finite matrix at byte {}",
            op.op_span.start
        )))
    }
}

pub(super) fn concat(ex: &mut Exec, op: &Op) -> Walked {
    let m = six(op)?;
    let ctm = mul(&m, &ex.gs.ctm);
    finite(&ctm, op)?;
    ex.gs.ctm = ctm;
    Ok(())
}

pub(super) fn line_param(ex: &mut Exec, op: &Op) -> Walked {
    let g = &mut ex.gs.gs;
    match op.operator {
        Operator::w => g.line_width = num(op, 0)?,
        Operator::J => g.line_cap = num(op, 0)? as i64,
        Operator::j => g.line_join = num(op, 0)? as i64,
        Operator::M => g.miter_limit = num(op, 0)?,
        Operator::i => g.flatness = num(op, 0)?,
        Operator::ri => set_bytes(
            &mut g.rendering_intent,
            op.operands
                .first()
                .and_then(Operand::as_name)
                .unwrap_or_default(),
        ),
        Operator::d => {
            let dashes = match op.operands.first() {
                Some(Operand::Array { items, .. }) => items
                    .iter()
                    .map(|i| i.as_number().filter(|v| v.is_finite()))
                    .collect::<Option<Vec<f64>>>()
                    .ok_or_else(|| Stop::malformed("dash array"))?,
                _ => return Err(Stop::malformed("dash array")),
            };
            set_dash(&mut g.dash, &dashes, num(op, 1)?);
        }
        _ => {}
    }
    Ok(())
}

/// Sets shared bytes, keeping the allocation in force when the value is the same.
fn set_bytes(slot: &mut Bytes, value: &[u8]) {
    if **slot != *value {
        *slot = Arc::from(value);
    }
}

/// Sets the dash pattern, keeping the shared array in force when it is the same.
fn set_dash(slot: &mut (Arc<[f64]>, f64), dashes: &[f64], phase: f64) {
    if *slot.0 != *dashes {
        slot.0 = Arc::from(dashes);
    }
    slot.1 = phase;
}

/// Path construction. Only the points an op adds are checked and only the path's bounding box is
/// kept, so a long unpainted path costs O(1) per op and no memory (it is cleared at its painting
/// op).
pub(super) fn path(ex: &mut Exec, op: &Op) -> Walked {
    let ctm = ex.gs.ctm;
    let pt = |i: usize| -> Result<(f64, f64), Stop> {
        finite_point(apply(&ctm, num(op, i)?, num(op, i + 1)?), op)
    };
    let p = &mut ex.path;
    let add = |bbox: &mut Option<[f64; 4]>, points: &[(f64, f64)]| {
        for &(x, y) in points {
            *bbox = Some(match *bbox {
                None => [x, y, x, y],
                Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
            });
        }
    };
    match op.operator {
        Operator::m | Operator::l => {
            add(&mut p.bbox, &[pt(0)?]);
            p.other = true;
        }
        Operator::c => {
            add(&mut p.bbox, &[pt(0)?, pt(2)?, pt(4)?]);
            p.other = true;
        }
        Operator::v | Operator::y => {
            add(&mut p.bbox, &[pt(0)?, pt(2)?]);
            p.other = true;
        }
        Operator::re => {
            let (x, y, w, h) = (num(op, 0)?, num(op, 1)?, num(op, 2)?, num(op, 3)?);
            let corners = [
                finite_point(apply(&ctm, x, y), op)?,
                finite_point(apply(&ctm, x + w, y), op)?,
                finite_point(apply(&ctm, x, y + h), op)?,
                finite_point(apply(&ctm, x + w, y + h), op)?,
            ];
            add(&mut p.bbox, &corners);
            p.re_count = p.re_count.saturating_add(1);
            p.re_axis = if p.re_count == 1 {
                axis_aligned(&ctm)
            } else {
                p.re_axis && axis_aligned(&ctm)
            };
            p.re_rect = bbox_of(&corners);
        }
        _ => {}
    }
    Ok(())
}

fn finite_point(p: (f64, f64), op: &Op) -> Result<(f64, f64), Stop> {
    if p.0.is_finite() && p.1.is_finite() {
        Ok(p)
    } else {
        Err(Stop::malformed(format!(
            "non-finite path at byte {}",
            op.op_span.start
        )))
    }
}

impl<'a> Walker<'a> {
    pub(super) fn paint_path(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        if op.operator != Operator::n {
            let seq = self.next_seq()?;
            let state = self.digest(ex)?;
            self.mem.hold(frame.chain.len() * size_of::<ObjectId>())?;
            let paint = PaintRecord {
                seq,
                depth: frame.depth,
                kind: PaintKind::Path(op.operator),
                span: frame.joined.then(|| op.span.clone()),
                state,
                bbox: ex.path.bbox,
                masked: ex.gs.gs.soft_mask,
                form_chain: frame.chain.clone(),
                local_span: op.span.clone(),
                xobject: None,
            };
            self.mem.push(&mut self.paints, paint)?;
        }
        if ex.clip_pending {
            let p = &ex.path;
            let rect = (!p.other && p.re_count == 1 && p.re_axis)
                .then_some(p.re_rect)
                .flatten();
            ex.gs.clip = match (&ex.gs.clip, rect) {
                (ClipState::Complex, _) | (_, None) => ClipState::Complex,
                (ClipState::None, Some(r)) => ClipState::Rect(r),
                (ClipState::Rect(c), Some(r)) => ClipState::Rect(intersect(*c, r)),
            };
            ex.clip_pending = false;
        }
        ex.path = Default::default();
        Ok(())
    }

    pub(super) fn ext_gstate(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let name = op
            .operands
            .first()
            .and_then(Operand::as_name)
            .unwrap_or_default();
        let dict = match frame.res.entry(self.doc, b"ExtGState", name) {
            Lookup::Found(e) => match e.value {
                Object::Dictionary(d) => d,
                _ => return Err(Stop::malformed("ExtGState is not a dictionary")),
            },
            Lookup::Missing => return Err(Stop::malformed("ExtGState name not in resources")),
            Lookup::Broken(w) => return Err(Stop::malformed(format!("ExtGState {w}"))),
        };
        if dict.len() > EXTGSTATE_KEYS_MAX {
            return Err(Stop::new(TextReason::PageTooComplex, "ExtGState keys"));
        }
        // The unmodelled keys depend on the dictionary alone: listed (and hashed) once per walk.
        let addr = dict as *const Dictionary as usize;
        let known = self.gs_others.get(&addr).cloned();
        let mut others: Vec<(Bytes, u64)> = Vec::new();
        for (key, raw) in dict.iter() {
            let Some((id, value)) = resolve(self.doc, raw) else {
                return Err(Stop::malformed("ExtGState reference"));
            };
            if key.as_slice() == b"Font" {
                self.gs_font(ex, value)?;
                continue;
            }
            let modelled = apply_gs_key(&mut ex.gs.gs, key, value, dict);
            let smask = key.as_slice() == b"SMask" && ex.gs.gs.soft_mask;
            if known.is_none() && ((!modelled && key.as_slice() != b"Type") || smask) {
                if key.len() > EXTGSTATE_KEY_BYTES_MAX {
                    return Err(Stop::new(TextReason::PageTooComplex, "ExtGState keys"));
                }
                self.mem.scratch(arc_slice(key.len(), 1))?;
                others.push((Arc::from(key.as_slice()), self.entry_hash(id, value)?));
            }
        }
        let others = match known {
            Some(list) => list,
            None => {
                let pair = size_of::<(Bytes, u64)>();
                self.mem
                    .scratch(arc_slice(others.len(), pair) + map_entry::<usize, Others>())?;
                let list = GsEffects::sorted_others(others);
                self.gs_others.insert(addr, Arc::clone(&list));
                list
            }
        };
        self.set_others(ex, others)?;
        if ex.gs.gs.other.len() > GS_OTHER_MAX {
            return Err(Stop::new(TextReason::PageTooComplex, "ExtGState keys"));
        }
        Ok(())
    }

    /// Puts the unmodelled keys `others` of one ExtGState in force (`GsEffects::set_others`). The
    /// dictionary applied last, re-applied to the list it put in force, changes nothing and is
    /// skipped in O(1); a new merged list is charged (scratch: the walk interns it) before the
    /// merge allocates it.
    fn set_others(&mut self, ex: &mut Exec, others: Others) -> Walked {
        if let Some((entries, result)) = &self.gs_last {
            if Arc::ptr_eq(entries, &others) && Arc::ptr_eq(result, &ex.gs.gs.other) {
                return Ok(());
            }
        }
        let n = ex.gs.gs.other.len().saturating_add(others.len());
        let pair = size_of::<(Bytes, u64)>();
        let merge = n.saturating_mul(pair);
        let list = arc_slice(n, pair) + map_entry::<Others, ()>();
        self.mem.scratch(merge.saturating_add(list))?;
        let interned = ex.gs.gs.set_others(&others, &mut self.others_interned);
        self.mem
            .unscratch(if interned { merge } else { merge + list });
        self.gs_last = Some((others, Arc::clone(&ex.gs.gs.other)));
        Ok(())
    }

    /// ExtGState `/Font [font size]`: the font in force without a `Tf` (B9). The font must be an
    /// indirect reference (ISO 32000-1 Table 58); viewers disagree on a direct dictionary there
    /// (poppler draws nothing), so it is malformed.
    fn gs_font(&mut self, ex: &mut Exec, value: &'a Object) -> Walked {
        let bad = || Stop::malformed("ExtGState /Font");
        let Object::Array(items) = value else {
            return Err(bad());
        };
        let (Some(font), Some(size)) = (items.first(), items.get(1)) else {
            return Err(bad());
        };
        let size = resolve(self.doc, size)
            .and_then(|(_, o)| number_of(o))
            .ok_or_else(bad)?;
        let (key, dict) = match resolve(self.doc, font) {
            Some((Some(id), Object::Dictionary(d))) => (FontKey::Indirect(id), d),
            Some((None, Object::Dictionary(_))) => {
                return Err(Stop::malformed(
                    "ExtGState /Font is not an indirect reference",
                ))
            }
            _ => return Err(bad()),
        };
        let model = self.load_font(key, dict)?;
        ex.gs.text.font = Some(FontUse {
            resource: None,
            content_hash: model.content_hash,
            from_extgstate: true,
            tf_op: None,
        });
        ex.gs.text.tfs = size;
        ex.gs.font_ref = Some((key, model));
        Ok(())
    }

    pub(super) fn color(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        use Operator as O;
        let bytes = frame.op_bytes(op);
        let stroke = matches!(op.operator, O::CS | O::SC | O::SCN | O::G | O::RG | O::K);
        let device = |space: ColorSpaceKind, n: usize| -> Result<Paint, Stop> {
            let comps = (0..n)
                .map(|i| num(op, i))
                .collect::<Result<Vec<f64>, Stop>>()?;
            Ok(Paint {
                space_op: None,
                color_op: Some(Arc::clone(&bytes)),
                effect: color_effect(&space, &comps),
                space,
                comps,
                pattern: false,
                pattern_hash: None,
            })
        };
        let new = match op.operator {
            O::G | O::g => device(ColorSpaceKind::DeviceGray, 1)?,
            O::RG | O::rg => device(ColorSpaceKind::DeviceRgb, 3)?,
            O::K | O::k => device(ColorSpaceKind::DeviceCmyk, 4)?,
            O::CS | O::cs => {
                let name = op
                    .operands
                    .first()
                    .and_then(Operand::as_name)
                    .unwrap_or_default();
                let (space, comps) = self.color_space(frame, name)?;
                Paint {
                    space_op: Some(bytes),
                    color_op: None,
                    effect: color_effect(&space, &comps),
                    pattern: space == ColorSpaceKind::Pattern,
                    space,
                    comps,
                    pattern_hash: None,
                }
            }
            _ => {
                let pattern_name = op.operands.last().and_then(Operand::as_name);
                let pattern_hash = match pattern_name {
                    Some(name) => match frame.res.entry(self.doc, b"Pattern", name) {
                        Lookup::Found(e) => Some(self.entry_hash(e.id, e.value)?),
                        Lookup::Missing => return Err(Stop::malformed("pattern not in resources")),
                        Lookup::Broken(w) => return Err(Stop::malformed(format!("pattern {w}"))),
                    },
                    None => None,
                };
                let cur = if stroke { &ex.gs.stroke } else { &ex.gs.fill };
                let comps = op
                    .operands
                    .iter()
                    .filter_map(Operand::as_number)
                    .collect::<Vec<f64>>();
                if comps.iter().any(|c| !c.is_finite()) {
                    return Err(Stop::malformed("colour component"));
                }
                let space = match &cur.space {
                    ColorSpaceKind::Default => ColorSpaceKind::DeviceGray,
                    other => other.clone(),
                };
                Paint {
                    space_op: cur.space_op.clone(),
                    color_op: Some(bytes),
                    effect: color_effect(&space, &comps),
                    pattern: cur.pattern || pattern_name.is_some(),
                    space,
                    comps,
                    pattern_hash,
                }
            }
        };
        if stroke {
            ex.gs.stroke = new;
        } else {
            ex.gs.fill = new;
        }
        Ok(())
    }

    /// `cs`/`CS` operand: a device family, `/Pattern`, or a `/ColorSpace` resource.
    fn color_space(
        &mut self,
        frame: &Frame<'_, 'a>,
        name: &[u8],
    ) -> Result<(ColorSpaceKind, Vec<f64>), Stop> {
        Ok(match name {
            b"DeviceGray" | b"G" => (ColorSpaceKind::DeviceGray, vec![0.0]),
            b"DeviceRGB" | b"RGB" => (ColorSpaceKind::DeviceRgb, vec![0.0; 3]),
            b"DeviceCMYK" | b"CMYK" => (ColorSpaceKind::DeviceCmyk, vec![0.0, 0.0, 0.0, 1.0]),
            b"Pattern" => (ColorSpaceKind::Pattern, Vec::new()),
            _ => match frame.res.entry(self.doc, b"ColorSpace", name) {
                Lookup::Found(e) => {
                    if names_pattern(self.doc, e.value) {
                        (ColorSpaceKind::Pattern, Vec::new())
                    } else {
                        let h = self.entry_hash(e.id, e.value)?;
                        (ColorSpaceKind::Named(Arc::from(name), h), Vec::new())
                    }
                }
                Lookup::Missing => return Err(Stop::malformed("colour space not in resources")),
                Lookup::Broken(w) => return Err(Stop::malformed(format!("colour space {w}"))),
            },
        })
    }

    pub(super) fn marked(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        use Operator as O;
        let tag = op
            .operands
            .first()
            .and_then(Operand::as_name)
            .unwrap_or_default();
        match op.operator {
            O::EMC => {
                if ex.marked.len() > ex.marked_base {
                    ex.marked.pop();
                }
                return Ok(());
            }
            O::MP => return Ok(()),
            O::DP => {
                if let Some(Operand::Name { bytes, .. }) = op.operands.get(1) {
                    self.properties(frame, bytes)?;
                }
                return Ok(());
            }
            _ => {}
        }
        if ex.marked.len() >= MARKED_DEPTH_MAX {
            return Err(Stop::new(
                TextReason::PageTooComplex,
                "marked-content nesting",
            ));
        }
        let is_oc = tag == b"OC";
        let mut entry = MarkedEntry {
            tag: Arc::from(tag),
            mcid: None,
            actual_text: false,
            oc: None,
        };
        match op.operands.get(1) {
            Some(Operand::Dict { entries, .. }) => {
                for (k, v) in entries {
                    match k.as_slice() {
                        b"MCID" => entry.mcid = v.as_number().map(|n| n as i64),
                        b"ActualText" | b"E" => entry.actual_text = true,
                        _ => {}
                    }
                }
                if is_oc {
                    entry.oc = Some(OcState::Unknown);
                }
            }
            Some(Operand::Name { bytes, .. }) => {
                let (id, value) = self.properties(frame, bytes)?;
                let dict = match value {
                    Object::Dictionary(d) => Some(d),
                    _ => None,
                };
                if let Some(d) = dict {
                    entry.mcid = d
                        .get(b"MCID")
                        .ok()
                        .and_then(|o| match resolve(self.doc, o) {
                            Some((_, Object::Integer(i))) => Some(*i),
                            _ => None,
                        });
                    entry.actual_text = d.has(b"ActualText") || d.has(b"E");
                }
                if is_oc {
                    entry.oc = Some(self.oc_state(id, value)?);
                }
            }
            _ => {}
        }
        ex.marked.push(entry);
        Ok(())
    }

    /// `/Properties` entry `name`: the id it was reached through (when indirect) and its value.
    fn properties(
        &self,
        frame: &Frame<'_, 'a>,
        name: &[u8],
    ) -> Result<(Option<ObjectId>, &'a Object), Stop> {
        match frame.res.entry(self.doc, b"Properties", name) {
            Lookup::Found(e) => Ok((e.id, e.value)),
            Lookup::Missing => Err(Stop::malformed("properties name not in resources")),
            Lookup::Broken(w) => Err(Stop::malformed(format!("properties {w}"))),
        }
    }
}

/// Applies a modelled ExtGState key; `false` when the key is not modelled (or its value has the
/// wrong type, in which case it is recorded verbatim through `other`).
fn apply_gs_key(g: &mut GsEffects, key: &[u8], value: &Object, dict: &Dictionary) -> bool {
    let n = number_of(value);
    let b = value.as_bool().ok();
    let name = value.as_name().ok();
    match (key, n, b, name) {
        (b"CA", Some(v), _, _) => g.ca_stroke = v,
        (b"ca", Some(v), _, _) => g.ca = v,
        (b"BM", _, _, Some(bm)) => set_bytes(&mut g.blend, bm),
        (b"BM", _, _, None) => match value.as_array().ok().and_then(|a| a.first()) {
            Some(Object::Name(bm)) => set_bytes(&mut g.blend, bm),
            _ => return false,
        },
        (b"SMask", _, _, _) => g.soft_mask = name != Some(b"None"),
        (b"OP", _, Some(v), _) => {
            g.overprint.0 = v;
            if !dict.has(b"op") {
                g.overprint.1 = v;
            }
        }
        (b"op", _, Some(v), _) => g.overprint.1 = v,
        (b"OPM", Some(v), _, _) => g.overprint.2 = v as i64,
        (b"LW", Some(v), _, _) => g.line_width = v,
        (b"LC", Some(v), _, _) => g.line_cap = v as i64,
        (b"LJ", Some(v), _, _) => g.line_join = v as i64,
        (b"ML", Some(v), _, _) => g.miter_limit = v,
        (b"RI", _, _, Some(ri)) => set_bytes(&mut g.rendering_intent, ri),
        (b"FL", Some(v), _, _) => g.flatness = v,
        (b"SA", _, Some(v), _) => g.stroke_adjust = v,
        (b"D", _, _, _) => match dash_of(value) {
            Some((dashes, phase)) => set_dash(&mut g.dash, &dashes, phase),
            None => return false,
        },
        _ => return false,
    }
    true
}

fn dash_of(value: &Object) -> Option<(Vec<f64>, f64)> {
    let items = value.as_array().ok()?;
    let (Some(Object::Array(arr)), Some(phase)) = (items.first(), items.get(1)) else {
        return None;
    };
    if arr.len() > GS_DASH_ITEMS_MAX {
        return None;
    }
    let dashes = arr.iter().map(number_of).collect::<Option<Vec<f64>>>()?;
    Some((dashes, number_of(phase)?))
}

/// A colour space value that is `/Pattern` or `[/Pattern …]`.
fn names_pattern(doc: &lopdf::Document, value: &Object) -> bool {
    match value {
        Object::Name(n) => n.as_slice() == b"Pattern",
        Object::Array(items) => items
            .first()
            .and_then(|f| resolve(doc, f))
            .is_some_and(|(_, o)| o.as_name().ok() == Some(b"Pattern")),
        _ => false,
    }
}
