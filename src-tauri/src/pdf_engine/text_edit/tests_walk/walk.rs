//! WALK-01…14: glyph math (B1, B2, Tc/Tw per code, Tz, Ts), render modes, text clips, the `q`
//! stack, clip containment, patterns, soft masks and ActualText.

use super::{close, ctx, model0, reason_of, run_with, walk};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::state::ClipState;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, cid_font, cid_hex, helvetica_doc, helvetica_page, ClipKind, DocBuilder, PageSpec,
};
use crate::pdf_engine::text_edit::walker::{RecElem, WalkMode};

/// A page whose `/F2` is a CID font over "AB C" (2-byte codes, width 500).
fn cid_page(content: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = cid_font(&mut d.b, "ABCDEF+Arimo", "AB C");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F2 {f} 0 R >>"),
    ));
    d.build()
}

#[test]
fn walk_01_b1_tj_kern_moves_the_tail() {
    let c = ctx(fx::kerned_then_tail());
    let w = walk(&c, 0, WalkMode::Edit);
    let (tj, tail) = (&w.records[0], &w.records[1]);
    // Helvetica A=B=667, C=D=722 at 10 pt; −500 → +5 pt.
    assert!(
        close(tj.advance_ts, 13.34 + 5.0 + 14.44),
        "WALK-01 advance_ts includes the kern: {}",
        tj.advance_ts
    );
    assert!(
        close(tail.pen_before.0, 72.0 + 32.78) && close(tail.pen_before.1, 700.0),
        "WALK-01 the tail starts after the kern: {:?}",
        tail.pen_before
    );
    assert!(
        matches!(tj.elems.get(2), Some(RecElem::Kern { value, .. }) if *value == -500.0),
        "WALK-01 the kern is an element: {:?}",
        tj.elems
    );
    assert!(close(tj.pen_after.0, tail.pen_before.0));
}

#[test]
fn walk_02_b2_effective_size_is_not_the_tf_operand() {
    let m = model0(fx::tf1_tm12());
    let r = run_with(&m, "Hi");
    assert_eq!(r.tfs, 1.0, "WALK-02 Tf operand");
    assert!(
        close(r.effective_size, 12.0),
        "WALK-02 effective {}",
        r.effective_size
    );
    assert!(close(r.text_to_user_x, 12.0));
    assert_eq!(r.reason, None);
    // Helvetica AFM: H = 722, i = 222 → 0.944 text units × 12.
    assert!(
        close(r.original_extent, 0.944 * 12.0),
        "WALK-02 extent {}",
        r.original_extent
    );
}

#[test]
fn walk_03_tc_counts_per_code_on_two_byte_codes() {
    let c = ctx(cid_page(&format!(
        "BT /F2 10 Tf 2 Tc 72 700 Td <{}> Tj ET",
        cid_hex("AB C", "AB")
    )));
    let w = walk(&c, 0, WalkMode::Edit);
    let r = &w.records[0];
    assert_eq!(r.glyphs.len(), 2, "WALK-03 two 2-byte codes");
    assert!(
        close(r.advance_ts, 2.0 * (5.0 + 2.0)),
        "WALK-03 Tc once per code, not per byte: {}",
        r.advance_ts
    );
}

#[test]
fn walk_04_tw_only_for_the_one_byte_space() {
    let c = ctx(helvetica_page(b"BT /F1 10 Tf 5 Tw 72 700 Td (a b) Tj ET"));
    let r = &walk(&c, 0, WalkMode::Edit).records[0];
    // a=556 space=278 b=556 at 10 pt + Tw 5 once.
    assert!(
        close(r.advance_ts, 13.9 + 5.0),
        "WALK-04 simple font: {}",
        r.advance_ts
    );
    let c = ctx(cid_page(&format!(
        "BT /F2 10 Tf 5 Tw 72 700 Td <{}> Tj ET",
        cid_hex("AB C", "A B")
    )));
    let r = &walk(&c, 0, WalkMode::Edit).records[0];
    assert!(
        close(r.advance_ts, 15.0),
        "WALK-04 a 2-byte code 0x0003 that reads as a space gets no Tw: {}",
        r.advance_ts
    );
}

#[test]
fn walk_05_tz_scales_the_pen_not_advance_ts() {
    let c = ctx(helvetica_page(b"BT /F1 10 Tf 80 Tz 72 700 Td (Hi) Tj ET"));
    let r = &walk(&c, 0, WalkMode::Edit).records[0];
    assert!(
        close(r.advance_ts, 9.44),
        "WALK-05 Th excluded: {}",
        r.advance_ts
    );
    assert!(
        close(r.pen_after.0 - r.pen_before.0, 9.44 * 0.8),
        "WALK-05 user advance × 0.8"
    );
    assert!(close(r.text_to_user[0], 8.0), "WALK-05 Tfs·Th");
}

#[test]
fn walk_06_rise_moves_origins_and_boxes() {
    let c = ctx(helvetica_page(b"BT /F1 10 Tf 3 Ts 72 700 Td (Hi) Tj ET"));
    let r = &walk(&c, 0, WalkMode::Edit).records[0];
    let g = &r.glyphs[0];
    assert!(close(g.origin.1, 703.0), "WALK-06 origin {:?}", g.origin);
    let descent = r.font.as_ref().expect("font").descent;
    assert!(
        close(g.bbox[1], 703.0 + descent * 10.0),
        "WALK-06 box bottom {} (descent {descent})",
        g.bbox[1]
    );
    assert!(close(r.pen_before.1, 703.0));
}

