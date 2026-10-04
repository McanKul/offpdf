//! Text operators of the walker (SPEC §B.10 glyph math): text state, positioning and the four
//! show ops. `Trm = [Tfs·Th 0 0 Tfs 0 Ts] × Tm × CTM`; each code advances by
//! `(w0·Tfs + Tc + Tw·[1-byte code 32])·Th` (Tc per code, never per byte), and a TJ number `n` by
//! `−n/1000·Tfs·Th`, which also counts in `advance_ts` (B1).
//!
//! The pen is tracked as reliable or not: an op whose advance the model cannot know (an
//! odd-length string in a 2-byte font, no font, a code without a width, or any glyph of a font
//! whose widths are not proven to be the viewers' — `advance_proven`) leaves `pen_unknown` set
//! for every later op of the same text object until a line move (`Td TD Tm T* ' "`) or `BT`.
//! A `Q` inside the text object that restores other text matrices (`Exec::tm_unsettled`) does
//! the same until `Tm` or `BT`.
//!
//! Every glyph, TJ kern and character of glyph text is charged to its page budget as it is
//! made (`KERNS_PER_PAGE_MAX`, `TEXT_CHARS_PER_PAGE_MAX`), never after the op: one TJ array of
//! 48 MiB of strings must not build millions of glyphs before it is refused. Their bytes (and the
//! record's) are charged to the page-model budget the same way (`budget::ModelBudget`).

use super::gfx::{finite, num, six};
use super::{
    Exec, Frame, GlyphRec, RecElem, ShowOp, ShowRecord, Stop, Walked, Walker, KERNS_PER_PAGE_MAX,
    TEXT_CHARS_PER_PAGE_MAX,
};
use crate::pdf_engine::text_edit::context::{resolve, Lookup};
use crate::pdf_engine::text_edit::fonts::{number_of, Code, FontModel};
use crate::pdf_engine::text_edit::geometry::{
    apply, apply_linear, bbox_of, mul, translate, Matrix,
};
use crate::pdf_engine::text_edit::lexer::{Op, Operand, Operator, Span};
use crate::pdf_engine::text_edit::limits::GLYPHS_PER_PAGE_MAX;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::{FontUse, StateDigest};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::mem::size_of;
use std::sync::Arc;

/// Ascent/descent (em) when there is no font to ask.
const FALLBACK_ASCENT: f64 = 0.8;
const FALLBACK_DESCENT: f64 = -0.2;

/// `Td`, `TD`, `Tm`, `T*` (inside a text object only).
pub(super) fn position(ex: &mut Exec, op: &Op) -> Walked {
    if !ex.in_text {
        return Err(Stop::malformed(format!(
            "text positioning outside BT at byte {}",
            op.op_span.start
        )));
    }
    match op.operator {
        Operator::Td => move_line(ex, num(op, 0)?, num(op, 1)?),
        Operator::TD => {
            let ty = num(op, 1)?;
            ex.gs.text.tl = -ty;
            move_line(ex, num(op, 0)?, ty);
        }
        Operator::Tm => {
            let m = six(op)?;
            ex.tm = m;
            ex.tlm = m;
            ex.tm_base = m;
            ex.tm_unsettled = false;
        }
        _ => next_line(ex),
    }
    ex.pen_unknown = false;
    finite(&ex.tm, op)
}

/// Whether the model has no width for `code` in a font that has its codes (a simple-font code
/// whose glyph has no `/Widths` entry and no `/MissingWidth`): viewers then use a width of their
/// own (poppler: the Standard-14 metrics under a Standard-14 name), so the pen after it is not
/// known. 2-byte codes always have one (`/W`, else `/DW`).
pub(crate) fn width_unknown(model: &FontModel, code: Code) -> bool {
    code.len == 1 && model.info(code).is_some_and(|i| i.width1000.is_none())
}

