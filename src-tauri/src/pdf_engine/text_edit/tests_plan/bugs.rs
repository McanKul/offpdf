//! The mobile Edit-text bugs (SPEC Appendix A) as named regressions on the desktop engine:
//! B1–B3, B7–B10, B14–B16 here; B13 (independent engines in the app) in `tests_independent.rs`.

use super::{
    ctx, edit, faced, filled, model, ok_plan, plan, plan_one, problem_of, replacement, run_with,
    sized, spaced, style,
};
use crate::pdf_engine::text_edit::reasons::{
    EditProblemCode as P, Face, ProblemCtx, StyleField, TextReason, TextWarningCode,
};
use crate::pdf_engine::text_edit::rewrite::{assemble_page_plan, RunPlan, TextEditIn};
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page};
use crate::pdf_engine::text_edit::verify::{walk_and_verify, VerifyFailure};

#[test]
fn b1_tj_kerns_count_in_the_pen_and_word_gaps_read_as_spaces() {
    // `[(AB) -500 (CD)] TJ (tail) Tj`: the −500 kern moves "CD" and "tail" (5 pt at 10 pt).
    let c = ctx(fx::kerned_then_tail());
    let m = model(&c, 0);
    let run = &m.runs[0];
    assert_eq!(run.text, "AB CDtail", "B1 a 0.5 em kern reads as one space");
    let out = plan(&c, &m, &[edit(&m, "AB CDtail", "AB CDEtail", style())]);
    let p = ok_plan(&out);
    let primary = &m.walk.records[run.members[0]];
    assert_eq!(
        p.runs[0].expected.primary_pen_after, primary.pen_after,
        "B1 pen after includes the kern"
    );
    let after = p.expected_content(&m.content);
    assert!(
        walk_and_verify(&c, 0, &after, &m.walk, &m.runs, p, None).is_ok(),
        "B1 the follower stays"
    );
    // pdfTeX word gaps: a typed space is written as a gap and reads back as a space.
    let (_, _, out) = plan_one(fx::pdftex(), "Hello World", "Hello Wet World", style());
    assert_eq!(
        ok_plan(&out).runs[0].expected.text,
        "Hello Wet World",
        "B1 no \"HelloWorld\""
    );
}

#[test]
fn b2_size_is_in_effective_points_not_the_tf_operand() {
    let (_, m, out) = plan_one(fx::tf1_tm12(), "Hi", "Hi", sized(14.0));
    assert_eq!(run_with(&m, "Hi").tfs, 1.0, "B2 the Tf operand is 1");
    let t = &ok_plan(&out).runs[0].target;
    assert_eq!(t.tfs, 1.1667, "B2 Tf' = 1 × 14 / 12, never 14");
    assert!(
        replacement(&out, 0).starts_with("/F1 1.1667 Tf ["),
        "B2 {}",
        replacement(&out, 0)
    );
}

#[test]
fn b3_a_subset_never_claims_glyphs_it_lacks() {
    let c = ctx(fx::subset_without_y());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Yellow", style())]);
    let p = out.verdicts[0].problem.as_ref().expect("B3 GLYPH_MISSING");
    assert_eq!(
        (p.code, p.chars.clone()),
        (P::GlyphMissing, vec!['Y', 'w']),
        "B3"
    );
    // The width table has a slot for Y (0) but no glyph: never typeable.
    let font = &m.walk.page_fonts[0].1;
    assert!(
        font.code_for('Y', &[], &[]).is_none(),
        "B3 Y not in the alphabet"
    );
}

#[test]
fn b7_no_op_toggles_write_nothing() {
    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let m = model(&c, 0);
    for s in [
        style(),
        faced(Face::Regular),
        sized(12.0),
        spaced(0.0),
        filled("#000000"),
    ] {
        let out = plan(&c, &m, &[edit(&m, "Hello", "Hello", s.clone())]);
        assert!(
            out.plan.is_none() && out.verdicts[0].problem.is_none(),
            "B7 {s:?}"
        );
    }
}

#[test]
fn b8_face_problem_names_the_requested_face_and_checks_its_alphabet() {
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        faced(Face::Italic),
    );
    let p = out.verdicts[0]
        .problem
        .clone()
        .expect("B8 FACE_UNAVAILABLE");
    assert_eq!(
        (p.code, p.face),
        (P::FaceUnavailable, Some(Face::Italic)),
        "B8"
    );
    let ctx = ProblemCtx {
        page_number: Some(1),
        file_name: None,
        face: None,
        reason: None,
    };
    let e = P::FaceUnavailable.to_app_error(&p, &ctx);
    assert!(
        e.message.contains("italic") && !e.message.contains("bold"),
        "B8 says italic: {}",
        e.message
    );
    // The missing list is computed against the target face (PLAN-30 has the bold subset case).
}

