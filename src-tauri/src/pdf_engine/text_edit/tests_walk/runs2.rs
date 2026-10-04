//! RUN-12…22: per-glyph pages, duplicates, blank runs, the run budget, refused runs joined per
//! reason, reading order (XY-cut, structure order, all-or-nothing), state-sensitive joins,
//! ExtGState fonts, and tagged/bookmarked Word pages that stay editable.

use super::{model0, reason_of, texts};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, DocBuilder, PageSpec, HELVETICA, TWO_COLUMN_STRUCT_ORDER,
};

#[test]
fn run_12_per_glyph_page() {
    let joined = model0(fx::per_glyph(None));
    assert_eq!(
        texts(&joined),
        ["Per glyph line"],
        "RUN-12 edge-to-edge glyphs join"
    );
    assert_eq!(reason_of(&joined, "Per glyph line"), None);
    let scattered = model0(fx::per_glyph(Some(30.0)));
    assert_eq!(
        scattered.runs.len(),
        12,
        "RUN-12 12 non-blank one-glyph runs (2 spaces)"
    );
    assert!(
        scattered
            .runs
            .iter()
            .all(|r| r.reason == Some(R::PerGlyphText)),
        "RUN-12 {:?}",
        scattered
            .runs
            .iter()
            .map(|r| (r.text.clone(), r.reasons.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn run_13_duplicate_text() {
    let m = model0(fx::duplicate_shadow());
    assert_eq!(m.runs.len(), 2);
    assert!(
        m.runs.iter().all(|r| r.reason == Some(R::DuplicateText)),
        "RUN-13 both refused"
    );
    let apart = model0(helvetica_page(
        b"BT /F1 12 Tf 72 720 Td (Shadow) Tj ET BT /F1 12 Tf 72 600 Td (Shadow) Tj ET",
    ));
    assert!(
        apart.runs.iter().all(|r| r.reason.is_none()),
        "RUN-13 no overlap, no refusal"
    );
}

#[test]
fn run_14_whitespace_only_runs_are_omitted() {
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 720 Td (Words) Tj ET BT /F1 12 Tf 72 600 Td (   ) Tj ET",
    ));
    assert_eq!(texts(&m), ["Words"], "RUN-14");
    assert_eq!(
        m.walk.records.len(),
        2,
        "RUN-14 the blank op stays in the walk"
    );
    // (`PageModel.record_run` is gone, fix pass 2026-10-03: nothing read it; the blank op's
    // record belongs to no listed run.)
    assert!(m.runs.iter().all(|r| r.members == [0]));
}

#[test]
fn run_15_too_many_runs_refuse_the_page() {
    let lines = crate::pdf_engine::text_edit::limits::RUNS_PER_PAGE_MAX + 1;
    let mut content = String::from("BT /F1 1 Tf 20 780 Td ");
    for _ in 0..lines {
        content.push_str("(a) Tj 0 -0.03 Td ");
    }
    content.push_str("ET");
    let m = model0(helvetica_page(content.as_bytes()));
    assert_eq!(
        m.page_reason,
        Some(R::PageTooComplex),
        "RUN-15 {:?}",
        m.page_detail
    );
    assert!(m.runs.is_empty());
}

#[test]
fn run_16_refused_runs_join_only_with_the_same_reason() {
    let m = model0(helvetica_page(
        b"BT 3 Tr /F1 12 Tf 72 700 Td (Invis) Tj (ible) Tj ET",
    ));
    assert_eq!(texts(&m), ["Invisible"], "RUN-16 same reason joins");
    assert_eq!(reason_of(&m, "Invisible"), Some(R::InvisibleText));
    // A shared part followed by an own part: same line, same state, different first reason.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    let shared = d.b.add_stream("", b"BT /F1 12 Tf 72 700 Td (Head) Tj");
    let tail1 = d.b.add_stream("", b"(Tail) Tj ET");
    let tail2 = d.b.add_stream("", b"(Tail) Tj ET");
    let spec = PageSpec::new(b"", &res);
    d.page_raw(&format!("[{shared} 0 R {tail1} 0 R]"), &spec);
    d.page_raw(&format!("[{shared} 0 R {tail2} 0 R]"), &spec);
    let m = model0(d.build());
    assert_eq!(
        texts(&m),
        ["Head", "Tail"],
        "RUN-16 different reasons do not join"
    );
    assert_eq!(reason_of(&m, "Head"), Some(R::SharedContent));
    assert_eq!(reason_of(&m, "Tail"), None);
}

#[test]
fn run_17_untagged_two_columns_read_by_xy_cut() {
    let m = model0(fx::two_column(false));
    assert_eq!(
        texts(&m),
        [
            "A two column page",
            "Left one",
            "Left two",
            "Left three",
            "Right one",
            "Right two",
            "Right three"
        ],
        "RUN-17 title, column 1, column 2"
    );
    let lines: Vec<u32> = m.runs.iter().map(|r| r.line).collect();
    assert_eq!(lines, [0, 1, 2, 3, 4, 5, 6], "RUN-17 one line each");
}

#[test]
fn run_18_tagged_page_uses_structure_order() {
    let m = model0(fx::two_column(true));
    let by_mcid = |mcid: i64| {
        crate::pdf_engine::text_edit::testkit::producers::TWO_COLUMN_LINES
            .iter()
            .find(|l| l.0 == mcid)
            .map(|l| l.4)
            .expect("line")
    };
    let want: Vec<&str> = TWO_COLUMN_STRUCT_ORDER
        .iter()
        .map(|m| by_mcid(*m))
        .collect();
    assert_eq!(texts(&m), want, "RUN-18 structure order");
}

#[test]
fn run_19_one_untagged_run_falls_back_to_xy_cut() {
    let m = model0(fx::two_column_with(true, true));
    assert_eq!(
        texts(&m).first().map(String::as_str),
        Some("A two column page"),
        "RUN-19 XY-cut for the whole page: {:?}",
        texts(&m)
    );
    assert_eq!(
        texts(&m),
        texts(&model0(fx::two_column(false))),
        "RUN-19 never a mix"
    );
}

#[test]
fn run_20_stroked_segments_with_different_line_state_do_not_join() {
    let same = model0(fx::stroke_text(2, [0.5, 0.5], ["[] 0", "[] 0"]));
    assert_eq!(texts(&same), ["Boldface"], "RUN-20 equal state joins");
    let widths = model0(fx::stroke_text(2, [0.5, 1.0], ["[] 0", "[] 0"]));
    assert_eq!(texts(&widths), ["Bold", "face"], "RUN-20 line width");
    let dashes = model0(fx::stroke_text(1, [0.5, 0.5], ["[] 0", "[2 1] 0"]));
    assert_eq!(texts(&dashes), ["Bold", "face"], "RUN-20 dash");
    let colours = model0(helvetica_page(
        b"0 0 1 RG BT /F1 12 Tf 2 Tr 72 700 Td (Bold) Tj ET 1 0 0 RG BT /F1 12 Tf 2 Tr 96.012 700 Td (face) Tj ET",
    ));
    assert_eq!(texts(&colours), ["Bold", "face"], "RUN-20 stroke colour");
}

#[test]
fn run_21_extgstate_font_never_joins_a_tf_font() {
    let page = {
        let mut d = DocBuilder::new();
        let f = d.add(HELVETICA);
        d.page(PageSpec::new(
            b"/GS1 gs BT 72 700 Td (Hi) Tj ET BT /F1 12 Tf 83.328 700 Td (there) Tj ET",
            &format!("/Font << /F1 {f} 0 R >> /ExtGState << /GS1 << /Font [{f} 0 R 12] >> >>"),
        ));
        d.build()
    };
    let m = model0(page);
    assert_eq!(texts(&m), ["Hi", "there"], "RUN-21");
    let control = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET BT /F1 12 Tf 83.328 700 Td (there) Tj ET",
    ));
    assert_eq!(
        texts(&control),
        ["Hithere"],
        "RUN-21 control: two Tf blocks join"
    );
}

