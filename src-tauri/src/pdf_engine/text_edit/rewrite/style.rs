//! Planner steps 1–5 (SPEC §B.12): resolve the run, validate the typed text (§A.3.3) and the style
//! values, drop style fields equal to the current value (B7), check that each requested change is
//! available (B8, B9), and compute the written targets (`Tf'`, `Tc'`, colour), each read back from
//! the number that will be written.

use super::{bad_edit, problem, SourceTextStyleIn, StyleTarget, TextEditIn};
use crate::error::AppError;
use crate::pdf_engine::text_edit::encode::num;
use crate::pdf_engine::text_edit::fonts::{face_surface, FontModel, TypingSurface};
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits, Operand, Operator};
use crate::pdf_engine::text_edit::limits::{
    EDIT_TEXT_CHARS_MAX, LETTER_SPACING_MAX_PT, LETTER_SPACING_MIN_PT, SIZE_MAX_PT, SIZE_MIN_PT,
    STYLE_EPSILON,
};
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode as P, StyleField};
use crate::pdf_engine::text_edit::runs::{PageModel, SpaceMode, TextRun};
use crate::pdf_engine::text_edit::walker::ShowRecord;
use std::sync::Arc;

/// A written size must reproduce the requested effective size within this (pt).
const SIZE_READBACK_TOL_PT: f64 = 0.005;

/// Step 1: the run named by the edit, unchanged since it was read, and editable.
pub(super) fn resolve<'m>(
    model: &'m PageModel,
    edit: &TextEditIn,
) -> Result<&'m TextRun, EditProblem> {
    if let Some(reason) = model.page_reason {
        let mut p = problem(P::TextEditRefused, "page refused");
        p.reason = Some(reason);
        return Err(p);
    }
    let run = model
        .run(&edit.run_id)
        .ok_or_else(|| problem(P::Stale, "line not found on the page"))?;
    if run.text != edit.original_text {
        return Err(problem(
            P::Stale,
            "the line's text differs from the request",
        ));
    }
    if let Some(reason) = run.reason {
        let mut p = problem(P::TextEditRefused, "line refused");
        p.reason = Some(reason);
        return Err(p);
    }
    Ok(run)
}

/// §A.3.3: no line breaks, tabs, C0/C1 controls or line/paragraph separators; ≤ 1,000 chars.
pub(super) fn validate_text(text: &str) -> Result<(), EditProblem> {
    let invalid = text.chars().any(|c| {
        let v = u32::from(c);
        v < 0x20 || (0x7f..=0x9f).contains(&v) || c == '\u{2028}' || c == '\u{2029}'
    });
    if invalid {
        return Err(problem(P::InvalidText, "control character"));
    }
    if text.chars().count() > EDIT_TEXT_CHARS_MAX {
        return Err(problem(P::TextTooLong, "more than 1,000 characters"));
    }
    Ok(())
}

/// `#rrggbb` → bytes.
pub(crate) fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let digits = s.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Step 2 (style half): malformed values are a malformed request (`BAD_EDIT`).
pub(super) fn validate_style(style: &SourceTextStyleIn) -> Result<(), AppError> {
    if let Some(v) = style.size_pt {
        if !v.is_finite() || !(SIZE_MIN_PT..=SIZE_MAX_PT).contains(&v) {
            return Err(bad_edit(&format!("sizePt {v} outside 4–144")));
        }
    }
    if let Some(v) = style.letter_spacing_pt {
        if !v.is_finite() || !(LETTER_SPACING_MIN_PT..=LETTER_SPACING_MAX_PT).contains(&v) {
            return Err(bad_edit(&format!("letterSpacingPt {v} outside -2–10")));
        }
    }
    if let Some(fill) = &style.fill {
        if parse_hex(fill).is_none() {
            return Err(bad_edit("fill is not #rrggbb"));
        }
    }
    Ok(())
}

/// The run's current letter spacing in effective points.
pub(crate) fn letter_spacing_pt(run: &TextRun) -> f64 {
    run.tc * run.th * run.text_to_user_x
}

/// Step 3: fields equal to the current value (within `STYLE_EPSILON`) are dropped (B7).
pub(super) fn normalise(run: &TextRun, style: &SourceTextStyleIn) -> SourceTextStyleIn {
    let size_pt = style
        .size_pt
        .filter(|v| (v - run.effective_size).abs() > STYLE_EPSILON);
    let face = style.face.filter(|f| *f != run.face);
    let fill = style.fill.as_ref().and_then(|f| {
        let lower = f.to_ascii_lowercase();
        (run.fill_hex.as_deref() != Some(lower.as_str())).then_some(lower)
    });
    let current = letter_spacing_pt(run);
    let letter_spacing_pt = style
        .letter_spacing_pt
        .filter(|v| (v - current).abs() > STYLE_EPSILON);
    SourceTextStyleIn {
        size_pt,
        face,
        fill,
        letter_spacing_pt,
    }
}

