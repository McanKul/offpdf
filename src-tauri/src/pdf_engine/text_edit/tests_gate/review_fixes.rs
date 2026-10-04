//! Regressions for the T4 review (`review-T4.md`): render masks per glyph (H1), the request as the
//! reference for A4/A5 (M1), the render budget on oversize pages (M3, with round 1's 25 DPI floor,
//! M-1), objects reached through `/DecodeParms` in A2 (M4), the fill restore after a bare `sc`
//! (L1), and the small robustness items (L5). The word-matching cost bound (M2) is in
//! `bounds.rs`; round 1's CID-keyed CFF masks (H-1) are IND-10 in `tests_independent.rs`.

use super::{ed, fails_at, skipping, Ed, Honest};
use crate::pdf_engine::text_edit::apply::{updates_for_plan, warning_text};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::graph::{graph_digest, graph_matches};
use crate::pdf_engine::text_edit::limits::RENDER_PIXELS_MAX;
use crate::pdf_engine::text_edit::poppler::{render_dpi, RENDER_DPI_MIN};
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::testkit::producers::{
    helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::tests_independent::{follower_doc, follower_embedded};
use crate::pdf_engine::text_edit::tests_plan::{
    ctx, filled, ok_plan, plan_one, replacement, style,
};
use std::collections::HashMap;

/// The right edge of the widest mask box of the plan of "Hello" → "Help" on `pdf`.
fn mask_right_edge(pdf: Vec<u8>) -> (f64, f64) {
    let (_, _, out) = plan_one(pdf, "Hello", "Help", style());
    let boxes = &ok_plan(&out).runs[0].expected.mask_boxes;
    assert!(!boxes.is_empty(), "masks planned");
    let left = boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
    let right = boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max);
    (left, right)
}

#[test]
fn h1_masks_follow_each_glyph_not_the_font_wide_box() {
    // "Hello" at x 72, 12 pt; the old "o" (advance 0.556 em) starts at 92.664 and is the
    // rightmost glyph. Fallback boxes reach at most 0.2 em past an upright glyph's advance
    // (0.1 em without a /FontBBox), never the 2 em of an Arial-like /FontBBox.
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let (left, right) = mask_right_edge(follower_doc(Some("-665 -325 2000 1006")));
    assert!(
        close(right, 92.664 + (0.556 + 0.2) * 12.0),
        "arial-like: {right}"
    );
    assert!(
        close(left, 72.0 - 0.2 * 12.0),
        "left reach bounded too: {left}"
    );
    let (_, right) = mask_right_edge(follower_doc(None));
    assert!(
        close(right, 92.664 + (0.556 + 0.1) * 12.0),
        "no /FontBBox: {right}"
    );
    // Embedded Liberation Sans: the program's own outline box of "o" (~0.51 em wide).
    let (_, right) = mask_right_edge(follower_embedded());
    assert!(
        right > 92.664 + 0.4 * 12.0 && right < 92.664 + 0.6 * 12.0,
        "program glyph box: {right}"
    );
    // An italic face without a program or /FontBBox may overhang up to 0.35 em.
    let mut d = DocBuilder::new();
    let f = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding >>",
    );
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let (_, right) = mask_right_edge(d.build());
    assert!(
        close(right, 92.664 + (0.556 + 0.35) * 12.0),
        "italic: {right}"
    );
    // Stroked text grows by half the line width, times the miter limit for miter joins.
    let (_, right) = mask_right_edge(helvetica_page(
        b"2 w BT /F1 12 Tf 2 Tr 72 700 Td (Hello) Tj ET",
    ));
    assert!(
        close(right, 92.664 + 0.656 * 12.0 + 10.0),
        "miter stroke: {right}"
    );
    let (_, right) = mask_right_edge(helvetica_page(
        b"2 w 1 j BT /F1 12 Tf 2 Tr 72 700 Td (Hello) Tj ET",
    ));
    assert!(
        close(right, 92.664 + 0.656 * 12.0 + 1.0),
        "round stroke: {right}"
    );
}

