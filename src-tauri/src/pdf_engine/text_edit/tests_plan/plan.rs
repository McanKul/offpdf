//! PLAN-01…10: the minimal diff, boundary kerns, hex middles, the §B.12 worked example, the
//! compensation, `'` and `"` prefixes, Tz/Tc/Tw and the B2 size rule.

use super::{after_text, close, ok_plan, plan_one, replacement, run_with, sized, style};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, word_font, DocBuilder, PageSpec,
};

/// A page drawing `content` with FX-WORD's WinAnsi TrueType subset of `chars` as `/F1`.
pub(crate) fn word_page(chars: &str, content: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = word_font(&mut d.b, "ABCDEF+Calibri", chars);
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

#[test]
fn plan_01_minimal_diff_keeps_prefix_and_suffix_codes_and_kerns() {
    // A V (−80) A T (20) x y z: "y" → "Q" keeps every other code and both kerns byte-exact.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(AV)-80.5(AT)20(xyz)]TJ ET"),
        "AVATxyz",
        "AVATxQz",
        style(),
    );
    // Q (778) replaces y (500): the pen is pulled back by 278 thousandths (a positive number).
    assert_eq!(
        replacement(&out, 0),
        "[<4156> -80.5 <4154> 20 <78517A> 278] TJ",
        "PLAN-01"
    );
    let run = &ok_plan(&out).runs[0];
    assert_eq!(run.expected.prefix_glyphs, 5, "PLAN-01 prefix A V A T x");
    assert_eq!(run.expected.suffix_glyphs, 1, "PLAN-01 suffix z");
}

#[test]
fn plan_02_boundary_kerns_dropped_only_when_not_synthetic() {
    // The pair kern 30 between "o" and "W" no longer has its pair once "W" changes: dropped.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(To)30(Wn)]TJ ET"),
        "ToWn",
        "ToMn",
        style(),
    );
    let r = replacement(&out, 0);
    assert!(!r.contains(" 30 "), "PLAN-02 pair kern dropped: {r}");
    assert!(r.starts_with("[<546F4D6E>"), "PLAN-02 codes: {r}");
    // A synthetic space (−333 ≥ 0.2 em) at the boundary is part of the kept text: kept.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td [(Hello)-333(World)]TJ ET"),
        "Hello World",
        "Hello Earth",
        style(),
    );
    let r = replacement(&out, 0);
    assert!(
        r.starts_with("[<48656C6C6F> -333 <"),
        "PLAN-02 synthetic kept: {r}"
    );
}

#[test]
fn plan_03_middle_is_uppercase_hex() {
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (abc) Tj ET"),
        "abc",
        "a\u{e9}c",
        style(),
    );
    // é is WinAnsi 0xE9; a Tj becomes a TJ.
    let r = replacement(&out, 0);
    assert!(r.starts_with("[<61E963>"), "PLAN-03 {r}");
    assert!(r.ends_with("] TJ"), "PLAN-03 {r}");
}

#[test]
fn plan_04_worked_example_byte_for_byte() {
    // §B.12: Word-style WinAnsi TrueType, "Invoice 2026" → "Invoice 2027".
    let content = "BT /F1 11.04 Tf 1 0 0 1 72 700 Tm [(Inv)12(oice 2026)]TJ ET";
    let (_, _, out) = plan_one(
        word_page("Invoice 20267", content),
        "Invoice 2026",
        "Invoice 2027",
        style(),
    );
    assert_eq!(
        replacement(&out, 0),
        "[<496E76> 12 <6F6963652032303237>] TJ",
        "PLAN-04"
    );
    assert_eq!(
        after_text(&out),
        "BT /F1 11.04 Tf 1 0 0 1 72 700 Tm [<496E76> 12 <6F6963652032303237>] TJ ET",
        "PLAN-04 page"
    );
    assert!(
        close(out.verdicts[0].delta_pt, 0.0, 1e-9),
        "PLAN-04 same width"
    );
}

#[test]
fn plan_05_compensation_sign_shorter_and_longer() {
    let pdf = || {
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 680 Td (tail) Tj ET")
    };
    // Shorter: the pen must be pushed forward (a negative TJ number).
    let (_, m, out) = plan_one(pdf(), "Hello", "Hell", style());
    let r = replacement(&out, 0);
    assert!(r.ends_with(" -556] TJ"), "PLAN-05 shorter: {r}");
    assert!(out.verdicts[0].delta_pt < 0.0, "PLAN-05 shorter delta");
    let old_width = run_with(&m, "Hello").rect[2];
    // (kept glyphs, newly encoded glyphs) of the planned run.
    let kept = |out: &crate::pdf_engine::text_edit::rewrite::PlanOutcome| {
        let run = &ok_plan(out).runs[0];
        let new = run.expected.glyph_new.iter().filter(|n| **n).count();
        (run.expected.glyph_new.len() - new, new)
    };
    assert_eq!(
        kept(&out),
        (4, 0),
        "PLAN-05 shorter: H e l l kept, nothing new"
    );
    verdict_matches_plan(&out, 4, |w| w < old_width, "shorter");
    // Longer: pulled back (positive).
    let (_, _, out) = plan_one(pdf(), "Hello", "Helloo", style());
    let r = replacement(&out, 0);
    assert!(r.ends_with(" 556] TJ"), "PLAN-05 longer: {r}");
    assert!(out.verdicts[0].delta_pt > 0.0, "PLAN-05 longer delta");
    assert_eq!(kept(&out), (5, 1), "PLAN-05 longer: Hello kept, one new o");
    verdict_matches_plan(&out, 6, |w| w > old_width, "longer");
}

