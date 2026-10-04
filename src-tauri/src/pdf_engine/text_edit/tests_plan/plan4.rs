//! PLAN-31…38: number formatting, the f32 and read-back guards, the replacement grammar, the
//! self-check, the boundary-kern rule, column-gap absorption and the full-digest rule for edited
//! glyphs.

use super::{ctx, edit, model, ok_plan, plan, plan_one, problem_of, replacement, sized, style};
use crate::pdf_engine::text_edit::encode::{check_replacement_grammar, fmt_num};
use crate::pdf_engine::text_edit::reasons::{EditProblemCode as P, TextWarningCode};
use crate::pdf_engine::text_edit::rewrite::{assemble_page_plan, RunPlan};
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page};
use crate::pdf_engine::text_edit::verify::{walk_and_verify, VerifyFailure};

#[test]
fn plan_31_fmt_num() {
    let ok = |v: f64| fmt_num(v).unwrap_or_else(|e| panic!("PLAN-31 {v}: {e:?}"));
    assert_eq!(ok(1.0), "1");
    assert_eq!(ok(0.5), "0.5");
    assert_eq!(ok(-2.5), "-2.5");
    assert_eq!(ok(1.23456), "1.2346", "4 decimals");
    assert_eq!(ok(0.00004), "0", "rounds to zero");
    assert_eq!(ok(-0.00004), "0", "-0 → 0");
    assert_eq!(ok(-0.0), "0", "-0 → 0");
    assert_eq!(ok(325.42857), "325.4286");
    assert_eq!(ok(1e9), "1000000000", "no exponent at the limit");
    assert_eq!(ok(123456789.123), "123456789.123");
    for bad in [1e9 + 1.0, -2e9, f64::NAN, f64::INFINITY, 1e20] {
        let e = fmt_num(bad)
            .err()
            .unwrap_or_else(|| panic!("PLAN-31 {bad} must fail"));
        assert_eq!(e.code, P::EditVerifyFailed, "PLAN-31 {bad}");
    }
    for v in [0.1, 2.0 / 3.0, 1e-3, 12345.6789, -987.65432] {
        let s = ok(v);
        assert!(
            !s.contains('e') && !s.contains('E'),
            "PLAN-31 no exponent: {s}"
        );
        assert!(
            s.split('.').nth(1).map_or(0, str::len) <= 4,
            "PLAN-31 ≤ 4 dp: {s}"
        );
    }
}

#[test]
fn plan_32_f32_and_readback_guards() {
    // An absorbed member that travels 360,000 pt (a trailing −30,000,000.3 kern): its written
    // number cannot survive a viewer's f32 parse within 0.002 pt ⇒ "number precision".
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj [(world)-30000000.3]TJ ET"),
        "Helloworld",
        "Help",
        style(),
    );
    let v = &out.verdicts[0];
    let p = v.problem.as_ref().expect("PLAN-32 f32 guard");
    assert_eq!(p.code, P::EditVerifyFailed, "PLAN-32");
    assert!(
        p.detail
            .as_deref()
            .unwrap_or("")
            .contains("number precision"),
        "PLAN-32 {p:?}"
    );
    // A tiny Tf operand under a large Tm: 4 decimals cannot express the new size (read back).
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 0.001 Tf 12000 0 0 12000 72 700 Tm (Hi) Tj ET"),
        "Hi",
        "Hi",
        sized(13.0),
    );
    let p = out.verdicts[0]
        .problem
        .as_ref()
        .expect("PLAN-32 read-back guard");
    assert_eq!(p.code, P::EditVerifyFailed, "PLAN-32 read-back");
}

#[test]
fn plan_33_grammar_rejects_everything_but_the_replacement_operators() {
    let ok =
        "0.2 Tw 0.1 Tc T* 1 Tc 0 0 1 rg /F2 12 Tf [<41> -2 <42>] TJ /CS0 cs 0.5 scn 0 Tc /F1 12 Tf";
    assert_eq!(
        check_replacement_grammar(ok.as_bytes(), 1),
        Ok(()),
        "PLAN-33 allowed"
    );
    assert_eq!(
        check_replacement_grammar(b"0 g 0 0 0 1 k 1 g [<>] TJ", 1),
        Ok(())
    );
    let bad: [(&str, &str); 14] = [
        ("q [<41>] TJ Q", "q"),
        ("0 0 9 9 re f [<41>] TJ", "re"),
        ("/Im0 Do [<41>] TJ", "Do"),
        ("2 Tr [<41>] TJ", "Tr"),
        ("1 0 0 1 5 5 cm [<41>] TJ", "cm"),
        ("50 Tz [<41>] TJ", "Tz"),
        ("3 Ts [<41>] TJ", "Ts"),
        ("BT [<41>] TJ ET", "BT"),
        ("1 0 0 1 0 0 Tm [<41>] TJ", "Tm"),
        ("5 0 Td [<41>] TJ", "Td"),
        ("/P BMC [<41>] TJ EMC", "BMC"),
        ("/GS1 gs [<41>] TJ", "gs"),
        ("2 w [<41>] TJ", "w"),
        ("[<41>] TJ [<42>] TJ", "TJ count"),
    ];
    for (bytes, why) in bad {
        let r = check_replacement_grammar(bytes.as_bytes(), 1);
        assert_eq!(r, Err(why), "PLAN-33 {bytes:?}");
    }
    assert!(
        check_replacement_grammar(b"[(unbalanced] TJ", 1).is_err(),
        "PLAN-33 lex"
    );
    assert!(
        check_replacement_grammar(b"BX foo EX [<41>] TJ", 1).is_err(),
        "PLAN-33 BX"
    );
    assert_eq!(
        check_replacement_grammar(b"[<41>] TJ", 2),
        Err("TJ count"),
        "PLAN-33 count"
    );
}

