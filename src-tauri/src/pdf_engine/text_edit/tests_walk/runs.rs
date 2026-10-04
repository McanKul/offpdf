//! RUN-01…11: joining (Word per-format blocks, column gaps, paints between, fonts, siblings with
//! converted gaps, MCIDs), synthetic spaces, ligatures, reading order on a rotated page, caret
//! offsets and run ids.

use super::{close, ctx, model, model0, reason_of, run_with, texts};
use crate::pdf_engine::text_edit::runs::{KernSrc, SpaceMode, Unit};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};

#[test]
fn run_01_word_per_format_blocks_join() {
    let m = model0(fx::word());
    assert_eq!(texts(&m), ["Invoice 2026", "Due"], "RUN-01");
    let r = run_with(&m, "Invoice 2026");
    assert_eq!(r.members.len(), 2, "RUN-01 two BT blocks, one line");
    assert_eq!(r.reason, None);
    assert_eq!(r.space_mode, SpaceMode::Glyph);
    assert!(
        r.id.starts_with(&format!("t1:{}:0:", m.fingerprint)),
        "RUN-01 id {}",
        r.id
    );
}

#[test]
fn run_02_column_gap_does_not_join() {
    // Helvetica "Left" = 556 + 556 + 278 + 278 = 1.668 em → 20.016 pt at 12 pt; `Td` moves from
    // the line start, so the next op starts 6 pt (0.5 em) after the pen.
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj 26.016 0 Td (Right) Tj ET",
    ));
    assert_eq!(texts(&m), ["Left", "Right"], "RUN-02 0.5 em gap");
    let joined = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj 21.216 0 Td (Right) Tj ET",
    ));
    assert_eq!(texts(&joined), ["LeftRight"], "RUN-02 a 0.1 em gap joins");
    let r = run_with(&joined, "LeftRight");
    assert!(
        r.units.iter().any(|u| matches!(
            u,
            Unit::Kern {
                src: KernSrc::Gap,
                synth_space: false,
                ..
            }
        )),
        "RUN-02 the joined gap is a converted kern"
    );
}

#[test]
fn run_03_a_paint_between_prevents_the_join() {
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj ET 0 0 1 1 re f BT /F1 12 Tf 92.016 700 Td (Right) Tj ET",
    ));
    assert_eq!(texts(&m), ["Left", "Right"], "RUN-03");
    let no_paint = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj ET BT /F1 12 Tf 92.016 700 Td (Right) Tj ET",
    ));
    assert_eq!(texts(&no_paint), ["LeftRight"], "RUN-03 control");
}

#[test]
fn run_04_non_sibling_fonts_do_not_join() {
    let mut d = DocBuilder::new();
    let f1 = d.add(HELVETICA);
    let f2 = d
        .add("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Sans) Tj /F2 12 Tf (Serif) Tj ET",
        &format!("/Font << /F1 {f1} 0 R /F2 {f2} 0 R >>"),
    ));
    let m = model0(d.build());
    assert_eq!(texts(&m), ["Sans", "Serif"], "RUN-04 Helvetica + Times");
}

#[test]
fn run_05_word_turkish_siblings_join_with_gaps_as_kerns() {
    let m = model0(fx::word_tr());
    let r = run_with(&m, "Sağlık Bakanlığı Raporu");
    assert_eq!(r.reason, None, "RUN-05 editable: {:?}", r.reasons);
    assert_eq!(r.members.len(), 7, "RUN-05 seven segments, one line");
    assert_eq!(
        r.surface.to_vec(),
        vec![b"F1".to_vec(), b"F2".to_vec()],
        "RUN-05 sibling surface"
    );
    let gaps: Vec<f64> = r
        .units
        .iter()
        .filter_map(|u| match u {
            Unit::Kern {
                value,
                src: KernSrc::Gap,
                ..
            } => Some(*value),
            _ => None,
        })
        .collect();
    // Segment 4 starts 0.01 pt late, segment 5 on time: ∓0.01 pt / 11 pt × 1000.
    assert_eq!(gaps.len(), 2, "RUN-05 converted gaps {gaps:?}");
    assert!(
        close(gaps[0], -10.0 / 11.0) && close(gaps[1], 10.0 / 11.0),
        "RUN-05 {gaps:?}"
    );
    let alphabet = m.surface(r).alphabet();
    for ch in ['ğ', 'ş', 'ı', 'İ', 'S', 'a'] {
        assert!(
            alphabet.contains(&ch),
            "RUN-05 {ch} typeable through the surface"
        );
    }
}

