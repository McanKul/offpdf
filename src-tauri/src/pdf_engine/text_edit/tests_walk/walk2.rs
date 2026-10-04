//! WALK-15…27: structure ActualText, optional content, Forms (paint, descent, cycles, depth),
//! ExtGState fonts, paint records, non-finite numbers, determinism, inline images, split and
//! shared content, unresolvable references, the `Wrapped` mode over a real qpdf overlay, and the
//! id-free digest.

use super::{close, content, ctx, model0, reason_of, run_with, walk};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::{run_tool, RunOpts};
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits, Operand, Operator};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::snapshot::read_snapshot;
use crate::pdf_engine::text_edit::state::{same_state, ColorEffect};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, helvetica_page, DocBuilder, InlineProofKind, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::walker::{walk_page, PageWalk, PaintKind, WalkMode};
use std::ffi::OsString;

#[test]
fn walk_15_actual_text_via_the_structure_tree() {
    let m = model0(fx::actual_text_struct());
    assert_eq!(
        reason_of(&m, "Struct"),
        Some(R::ActualText),
        "WALK-15 StructElem /ActualText"
    );
    assert_eq!(
        reason_of(&m, "Free"),
        None,
        "WALK-15 sibling element without it"
    );
}

#[test]
fn walk_16_optional_content_visible_hidden_ocmd() {
    let m = model0(fx::indd());
    assert_eq!(reason_of(&m, "visible layer"), None, "WALK-16 ON group");
    assert_eq!(
        reason_of(&m, "hidden layer"),
        Some(R::OptionalContent),
        "WALK-16 OFF group"
    );
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let ocg = d.add("<< /Type /OCG /Name (Layer) >>");
    let ocmd = d.add(format!("<< /Type /OCMD /OCGs [{ocg} 0 R] /P /AnyOn >>"));
    d.catalog_extra = format!("/OCProperties << /OCGs [{ocg} 0 R] /D << >> >>");
    d.page(PageSpec::new(
        b"/OC /M1 BDC BT /F1 12 Tf 72 700 Td (Membership) Tj ET EMC /OC <</Type /OCG>> BDC BT /F1 12 Tf 72 680 Td (Inline) Tj ET EMC",
        &format!("/Font << /F1 {f} 0 R >> /Properties << /M1 {ocmd} 0 R >>"),
    ));
    let m = model0(d.build());
    assert_eq!(
        reason_of(&m, "Membership"),
        Some(R::OptionalContent),
        "WALK-16 OCMD"
    );
    assert_eq!(
        reason_of(&m, "Inline"),
        Some(R::OptionalContent),
        "WALK-16 inline dict"
    );
}

#[test]
fn walk_17_form_text_is_a_paint_in_edit_and_nested_in_classify() {
    let c = ctx(fx::nested_form());
    let edit = walk(&c, 0, WalkMode::Edit);
    assert!(edit.records.is_empty(), "WALK-17 Edit does not descend");
    assert!(
        edit.paints
            .iter()
            .any(|p| matches!(&p.kind, PaintKind::FormXObject { name, .. } if name == b"Fm0")),
        "WALK-17 the Form is a paint"
    );
    let classify = walk(&c, 0, WalkMode::Classify);
    let r = classify
        .records
        .first()
        .expect("WALK-17 record inside the Form");
    assert_eq!((r.depth, r.form_chain.len(), r.span.clone()), (1, 1, None));
    assert!(
        close(r.pen_before.0, 82.0) && close(r.pen_before.1, 610.0),
        "WALK-17 Matrix × CTM"
    );
    let page = content(&c, 0);
    let mut rc = crate::pdf_engine::text_edit::runs::reasons::ReasonCtx::new(&c, &page, &classify);
    assert_eq!(
        rc.record_reasons(r).first(),
        Some(&R::NestedForm),
        "WALK-17 NESTED_FORM"
    );
}

