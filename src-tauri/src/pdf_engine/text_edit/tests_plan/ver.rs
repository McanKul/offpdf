//! VER-01…11: the verification re-walk (§B.14) shared by the self-check, the preview and Save.

use super::{ctx, edit, model, ok_plan, plan, spaced, style};
use crate::pdf_engine::text_edit::apply::{apply_update, updates_for_plan, write_update_json};
use crate::pdf_engine::text_edit::content::page_content;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::engines::RunOpts;
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::{PAGE_DECODE_BUDGET, VERIFY_CAP_MARGIN_BYTES};
use crate::pdf_engine::text_edit::reasons::EditProblemCode as P;
use crate::pdf_engine::text_edit::rewrite::{assemble_page_plan, PagePlan, RunPlan, Splice};
use crate::pdf_engine::text_edit::runs::PageModel;
use crate::pdf_engine::text_edit::snapshot::read_verification_snapshot;
use crate::pdf_engine::text_edit::state::{same_paint, ColorEffect, ColorSpaceKind, Paint};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::verify::{map_span, test_seams, walk_and_verify, VerifyFailure};
use std::sync::Arc;

fn verify(c: &SnapshotContext, m: &PageModel, p: &PagePlan) -> Result<(), VerifyFailure> {
    let after = p.expected_content(&m.content);
    walk_and_verify(c, m.page_index, &after, &m.walk, &m.runs, p, None).map(|_| ())
}

/// `p` with its first run's primary replacement rewritten by `f` (re-assembled consistently).
fn tampered(m: &PageModel, p: &PagePlan, f: impl Fn(&str) -> String) -> PagePlan {
    let mut runs: Vec<RunPlan> = p.runs.clone();
    let s = &mut runs[0].splices[0];
    s.bytes = f(&String::from_utf8_lossy(&s.bytes)).into_bytes();
    assemble_page_plan(&m.content, m.page_index, runs)
}

/// The expected content of `p` with part 0's bytes changed by `f` outside the plan.
fn after_with(
    m: &PageModel,
    p: &PagePlan,
    f: impl Fn(&str) -> String,
) -> crate::pdf_engine::text_edit::content::PageContent {
    let after = p.expected_content(&m.content);
    let bytes = f(&String::from_utf8_lossy(after.part_bytes(0))).into_bytes();
    after.with_replaced_parts(&[(0, bytes)])
}

fn hello_other() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Other) Tj ET 72 600 50 20 re f")
}

#[test]
fn ver_01_honest_edits_pass() {
    for (pdf, old, new) in [
        (hello_other(), "Hello", "Help"),
        (fx::word(), "Due", "Due 7"),
        (
            fx::word_tr(),
            "Sağlık Bakanlığı Raporu",
            "Sağlığı Bakanlık Raporu",
        ),
        (fx::pdftex(), "Hello World", "Hello Wet World"),
        (fx::skia(), "Chrome", "Chrom"),
        (fx::quote_ops(), "Line two", "Line 2"),
    ] {
        let c = ctx(pdf);
        let m = model(&c, 0);
        let out = plan(&c, &m, &[edit(&m, old, new, style())]);
        assert_eq!(
            verify(&c, &m, ok_plan(&out)),
            Ok(()),
            "VER-01 {old:?} → {new:?}"
        );
    }
}

#[test]
fn ver_02_span_mapping_across_splices_and_parts() {
    let s = |start: usize, end: usize, len: usize| Splice {
        part: 0,
        local: start..end,
        joined: start..end,
        bytes: vec![b'x'; len],
    };
    let splices = [s(10, 20, 15), s(30, 32, 0), s(50, 60, 10)];
    let map = |a: usize, b: usize| map_span(&splices, &(a..b));
    assert_eq!(map(0, 5), 0..5, "before every splice");
    assert_eq!(map(20, 25), 25..30, "after the first (+5)");
    assert_eq!(map(32, 40), 35..43, "after the second (+5 −2)");
    assert_eq!(map(70, 80), 73..83, "after all (+5 −2 +0)");
    let _: Span = map(0, 0);
    // A page with edits in parts 0 and 2 (two in part 0) verifies through the mapping.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 700 Td (One) Tj ET BT /F1 12 Tf 72 680 Td (Two) Tj ET",
            b"BT /F1 12 Tf 72 660 Td (Three) Tj ET",
            b"BT /F1 12 Tf 72 640 Td (Four) Tj ET 0 0 1 rg 72 600 20 20 re f",
        ],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let c = ctx(d.build());
    let m = model(&c, 0);
    let out = plan(
        &c,
        &m,
        &[
            edit(&m, "One", "Uno", style()),
            edit(&m, "Two", "Dos", style()),
            edit(&m, "Four", "Cuatro", style()),
        ],
    );
    let p = ok_plan(&out);
    assert_eq!(p.edited_parts, vec![0, 2], "VER-02");
    assert_eq!(verify(&c, &m, p), Ok(()), "VER-02 multi-splice, multi-part");
}