#[test]
fn plan_34_self_check_detects_an_injected_wrong_compensation() {
    let c = ctx(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 680 Td (Next) Tj (after) Tj ET",
    ));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Help", style())]);
    let good = ok_plan(&out);
    let after = good.expected_content(&m.content);
    assert!(
        walk_and_verify(&c, 0, &after, &m.walk, &m.runs, good, None).is_ok(),
        "PLAN-34 the honest plan passes"
    );
    // The same plan with the compensation off by one unit (0.012 pt at 12 pt).
    let mut runs: Vec<RunPlan> = good.runs.clone();
    let s = &mut runs[0].splices[0];
    let text = String::from_utf8_lossy(&s.bytes).replace("] TJ", " 1] TJ");
    s.bytes = text.into_bytes();
    let bad = assemble_page_plan(&m.content, 0, runs);
    let after = bad.expected_content(&m.content);
    let err = walk_and_verify(&c, 0, &after, &m.walk, &m.runs, &bad, None)
        .err()
        .expect("PLAN-34 a wrong compensation must fail");
    assert!(
        matches!(
            err,
            VerifyFailure::EditedMismatch { what: "pen", .. } | VerifyFailure::Drift { .. }
        ),
        "PLAN-34 {err:?}"
    );
}

#[test]
fn plan_35_boundary_rule() {
    // A synthetic-space kern at the prefix/middle boundary is part of the kept text.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(Hello)-333(World)]TJ ET"),
        "Hello World",
        "Hello Earth",
        style(),
    );
    assert!(
        replacement(&out, 0).starts_with("[<48656C6C6F> -333 <45617274"),
        "PLAN-35 synthetic"
    );
    // A pair kern at the boundary is dropped (its pair is gone).
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(AV)-80(x)]TJ ET"),
        "AVx",
        "AWx",
        style(),
    );
    let r = replacement(&out, 0);
    assert!(
        r.starts_with("[<415778> ") && !r.contains("-80"),
        "PLAN-35 pair kern: {r}"
    );
    // An empty middle: the sides meet. Original neighbours keep their kern; new neighbours don't.
    let pdf = || helvetica_page(b"BT /F1 12 Tf 72 700 Td [(A)-80(V)(x)]TJ ET");
    let (_, _, out) = plan_one(pdf(), "AVx", "AV", style());
    assert!(
        replacement(&out, 0).starts_with("[<41> -80 <56> "),
        "PLAN-35 original neighbours"
    );
    let (_, _, out) = plan_one(pdf(), "AVx", "Ax", style());
    let r = replacement(&out, 0);
    assert!(
        r.starts_with("[<4178> ") && !r.contains("-80"),
        "PLAN-35 new neighbours: {r}"
    );
}

#[test]
fn plan_36_column_gap_absorbs_the_width_change() {
    // "Name" → "Names": s = 500 thousandths; the −3000 gap becomes −2500, "Value" stays put,
    // and no compensation is written.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(Name) -3000 (Value)] TJ ET"),
        "Name Value",
        "Names Value",
        style(),
    );
    assert_eq!(
        replacement(&out, 0),
        "[<4E616D6573> -2500 <56616C7565>] TJ",
        "PLAN-36"
    );
    let exp = &ok_plan(&out).runs[0].expected;
    assert_eq!(
        exp.unshifted_from,
        Some(5),
        "PLAN-36 Value keeps its exact origin"
    );
}

#[test]
fn plan_37_absorption_that_would_shrink_the_gap_below_a_space_is_not_applied() {
    // A 1.1 em gap; MM (1.666 em) would leave −566 (not a space): normal compensation instead,
    // and the longer line now runs into "Next".
    let (_, _, out) = plan_one(
        helvetica_page(
            b"BT /F1 12 Tf 72 700 Td [(Name) -1100 (Value)] TJ ET BT /F1 12 Tf 155 700 Td (Next) Tj ET",
        ),
        "Name Value",
        "NameMM Value",
        style(),
    );
    assert_eq!(
        replacement(&out, 0),
        "[<4E616D654D4D> -1100 <56616C7565> 1666] TJ",
        "PLAN-37"
    );
    assert_eq!(
        out.verdicts[0].warnings,
        vec![TextWarningCode::NextTextOverlap],
        "PLAN-37"
    );
    assert_eq!(
        ok_plan(&out).runs[0].expected.unshifted_from,
        None,
        "PLAN-37"
    );
}

#[test]
fn plan_38_edited_tr2_run_keeps_stroke_line_width_and_dash() {
    let c = ctx(fx::stroke_text(2, [0.5, 0.5], ["[3] 0", "[3] 0"]));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Boldface", "Bold face", style())]);
    let p = ok_plan(&out);
    let after = p.expected_content(&m.content);
    let walk = walk_and_verify(&c, 0, &after, &m.walk, &m.runs, p, None)
        .unwrap_or_else(|f| panic!("PLAN-38 self-check: {f}"));
    let edited: Vec<_> = walk
        .records
        .iter()
        .filter(|r| !r.glyphs.is_empty())
        .collect();
    assert!(!edited.is_empty(), "PLAN-38");
    for r in edited {
        let s = &r.before;
        assert_eq!(s.text.tr, 2, "PLAN-38 Tr");
        assert_eq!(s.gs.line_width, 0.5, "PLAN-38 line width");
        assert_eq!(&s.gs.dash.0[..], &[3.0], "PLAN-38 dash");
    }
    // Colour is not offered for Tr 2 (only the fill would change, not the stroke).
    let out = plan(
        &c,
        &m,
        &[edit(&m, "Boldface", "Boldface", super::filled("#ff0000"))],
    );
    assert_eq!(problem_of(&out), P::StyleUnavailable, "PLAN-38 colour");
}