#[test]
fn m1_a_drawable_wrong_glyph_with_consistent_expectations_fails() {
    // The user asks for "Hollo"; a planner bug writes the drawable "a" and expects "Hallo".
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Other line) Tj ET",
    );
    let Some(h) = Honest::new("m1_requested", pdf, 0, &[ed("Hello", "Hollo")]) else {
        return;
    };
    assert_eq!(h.plan.runs[0].expected.requested_text, "Hollo");
    let (plan, digest, out) = h.bad_run("wrong.pdf", |run| {
        let s = &mut run.splices[0];
        let text = String::from_utf8_lossy(&s.bytes).replacen("<486F", "<4861", 1);
        s.bytes = text.into_bytes();
        run.expected.glyphs[1].2.value = 0x61;
        run.expected.text = "Hallo".into();
    });
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "what=requested_text",
        "M1 A4",
    );
    let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A5",
        "\"Hollo\" not extracted",
        "M1 A5",
    );
}

#[test]
fn m3_render_resolution_keeps_every_page_within_the_pixel_budget() {
    let geom = |w: f64, h: f64| PageGeometry {
        media: [0.0, 0.0, w, h],
        crop: [0.0, 0.0, w, h],
        visible: [0.0, 0.0, w, h],
        rotate: 0,
        user_unit: 1.0,
    };
    let pixels = |w: f64, h: f64, dpi: u32| {
        (w / 72.0 * f64::from(dpi)).ceil() * (h / 72.0 * f64::from(dpi)).ceil()
    };
    assert_eq!(render_dpi(&geom(612.0, 792.0)), Some(96), "letter");
    assert_eq!(
        render_dpi(&geom(14_400.0, 14_400.0)),
        Some(RENDER_DPI_MIN),
        "the largest legal page"
    );
    for side in [4_000.0, 7_200.0, 10_000.0, 14_400.0] {
        let dpi = render_dpi(&geom(side, side)).expect("a resolution");
        assert!(
            pixels(side, side, dpi) <= RENDER_PIXELS_MAX as f64,
            "{side} pt at {dpi} DPI is over the budget"
        );
        assert!(
            pixels(side, side, dpi + 1) > RENDER_PIXELS_MAX as f64,
            "{side} pt: {dpi} DPI is the highest that fits"
        );
    }
    // Review round 1 (M-1): below 25 DPI the pixel pads and threshold hide a moved follower.
    for side in [14_500.0, 20_000.0, 30_000.0, 1.0e6] {
        assert_eq!(render_dpi(&geom(side, side)), None, "{side} pt square");
    }
    assert_eq!(render_dpi(&geom(0.0, 792.0)), None, "an empty media box");
}