/// A chain of `depth` Forms, the innermost drawing text (`cycle`: the last one paints itself).
fn form_chain(depth: usize, cycle: bool) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let ids: Vec<u32> = (0..depth).map(|_| d.reserve()).collect();
    for (i, id) in ids.iter().enumerate() {
        let (res, body) = match ids.get(i + 1) {
            Some(next) => (
                format!("/XObject << /Fm {next} 0 R >>"),
                "/Fm Do".to_string(),
            ),
            None if cycle => (format!("/XObject << /Fm {id} 0 R >>"), "/Fm Do".to_string()),
            None => (
                format!("/Font << /F1 {f} 0 R >>"),
                "BT /F1 12 Tf 10 10 Td (Deep) Tj ET".to_string(),
            ),
        };
        let stream = crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder::stream_body(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 500 500] /Resources << {res} >>"),
            body.as_bytes(),
        );
        d.b.set(*id, stream);
    }
    d.page(PageSpec::new(
        b"q 1 0 0 1 72 500 cm /Fm Do Q BT /F1 12 Tf 72 700 Td (Page text) Tj ET",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Fm {} 0 R >>", ids[0]),
    ));
    d.build()
}

#[test]
fn walk_18_form_cycle_and_depth_nine_are_malformed() {
    for (pdf, what) in [
        (form_chain(9, false), "depth 9"),
        (form_chain(2, true), "cycle"),
    ] {
        let c = ctx(pdf);
        let classify = walk(&c, 0, WalkMode::Classify);
        assert_eq!(
            classify.page_reason,
            Some(R::MalformedContent),
            "WALK-18 {what}"
        );
        assert!(
            classify.records.is_empty(),
            "WALK-18 {what}: no partial list"
        );
        let m = super::model(&c, 0);
        assert_eq!(
            reason_of(&m, "Page text"),
            None,
            "WALK-18 {what}: Edit only paints the Form"
        );
    }
    let c = ctx(form_chain(8, false));
    let classify = walk(&c, 0, WalkMode::Classify);
    assert_eq!(classify.page_reason, None, "WALK-18 depth 8 is fine");
    assert_eq!(classify.records.iter().map(|r| r.depth).max(), Some(8));
}

#[test]
fn walk_19_extgstate_font() {
    let m = model0(fx::extgstate_font());
    let r = run_with(&m, "Hi there");
    assert!(r.font_from_extgstate, "WALK-19 font from the ExtGState");
    assert_eq!(
        r.reason, None,
        "WALK-19 editable text (B9: size/face disabled later)"
    );
    assert!(r.surface.is_empty(), "WALK-19 no resource name");
    let surface = m.surface(r);
    assert_eq!(
        surface.fonts.len(),
        1,
        "WALK-19 the typing surface is that font alone"
    );
    assert!(close(r.effective_size, 12.0));
    let use_ = m.walk.records[0]
        .before
        .text
        .font
        .clone()
        .expect("font use");
    assert!(use_.from_extgstate && use_.resource.is_none() && use_.tf_op.is_none());
}

#[test]
fn walk_20_paint_records_carry_their_state() {
    let c = ctx(helvetica_page(
        b"1 0 0 rg 10 10 50 50 re f 0 0 1 RG 3 w 10 10 m 60 60 l S BT /F1 12 Tf 72 700 Td (After) Tj ET",
    ));
    let w = walk(&c, 0, WalkMode::Edit);
    assert_eq!(w.paints.len(), 2, "WALK-20 two path paints");
    assert_eq!(w.paints[0].kind, PaintKind::Path(Operator::f));
    assert_eq!(
        w.paints[0].state.fill.effect,
        ColorEffect::Rgb([1.0, 0.0, 0.0]),
        "WALK-20 fill"
    );
    assert_eq!(
        w.paints[1].state.stroke.effect,
        ColorEffect::Rgb([0.0, 0.0, 1.0]),
        "WALK-20 stroke"
    );
    assert_eq!(w.paints[1].state.gs.line_width, 3.0);
    assert_eq!(w.paints[0].bbox, Some([10.0, 10.0, 60.0, 60.0]));
    assert!(w.paints[0].seq < w.paints[1].seq && w.paints[1].seq < w.records[0].seq);
    assert_eq!(w.records[0].before.fill.hex().as_deref(), Some("#ff0000"));
}