/// Whether viewers advance the pen by exactly the model's widths (`FontModel::width`) for every
/// code of this font, so the pen after its glyphs stays known. A model keeps only its
/// lowest-numbered refusal (§A.10), so a refusal is trusted only when nothing width-related can
/// hide behind it, and the dictionary is checked where one could:
/// - simple fonts (`Type1`, `TrueType`): no refusal, `NO_TOUNICODE` or `AMBIGUOUS_UNICODE` (the
///   only refusals ranked after `MISSING_WIDTHS`; a model refused earlier may have no codes at
///   all, e.g. a short `/Widths` or an unreadable descriptor, and then counts every advance 0);
/// - `Type0`: horizontal, `/Encoding /Identity-H` (code = CID, widths by `/W` and `/DW`; a
///   predefined CMap such as `UniJIS-UCS2-H` maps codes to other CIDs) and not `FONT_UNSUPPORTED`
///   (a bad `/W`, `/DW`, descendant or descriptor); `FONT_NOT_EMBEDDED` and the program refusals
///   leave `/W` untouched;
/// - `Type3`: `/FontMatrix` of six numbers with `a ≠ 0` and `b = 0` (viewers advance by `w × a`
///   along the baseline), a well-formed `/Widths`, and no `/FontDescriptor /MissingWidth` other
///   than 0 (`type3_no_missing_width`).
pub(crate) fn advance_proven(doc: &Document, dict: &Dictionary, model: &FontModel) -> bool {
    use TextReason as R;
    if model.vertical {
        return false;
    }
    let name = |key: &[u8]| match dict.get(key).ok().and_then(|o| resolve(doc, o)) {
        Some((_, Object::Name(n))) => Some(n.as_slice()),
        _ => None,
    };
    match name(b"Subtype") {
        Some(b"Type1" | b"TrueType") => matches!(
            model.refusal,
            None | Some(R::NoTounicode | R::AmbiguousUnicode)
        ),
        Some(b"Type0") => {
            name(b"Encoding") == Some(&b"Identity-H"[..])
                && !matches!(model.refusal, Some(R::FontUnsupported | R::Vertical))
        }
        Some(b"Type3") => {
            type3_advances_along_baseline(doc, dict)
                && widths_well_formed(doc, dict)
                && type3_no_missing_width(doc, dict)
        }
        _ => false,
    }
}

/// No `/FontDescriptor`, or one whose `/MissingWidth` is absent or 0. The font model gives a
/// Type3 code outside `/FirstChar…/LastChar` width 0, while poppler and pdf.js give it the
/// descriptor's `/MissingWidth` (and scale it differently: × 0.001 and × `/FontMatrix`).
fn type3_no_missing_width(doc: &Document, dict: &Dictionary) -> bool {
    let Ok(raw) = dict.get(b"FontDescriptor") else {
        return true;
    };
    let Some((_, Object::Dictionary(desc))) = resolve(doc, raw) else {
        return false;
    };
    match desc.get(b"MissingWidth") {
        Err(_) => true,
        Ok(mw) => resolve(doc, mw)
            .and_then(|(_, o)| number_of(o))
            .is_some_and(|v| v == 0.0),
    }
}

/// Charges `n` to one of the walk's per-page counts; past `max` the page is `PAGE_TOO_COMPLEX`.
fn charge(count: &mut usize, n: usize, max: usize, what: &'static str) -> Walked {
    *count = count.saturating_add(n);
    if *count > max {
        return Err(Stop::new(TextReason::PageTooComplex, what));
    }
    Ok(())
}

/// `/FontMatrix [a b c d e f]` with `a ≠ 0` (and `1000·a` finite, the scale the font model uses)
/// and `b = 0`: a glyph's displacement `(w, 0)` maps to `(w·a, 0)`.
fn type3_advances_along_baseline(doc: &Document, dict: &Dictionary) -> bool {
    let Some((_, Object::Array(items))) =
        dict.get(b"FontMatrix").ok().and_then(|o| resolve(doc, o))
    else {
        return false;
    };
    let values: Option<Vec<f64>> = items
        .iter()
        .map(|i| resolve(doc, i).and_then(|(_, o)| number_of(o)))
        .collect();
    matches!(
        values.as_deref(),
        Some([a, b, _, _, _, _]) if *a != 0.0 && (a * 1000.0).is_finite() && *b == 0.0
    )
}