#[test]
fn b9_extgstate_font_size_is_unavailable_not_ignored() {
    let (_, _, out) = plan_one(fx::extgstate_font(), "Hi there", "Hi there", sized(20.0));
    let p = out.verdicts[0].problem.as_ref().expect("B9");
    assert_eq!(
        (p.code, p.field),
        (P::StyleUnavailable, Some(StyleField::Size)),
        "B9"
    );
    assert!(out.plan.is_none(), "B9 nothing written");
}

#[test]
fn b10_a_rewrite_is_never_skipped_silently() {
    // A show op that straddles two parts is refused, not skipped.
    let c = ctx(fx::straddling_op());
    let m = model(&c, 0);
    let refused = m.runs.iter().find(|r| r.text == "Hello").expect("run");
    assert_eq!(
        refused.reason,
        Some(TextReason::SplitContent),
        "B10 refused up front"
    );
    let e = TextEditIn {
        run_id: refused.id.clone(),
        original_text: "Hello".into(),
        text: "Help".into(),
        style: style(),
    };
    let out = plan(&c, &m, &[e, edit(&m, "After", "Later", style())]);
    assert_eq!(out.verdicts.len(), 2, "B10 one verdict per edit");
    assert_eq!(
        out.verdicts[0].problem.as_ref().map(|p| p.code),
        Some(P::TextEditRefused),
        "B10"
    );
    assert!(
        out.plan.is_none(),
        "B10 no partial plan when any edit fails"
    );
    // Two edits of one line conflict: both verdicts say so.
    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let m = model(&c, 0);
    let e = edit(&m, "Hello", "Help", style());
    let out = plan(&c, &m, &[e.clone(), e]);
    assert!(
        out.verdicts
            .iter()
            .all(|v| v.problem.as_ref().map(|p| p.code) == Some(P::EditConflict)),
        "B10 conflict"
    );
}

#[test]
fn b14_restores_are_verbatim_never_rounded() {
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 0.123456 Tc /F1 12.34567 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        spaced(1.0),
    );
    assert!(
        replacement(&out, 0).ends_with("] TJ 0.123456 Tc"),
        "B14 Tc: {}",
        replacement(&out, 0)
    );
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12.34567 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        sized(14.0),
    );
    assert!(
        replacement(&out, 0).ends_with("] TJ /F1 12.34567 Tf"),
        "B14 Tf: {}",
        replacement(&out, 0)
    );
}

#[test]
fn b15_paint_state_after_the_edit_is_compared() {
    // A colour change whose restore is lost turns the later square red: refused (B15).
    let c = ctx(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 72 600 50 50 re f",
    ));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Hello", filled("#ff0000"))]);
    let good = ok_plan(&out);
    let mut runs: Vec<RunPlan> = good.runs.clone();
    let s = &mut runs[0].splices[0];
    s.bytes = String::from_utf8_lossy(&s.bytes)
        .replace("] TJ 0 g", "] TJ")
        .into_bytes();
    let bad = assemble_page_plan(&m.content, 0, runs);
    let after = bad.expected_content(&m.content);
    let r = walk_and_verify(&c, 0, &after, &m.walk, &m.runs, &bad, None).err();
    assert_eq!(
        r,
        Some(VerifyFailure::PaintChanged {
            index: 0,
            field: "fill"
        }),
        "B15"
    );
}

#[test]
fn b16_text_past_the_page_edge_is_blocked_and_overlap_is_a_warning() {
    let pdf = || {
        helvetica_page(b"BT /F1 12 Tf 500 700 Td (Edge) Tj ET BT /F1 12 Tf 72 680 Td (Left) Tj ET BT /F1 12 Tf 120 680 Td (Right) Tj ET")
    };
    // 612 − 500 = 112 pt to the MediaBox edge; ten W = 113.28 pt.
    let (_, _, out) = plan_one(pdf(), "Edge", &"W".repeat(10), style());
    assert_eq!(problem_of(&out), P::TextOutsideVisibleArea, "B16 page edge");
    let (_, _, out) = plan_one(pdf(), "Left", "Left side long", style());
    assert_eq!(
        out.verdicts[0].warnings,
        vec![TextWarningCode::NextTextOverlap],
        "B16 overlap warns"
    );
    assert!(out.plan.is_some(), "B16 overlap is not blocking");
}
