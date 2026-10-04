//! XObjects, inline images and shadings (SPEC §B.10): image and Form paints, Form descent in
//! `Classify` mode (depth ≤ 8, cycle check, `/Matrix`, `/BBox` clip) and the wrapper Form of
//! `Wrapped` mode (qpdf's overlay wrapper, §B.16.2), whose content is walked as depth 0 after the
//! wrapper conditions are re-checked. A Form's decoded content and lexed ops are charged to the
//! walk's budget while the Form runs (`budget::ModelBudget` scratch), its paints and records for
//! good.

use super::budget::{lex_ops, MODEL_SIZE};
use super::{Exec, Frame, PaintKind, PaintRecord, Stop, WalkMode, Walked, Walker, XObjectUse};
use crate::pdf_engine::text_edit::context::{resolve, Lookup, Res};
use crate::pdf_engine::text_edit::decode::decode_stream;
use crate::pdf_engine::text_edit::fonts::number_of;
use crate::pdf_engine::text_edit::geometry::{
    axis_aligned, contains, intersect, mul, transform_rect, Matrix, IDENTITY,
};
use crate::pdf_engine::text_edit::lexer::{Op, Operand, Operator};
use crate::pdf_engine::text_edit::limits::{
    FORM_DEPTH_MAX, FORM_PAINTS_PER_PAGE_MAX, PAGE_CONTENT_MAX_DECODED, STREAM_MAX_DECODED,
    WRAPPER_MATRIX_EPSILON, WRAPPER_TRANSLATION_TOL_PT,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::snapshot::fnv1a_u64;
use crate::pdf_engine::text_edit::state::{ClipState, GState, MarkedStack};
use lopdf::{Dictionary, Object, ObjectId, Stream};

/// The wrapper Form's `/BBox` must cover the visible box within this (§B.16.2).
const WRAPPER_BBOX_TOL_PT: f64 = 0.01;
/// The keys qpdf writes on its overlay wrapper Form (`getFormXObjectForPage`) besides `/Group`,
/// which must equal the page's. Any other key (`/OC`, `/Ref`, `/OPI`, …) could change what the
/// wrapper paints without changing the walk, so it is refused.
const WRAPPER_KEYS: [&[u8]; 10] = [
    b"Type",
    b"Subtype",
    b"FormType",
    b"BBox",
    b"Matrix",
    b"Resources",
    b"Length",
    b"Filter",
    b"DecodeParms",
    b"DL",
];
const UNIT_SQUARE: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

/// A resolved `/XObject` entry.
struct XObj<'a> {
    id: ObjectId,
    stream: &'a Stream,
    shared_path: bool,
}