/// `/Widths` as the font model reads it: present, `/FirstChar` and `/LastChar` integers 0–255
/// with `LastChar ≥ FirstChar`, and exactly `LastChar − FirstChar + 1` numbers.
fn widths_well_formed(doc: &Document, dict: &Dictionary) -> bool {
    let get = |key: &[u8]| dict.get(key).ok().and_then(|o| resolve(doc, o));
    let int = |key: &[u8]| match get(key) {
        Some((_, Object::Integer(v))) if (0..=255).contains(v) => usize::try_from(*v).ok(),
        _ => None,
    };
    let Some((_, Object::Array(items))) = get(b"Widths") else {
        return false;
    };
    let (Some(first), Some(last)) = (int(b"FirstChar"), int(b"LastChar")) else {
        return false;
    };
    last >= first
        && last.checked_sub(first).and_then(|n| n.checked_add(1)) == Some(items.len())
        && items
            .iter()
            .all(|i| resolve(doc, i).and_then(|(_, o)| number_of(o)).is_some())
}

fn move_line(ex: &mut Exec, tx: f64, ty: f64) {
    ex.tlm = mul(&translate(tx, ty), &ex.tlm);
    ex.tm = ex.tlm;
}

fn next_line(ex: &mut Exec) {
    let tl = ex.gs.text.tl;
    move_line(ex, 0.0, -tl);
}

/// One element of a show op's operands.
enum Item<'o> {
    Str(&'o [u8]),
    Kern(f64, Span),
}

