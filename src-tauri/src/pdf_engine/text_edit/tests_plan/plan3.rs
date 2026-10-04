//! PLAN-22…30: style controls (size, colour, letter spacing, faces) with verbatim restores (B14),
//! sibling segments, absorbed members, and the availability rules (B8, B9).

use super::{
    ctx, edit, faced, filled, model, ok_plan, plan, plan_one, replacement, sized, spaced, style,
};
use crate::pdf_engine::text_edit::reasons::{EditProblemCode as P, Face, StyleField};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, word_font, DocBuilder, PageSpec, HELVETICA,
};

/// Helvetica `/F1` and Helvetica-Bold `/F2` (a bold sibling) on one page.
pub(crate) fn helvetica_pair(content: &[u8]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let r = d.add(HELVETICA);
    let b = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );
    d.page(PageSpec::new(
        content,
        &format!("/Font << /F1 {r} 0 R /F2 {b} 0 R >>"),
    ));
    d.build()
}

#[test]
fn plan_22_size_change_sets_and_restores_tf_verbatim() {
    // (2.278 × 14 − 2.278 × 12) × 1000 / 14 = 325.4286.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12.0 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        sized(14.0),
    );
    assert_eq!(
        replacement(&out, 0),
        "/F1 14 Tf [<48656C6C6F> 325.4286] TJ /F1 12.0 Tf",
        "PLAN-22"
    );
    let t = &ok_plan(&out).runs[0].target;
    assert!(t.size_changed && !t.tc_changed && !t.fill_changed && !t.face_changed);
}

#[test]
fn plan_23_colour_sets_rg_and_restores_verbatim() {
    // A named colour space in force: restored with its own `cs` and `scn` bytes.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf /CS0 cs 0.2 0.4  0.6 scn 72 700 Td (Hello) Tj ET 72 600 50 50 re f",
        &format!("/Font << /F1 {f} 0 R >> /ColorSpace << /CS0 /DeviceRGB >>"),
    ));
    let (_, _, out) = plan_one(d.build(), "Hello", "Hello", filled("#0000ff"));
    assert_eq!(
        replacement(&out, 0),
        "0 0 1 rg [<48656C6C6F>] TJ /CS0 cs 0.2 0.4  0.6 scn",
        "PLAN-23 verbatim cs + scn"
    );
    // Never set on the page: restored as `0 g`.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 72 600 50 50 re f"),
        "Hello",
        "Hello",
        filled("#C71C1C"),
    );
    assert_eq!(
        replacement(&out, 0),
        "0.7804 0.1098 0.1098 rg [<48656C6C6F>] TJ 0 g",
        "PLAN-23 default black"
    );
}

#[test]
fn plan_24_b14_letter_spacing_restores_tc_verbatim() {
    // +1 pt letter spacing on 5 glyphs: Tc' = 1; compensation (1 − 0.123456) × 5 × 1000 / 12.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 0.123456 Tc 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        spaced(1.0),
    );
    assert_eq!(
        replacement(&out, 0),
        "1 Tc [<48656C6C6F> 365.2267] TJ 0.123456 Tc",
        "PLAN-24 verbatim restore (B14: never rounded to 4 dp)"
    );
    // Never set: restored as `0 Tc`.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET"),
        "Hi",
        "Hi",
        spaced(-0.5),
    );
    assert_eq!(
        replacement(&out, 0),
        "-0.5 Tc [<4869> -83.3333] TJ 0 Tc",
        "PLAN-24 0 Tc"
    );
}