impl<'a> Walker<'a> {
    fn xobject(&self, frame: &Frame<'_, 'a>, name: &[u8]) -> Result<XObj<'a>, Stop> {
        let e = match frame.res.entry(self.doc, b"XObject", name) {
            Lookup::Found(e) => e,
            Lookup::Missing => return Err(Stop::malformed("XObject name not in resources")),
            Lookup::Broken(w) => return Err(Stop::malformed(format!("XObject {w}"))),
        };
        let (Some(id), Object::Stream(stream)) = (e.id, e.value) else {
            return Err(Stop::malformed("XObject is not a stream"));
        };
        let refs = self.ctx.refs();
        let shared_path = frame.res.inherited
            || frame.res.dict_id.is_some_and(|d| refs.count(d) > 1)
            || e.category_id.is_some_and(|c| refs.count(c) > 1);
        Ok(XObj {
            id,
            stream,
            shared_path,
        })
    }

    pub(super) fn do_xobject(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let name = op
            .operands
            .first()
            .and_then(Operand::as_name)
            .unwrap_or_default();
        let x = self.xobject(frame, name)?;
        let subtype = x
            .stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok());
        match subtype {
            Some(b"Image") => {
                let hash = self.stream_hash(x.id)?;
                self.mem.hold(name.len())?;
                let masked = ex.gs.gs.soft_mask || image_masked(self.doc, &x.stream.dict);
                let bbox = transform_rect(&ex.gs.ctm, UNIT_SQUARE);
                self.push_paint(
                    frame,
                    ex,
                    op,
                    PaintKind::ImageXObject {
                        name: name.to_vec(),
                        hash,
                    },
                    Some(bbox),
                    masked,
                    Some(&x),
                )
            }
            Some(b"Form") => self.form(frame, ex, op, name, &x),
            Some(b"PS") => Ok(()),
            _ => Err(Stop::malformed("XObject subtype")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_paint(
        &mut self,
        frame: &Frame<'_, 'a>,
        ex: &mut Exec,
        op: &Op,
        kind: PaintKind,
        bbox: Option<[f64; 4]>,
        masked: bool,
        x: Option<&XObj<'a>>,
    ) -> Walked {
        let seq = self.next_seq()?;
        let state = self.digest(ex)?;
        self.mem
            .hold(frame.chain.len() * std::mem::size_of::<ObjectId>())?;
        let paint = PaintRecord {
            seq,
            depth: frame.depth,
            kind,
            span: frame.joined.then(|| op.span.clone()),
            state,
            bbox,
            masked,
            form_chain: frame.chain.clone(),
            local_span: op.span.clone(),
            xobject: x.map(|x| XObjectUse {
                id: x.id,
                shared_path: x.shared_path,
            }),
        };
        self.mem.push(&mut self.paints, paint)
    }

    fn form(
        &mut self,
        frame: &Frame<'_, 'a>,
        ex: &mut Exec,
        op: &Op,
        name: &[u8],
        x: &XObj<'a>,
    ) -> Walked {
        self.form_paints = self.form_paints.saturating_add(1);
        if self.form_paints > FORM_PAINTS_PER_PAGE_MAX {
            return Err(Stop::new(
                TextReason::PageTooComplex,
                "form paints per page",
            ));
        }
        let matrix = form_matrix(self.doc, &x.stream.dict)?;
        let bbox = form_bbox(self.doc, &x.stream.dict);
        let composite = mul(&matrix, &ex.gs.ctm);
        let hash = self.stream_hash(x.id)?;
        let paint_bbox = bbox.map(|b| transform_rect(&composite, b));
        self.mem.hold(name.len())?;
        let kind = PaintKind::FormXObject {
            name: name.to_vec(),
            hash,
        };
        let masked = ex.gs.gs.soft_mask;
        self.push_paint(frame, ex, op, kind, paint_bbox, masked, Some(x))?;
        if self.mode != WalkMode::Classify {
            return Ok(());
        }
        let depth = frame.depth.saturating_add(1);
        if usize::from(depth) > FORM_DEPTH_MAX {
            return Err(Stop::malformed(format!(
                "Form XObjects nested deeper than {FORM_DEPTH_MAX}"
            )));
        }
        if frame.chain.contains(&x.id) {
            return Err(Stop::malformed("a Form XObject paints itself"));
        }
        let mut gs = ex.gs.clone();
        gs.ctm = composite;
        if let Some(b) = bbox {
            gs.clip = clip_with(&gs.clip, &composite, b);
        }
        let mut chain = frame.chain.clone();
        chain.push(x.id);
        let res = match Res::of_form(self.doc, x.id, x.stream) {
            None => frame.res,
            Some(Ok(r)) => r,
            Some(Err(w)) => return Err(Stop::malformed(format!("Form {w}"))),
        };
        let mut sub = Exec::new(gs, ex.marked.clone());
        self.run_stream(x.stream, res, depth, chain, false, &mut sub)
    }

    /// Decodes, lexes and runs one Form's content (the wrapper Form of `Wrapped` mode, `root`,
    /// holds the page content and gets the page content cap).
    fn run_stream(
        &mut self,
        stream: &'a Stream,
        res: Res<'a>,
        depth: u8,
        chain: Vec<ObjectId>,
        root: bool,
        ex: &mut Exec,
    ) -> Walked {
        let cap = if root {
            PAGE_CONTENT_MAX_DECODED
        } else {
            STREAM_MAX_DECODED
        };
        // The decoded content and the lexed ops live while the Form runs (nested Forms add theirs
        // on top); decoding stops where the byte budget would.
        let left = self.mem.left();
        let data = decode_stream(stream, cap.min(left), &mut self.budget).map_err(|e| {
            if left < cap && e.page_reason() == TextReason::PageTooComplex {
                Stop::new(TextReason::PageTooComplex, MODEL_SIZE)
            } else {
                Stop::new(e.page_reason(), format!("Form XObject: {e}"))
            }
        })?;
        self.mem.scratch(data.capacity())?;
        let lexed = lex_ops(
            &data,
            self.ops_left,
            self.operand_nodes,
            &mut self.mem,
            self.cancel,
            false,
            "Form XObject: ",
        )?;
        let (ops, nodes) = (lexed.ops, lexed.nodes);
        self.ops_left = self.ops_left.saturating_sub(ops.len());
        self.operand_nodes = self.operand_nodes.saturating_add(nodes);
        let frame = Frame {
            bytes: &data,
            res,
            depth,
            chain,
            joined: false,
            root,
        };
        let walked = self.run(&frame, ex, &ops);
        self.operand_nodes = self.operand_nodes.saturating_sub(nodes);
        self.mem
            .unscratch(lexed.bytes.saturating_add(data.capacity()));
        walked
    }

    pub(super) fn inline_image(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let hash = fnv1a_u64(frame.span_bytes(&op.span));
        let stencil = op.inline_image.as_ref().is_some_and(|img| {
            img.dict.iter().any(|(k, v)| {
                matches!(k.as_slice(), b"IM" | b"ImageMask")
                    && matches!(v, Operand::Bool { value: true, .. })
            })
        });
        let masked = ex.gs.gs.soft_mask || stencil;
        let bbox = transform_rect(&ex.gs.ctm, UNIT_SQUARE);
        self.push_paint(
            frame,
            ex,
            op,
            PaintKind::InlineImage { hash },
            Some(bbox),
            masked,
            None,
        )
    }

    pub(super) fn shading(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let name = op
            .operands
            .first()
            .and_then(Operand::as_name)
            .unwrap_or_default();
        let hash = match frame.res.entry(self.doc, b"Shading", name) {
            Lookup::Found(e) => self.entry_hash(e.id, e.value)?,
            Lookup::Missing => return Err(Stop::malformed("shading not in resources")),
            Lookup::Broken(w) => return Err(Stop::malformed(format!("shading {w}"))),
        };
        let bbox = match &ex.gs.clip {
            ClipState::Rect(r) => Some(*r),
            _ => Some(self.geometry.visible),
        };
        self.mem.hold(name.len())?;
        let kind = PaintKind::Shading {
            name: name.to_vec(),
            hash,
        };
        let masked = ex.gs.gs.soft_mask;
        self.push_paint(frame, ex, op, kind, bbox, masked, None)
    }

    /// `Wrapped` mode (§B.10): the page content may only hold `q`, `Q`, `cm` and `Do`, exactly one
    /// `Do` names the wrapper Form, whose composite matrix is the identity within tolerance, whose
    /// `/BBox` covers the visible box and whose dictionary holds only qpdf's keys (a `/Group`
    /// equal to the page's); its content is walked as depth 0 with the clip factored out. Every
    /// other `Do` stays a paint.
    pub(super) fn walk_wrapped(
        &mut self,
        frame: &Frame<'_, 'a>,
        ops: &[Op],
        name: &[u8],
        page_id: ObjectId,
    ) -> Walked {
        let wrapper = |what: &str| Stop::malformed(format!("wrapper: {what}"));
        let shape_ok = ops.iter().all(|o| {
            matches!(
                o.operator,
                Operator::q | Operator::Q | Operator::cm | Operator::Do
            )
        });
        if !shape_ok {
            return Err(wrapper("page content is not q/cm/Do/Q only"));
        }
        let hits = ops
            .iter()
            .filter(|o| {
                o.operator == Operator::Do
                    && o.operands.first().and_then(Operand::as_name) == Some(name)
            })
            .count();
        if hits != 1 {
            return Err(wrapper("the wrapper Form is not painted exactly once"));
        }
        let mut ex = Exec::new(GState::initial(IDENTITY), MarkedStack::default());
        for op in ops {
            let is_wrapper = op.operator == Operator::Do
                && op.operands.first().and_then(Operand::as_name) == Some(name);
            if !is_wrapper {
                self.op(frame, &mut ex, op)?;
                continue;
            }
            let x = self.xobject(frame, name)?;
            if x.stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                != Some(b"Form")
            {
                return Err(wrapper("the wrapper is not a Form XObject"));
            }
            self.wrapper_keys(&x.stream.dict, page_id)?;
            let m = mul(&form_matrix(self.doc, &x.stream.dict)?, &ex.gs.ctm);
            let linear_ok = (m[0] - 1.0).abs() <= WRAPPER_MATRIX_EPSILON
                && m[1].abs() <= WRAPPER_MATRIX_EPSILON
                && m[2].abs() <= WRAPPER_MATRIX_EPSILON
                && (m[3] - 1.0).abs() <= WRAPPER_MATRIX_EPSILON;
            let shift_ok = m[4].abs() <= WRAPPER_TRANSLATION_TOL_PT
                && m[5].abs() <= WRAPPER_TRANSLATION_TOL_PT;
            if !linear_ok || !shift_ok {
                return Err(wrapper("composite matrix is not the identity"));
            }
            let bbox = form_bbox(self.doc, &x.stream.dict).ok_or_else(|| wrapper("no /BBox"))?;
            if !contains(
                transform_rect(&m, bbox),
                self.geometry.visible,
                WRAPPER_BBOX_TOL_PT,
            ) {
                return Err(wrapper("/BBox does not cover the visible page"));
            }
            let res = match Res::of_form(self.doc, x.id, x.stream) {
                None => frame.res,
                Some(Ok(r)) => r,
                Some(Err(w)) => return Err(wrapper(w)),
            };
            self.root_res = res;
            let mut sub = Exec::new(GState::initial(m), MarkedStack::default());
            self.run_stream(x.stream, res, 0, Vec::new(), true, &mut sub)?;
        }
        Ok(())
    }

    /// The wrapper Form's dictionary holds only `WRAPPER_KEYS` and a `/Group` whose deep hash
    /// equals the page's `/Group` (qpdf copies it); anything else is `MALFORMED_CONTENT`
    /// "wrapper". The two `/Group`s are hashed with hash state of their own
    /// (`with_own_hash_state`), not the content's.
    fn wrapper_keys(&mut self, dict: &'a Dictionary, page_id: ObjectId) -> Walked {
        let wrapper = |what: &str| Stop::malformed(format!("wrapper: {what}"));
        for (key, raw) in dict.iter() {
            if WRAPPER_KEYS.contains(&key.as_slice()) {
                continue;
            }
            if key.as_slice() != b"Group" {
                let shown = key.get(..64).unwrap_or(key);
                return Err(wrapper(&format!(
                    "unexpected /{} on the Form",
                    String::from_utf8_lossy(shown)
                )));
            }
            let page_group = match self.doc.objects.get(&page_id) {
                Some(Object::Dictionary(page)) => page.get(b"Group").ok(),
                _ => None,
            };
            let same = match (
                resolve(self.doc, raw),
                page_group.and_then(|g| resolve(self.doc, g)),
            ) {
                (Some((a_id, a)), Some((b_id, b))) => self.with_own_hash_state(|w| {
                    Ok(w.entry_hash(a_id, a)? == w.entry_hash(b_id, b)?)
                })?,
                _ => false,
            };
            if !same {
                return Err(wrapper("/Group differs from the page's"));
            }
        }
        Ok(())
    }
}

/// A Form's `/Matrix` (identity when absent); anything but six numbers is malformed.
fn form_matrix(doc: &lopdf::Document, dict: &Dictionary) -> Result<Matrix, Stop> {
    let Some(raw) = dict.get(b"Matrix").ok() else {
        return Ok(IDENTITY);
    };
    let bad = || Stop::malformed("Form /Matrix");
    let items = match resolve(doc, raw) {
        Some((_, Object::Array(items))) if items.len() == 6 => items,
        _ => return Err(bad()),
    };
    let mut m = IDENTITY;
    for (slot, item) in m.iter_mut().zip(items) {
        *slot = resolve(doc, item)
            .and_then(|(_, o)| number_of(o))
            .ok_or_else(bad)?;
    }
    Ok(m)
}

/// A Form's `/BBox`, normalised; `None` when absent or unreadable.
fn form_bbox(doc: &lopdf::Document, dict: &Dictionary) -> Option<[f64; 4]> {
    let items = match resolve(doc, dict.get(b"BBox").ok()?)? {
        (_, Object::Array(items)) if items.len() == 4 => items,
        _ => return None,
    };
    let v: Vec<f64> = items
        .iter()
        .map(|i| resolve(doc, i).and_then(|(_, o)| number_of(o)))
        .collect::<Option<Vec<f64>>>()?;
    match v.as_slice() {
        [a, b, c, d] => Some([a.min(*c), b.min(*d), a.max(*c), b.max(*d)]),
        _ => None,
    }
}

/// The clip after intersecting with a Form `/BBox` mapped by `m`.
fn clip_with(clip: &ClipState, m: &Matrix, bbox: [f64; 4]) -> ClipState {
    if !axis_aligned(m) {
        return ClipState::Complex;
    }
    let r = transform_rect(m, bbox);
    match clip {
        ClipState::None => ClipState::Rect(r),
        ClipState::Rect(c) => ClipState::Rect(intersect(*c, r)),
        ClipState::Complex => ClipState::Complex,
    }
}

/// `/SMask` (other than `/None`), `/Mask` or `/ImageMask true` on an image.
fn image_masked(doc: &lopdf::Document, dict: &Dictionary) -> bool {
    let smask = dict
        .get(b"SMask")
        .ok()
        .and_then(|o| resolve(doc, o))
        .is_some_and(|(_, o)| o.as_name().ok() != Some(b"None"));
    let image_mask = matches!(
        dict.get(b"ImageMask").ok().and_then(|o| resolve(doc, o)),
        Some((_, Object::Boolean(true)))
    );
    smask || dict.has(b"Mask") || image_mask
}