#[test]
fn walk_21_non_finite_numbers_refuse_the_page() {
    let blowup = format!(
        "{}BT /F1 12 Tf 72 700 Td (Huge) Tj ET",
        "1000000000 0 0 1000000000 0 0 cm ".repeat(40)
    );
    let m = model0(helvetica_page(blowup.as_bytes()));
    assert_eq!(
        m.page_reason,
        Some(R::MalformedContent),
        "WALK-21 {:?}",
        m.page_detail
    );
    assert!(m.page_detail.unwrap_or_default().contains("non-finite"));
}

/// The comparable, id-free shape of a walk.
fn summary(w: &PageWalk) -> Vec<String> {
    let mut out: Vec<String> = w
        .records
        .iter()
        .map(|r| {
            format!(
                "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
                r.seq,
                r.op,
                r.span,
                r.glyphs
                    .iter()
                    .map(|g| (g.code, g.text.clone(), g.origin))
                    .collect::<Vec<_>>(),
                r.pen_after,
                r.advance_ts,
                r.before,
                r.after
            )
        })
        .collect();
    out.extend(
        w.paints
            .iter()
            .map(|p| format!("{} {:?} {:?} {:?}", p.seq, p.kind, p.bbox, p.state)),
    );
    out
}

#[test]
fn walk_22_walks_are_deterministic() {
    for pdf in [fx::word(), fx::word_tr(), fx::indd(), fx::skia()] {
        let (a, b) = (ctx(pdf.clone()), ctx(pdf));
        for mode in [WalkMode::Edit, WalkMode::Classify] {
            assert_eq!(
                summary(&walk(&a, 0, mode.clone())),
                summary(&walk(&b, 0, mode.clone())),
                "WALK-22 two walks agree"
            );
            assert_eq!(
                summary(&walk(&a, 0, mode.clone())),
                summary(&walk(&a, 0, mode))
            );
        }
        let (ma, mb) = (super::model(&a, 0), super::model(&b, 0));
        let ids = |m: &crate::pdf_engine::text_edit::runs::PageModel| {
            m.runs
                .iter()
                .map(|r| (r.id.clone(), r.text.clone(), r.order))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&ma), ids(&mb), "WALK-22 runs");
    }
}

#[test]
fn walk_23_inline_image_proofs() {
    for proof in [
        InlineProofKind::Length,
        InlineProofKind::Unfiltered,
        InlineProofKind::Flate,
    ] {
        let m = model0(fx::inline_image(proof));
        assert_eq!(
            reason_of(&m, "After the image"),
            None,
            "WALK-23 proven: {proof:?}"
        );
        assert!(m
            .walk
            .paints
            .iter()
            .any(|p| matches!(p.kind, PaintKind::InlineImage { .. })));
    }
    let m = model0(fx::inline_image(InlineProofKind::Dct));
    assert_eq!(
        reason_of(&m, "After the image"),
        Some(R::InlineImage),
        "WALK-23 an unproven end refuses what follows"
    );
}

#[test]
fn walk_24_split_and_shared_content() {
    let m = model0(fx::straddling_op());
    assert_eq!(
        reason_of(&m, "Hello"),
        Some(R::SplitContent),
        "WALK-24 straddles the parts"
    );
    assert_eq!(reason_of(&m, "After"), None);
    let c = ctx(fx::shared());
    for page in 0..4 {
        let m = super::model(&c, page);
        let want = match page {
            0 | 1 => vec![("Shared body", Some(R::SharedContent))],
            _ => vec![
                ("Letterhead", Some(R::SharedContent)),
                (if page == 2 { "Body three" } else { "Body four" }, None),
            ],
        };
        for (text, reason) in want {
            assert_eq!(reason_of(&m, text), reason, "WALK-24 page {page} {text}");
        }
    }
}