#[test]
fn ver_03_an_unedited_record_unpaired_fails() {
    let c = ctx(hello_other());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Help", style())]);
    let p = ok_plan(&out);
    let after = after_with(&m, p, |s| s.replace("(Other) Tj", "(Other)'"));
    let r = walk_and_verify(&c, 0, &after, &m.walk, &m.runs, p, None).err();
    assert!(
        matches!(r, Some(VerifyFailure::RecordUnpaired { .. })),
        "VER-03 {r:?}"
    );
}

#[test]
fn ver_04_a_paint_count_change_fails() {
    let c = ctx(hello_other());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Help", style())]);
    let p = ok_plan(&out);
    let after = after_with(&m, p, |s| format!("{s} 0 0 1 rg 100 100 5 5 re f"));
    let r = walk_and_verify(&c, 0, &after, &m.walk, &m.runs, p, None).err();
    assert!(
        matches!(r, Some(VerifyFailure::RecordCount { .. })),
        "VER-04 {r:?}"
    );
}

#[test]
fn ver_05_each_failure_maps_to_its_problem_code() {
    let cases = [
        (
            VerifyFailure::Drift {
                record: 0,
                glyph: 0,
                pt: 0.5,
            },
            P::PenDrift,
        ),
        (
            VerifyFailure::StateChanged {
                record: 0,
                field: "tc",
            },
            P::StateChanged,
        ),
        (
            VerifyFailure::PaintChanged {
                index: 0,
                field: "fill",
            },
            P::StateChanged,
        ),
        (
            VerifyFailure::PageRefused(
                crate::pdf_engine::text_edit::reasons::TextReason::MalformedContent,
            ),
            P::EditVerifyFailed,
        ),
        (
            VerifyFailure::RecordUnpaired { index: 0 },
            P::EditVerifyFailed,
        ),
        (
            VerifyFailure::RecordCount {
                expected: 1,
                found: 2,
            },
            P::EditVerifyFailed,
        ),
        (
            VerifyFailure::EditedMismatch {
                run_id: "r".into(),
                what: "text",
            },
            P::EditVerifyFailed,
        ),
        (
            VerifyFailure::ForbiddenOperator { op: "q" },
            P::EditVerifyFailed,
        ),
    ];
    for (f, code) in cases {
        assert_eq!(f.problem_code(), code, "VER-05 {f}");
        assert!(!f.to_string().is_empty(), "VER-05 detail");
    }
}