#[test]
fn plan_25_face_swap_to_a_sibling() {
    // Helvetica-Bold: 722 + 556 + 278 + 278 + 611 = 2445 vs 2278 ⇒ 167 back.
    let (_, _, out) = plan_one(
        helvetica_pair(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F2 12 Tf 72 680 Td (Bold) Tj ET"),
        "Hello",
        "Hello",
        faced(Face::Bold),
    );
    assert_eq!(
        replacement(&out, 0),
        "/F2 12 Tf [<48656C6C6F> 167] TJ /F1 12 Tf",
        "PLAN-25"
    );
    let run = &ok_plan(&out).runs[0];
    assert!(run.target.face_changed, "PLAN-25");
    assert!(
        run.expected.glyphs.iter().all(|(res, _, _)| res == b"F2"),
        "PLAN-25 all in F2"
    );
}

#[test]
fn plan_26_multi_segment_sibling_body_and_restore() {
    let tr = "Sağlık Bakanlığı Raporu";
    // Ends in F1 (the primary's font): no restore.
    let (_, _, out) = plan_one(fx::word_tr(), tr, "Sağlık Bakanlığı Rapor", style());
    let r = replacement(&out, 0);
    let switches: Vec<&str> = r.matches(" Tf").collect();
    assert_eq!(switches.len(), 6, "PLAN-26 F1/F2/F1/F2/F1/F2/F1: {r}");
    assert!(
        r.starts_with("[<5361>] TJ /F2 11 Tf [<0001>] TJ /F1 11 Tf [<6C> "),
        "PLAN-26 F1 in force first, then F2 with the verbatim size token: {r}"
    );
    assert!(
        r.ends_with("] TJ"),
        "PLAN-26 no restore when F1 ends the body: {r}"
    );
    // Ends in F2: the primary's `Tf` is restored verbatim.
    let (_, _, out) = plan_one(fx::word_tr(), tr, "Sağlık Bakanlığı Raporş", style());
    let r = replacement(&out, 0);
    assert!(r.ends_with("] TJ /F1 11 Tf"), "PLAN-26 restore: {r}");
    // The six other members draw nothing and keep their travel.
    for m in 1..7 {
        let a = replacement(&out, m);
        assert!(
            a.starts_with("[<> ") || a == "[<>] TJ",
            "PLAN-26 absorbed {m}: {a}"
        );
    }
}

#[test]
fn plan_27_absorbed_members_keep_followers() {
    // Td-positioned member (`18 0 Td` to the pen) and a pen-chained follower drawn at 14 pt
    // (not joined: another size); the follower must stay where it was (self-check).
    let pdf =
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hel) Tj 18 0 Td (lo) Tj /F1 14 Tf ( tail) Tj ET");
    let (_, _, out) = plan_one(pdf, "Hello", "Help", style());
    // Primary: Help = 722 + 556 + 222 + 556 = 2056 vs Hel 1500 ⇒ 556 back.
    assert_eq!(
        replacement(&out, 0),
        "[<48656C70> 556] TJ",
        "PLAN-27 primary"
    );
    // Absorbed `(lo)`: 222 + 556 = 778 forward.
    assert_eq!(replacement(&out, 1), "[<> -778] TJ", "PLAN-27 absorbed");
    let exp = &ok_plan(&out).runs[0].expected;
    assert_eq!(exp.emitted_records, vec![1, 1], "PLAN-27 one show op each");
    // Pen-chained members in one op sequence: `(Hel) Tj (lo) Tj`.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hel) Tj (lo) Tj /F1 14 Tf ( tail) Tj ET"),
        "Hello",
        "Hallo there",
        style(),
    );
    assert!(
        replacement(&out, 1).starts_with("[<> -778] TJ"),
        "PLAN-27 chained"
    );
}

#[test]
fn plan_28_b9_extgstate_font_size_and_face_unavailable() {
    let c = ctx(fx::extgstate_font());
    let m = model(&c, 0);
    for (s, field) in [
        (sized(14.0), StyleField::Size),
        (faced(Face::Bold), StyleField::Face),
    ] {
        let out = plan(&c, &m, &[edit(&m, "Hi there", "Hi there", s)]);
        let p = out.verdicts[0].problem.as_ref().expect("STYLE_UNAVAILABLE");
        assert_eq!(
            (p.code, p.field),
            (P::StyleUnavailable, Some(field)),
            "PLAN-28"
        );
    }
    // Text and colour stay available with an ExtGState font (no Tf is written).
    let out = plan(
        &c,
        &m,
        &[edit(&m, "Hi there", "Hi here", filled("#ff0000"))],
    );
    let r = replacement(&out, 0);
    assert!(!r.contains("Tf"), "PLAN-28 never a Tf: {r}");
    assert!(r.starts_with("1 0 0 rg [<"), "PLAN-28 {r}");
}

#[test]
fn plan_29_tr2_colour_unavailable() {
    let c = ctx(fx::skia());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Bold", "Bold", filled("#ff0000"))]);
    let p = out.verdicts[0].problem.as_ref().expect("STYLE_UNAVAILABLE");
    assert_eq!(
        (p.code, p.field),
        (P::StyleUnavailable, Some(StyleField::Colour)),
        "PLAN-29"
    );
    // The text itself stays editable.
    let out = plan(&c, &m, &[edit(&m, "Bold", "Bil", style())]);
    assert!(
        out.verdicts[0].problem.is_none(),
        "PLAN-29 text: {:?}",
        out.verdicts[0].problem
    );
}

#[test]
fn plan_30_b8_face_unavailable_with_chars() {
    // A bold subset that lacks W, r and d.
    let mut d = DocBuilder::new();
    let f1 = word_font(&mut d.b, "ABCDEF+Calibri", "Hello World");
    let f2 = word_font(&mut d.b, "ABCDEF+Calibri-Bold", "Helo");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello World) Tj ET BT /F2 12 Tf 72 680 Td (Helo) Tj ET",
        &format!("/Font << /F1 {f1} 0 R /F2 {f2} 0 R >>"),
    ));
    let (_, _, out) = plan_one(d.build(), "Hello World", "Hello World", faced(Face::Bold));
    let p = out.verdicts[0].problem.as_ref().expect("FACE_UNAVAILABLE");
    assert_eq!(p.code, P::FaceUnavailable, "PLAN-30");
    assert_eq!(p.face, Some(Face::Bold), "PLAN-30 face");
    assert_eq!(
        p.chars,
        vec!['W', 'r', 'd'],
        "PLAN-30 chars in typing order"
    );
    // No italic group at all: FACE_UNAVAILABLE without chars.
    let (_, _, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Hello",
        faced(Face::Italic),
    );
    let p = out.verdicts[0].problem.as_ref().expect("FACE_UNAVAILABLE");
    assert_eq!(
        (p.code, p.face, p.chars.len()),
        (P::FaceUnavailable, Some(Face::Italic), 0),
        "PLAN-30"
    );
}