/// The verdict of the only edit agrees with its run plan: same width change, the planned new
/// box (`width_ok` on its width), `chars + 1` monotonic caret offsets ending at the new advance,
/// and the page plan is for page 0.
fn verdict_matches_plan(
    out: &crate::pdf_engine::text_edit::rewrite::PlanOutcome,
    chars: usize,
    width_ok: impl Fn(f64) -> bool,
    case: &str,
) {
    let plan = ok_plan(out);
    let (v, run) = (&out.verdicts[0], &plan.runs[0]);
    assert_eq!(plan.page_index, 0, "PLAN-05 {case} page");
    assert_eq!(v.new_rect, Some(run.new_rect), "PLAN-05 {case} rect");
    assert!(
        width_ok(run.new_rect[2]),
        "PLAN-05 {case} width {:?}",
        run.new_rect
    );
    let carets = v.caret_offsets.as_ref().expect("caret offsets");
    assert_eq!(carets.len(), chars + 1, "PLAN-05 {case} carets {carets:?}");
    assert!(
        carets.windows(2).all(|w| w[0] <= w[1]) && carets[0] == 0.0,
        "PLAN-05 {case} carets monotonic from 0: {carets:?}"
    );
}

#[test]
fn plan_06_equal_width_writes_no_compensation() {
    // Digits share one width in Helvetica.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Total 2026) Tj ET"),
        "Total 2026",
        "Total 2027",
        style(),
    );
    assert_eq!(
        replacement(&out, 0),
        "[<546F74616C2032303237>] TJ",
        "PLAN-06"
    );
}

#[test]
fn plan_07_quote_keeps_its_line_move() {
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 14 TL 72 700 Td (First) Tj (Second)' ET"),
        "Second",
        "Seconds",
        style(),
    );
    let r = replacement(&out, 0);
    assert!(r.starts_with("T* [<5365636F6E6473>"), "PLAN-07 {r}");
}

#[test]
fn plan_08_double_quote_keeps_aw_ac_verbatim() {
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 14 TL 72 700 Td (First) Tj 2.50 0.125 (Second)\" ET"),
        "Second",
        "Secant",
        style(),
    );
    let r = replacement(&out, 0);
    assert!(r.starts_with("2.50 Tw 0.125 Tc T* [<"), "PLAN-08 {r}");
}

#[test]
fn plan_09_tz_tc_tw_and_the_word_space_code() {
    // Tz 50, Tc 1, Tw 5: a WinAnsi space (code 32) takes Tw. One more space glyph =
    // 0.278·12 + Tc 1 + Tw 5 = 9.336 text units (Th excluded) = 778 thousandths of Tf 12.
    let (_, _, out) = plan_one(
        helvetica_page(
            b"BT /F1 12 Tf 50 Tz 1 Tc 5 Tw 72 700 Td (a b) Tj ET BT /F1 12 Tf 72 680 Td (end) Tj ET",
        ),
        "a b",
        "a  b",
        style(),
    );
    assert_eq!(
        replacement(&out, 0),
        "[<61202062> 778] TJ",
        "PLAN-09 code 32 gets Tw"
    );
    // A 2-byte space (Identity-H, CID 3) is never a word space: 0.5·12 + Tc 1 = 7 text units.
    let mut d = DocBuilder::new();
    let f1 = fx::cid_font(&mut d.b, "AAAAAA+Arimo", "ab ");
    let content = format!(
        "BT /F1 12 Tf 50 Tz 1 Tc 5 Tw 72 700 Td <{}> Tj ET",
        fx::cid_hex("ab ", "a b")
    );
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    let (_, _, out) = plan_one(d.build(), "a b", "a  b", style());
    assert_eq!(
        replacement(&out, 0),
        "[<00010003000300020> 583.3333] TJ".replace("00020>", "0002>"),
        "PLAN-09 2-byte space gets no Tw"
    );
}

#[test]
fn plan_10_b2_tf1_tm12_size_plus_one_is_effective_13() {
    // `/F1 1 Tf 12 0 0 12 x y Tm`: effective 12, Tf operand 1. +1 pt ⇒ Tf' = 1.0833.
    let (_, m, out) = plan_one(fx::tf1_tm12(), "Hi", "Hi", sized(13.0));
    let run = run_with(&m, "Hi");
    assert!(
        close(run.effective_size, 12.0, 1e-9),
        "PLAN-10 effective 12"
    );
    let plan = ok_plan(&out);
    let t = &plan.runs[0].target;
    assert!(close(t.tfs, 1.0833, 1e-12), "PLAN-10 Tf' {}", t.tfs);
    let r = replacement(&out, 0);
    assert!(r.contains("/F1 1.0833 Tf ["), "PLAN-10 sets Tf: {r}");
    assert!(r.ends_with("/F1 1 Tf"), "PLAN-10 restores Tf verbatim: {r}");
    assert!(
        close(plan.runs[0].expected.effective_size, 13.0, 1e-9),
        "PLAN-10 expected 13"
    );
}