#[test]
fn walk_25_unresolvable_references_refuse_the_page() {
    let cases: [(&[u8], &str, &str); 5] = [
        (
            b"/GSX gs BT /F1 12 Tf 72 700 Td (A) Tj ET",
            "",
            "ExtGState name",
        ),
        (
            b"/GS1 gs BT /F1 12 Tf 72 700 Td (A) Tj ET",
            "/ExtGState << /GS1 999 0 R >>",
            "dangling ExtGState",
        ),
        (
            b"q /Xn Do Q BT /F1 12 Tf 72 700 Td (A) Tj ET",
            "/XObject << /Xn 998 0 R >>",
            "dangling XObject",
        ),
        (
            b"/Span /MCX BDC BT /F1 12 Tf 72 700 Td (A) Tj ET EMC",
            "",
            "Properties name",
        ),
        (
            b"BT /F9 12 Tf 72 700 Td (A) Tj ET",
            "/Font << /F9 997 0 R >>",
            "dangling font",
        ),
    ];
    for (content, extra, what) in cases {
        let m = model0(helvetica_doc(content, extra, ""));
        assert_eq!(m.page_reason, Some(R::MalformedContent), "WALK-25 {what}");
    }
    let m = model0(helvetica_page(b"BT /F7 12 Tf 72 700 Td (Nameless) Tj ET"));
    assert_eq!(
        m.page_reason, None,
        "WALK-25 an absent font name stays a run reason"
    );
    assert_eq!(
        reason_of(&m, "\u{fffd}".repeat(8).as_str()),
        Some(R::MissingFont)
    );
}