#[test]
fn m3_an_edit_on_a_page_too_large_to_verify_fails_closed() {
    // 30,000 pt square (beyond Annex C's 14,400) would render below 25 DPI: A5 refuses it
    // before rendering anything (review round 1, M-1), instead of comparing near-blind pixels.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(
        PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Other line) Tj ET",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .media(Some("[0 0 30000 30000]")),
    );
    let Some(h) = Honest::unverified("m3_oversize", d.build(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    fails_at(
        h.phase_a(&h.staged),
        "EDIT_VERIFY_FAILED",
        "A5",
        "too large to verify: the page renders below 25 DPI",
        "M-1 oversize",
    );
}

#[test]
fn m3_an_honest_edit_on_the_largest_legal_page_passes_phase_a() {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(
        PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Other line) Tj ET",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .media(Some("[0 0 14400 14400]")),
    );
    let Some(h) = Honest::new("m3_legal", d.build(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    assert_eq!(
        h.report.proofs.len(),
        1,
        "Phase A passes at 25 DPI, A5 included"
    );
}

/// A page with a JBIG2 image whose `/DecodeParms` reference a globals stream.
fn jbig2_doc(globals: &str, parms_extra: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let g = d.add(format!(
        "<< /Length {} >>\nstream\n{globals}\nendstream",
        globals.len() + 1
    ));
    let img = d.add(format!(
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 1 \
         /ColorSpace /DeviceGray /Filter /JBIG2Decode \
         /DecodeParms << /JBIG2Globals {g} 0 R {parms_extra} >> /Length 5 >>\nstream\nXXXX\nendstream"
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET q 10 0 0 10 300 300 cm /Im0 Do Q",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Im0 {img} 0 R >>"),
    ));
    d.build()
}

#[test]
fn m4_objects_behind_decode_parms_are_part_of_the_graph() {
    let before = ctx(jbig2_doc("GLOBALS-AAAA", "/Zzz 1"));
    let digest = graph_digest(before.doc(), &HashMap::new(), None).expect("digest");
    let check = |pdf: Vec<u8>| {
        let after = ctx(pdf);
        graph_matches(after.doc(), &digest, &mut DecodeBudget::new(1 << 30), None)
    };
    assert_eq!(
        check(jbig2_doc("GLOBALS-AAAA", "/Zzz 1")),
        Ok(()),
        "unchanged"
    );
    let m = check(jbig2_doc("GLOBALS-BBBB", "/Zzz 1")).expect_err("globals data changed");
    assert_eq!(m.what, "data", "{m:?}");
    assert!(m.path.ends_with("/DecodeParms/JBIG2Globals"), "{m:?}");
    let m = check(jbig2_doc("GLOBALS-AAAA", "/Zzz 2")).expect_err("a DecodeParms key changed");
    assert_eq!(m.what, "data", "{m:?}");
    assert!(m.path.ends_with("/XObject/Im0"), "{m:?}");
}

#[test]
fn l1_colour_change_after_a_bare_sc_restores_its_device_space() {
    for (name, setup, restore) in [
        ("gray", "0.5 sc", "/DeviceGray cs 0.5 sc"),
        (
            "rgb",
            "1 0 0 rg 0.2 0.4 0.6 sc",
            "/DeviceRGB cs 0.2 0.4 0.6 sc",
        ),
        (
            "cmyk",
            "0 0 0 1 k 0 0 0 0.5 sc",
            "/DeviceCMYK cs 0 0 0 0.5 sc",
        ),
    ] {
        let content = format!(
            "{setup} BT /F1 12 Tf 72 700 Td (Gray sc) Tj ET 72 600 50 50 re f \
             BT /F1 12 Tf 72 650 Td (Next) Tj ET"
        );
        let pdf = helvetica_page(content.as_bytes());
        let (_, _, out) = plan_one(pdf.clone(), "Gray sc", "Gray sc", filled("#c71c1c"));
        let bytes = replacement(&out, 0);
        assert!(
            bytes.ends_with(restore),
            "L1 {name}: the restore re-selects the device space: {bytes}"
        );
        let edits = [Ed {
            old: "Gray sc",
            new: "Gray sc",
            style: SourceTextStyleIn {
                fill: Some("#c71c1c".into()),
                ..SourceTextStyleIn::default()
            },
        }];
        let Some(h) = Honest::new(&format!("l1_{name}"), pdf, 0, &edits) else {
            return;
        };
        assert_eq!(h.report.proofs.len(), 1, "L1 {name}: Phase A passes");
    }
}

#[test]
fn l5_warnings_compare_without_their_file_names() {
    let a = "WARNING: /Users/me/my.pdfs/report.pdf: file is damaged";
    let b = "WARNING: /tmp/work/source.pdf: file is damaged";
    assert_eq!(warning_text(a), "file is damaged");
    assert_eq!(warning_text(a), warning_text(b));
    assert_eq!(
        warning_text("WARNING: /a/b.pdf/c.pdf (offset 12): xref not found"),
        "(offset 12): xref not found"
    );
    assert_eq!(
        warning_text("WARNING: no file name here"),
        "no file name here"
    );
}

#[test]
fn l5_an_edited_part_off_the_page_is_an_error_not_a_skip() {
    let (_, m, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Help",
        style(),
    );
    let mut plan = ok_plan(&out).clone();
    assert_eq!(
        updates_for_plan(&m.content, &plan).map(|u| u.len()).ok(),
        Some(1)
    );
    plan.edited_parts.push(7);
    let e = updates_for_plan(&m.content, &plan).expect_err("part 7 is not on the page");
    assert_eq!(e.code, "EDIT_VERIFY_FAILED");
}