#[test]
fn run_06_different_mcids_do_not_join() {
    let m = model0(helvetica_page(
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Left) Tj ET EMC \
          /P <</MCID 1>> BDC BT /F1 12 Tf 92.016 700 Td (Right) Tj ET EMC",
    ));
    assert_eq!(texts(&m), ["Left", "Right"], "RUN-06");
}

#[test]
fn run_07_pdftex_kerned_word_gaps_read_as_spaces() {
    let m = model0(fx::pdftex());
    let r = run_with(&m, "Hello World");
    assert_eq!(r.reason, None, "RUN-07 editable: {:?}", r.reasons);
    assert_eq!(
        r.space_mode,
        SpaceMode::Kern,
        "RUN-07 no space glyph: kern-space writing"
    );
    assert_eq!(r.kern_space, -333.0, "RUN-07 the run's word gap");
    let synth = r
        .units
        .iter()
        .filter(|u| {
            matches!(
                u,
                Unit::Kern {
                    synth_space: true,
                    ..
                }
            )
        })
        .count();
    assert_eq!(synth, 1, "RUN-07 one synthetic space");
}

#[test]
fn run_08_ligatures_expand() {
    let m = model0(fx::pdftex());
    let r = run_with(&m, "find don\u{2019}t");
    let lig = r.units.iter().find_map(|u| match u {
        Unit::Glyph { text, code, .. } if code.value == 12 => Some(text.clone()),
        _ => None,
    });
    assert_eq!(
        lig.as_deref(),
        Some("fi"),
        "RUN-08 one glyph reads as two letters"
    );
    assert_eq!(r.caret_offsets.len(), r.text.chars().count() + 1);
}

#[test]
fn run_09_reading_order_on_a_rotated_page() {
    // /Rotate 90 with counter-rotated lines: user x grows downwards on screen.
    let page = helvetica_doc(
        b"BT /F1 12 Tf 0 1 -1 0 320 300 Tm (Second) Tj ET BT /F1 12 Tf 0 1 -1 0 300 300 Tm (First) Tj ET",
        "",
        "/Rotate 90",
    );
    let m = model0(page);
    assert_eq!(texts(&m), ["First", "Second"], "RUN-09 display order");
    assert_eq!((m.runs[0].line, m.runs[1].line), (0, 1));
    assert!(m.runs.iter().all(|r| r.reason.is_none()));
}

#[test]
fn run_10_caret_offsets_are_monotonic() {
    for pdf in [
        fx::word(),
        fx::pdftex(),
        fx::word_tr(),
        fx::kerned_then_tail(),
    ] {
        for r in &model0(pdf).runs {
            assert_eq!(
                r.caret_offsets.len(),
                r.text.chars().count() + 1,
                "RUN-10 {}",
                r.text
            );
            assert!(
                r.caret_offsets.windows(2).all(|w| w[0] <= w[1]),
                "RUN-10 monotonic {:?}",
                r.caret_offsets
            );
        }
    }
    let m = model0(fx::word());
    let r = run_with(&m, "Invoice 2026");
    // 12 glyphs of 5.52 pt (the 12/1000 kern after "v" pulls 0.13 pt back).
    let last = *r.caret_offsets.last().expect("offsets");
    assert!(
        (last - (12.0 * 5.52 - 0.13248)).abs() < 0.001,
        "RUN-10 end {last}"
    );
}

