//! PLAN-11…21: no-ops (B7), removal, kern-space writing, typing problems, the fit policy (B16),
//! conflicts and refusals, and several edits on one page.

use super::{
    after_text, ctx, edit, faced, filled, model, ok_plan, plan, plan_one, problem_of, replacement,
    run_with, sized, spaced, style,
};
use crate::pdf_engine::text_edit::reasons::{
    EditProblemCode as P, Face, TextReason, TextWarningCode,
};
use crate::pdf_engine::text_edit::rewrite::{plan_page, SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};

#[test]
fn plan_11_b7_no_ops_write_nothing() {
    let pdf = || helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let c = ctx(pdf());
    let m = model(&c, 0);
    let cases: Vec<(&str, SourceTextStyleIn)> = vec![
        ("text equal, style empty", style()),
        ("bold off (already regular)", faced(Face::Regular)),
        ("size back to the current 12 pt", sized(12.0)),
        ("size within STYLE_EPSILON", sized(12.0004)),
        ("colour equal to the original", filled("#000000")),
        ("letter spacing equal to the current 0", spaced(0.0)),
    ];
    for (what, s) in cases {
        let out = plan(&c, &m, &[edit(&m, "Hello", "Hello", s)]);
        let v = &out.verdicts[0];
        assert!(v.problem.is_none(), "PLAN-11 {what}: {:?}", v.problem);
        assert!(out.plan.is_none(), "PLAN-11 {what}: no plan, no bytes");
        assert_eq!(v.delta_pt, 0.0, "PLAN-11 {what}");
    }
}

#[test]
fn plan_12_empty_text_removes_the_glyphs_and_keeps_the_pen() {
    // Hello = 722 + 556 + 222 + 222 + 556 = 2278 thousandths: pushed forward by that much.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj (World) Tj ET"),
        "HelloWorld",
        "",
        style(),
    );
    assert_eq!(replacement(&out, 0), "[<> -2278] TJ", "PLAN-12 primary");
    // World = 944 + 556 + 333 + 222 + 556: the absorbed member keeps its own travel.
    assert_eq!(
        replacement(&out, 1),
        "[<> -2611] TJ",
        "PLAN-12 absorbed member"
    );
    let run = &ok_plan(&out).runs[0];
    assert!(
        run.expected.glyphs.is_empty() && run.expected.text.is_empty(),
        "PLAN-12"
    );
}

#[test]
fn plan_13_kern_space_writing_and_space_not_writable() {
    // FX-PDFTEX: no space glyph, word gaps are −333 kerns ⇒ kern mode.
    let c = ctx(fx::pdftex());
    let m = model(&c, 0);
    let run = run_with(&m, "Hello World");
    assert_eq!(
        run.space_mode,
        crate::pdf_engine::text_edit::runs::SpaceMode::Kern
    );
    let out = plan(
        &c,
        &m,
        &[edit(&m, "Hello World", "Hello Wet World", style())],
    );
    let r = replacement(&out, 0);
    // Kept "Hello", its −333 gap and "W"; new "et", the typed space as the run's median gap
    // (−333) and "W"; the kept "orld" (the pair kern 80 after the old "W" is dropped).
    assert!(
        r.starts_with("[<48656C6C6F> -333 <576574> -333 <576F726C64> "),
        "PLAN-13 kern space: {r}"
    );
    assert_eq!(
        ok_plan(&out).runs[0].expected.text,
        "Hello Wet World",
        "PLAN-13 reads back"
    );
    // " World" and "Hello " keep the −333 gap at an end of the line, where it is no longer
    // between two glyphs and would not read as a space (review round 1, L-1): refused like a
    // typed space, not as an internal verification failure.
    for bad in [
        "Hello  World",
        " Hello World",
        "Hello World ",
        "Hello W  orld",
        " World",
        "Hello ",
    ] {
        let out = plan(&c, &m, &[edit(&m, "Hello World", bad, style())]);
        assert_eq!(problem_of(&out), P::SpaceNotWritable, "PLAN-13 {bad:?}");
    }
    for good in ["World", "Hello"] {
        let out = plan(&c, &m, &[edit(&m, "Hello World", good, style())]);
        assert_eq!(
            ok_plan(&out).runs[0].expected.text,
            good,
            "PLAN-13 {good:?}"
        );
    }
}