#[test]
fn ver_06_honest_edit_passes_after_qpdf_renumbering() {
    let Some(engines) = engines_or_skip("ver_06") else {
        return;
    };
    let dir = Scratch::new("ver_06");
    let pdf = fx::word();
    let source = dir.write("source.pdf", &pdf);
    let c = ctx(pdf);
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Due", "Due 7", style())]);
    let p = ok_plan(&out);
    let update = dir.path("update.json");
    write_update_json(
        &updates_for_plan(&m.content, p).expect("updates"),
        c.doc().max_id,
        &update,
    )
    .expect("json");
    let staged = dir.path("staged.pdf");
    apply_update(
        &engines,
        &source,
        &update,
        &staged,
        &[],
        &RunOpts::default(),
    )
    .expect("qpdf");
    let snap = read_verification_snapshot(&staged, VERIFY_CAP_MARGIN_BYTES).expect("staged");
    let s = SnapshotContext::new(snap);
    let ids: Vec<_> = (0..s.snap.pages.len()).map(|i| s.snap.pages[i]).collect();
    assert_ne!(ids, c.snap.pages, "VER-06 qpdf renumbered the objects");
    let page = s.page_id(0).expect("page");
    let content =
        page_content(s.doc(), page, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("content");
    let r = walk_and_verify(&s, 0, &content, &m.walk, &m.runs, p, None).map(|_| ());
    assert_eq!(r, Ok(()), "VER-06 id-free comparisons");
}

#[test]
fn ver_07_b14_post_state_catches_a_missing_restore() {
    // No follower in the text object: only the post-state probe sees Tc still at 1.
    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Hello", spaced(1.0))]);
    let p = ok_plan(&out);
    let bad = tampered(&m, p, |s| s.replace("] TJ 0 Tc", "] TJ"));
    let r = verify(&c, &m, &bad);
    assert!(
        matches!(r, Err(VerifyFailure::StateChanged { field: "tc", .. })),
        "VER-07 {r:?}"
    );
}

#[test]
fn ver_08_grammar_rechecked_on_the_rewalk() {
    let c = ctx(hello_other());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Help", style())]);
    let p = ok_plan(&out);
    let bad = tampered(&m, p, |s| format!("2 w {s}"));
    assert_eq!(
        verify(&c, &m, &bad),
        Err(VerifyFailure::ForbiddenOperator { op: "w" }),
        "VER-08"
    );
}

#[test]
fn ver_09_swapped_image_behind_the_same_name() {
    let c = ctx(fx::swapped_image(false));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Image page", "Image pages", style())]);
    let p = ok_plan(&out);
    assert_eq!(verify(&c, &m, p), Ok(()), "VER-09 honest");
    // The same content walked in the file whose /Im0 holds other pixels.
    let swapped = ctx(fx::swapped_image(true));
    let after = p.expected_content(&m.content);
    let r = walk_and_verify(&swapped, 0, &after, &m.walk, &m.runs, p, None).err();
    assert_eq!(
        r,
        Some(VerifyFailure::PaintChanged {
            index: 0,
            field: "kind"
        }),
        "VER-09"
    );
    assert_eq!(
        r.map(|f| f.problem_code()),
        Some(P::StateChanged),
        "VER-09 code"
    );
}

#[test]
fn ver_10_leaked_line_width_on_an_edited_tr1_glyph() {
    let c = ctx(fx::stroke_text(1, [0.5, 0.5], ["[] 0", "[] 0"]));
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Boldface", "Bold face", style())]);
    let p = ok_plan(&out);
    let bad = tampered(&m, p, |s| format!("2 w {s}"));
    let _g = test_seams::skip_grammar();
    let r = verify(&c, &m, &bad);
    assert!(
        matches!(
            r,
            Err(VerifyFailure::StateChanged {
                field: "line_width",
                ..
            })
        ),
        "VER-10 {r:?}"
    );
}

#[test]
fn ver_11_same_paint() {
    let paint = |space: ColorSpaceKind, comps: &[f64], op: Option<&str>| Paint {
        space_op: None,
        color_op: op.map(|o| Arc::from(o.as_bytes())),
        space,
        comps: comps.to_vec(),
        effect: ColorEffect::Rgb([0.0; 3]),
        pattern: false,
        pattern_hash: None,
    };
    let never = Paint::initial();
    let rgb0 = paint(
        ColorSpaceKind::DeviceRgb,
        &[0.0, 0.0, 0.0],
        Some("0 0 0 rg"),
    );
    let gray0 = paint(ColorSpaceKind::DeviceGray, &[0.0], Some("0 g"));
    let k_black = paint(
        ColorSpaceKind::DeviceCmyk,
        &[0.0, 0.0, 0.0, 1.0],
        Some("0 0 0 1 k"),
    );
    let rich = paint(
        ColorSpaceKind::DeviceCmyk,
        &[0.6, 0.4, 0.4, 1.0],
        Some(".6 .4 .4 1 k"),
    );
    assert!(same_paint(&never, &rgb0), "VER-11 never set vs 0 0 0 rg");
    assert!(
        same_paint(&never, &gray0) && same_paint(&never, &k_black),
        "VER-11 default black"
    );
    assert!(
        !same_paint(&gray0, &k_black),
        "VER-11 explicit 0 g vs 0 0 0 1 k"
    );
    assert!(
        !same_paint(&gray0, &rgb0),
        "VER-11 explicit 0 g vs 0 0 0 rg"
    );
    assert!(
        !same_paint(&rich, &k_black),
        "VER-11 rich black vs K-only black"
    );
    let near = paint(
        ColorSpaceKind::DeviceRgb,
        &[0.0, 0.0, 0.0000005],
        Some("0 0 0.0000005 rg"),
    );
    assert!(same_paint(&rgb0, &near), "VER-11 within COLOR_EPSILON");
}