#[test]
fn run_11_ids_are_stable_and_follow_the_bytes() {
    let (a, b) = (ctx(fx::word()), ctx(fx::word()));
    let ids = |m: &crate::pdf_engine::text_edit::runs::PageModel| {
        m.runs.iter().map(|r| r.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(&model(&a, 0)), ids(&model(&b, 0)), "RUN-11 stable");
    let shifted = model0(helvetica_page(b"q Q BT /F1 12 Tf 72 700 Td (Hi) Tj ET"));
    let plain = model0(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET"));
    assert_ne!(ids(&shifted), ids(&plain), "RUN-11 other bytes, other id");
    assert!(
        plain.runs[0].id.ends_with(":0:23-30"),
        "RUN-11 span in the id: {}",
        plain.runs[0].id
    );
    assert_eq!(
        plain.run(&plain.runs[0].id).map(|r| r.text.as_str()),
        Some("Hi")
    );
    assert_eq!(reason_of(&plain, "Hi"), None);
}

#[test]
fn run_fields_describe_the_line() {
    use crate::pdf_engine::text_edit::reasons::Face;
    use crate::pdf_engine::text_edit::structure::struct_actual_text;
    let m = model0(helvetica_page(
        b"1 0 0 rg BT /F1 10 Tf 2 Tw 90 Tz 72 700 Td [(Ab) -50 (c)] TJ ET",
    ));
    let r = run_with(&m, "Abc");
    assert_eq!((r.tw, r.th, r.face), (2.0, 0.9, Face::Regular));
    assert_eq!(r.fill_hex.as_deref(), Some("#ff0000"));
    assert!(
        close(r.visible_extent, 540.0),
        "visible extent {}",
        r.visible_extent
    );
    // `end_pen` and `PageWalk.page_index` are gone (dead fields, fix pass 2026-10-03): the pen
    // after the line is the last member's `pen_after`, and the page is `PageModel.page_index`.
    let rec = &m.walk.records[0];
    assert!(rec.tm_after[4] > rec.tm_before[4], "Tm moved by the show");
    assert_eq!(rec.operand_spans.len(), 1, "the TJ array");
    assert_eq!(m.page_index, 0);
    let kern = r.units.iter().find_map(|u| match u {
        Unit::Kern {
            src: KernSrc::Tj { span },
            value,
            ..
        } => Some((span.clone(), *value)),
        _ => None,
    });
    let (span, value) = kern.expect("TJ kern");
    assert_eq!(
        (value, &m.content.joined[span]),
        (-50.0, &b"-50"[..]),
        "the kern's own bytes"
    );
    let first = r.units.iter().find_map(|u| match u {
        Unit::Glyph {
            member,
            font_res,
            font_hash,
            width1000,
            ..
        } => Some((*member, font_res.clone(), *font_hash, *width1000)),
        _ => None,
    });
    let (member, res, hash, width) = first.expect("glyph");
    assert_eq!(
        (member, res.as_deref(), width, r.tfs),
        (0, Some(&b"F1"[..]), 667.0, 10.0)
    );
    assert_eq!(hash, m.walk.page_fonts[0].1.content_hash);
    let tagged = ctx(fx::actual_text_struct());
    let page = tagged.snap.pages[0];
    assert_eq!(struct_actual_text(tagged.doc(), page, 0), Ok(true));
    assert_eq!(struct_actual_text(tagged.doc(), page, 1), Ok(false));
    let page = helvetica_page(b"0 0 10 10 re f BT /F1 12 Tf 72 700 Td (x) Tj ET");
    let w = super::walk(
        &ctx(page),
        0,
        crate::pdf_engine::text_edit::walker::WalkMode::Edit,
    );
    assert_eq!(
        w.paints[0].span,
        Some(13..14),
        "paint spans in the joined buffer (the `f` op)"
    );
}