fn unavailable(field: StyleField) -> EditProblem {
    let mut p = problem(P::StyleUnavailable, format!("{field:?} unavailable"));
    p.field = Some(field);
    p
}

/// Step 4: size and face need a `Tf` font (B9); colour needs fill-only rendering; a face needs
/// a sibling group on the page that can draw the whole text (B8). Returns the face surface.
pub(super) fn availability(
    run: &TextRun,
    primary: Option<&FontModel>,
    page_fonts: &[(Vec<u8>, Arc<FontModel>)],
    wanted: &SourceTextStyleIn,
    text: &str,
) -> Result<Option<TypingSurface>, EditProblem> {
    if wanted.size_pt.is_some() && run.font_from_extgstate {
        return Err(unavailable(StyleField::Size));
    }
    if wanted.face.is_some() && run.font_from_extgstate {
        return Err(unavailable(StyleField::Face));
    }
    if wanted.fill.is_some() && run.tr != 0 {
        return Err(unavailable(StyleField::Colour));
    }
    let Some(face) = wanted.face else {
        return Ok(None);
    };
    let face_unavailable = |chars: Vec<char>| {
        let mut p = problem(P::FaceUnavailable, format!("{face:?} face"));
        p.face = Some(face);
        p.chars = chars;
        p
    };
    let surface = primary
        .and_then(|m| face_surface(page_fonts, m, face))
        .ok_or_else(|| face_unavailable(Vec::new()))?;
    let mut missing: Vec<char> = Vec::new();
    for ch in text.chars() {
        if ch == ' ' && run.space_mode == SpaceMode::Kern {
            continue;
        }
        if surface.writer_for(ch).is_none() && !missing.contains(&ch) {
            missing.push(ch);
        }
    }
    if !missing.is_empty() {
        return Err(face_unavailable(missing));
    }
    Ok(Some(surface))
}

/// The size operand of a verbatim `Tf` op (`/F1 11.04 Tf` → `11.04`).
fn tf_token(tf_op: &[u8]) -> Option<Vec<u8>> {
    let ops = lex_content(tf_op, &LexLimits::page(), None).ok()?;
    let [op] = ops.as_slice() else {
        return None;
    };
    if op.operator != Operator::Tf {
        return None;
    }
    match op.operands.get(1)? {
        Operand::Number { span, .. } => tf_op.get(span.clone()).map(<[u8]>::to_vec),
        _ => None,
    }
}

/// Step 5: the targets. `Tf' = Tfs × size / effective`; `Tc' = spacing / (Th × text_to_user_x)`;
/// colour components `/255`; every value is the one read back from what is written.
pub(super) fn targets(
    primary: &ShowRecord,
    run: &TextRun,
    wanted: &SourceTextStyleIn,
    face_surface: Option<TypingSurface>,
) -> Result<StyleTarget, EditProblem> {
    let precision = || problem(P::EditVerifyFailed, "number precision");
    let (tfs, tfs_token, effective_size, size_changed) = match wanted.size_pt {
        Some(pt) => {
            if run.effective_size <= 0.0 || run.tfs == 0.0 {
                return Err(precision());
            }
            let (text, value) = num(run.tfs * pt / run.effective_size)?;
            let row2 = run.effective_size / run.tfs.abs();
            if value <= 0.0 || (value * row2 - pt).abs() > SIZE_READBACK_TOL_PT {
                return Err(precision());
            }
            (value, text.into_bytes(), pt, true)
        }
        None => {
            let token = primary
                .before
                .text
                .font
                .as_ref()
                .and_then(|f| f.tf_op.as_deref())
                .and_then(tf_token)
                .unwrap_or_default();
            (run.tfs, token, run.effective_size, false)
        }
    };
    let (tc, tc_changed) = match wanted.letter_spacing_pt {
        Some(pt) => {
            let scale = run.th * run.text_to_user_x;
            if scale == 0.0 || !scale.is_finite() {
                return Err(precision());
            }
            (num(pt / scale)?.1, true)
        }
        None => (run.tc, false),
    };
    let fill = match &wanted.fill {
        Some(hex) => {
            let bytes = parse_hex(hex).ok_or_else(precision)?;
            let mut rgb = [0.0; 3];
            for (slot, b) in rgb.iter_mut().zip(bytes) {
                *slot = num(f64::from(b) / 255.0)?.1;
            }
            Some(rgb)
        }
        None => None,
    };
    Ok(StyleTarget {
        tfs,
        tfs_token,
        tc,
        fill_changed: fill.is_some(),
        fill,
        face_changed: face_surface.is_some(),
        face_surface,
        size_changed,
        tc_changed,
        face: wanted.face,
        effective_size,
    })
}