#[test]
fn plan_14_glyph_missing_dedup_in_typing_order() {
    // A Word subset of "Hello" (B3): Y, a and y have no glyph.
    let (_, _, out) = plan_one(fx::subset_without_y(), "Hello", "Yay yo Hey", style());
    let v = &out.verdicts[0];
    let p = v.problem.as_ref().expect("GLYPH_MISSING");
    assert_eq!(p.code, P::GlyphMissing, "PLAN-14");
    assert_eq!(p.chars, vec!['Y', 'a', 'y'], "PLAN-14 dedup, typing order");
    assert!(out.plan.is_none(), "PLAN-14 nothing planned");
}

#[test]
fn plan_15_invalid_text() {
    let pdf = || helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    for bad in [
        "Hel\nlo",
        "Hel\tlo",
        "Hel\u{2028}lo",
        "Hel\u{7}lo",
        "Hel\u{85}lo",
        "Hel\rlo",
    ] {
        let (_, _, out) = plan_one(pdf(), "Hello", bad, style());
        assert_eq!(problem_of(&out), P::InvalidText, "PLAN-15 {bad:?}");
    }
}

#[test]
fn plan_16_text_too_long_at_1001() {
    let pdf = || helvetica_page(b"BT /F1 4 Tf 1 700 Td (i) Tj ET");
    let (_, _, out) = plan_one(pdf(), "i", &"i".repeat(1001), style());
    assert_eq!(problem_of(&out), P::TextTooLong, "PLAN-16 1,001");
    let (_, _, out) = plan_one(pdf(), "i", &"i".repeat(1000), style());
    let v = &out.verdicts[0];
    assert_ne!(
        v.problem.as_ref().map(|p| p.code),
        Some(P::TextTooLong),
        "PLAN-16 1,000"
    );
}

#[test]
fn plan_17_b16_text_outside_visible_area_crop_edge_and_clip() {
    // CropBox [36 48 576 744], line at x 72: 504 pt to the crop edge. W = 11.328 pt at 12 pt.
    let crop = || fx::cropped_offset();
    let (_, _, out) = plan_one(crop(), "Cropped page", &"W".repeat(45), style());
    assert_eq!(
        problem_of(&out),
        P::TextOutsideVisibleArea,
        "PLAN-17 crop edge"
    );
    let (_, _, out) = plan_one(crop(), "Cropped page", &"W".repeat(44), style());
    assert!(
        out.verdicts[0].problem.is_none(),
        "PLAN-17 fits: {:?}",
        out.verdicts[0].problem
    );
    // A clip rectangle 128 pt past the origin.
    let clip = || helvetica_page(b"q 0 0 200 792 re W n BT /F1 12 Tf 72 720 Td (Clip me) Tj ET Q");
    let (_, _, out) = plan_one(clip(), "Clip me", &"W".repeat(12), style());
    assert_eq!(
        problem_of(&out),
        P::TextOutsideVisibleArea,
        "PLAN-17 clip rect"
    );
    let (_, _, out) = plan_one(clip(), "Clip me", &"W".repeat(11), style());
    assert!(out.verdicts[0].problem.is_none(), "PLAN-17 clip fits");
}

#[test]
fn plan_18_overlap_is_a_warning() {
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj ET BT /F1 12 Tf 200 700 Td (Right) Tj ET",
    );
    let (_, _, out) = plan_one(pdf, "Left", &"W".repeat(12), style());
    let v = &out.verdicts[0];
    assert!(v.problem.is_none(), "PLAN-18 not blocking: {:?}", v.problem);
    assert_eq!(
        v.warnings,
        vec![TextWarningCode::NextTextOverlap],
        "PLAN-18"
    );
    assert!(out.plan.is_some(), "PLAN-18 still planned");
}