fn items(op: &Op) -> Vec<Item<'_>> {
    match op.operator {
        Operator::TJ => match op.operands.first() {
            Some(Operand::Array { items, .. }) => items
                .iter()
                .filter_map(|i| match i {
                    Operand::Str { bytes, .. } => Some(Item::Str(bytes)),
                    Operand::Number { value, span } => Some(Item::Kern(*value, span.clone())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        },
        _ => op
            .operands
            .last()
            .and_then(Operand::as_str_bytes)
            .map(|b| vec![Item::Str(b)])
            .unwrap_or_default(),
    }
}

impl<'a> Walker<'a> {
    /// `Tc Tw Tz TL Tf Tr Ts`.
    pub(super) fn text_state(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let t = &mut ex.gs.text;
        match op.operator {
            Operator::Tc => {
                t.tc = num(op, 0)?;
                t.tc_src = Some(frame.op_bytes(op));
            }
            Operator::Tw => {
                t.tw = num(op, 0)?;
                t.tw_src = Some(frame.op_bytes(op));
            }
            Operator::Tz => t.th = num(op, 0)? / 100.0,
            Operator::TL => t.tl = num(op, 0)?,
            Operator::Ts => t.ts = num(op, 0)?,
            Operator::Tr => {
                let tr = num(op, 0)?;
                if tr.fract() != 0.0 || !(0.0..=7.0).contains(&tr) {
                    return Err(Stop::malformed(format!(
                        "text render mode {tr} at byte {}",
                        op.op_span.start
                    )));
                }
                t.tr = tr as i64;
            }
            Operator::Tf => return self.set_font(frame, ex, op),
            _ => {}
        }
        Ok(())
    }

    fn set_font(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        let name = op
            .operands
            .first()
            .and_then(Operand::as_name)
            .unwrap_or_default();
        let size = num(op, 1)?;
        let (hash, font_ref) = match frame.res.font(self.doc, name) {
            Lookup::Found((key, dict)) => {
                let model = self.load_font(key, dict)?;
                if frame.root {
                    self.note_root_font(name, &model)?;
                }
                (model.content_hash, Some((key, model)))
            }
            Lookup::Missing => (0, None),
            Lookup::Broken(w) => return Err(Stop::malformed(format!("font {w}"))),
        };
        ex.gs.text.font = Some(FontUse {
            resource: Some(Arc::from(name)),
            content_hash: hash,
            from_extgstate: false,
            tf_op: Some(frame.op_bytes(op)),
        });
        ex.gs.text.tfs = size;
        ex.gs.font_ref = font_ref;
        Ok(())
    }

    /// `Tj`, `TJ`, `'`, `"`.
    pub(super) fn show(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        if !ex.in_text {
            return Err(Stop::malformed(format!(
                "text shown outside BT at byte {}",
                op.op_span.start
            )));
        }
        let kind = match op.operator {
            Operator::TJ => ShowOp::TJ,
            Operator::Quote => ShowOp::Quote,
            Operator::DoubleQuote => ShowOp::DoubleQuote,
            _ => ShowOp::Tj,
        };
        if kind == ShowOp::DoubleQuote {
            let span_text = |i: usize| -> Vec<u8> {
                op.operands
                    .get(i)
                    .map(|o| frame.span_bytes(o.span()).to_vec())
                    .unwrap_or_default()
            };
            let (aw, ac) = (num(op, 0)?, num(op, 1)?);
            let t = &mut ex.gs.text;
            t.tw = aw;
            t.tw_src = Some(Arc::from([span_text(0), b" Tw".to_vec()].concat()));
            t.tc = ac;
            t.tc_src = Some(Arc::from([span_text(1), b" Tc".to_vec()].concat()));
        }
        if matches!(kind, ShowOp::Quote | ShowOp::DoubleQuote) {
            next_line(ex);
            ex.pen_unknown = false;
        }
        let before = self.digest(ex)?;
        let rec = self.glyph_run(frame, ex, op, kind, before)?;
        if ex.gs.text.tr >= 4 && !rec.glyphs.is_empty() {
            ex.text_clip = true;
        }
        let proven = rec
            .font_key
            .and_then(|k| self.pen_proofs.get(&k))
            .copied()
            .unwrap_or(false);
        let advance_unknown = rec.split_error
            || match &rec.font {
                None => !rec.glyphs.is_empty(),
                Some(m) => {
                    (!proven && !rec.glyphs.is_empty())
                        || rec.glyphs.iter().any(|g| width_unknown(m, g.code))
                }
            };
        if advance_unknown {
            ex.pen_unknown = true;
        }
        self.mem.push(&mut self.records, rec)
    }

    /// The glyph math of one show op, drawn with the state `before`.
    fn glyph_run(
        &mut self,
        frame: &Frame<'_, 'a>,
        ex: &mut Exec,
        op: &Op,
        kind: ShowOp,
        before: Arc<StateDigest>,
    ) -> Result<ShowRecord, Stop> {
        let tm_before = ex.tm;
        let t = ex.gs.text.clone();
        let (tfs, th, tc, tw, ts) = (t.tfs, t.th, t.tc, t.tw, t.ts);
        let font = ex.gs.font_ref.clone();
        let model = font.as_ref().map(|(_, m)| m);
        let (ascent, descent) = model.map_or((FALLBACK_ASCENT, FALLBACK_DESCENT), |m| {
            (m.ascent, m.descent)
        });
        let params: Matrix = [tfs * th, 0.0, 0.0, tfs, 0.0, ts];
        let text_to_user = mul(&params, &mul(&ex.tm, &ex.gs.ctm));
        let pen_before = apply(&text_to_user, 0.0, 0.0);
        let mut glyphs: Vec<GlyphRec> = Vec::new();
        let mut elems: Vec<RecElem> = Vec::new();
        let mut advance_ts = 0.0;
        let mut split_error = false;
        // The op's items and each string's codes live while the op is shown.
        let listed = match op.operands.first() {
            Some(Operand::Array { items, .. }) => items.len(),
            _ => 1,
        };
        self.mem.scratch(listed * size_of::<Item<'_>>())?;
        for item in items(op) {
            match item {
                Item::Kern(n, span) => {
                    charge(&mut self.kerns, 1, KERNS_PER_PAGE_MAX, "TJ kerns per page")?;
                    let tx = -n / 1000.0 * tfs * th;
                    advance_ts += -n / 1000.0 * tfs;
                    ex.tm = mul(&translate(tx, 0.0), &ex.tm);
                    self.mem
                        .push(&mut elems, RecElem::Kern { value: n, span })?;
                }
                Item::Str(bytes) => {
                    let codes_bytes = bytes.len() * size_of::<Code>();
                    self.mem.scratch(codes_bytes)?;
                    let codes = match model {
                        Some(m) => m.split_codes(bytes).unwrap_or_else(|_| {
                            split_error = true;
                            m.split_codes(bytes.get(..bytes.len() & !1).unwrap_or_default())
                                .unwrap_or_default()
                        }),
                        None => bytes
                            .iter()
                            .map(|b| Code {
                                value: u32::from(*b),
                                len: 1,
                            })
                            .collect(),
                    };
                    for code in codes {
                        charge(&mut self.glyphs, 1, GLYPHS_PER_PAGE_MAX, "glyphs per page")?;
                        let text = model.and_then(|m| m.text(code));
                        let chars = text.map_or(0, |t| t.chars().count());
                        charge(
                            &mut self.text_chars,
                            chars,
                            TEXT_CHARS_PER_PAGE_MAX,
                            "text characters per page",
                        )?;
                        self.mem.hold(text.map_or(0, str::len))?;
                        self.mem.grow(&mut glyphs, 1)?;
                        self.mem.grow(&mut elems, 1)?;
                        let w1000 = model.map_or(0.0, |m| m.width(code));
                        let w0 = w1000 / 1000.0;
                        let word = model.is_some_and(|m| m.is_word_space(code));
                        let m0 = mul(&ex.tm, &ex.gs.ctm);
                        let trm = mul(&params, &m0);
                        let tx = (w0 * tfs + tc + if word { tw } else { 0.0 }) * th;
                        let corners = [
                            apply(&trm, 0.0, descent),
                            apply(&trm, w0, descent),
                            apply(&trm, 0.0, ascent),
                            apply(&trm, w0, ascent),
                        ];
                        glyphs.push(GlyphRec {
                            code,
                            text: text.map(str::to_string),
                            origin: apply(&trm, 0.0, 0.0),
                            advance_user: apply_linear(&m0, tx, 0.0),
                            width1000: w1000,
                            bbox: bbox_of(&corners).unwrap_or_default(),
                        });
                        elems.push(RecElem::Glyph(glyphs.len().saturating_sub(1)));
                        advance_ts += w0 * tfs + tc + if word { tw } else { 0.0 };
                        ex.tm = mul(&translate(tx, 0.0), &ex.tm);
                    }
                    self.mem.unscratch(codes_bytes);
                }
            }
        }
        self.mem.unscratch(listed * size_of::<Item<'_>>());
        // A record is kept for the page's lifetime: no spare capacity (a one-glyph op would
        // otherwise hold room for four glyphs and four elements).
        self.mem.fit(&mut glyphs);
        self.mem.fit(&mut elems);
        let pen_after = apply(&mul(&params, &mul(&ex.tm, &ex.gs.ctm)), 0.0, 0.0);
        let numbers_ok = advance_ts.is_finite()
            && [pen_before.0, pen_before.1, pen_after.0, pen_after.1]
                .iter()
                .all(|v| v.is_finite())
            && glyphs.iter().all(|g| {
                g.bbox.iter().all(|v| v.is_finite())
                    && g.advance_user.0.is_finite()
                    && g.advance_user.1.is_finite()
            });
        if !numbers_ok || !text_to_user.iter().all(|v| v.is_finite()) {
            return Err(Stop::malformed(format!(
                "non-finite text position at byte {}",
                op.op_span.start
            )));
        }
        let spans = op.operands.len() * size_of::<Span>();
        self.mem
            .hold(spans + frame.chain.len() * size_of::<ObjectId>())?;
        let after = self.digest(ex)?;
        Ok(ShowRecord {
            seq: self.next_seq()?,
            depth: frame.depth,
            form_chain: frame.chain.clone(),
            op: kind,
            span: frame.joined.then(|| op.span.clone()),
            local_span: op.span.clone(),
            operand_spans: op.operands.iter().map(|o| o.span().clone()).collect(),
            before,
            after,
            tm_before,
            tm_after: ex.tm,
            glyphs,
            elems,
            pen_before,
            pen_after,
            advance_ts,
            text_to_user,
            after_unproven_inline_image: op.after_unproven_inline_image,
            font_key: font.as_ref().map(|(k, _)| *k),
            font: font.map(|(_, m)| m),
            split_error,
            pen_unknown: ex.pen_unknown || ex.tm_unsettled,
        })
    }
}
