//! Unit tests of the Save integration's own checks: proofs and warnings are bound to planned
//! pages explicitly (review-T5 L2), and `verify_final`'s independent #34 assertion (review-T5 M4).

use super::map::SourceEdits;
use super::{kept_models_budget, outcome, verify_final, PlannedPage, PreparedTextEdits};
use crate::error::AppError;
use crate::pdf_engine::text_edit::content::page_content;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::gate::{
    verify_edited_copy, BeforeModel, EditedPageInput, ModelPrint, PageProof, PhaseAInput,
    PhaseAReport, PopplerRef, TextWarning,
};
use crate::pdf_engine::text_edit::limits::{PAGE_DECODE_BUDGET, VERIFY_CAP_MARGIN_BYTES};
use crate::pdf_engine::text_edit::reasons::TextWarningCode;
use crate::pdf_engine::text_edit::rewrite::assemble_page_plan;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::producers::helvetica_page;
use crate::pdf_engine::text_edit::testkit::Scratch;
use crate::pdf_engine::text_edit::tests_gate::{ed, opts, Honest};
use crate::pdf_engine::validate_output::{content_digest, ContentDigest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Source page 0 planned with no change, saved as destination page 3.
fn planned(dir: &Scratch) -> Vec<PlannedPage> {
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Bound) Tj ET");
    let path = dir.write("bound.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let id = ctx.page_id(0).expect("page");
    let content =
        page_content(ctx.doc(), id, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("content");
    let plan = assemble_page_plan(&content, 0, Vec::new());
    vec![PlannedPage {
        dest: 3,
        print: ModelPrint::of(&build_page_model(&ctx, 0, None).expect("model")),
        content: Arc::new(content),
        model: None,
        plan,
        warnings: Vec::new(),
    }]
}

fn proof(source_page_index: u32) -> PageProof {
    PageProof {
        source_page_index,
        expected_parts: Vec::new(),
        part_digests: Vec::new(),
        original_edited_digests: Vec::new(),
        original_edited_rolling: Vec::new(),
        expected_page_digest: content_digest(b""),
        records: Vec::new(),
    }
}

fn source() -> SourceEdits<'static> {
    SourceEdits {
        path: "bound.pdf".into(),
        key: PathBuf::from("bound.pdf"),
        pages: BTreeMap::new(),
    }
}

fn gate_failure(r: Result<impl Sized, AppError>, what: &str, case: &str) {
    let Err(e) = r else {
        panic!("{case}: accepted");
    };
    assert_eq!(e.code, "SOURCE_EDIT_GATE_FAILED", "{case}: {e}");
    let details = e.details.unwrap_or_default();
    assert!(details.contains(what), "{case}: {details}");
}

fn report(proofs: Vec<PageProof>, warnings: Vec<TextWarning>) -> PhaseAReport {
    PhaseAReport { proofs, warnings }
}

/// review-T5 L2: `outcome` used to key a proof or warning of an unplanned source page to that
/// page's own number, as if it were a destination page.
#[test]
fn l2_proofs_and_warnings_bind_to_planned_pages_only() {
    let dir = Scratch::new("export_l2");
    let planned = planned(&dir);
    let src = source();
    let edited = PathBuf::from("edited.pdf");
    let ok = outcome(
        &src,
        &planned,
        report(vec![proof(0)], Vec::new()),
        edited.clone(),
    )
    .unwrap_or_else(|e| panic!("bound proof: {e} {:?}", e.details));
    assert_eq!(ok.proofs.len(), 1);
    assert_eq!(ok.proofs[0].0, 3, "keyed by the destination page");
    let r = outcome(
        &src,
        &planned,
        report(vec![proof(1)], Vec::new()),
        edited.clone(),
    );
    gate_failure(r, "source page 2 was not planned", "unplanned proof");
    let r = outcome(
        &src,
        &planned,
        report(Vec::new(), Vec::new()),
        edited.clone(),
    );
    gate_failure(r, "0 proofs for 1 planned pages", "missing proof");
    let twice = report(vec![proof(0), proof(0)], Vec::new());
    gate_failure(
        outcome(&src, &planned, twice, edited.clone()),
        "2 proofs",
        "two proofs",
    );
    let stray = TextWarning {
        page_index: 5,
        run_id: "r".into(),
        code: TextWarningCode::EditNotVisible,
        detail: None,
    };
    let r = outcome(&src, &planned, report(vec![proof(0)], vec![stray]), edited);
    gate_failure(r, "source page 6 was not planned", "unplanned warning");
}

/// review-T5 M4: after Phase B passes on a real staged file, #34's snapshot digest of each edited
/// destination page must be the proof's independently planned digest.
#[test]
fn m4_verify_final_asserts_the_planned_34_digest() {
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello world) Tj ET");
    let Some(h) = Honest::new("export_m4", pdf, 0, &[ed("Hello world", "Hello there")]) else {
        return;
    };
    let size = std::fs::metadata(&h.staged).map_or(0, |m| m.len());
    let prepared = PreparedTextEdits {
        content_groups: Vec::new(),
        proofs: vec![(0, h.proof().clone())],
        warnings: Vec::new(),
        verify_cap: 4 * size + VERIFY_CAP_MARGIN_BYTES,
    };
    let planned = h.proof().expected_page_digest;
    let run = |digests: &[ContentDigest]| {
        verify_final(&h.staged, &prepared, digests, &h.engines, &opts())
    };
    assert!(run(&[planned]).is_ok(), "the planned digest");
    let other = ContentDigest {
        hash: planned.hash ^ 1,
        len: planned.len,
    };
    for (case, digests) in [("altered", vec![other]), ("out of range", Vec::new())] {
        let e = run(&digests)
            .err()
            .unwrap_or_else(|| panic!("{case}: accepted"));
        assert_eq!(e.code, "SOURCE_EDIT_GATE_FAILED", "{case}");
        let details = e.details.clone().unwrap_or_default();
        assert!(
            details.contains("phase=B check=34 page=1"),
            "{case}: {details}"
        );
        let suggestion = e.suggestion.clone().unwrap_or_default();
        assert_eq!(
            suggestion
                .matches("The original file was not changed.")
                .count(),
            1,
            "{case}: {suggestion}"
        );
    }
}