#[test]
fn walk_07_render_modes() {
    for (tr, want) in [
        (0, None),
        (1, None),
        (2, None),
        (3, Some(R::InvisibleText)),
        (4, Some(R::TextClipMode)),
        (7, Some(R::TextClipMode)),
    ] {
        let page =
            helvetica_page(format!("BT {tr} Tr /F1 12 Tf 72 700 Td (Mode) Tj ET").as_bytes());
        assert_eq!(reason_of(&model0(page), "Mode"), want, "WALK-07 Tr {tr}");
    }
}

#[test]
fn walk_08_text_after_a_text_clip_is_clipped() {
    let m = model0(fx::tr7());
    assert_eq!(reason_of(&m, "Clip text"), Some(R::TextClipMode), "WALK-08");
    assert_eq!(reason_of(&m, "After clip"), Some(R::Clipped), "WALK-08");
    let after = &m.walk.records[1];
    assert_eq!(
        after.before.clip,
        ClipState::Complex,
        "WALK-08 clip after ET"
    );
}

#[test]
fn walk_09_q_overflow_is_refused_and_a_stray_q_tolerated() {
    let over = format!("{}BT /F1 12 Tf 72 700 Td (Deep) Tj ET", "q ".repeat(65));
    let m = model0(helvetica_page(over.as_bytes()));
    assert_eq!(m.page_reason, Some(R::MalformedContent), "WALK-09 65 × q");
    assert!(
        m.runs.is_empty() && m.walk.records.is_empty(),
        "WALK-09 never a partial list"
    );
    let fine = format!(
        "{}BT /F1 12 Tf 72 700 Td (Deep) Tj ET{}",
        "q ".repeat(64),
        " Q".repeat(64)
    );
    assert_eq!(
        reason_of(&model0(helvetica_page(fine.as_bytes())), "Deep"),
        None
    );
    let stray = model0(helvetica_page(b"Q Q BT /F1 12 Tf 72 700 Td (Stray) Tj ET"));
    assert_eq!(
        reason_of(&stray, "Stray"),
        None,
        "WALK-09 Q on an empty stack is ignored"
    );
}

#[test]
fn walk_10_clip_page_small_curve() {
    assert_eq!(
        reason_of(&model0(fx::clip(ClipKind::Page)), "Clip me"),
        None,
        "WALK-10 page"
    );
    assert_eq!(
        reason_of(&model0(fx::clip(ClipKind::Small)), "Clip me"),
        Some(R::Clipped),
        "WALK-10 small"
    );
    assert_eq!(
        reason_of(&model0(fx::clip(ClipKind::Curve)), "Clip me"),
        Some(R::Clipped),
        "WALK-10 curve"
    );
    let m = model0(fx::clip(ClipKind::Page));
    assert_eq!(
        m.walk.records[0].before.clip,
        ClipState::Rect([0.0, 0.0, 612.0, 792.0])
    );
    let outside = model0(helvetica_page(b"BT /F1 12 Tf 600 700 Td (Edge) Tj ET"));
    assert_eq!(
        reason_of(&outside, "Edge"),
        Some(R::Clipped),
        "WALK-10 past the page"
    );
}

#[test]
fn walk_11_pattern_fill_and_stroke() {
    assert_eq!(
        reason_of(&model0(fx::pattern_fill()), "Pattern"),
        Some(R::Pattern),
        "WALK-11 fill"
    );
    let stroke = |tr: i64| {
        let c = format!("BT /F1 12 Tf /Cs1 CS /P1 SCN {tr} Tr 72 720 Td (Stroke) Tj ET");
        reason_of(&model0(fx::pattern_doc(c.as_bytes())), "Stroke")
    };
    assert_eq!(
        stroke(1),
        Some(R::Pattern),
        "WALK-11 stroked text, pattern stroke"
    );
    assert_eq!(
        stroke(0),
        None,
        "WALK-11 filled text ignores the stroke paint"
    );
}

#[test]
fn walk_12_soft_mask() {
    assert_eq!(
        reason_of(&model0(fx::smask_text()), "Masked"),
        Some(R::SoftMask),
        "WALK-12"
    );
    let none = helvetica_doc(
        b"/GS1 gs BT /F1 12 Tf 72 720 Td (No mask) Tj ET",
        "/ExtGState << /GS1 << /SMask /None /ca 0.5 >> >>",
        "",
    );
    let m = model0(none);
    assert_eq!(reason_of(&m, "No mask"), None, "WALK-12 /SMask /None");
    assert_eq!(m.walk.records[0].before.gs.ca, 0.5);
}

#[test]
fn walk_13_actual_text_inline() {
    let m = model0(fx::actual_text_span());
    assert_eq!(
        reason_of(&m, "fi"),
        Some(R::ActualText),
        "WALK-13 inline /ActualText"
    );
    assert_eq!(reason_of(&m, "Plain"), None);
    let e = model0(helvetica_page(
        b"/Span <</E (expansion)>> BDC BT /F1 12 Tf 72 720 Td (abbr) Tj ET EMC",
    ));
    assert_eq!(reason_of(&e, "abbr"), Some(R::ActualText), "WALK-13 /E");
}

#[test]
fn walk_14_actual_text_via_properties() {
    let page = helvetica_doc(
        b"/Span /MC0 BDC BT /F1 12 Tf 72 720 Td (Prop) Tj ET EMC /Span /MC1 BDC BT /F1 12 Tf 72 700 Td (Other) Tj ET EMC",
        "/Properties << /MC0 << /ActualText (x) >> /MC1 << /Lang (en) >> >>",
        "",
    );
    let m = model0(page);
    assert_eq!(reason_of(&m, "Prop"), Some(R::ActualText), "WALK-14");
    assert_eq!(reason_of(&m, "Other"), None, "WALK-14 other properties");
}