#[test]
fn plan_19_conflicts_stale_refused_and_bad_style() {
    let c = ctx(fx::duplicate_shadow());
    let m = model(&c, 0);
    let refused = m.runs.first().expect("a run");
    assert_eq!(refused.reason, Some(TextReason::DuplicateText));
    let e = TextEditIn {
        run_id: refused.id.clone(),
        original_text: refused.text.clone(),
        text: "Other".into(),
        style: style(),
    };
    let out = plan(&c, &m, &[e]);
    let p = out.verdicts[0].problem.as_ref().expect("refused");
    assert_eq!(
        (p.code, p.reason),
        (P::TextEditRefused, Some(TextReason::DuplicateText)),
        "PLAN-19 refused"
    );

    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let m = model(&c, 0);
    let e = edit(&m, "Hello", "Help", style());
    let out = plan(&c, &m, &[e.clone(), e.clone()]);
    assert!(out.plan.is_none(), "PLAN-19 duplicate run id");
    for v in &out.verdicts {
        assert_eq!(
            v.problem.as_ref().map(|p| p.code),
            Some(P::EditConflict),
            "PLAN-19 conflict"
        );
    }
    let stale_text = TextEditIn {
        original_text: "Hullo".into(),
        ..e.clone()
    };
    assert_eq!(
        problem_of(&plan(&c, &m, &[stale_text])),
        P::Stale,
        "PLAN-19 text changed"
    );
    let stale_id = TextEditIn {
        run_id: "t1:0-0:0:1-2".into(),
        ..e.clone()
    };
    assert_eq!(
        problem_of(&plan(&c, &m, &[stale_id])),
        P::Stale,
        "PLAN-19 unknown run"
    );
    for bad in [
        sized(3.9),
        sized(f64::NAN),
        spaced(10.5),
        filled("red"),
        filled("#12345"),
    ] {
        let err = plan_page(
            &c,
            &m,
            &[TextEditIn {
                style: bad.clone(),
                ..e.clone()
            }],
        )
        .err()
        .unwrap_or_else(|| panic!("PLAN-19 {bad:?} must be BAD_EDIT"));
        assert_eq!(err.code, "BAD_EDIT", "PLAN-19 {bad:?}");
    }
}

#[test]
fn plan_20_two_edits_in_one_part() {
    let c = ctx(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (First line) Tj ET BT /F1 12 Tf 72 680 Td (Second line) Tj ET",
    ));
    let m = model(&c, 0);
    let out = plan(
        &c,
        &m,
        &[
            edit(&m, "Second line", "2nd line", style()),
            edit(&m, "First line", "1st line", style()),
        ],
    );
    let p = ok_plan(&out);
    assert_eq!(p.edited_parts, vec![0], "PLAN-20 one part");
    assert_eq!(p.splices.len(), 2, "PLAN-20 two splices");
    assert!(
        p.splices[0].joined.start < p.splices[1].joined.start,
        "PLAN-20 sorted"
    );
    let text = after_text(&out);
    assert!(
        text.starts_with("BT /F1 12 Tf 72 700 Td [<31737420"),
        "PLAN-20 {text}"
    );
    assert!(text.contains("Td [<326E64206C696E65>"), "PLAN-20 {text}");
}

#[test]
fn plan_21_edits_in_parts_0_and_2_of_three() {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 700 Td (Part zero) Tj ET",
            b"0 0 1 rg 72 600 100 20 re f",
            b"BT /F1 12 Tf 72 500 Td (Part two) Tj ET",
        ],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let c = ctx(d.build());
    let m = model(&c, 0);
    let out = plan(
        &c,
        &m,
        &[
            edit(&m, "Part zero", "Part 0", style()),
            edit(&m, "Part two", "Part 2", style()),
        ],
    );
    let p = ok_plan(&out);
    assert_eq!(p.edited_parts, vec![0, 2], "PLAN-21");
    assert_eq!(
        p.expected_parts[1], b"0 0 1 rg 72 600 100 20 re f",
        "PLAN-21 part 1 untouched"
    );
    assert_eq!(
        p.splices.iter().map(|s| s.part).collect::<Vec<_>>(),
        vec![0, 2],
        "PLAN-21"
    );
    let concat: Vec<u8> = p.expected_parts.concat();
    assert_eq!(
        p.expected_page_digest,
        crate::pdf_engine::validate_output::content_digest(&concat),
        "PLAN-21 digest of the parts without separator"
    );
}