#[test]
fn run_22_tagged_bookmarked_word_page_stays_editable() {
    for pdf in [fx::word(), fx::word_tr(), fx::tagged_bookmarked_page()] {
        let m = model0(pdf);
        assert!(!m.runs.is_empty());
        for r in &m.runs {
            assert_eq!(r.reason, None, "RUN-22 {}: {:?}", r.text, r.reasons);
            assert!(!r.reasons.contains(&R::SharedContent));
        }
    }
    // A page an internal link points back to: the page object is referenced twice.
    let with_link = {
        let mut d = DocBuilder::new();
        let f = d.add(HELVETICA);
        let page = d.reserve();
        let link = d.add(format!(
            "<< /Type /Annot /Subtype /Link /Rect [72 690 140 714] /Border [0 0 0] \
             /Dest [{page} 0 R /XYZ 0 792 0] >>"
        ));
        d.page_at(
            page,
            PageSpec::new(
                b"BT /F1 12 Tf 72 700 Td (Linked) Tj ET",
                &format!("/Font << /F1 {f} 0 R >>"),
            )
            .with(&format!("/Annots [{link} 0 R]")),
        );
        d.build()
    };
    assert_eq!(reason_of(&model0(with_link), "Linked"), None);
}

#[test]
fn kern_sequences_sum_and_negligible_gaps_vanish() {
    use super::run_with;
    use crate::pdf_engine::text_edit::runs::Unit;
    let kerns = |u: &[Unit]| -> Vec<(f64, bool)> {
        u.iter()
            .filter_map(|u| match u {
                Unit::Kern {
                    value, synth_space, ..
                } => Some((*value, *synth_space)),
                Unit::Glyph { .. } => None,
            })
            .collect()
    };
    // Two TJ numbers between the same glyphs: their sum (0.25 em) reads as one space, carried by
    // the larger one; a sum under 0.2 em reads as none.
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td [(Ab) -120 -130 (Cd)] TJ ET \
          BT /F1 12 Tf 72 600 Td [(Ef) -100 -90 (Gh)] TJ ET",
    ));
    assert_eq!(texts(&m), ["Ab Cd", "EfGh"], "kern sums");
    assert_eq!(
        kerns(&run_with(&m, "Ab Cd").units),
        [(-120.0, false), (-130.0, true)],
        "the larger kern carries the space"
    );
    // A Td that lands on the pen (A 667 + b 556 at 12 pt = 14.676) adds no gap kern.
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Ab) Tj 14.676 0 Td (Cd) Tj ET",
    ));
    let r = run_with(&m, "AbCd");
    assert_eq!(r.members.len(), 2, "two members, one run");
    assert_eq!(kerns(&r.units), [], "no zero-width gap kern");
}