/// Records of `a` and `b` agree id-free (op, font resource + hash, codes, text, origins, state).
fn assert_same_records(a: &PageWalk, b: &PageWalk, id: &str) {
    assert_eq!(a.records.len(), b.records.len(), "{id} record count");
    for (x, y) in a.records.iter().zip(&b.records) {
        assert_eq!(x.op, y.op, "{id}");
        let font = |r: &crate::pdf_engine::text_edit::walker::ShowRecord| {
            r.before
                .text
                .font
                .as_ref()
                .map(|f| (f.resource.clone(), f.content_hash))
        };
        assert_eq!(font(x), font(y), "{id} font");
        let codes = |r: &crate::pdf_engine::text_edit::walker::ShowRecord| {
            r.glyphs
                .iter()
                .map(|g| (g.code, g.text.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(codes(x), codes(y), "{id} codes");
        for (g, h) in x.glyphs.iter().zip(&y.glyphs) {
            assert!(
                (g.origin.0 - h.origin.0).abs() < 0.01 && (g.origin.1 - h.origin.1).abs() < 0.01,
                "{id} origin {:?} vs {:?}",
                g.origin,
                h.origin
            );
        }
        assert_eq!(
            same_state(&x.before, &y.before),
            Ok(()),
            "{id} state before"
        );
        assert_eq!(same_state(&x.after, &y.after), Ok(()), "{id} state after");
    }
}

#[test]
fn walk_26_wrapped_mode_follows_the_qpdf_overlay_wrapper() {
    let Some(engines) = engines_or_skip("walk_26_wrapped_mode_follows_the_qpdf_overlay_wrapper")
    else {
        return;
    };
    let s = Scratch::new("walk26");
    for (rotate, pdf) in [
        (0, fx::word()),
        (90, fx::rotated(90, true)),
        (270, fx::rotated(270, true)),
    ] {
        let src = s.write(&format!("src{rotate}.pdf"), &pdf);
        let blank = s.write(
            &format!("blank{rotate}.pdf"),
            &helvetica_doc(b"", "", &format!("/Rotate {rotate}")),
        );
        let out = s.path(&format!("out{rotate}.pdf"));
        let args: Vec<OsString> = vec![
            src.into(),
            "--overlay".into(),
            blank.into(),
            "--".into(),
            out.clone().into(),
        ];
        let r = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).expect("qpdf --overlay");
        assert!(r.code == 0 || r.code == 3, "WALK-26 qpdf: {}", r.stderr);
        let src_ctx = ctx(pdf);
        let out_ctx = SnapshotContext::new(read_snapshot(&out).expect("overlay output opens"));
        let before = walk(&src_ctx, 0, WalkMode::Edit);
        let out_content = content(&out_ctx, 0);
        let ops =
            lex_content(&out_content.joined, &LexLimits::page(), None).expect("wrapper lexes");
        let name = ops
            .iter()
            .find(|o| o.operator == Operator::Do)
            .and_then(|o| o.operands.first().and_then(Operand::as_name))
            .expect("WALK-26 the page paints a wrapper Form")
            .to_vec();
        let edit_out = walk_page(&out_ctx, 0, &out_content, WalkMode::Edit, None);
        assert!(
            edit_out.records.is_empty(),
            "WALK-26 Edit sees only the wrapper paint"
        );
        let wrapped = walk_page(&out_ctx, 0, &out_content, WalkMode::Wrapped { name }, None);
        assert_eq!(
            wrapped.page_reason, None,
            "WALK-26 /Rotate {rotate}: {:?}",
            wrapped.page_detail
        );
        assert!(!before.records.is_empty());
        assert_same_records(&before, &wrapped, &format!("WALK-26 /Rotate {rotate}"));
        let bad = walk_page(
            &out_ctx,
            0,
            &out_content,
            WalkMode::Wrapped {
                name: b"Nope".to_vec(),
            },
            None,
        );
        assert_eq!(
            bad.page_reason,
            Some(R::MalformedContent),
            "WALK-26 a name that is not painted"
        );
    }
    let not_wrapper = ctx(fx::word());
    let c = content(&not_wrapper, 0);
    let w = walk_page(
        &not_wrapper,
        0,
        &c,
        WalkMode::Wrapped {
            name: b"Fx0".to_vec(),
        },
        None,
    );
    assert_eq!(
        w.page_reason,
        Some(R::MalformedContent),
        "WALK-26 shape violation"
    );
    assert!(w.page_detail.unwrap_or_default().starts_with("wrapper"));
}

#[test]
fn walk_27_digest_captures_line_state_and_paints_are_id_free() {
    let m = model0(helvetica_page(
        b"[3 2] 1 d 2 J 1 j 5 M 0 0 1 RG 0.5 G BT /F1 12 Tf 72 700 Td (Lines) Tj ET",
    ));
    let gs = &m.walk.records[0].before.gs;
    assert_eq!(
        (
            (gs.dash.0.to_vec(), gs.dash.1),
            gs.line_cap,
            gs.line_join,
            gs.miter_limit
        ),
        ((vec![3.0, 2.0], 1.0), 2, 1, 5.0),
        "WALK-27"
    );
    assert_eq!(
        m.walk.records[0].before.stroke.effect,
        ColorEffect::Rgb([0.5, 0.5, 0.5])
    );
    // The same page with renumbered objects: paint kinds compare equal (no ids inside).
    let renumbered = {
        let mut d = DocBuilder::new();
        for _ in 0..5 {
            d.add("<< /Padding true >>");
        }
        let f = d.add(HELVETICA);
        let img = d.b.add_stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8",
            &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16],
        );
        d.page(PageSpec::new(
            b"q 40 0 0 40 72 400 cm /Im0 Do Q BT /F1 12 Tf 72 720 Td (Image page) Tj ET",
            &format!("/Font << /F1 {f} 0 R >> /XObject << /Im0 {img} 0 R >>"),
        ));
        d.build()
    };
    let kinds = |pdf: Vec<u8>| -> Vec<PaintKind> {
        let c = ctx(pdf);
        walk(&c, 0, WalkMode::Edit)
            .paints
            .into_iter()
            .map(|p| p.kind)
            .collect()
    };
    let original = kinds(fx::swapped_image(false));
    assert_eq!(
        original,
        kinds(renumbered),
        "WALK-27 renumbering keeps paint kinds"
    );
    let swapped = kinds(fx::swapped_image(true));
    assert_ne!(
        original, swapped,
        "WALK-27 a swapped image behind /Im0 differs"
    );
    assert!(matches!(&swapped[0], PaintKind::ImageXObject { name, .. } if name == b"Im0"));
}