/// review-final MEDIUM-1: the kept page models are bounded by twice the file, with no 160 MiB
/// cap: on a 151 MiB file the cap kept the source context (6.1 × file) where the models cost less.
#[test]
fn medium1_kept_models_are_bounded_by_twice_the_file_without_a_cap() {
    const MIB: u64 = 1 << 20;
    assert_eq!(kept_models_budget(151 * MIB), 302 << 20);
    assert_eq!(
        kept_models_budget(MIB),
        16 << 20,
        "at least KEPT_MODELS_MIN"
    );
    assert_eq!(kept_models_budget(u64::MAX), usize::MAX);
}

/// review-final LOW-5: a before-model Phase A builds again from the source context must be the
/// planned one (its print); another page's print is `STALE`, never verified against.
#[test]
fn low5_a_model_built_again_must_match_the_planned_print() {
    let Some(h) = Honest::new(
        "low5_print",
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        0,
        &[ed("Hello", "Help")],
    ) else {
        return;
    };
    let other = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hellp) Tj ET");
    let other_path = h.path("other.pdf");
    let other_ctx =
        SnapshotContext::new(snapshot_from_bytes(&other_path, other, None).expect("snapshot"));
    let other_print = ModelPrint::of(&build_page_model(&other_ctx, 0, None).expect("model"));
    let planned_print = ModelPrint::of(&h.model);
    assert_ne!(planned_print, other_print);
    let phase_a = |print: &ModelPrint| {
        let input = PhaseAInput {
            before: &h.digest,
            before_page_count: h.page_count,
            staged: &h.staged,
            staged_cap: 1 << 24,
            pages: vec![EditedPageInput {
                model: BeforeModel::Build {
                    ctx: &h.ctx,
                    page_index: 0,
                    print,
                },
                plan: &h.plan,
                input_page_index: 0,
                input_render: PopplerRef {
                    pdf: h.source.clone(),
                    page_1: 1,
                },
            }],
            source_benign: &[],
        };
        verify_edited_copy(&input, &h.engines, h.dir.dir(), &opts())
    };
    phase_a(&planned_print).unwrap_or_else(|e| panic!("planned print: {e} {:?}", e.details));
    let e = phase_a(&other_print)
        .err()
        .expect("another page's print must fail");
    assert_eq!(e.code, "STALE", "{:?}", e.details);
    assert!(e
        .details
        .unwrap_or_default()
        .contains("built again differs"));
}
