//! T4 tests (SPEC §E.4): PLAN-01…38 (`tests_plan/{plan,plan2,plan3,plan4}.rs`), VER-01…11
//! (`ver.rs`), APP-01…09 (`app.rs`), MEAS-01 (`meas.rs`), the plan fuzz (§B.21, `fuzz.rs`) and
//! the named mobile-bug regressions B1–B3, B7–B10, B13–B16 (`bugs.rs`). Shared helpers live here.

mod app;
mod bugs;
mod fuzz;
mod meas;
mod plan;
mod plan2;
mod plan3;
mod plan4;
mod ver;

use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::reasons::{EditProblemCode, Face};
use crate::pdf_engine::text_edit::rewrite::{
    plan_page, EditVerdict, PagePlan, PlanOutcome, SourceTextStyleIn, TextEditIn,
};
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel, TextRun};
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use std::path::Path;

pub(crate) use fuzz::thread_cpu;

/// A context over `pdf` read by the production snapshot reader.
pub(crate) fn ctx(pdf: Vec<u8>) -> SnapshotContext {
    let snap = snapshot_from_bytes(Path::new("plan-fixture.pdf"), pdf, None)
        .unwrap_or_else(|e| panic!("fixture must open: {e} {:?}", e.details));
    SnapshotContext::new(snap)
}

pub(crate) fn model(ctx: &SnapshotContext, page: u32) -> PageModel {
    build_page_model(ctx, page, None).unwrap_or_else(|e| panic!("page model: {e}"))
}

/// The run whose text is `text` (panics with the page's runs otherwise).
pub(crate) fn run_with<'m>(m: &'m PageModel, text: &str) -> &'m TextRun {
    m.runs.iter().find(|r| r.text == text).unwrap_or_else(|| {
        panic!(
            "no run {text:?}; runs: {:?}; page_reason {:?} ({:?})",
            m.runs
                .iter()
                .map(|r| (r.text.clone(), r.reason, r.reasons.clone()))
                .collect::<Vec<_>>(),
            m.page_reason,
            m.page_detail
        )
    })
}

pub(crate) fn style() -> SourceTextStyleIn {
    SourceTextStyleIn::default()
}

pub(crate) fn sized(pt: f64) -> SourceTextStyleIn {
    SourceTextStyleIn {
        size_pt: Some(pt),
        ..style()
    }
}

pub(crate) fn faced(face: Face) -> SourceTextStyleIn {
    SourceTextStyleIn {
        face: Some(face),
        ..style()
    }
}

pub(crate) fn filled(hex: &str) -> SourceTextStyleIn {
    SourceTextStyleIn {
        fill: Some(hex.to_string()),
        ..style()
    }
}

pub(crate) fn spaced(pt: f64) -> SourceTextStyleIn {
    SourceTextStyleIn {
        letter_spacing_pt: Some(pt),
        ..style()
    }
}

/// An edit of the run whose current text is `old`.
pub(crate) fn edit(m: &PageModel, old: &str, new: &str, s: SourceTextStyleIn) -> TextEditIn {
    TextEditIn {
        run_id: run_with(m, old).id.clone(),
        original_text: old.to_string(),
        text: new.to_string(),
        style: s,
    }
}

/// `plan_page`, which must not fail with an `AppError`.
pub(crate) fn plan(ctx: &SnapshotContext, m: &PageModel, edits: &[TextEditIn]) -> PlanOutcome {
    plan_page(ctx, m, edits).unwrap_or_else(|e| panic!("plan_page: {e} {:?}", e.details))
}

/// One fixture page, one edit: (context, model, outcome).
pub(crate) fn plan_one(
    pdf: Vec<u8>,
    old: &str,
    new: &str,
    s: SourceTextStyleIn,
) -> (SnapshotContext, PageModel, PlanOutcome) {
    let c = ctx(pdf);
    let m = model(&c, 0);
    let e = edit(&m, old, new, s);
    let out = plan(&c, &m, &[e]);
    (c, m, out)
}

/// The plan of an outcome whose verdicts must all be ok.
pub(crate) fn ok_plan(out: &PlanOutcome) -> &PagePlan {
    for v in &out.verdicts {
        assert!(v.problem.is_none(), "verdict failed: {:?}", v.problem);
    }
    out.plan.as_ref().expect("a plan")
}

/// The problem code of the only verdict.
pub(crate) fn problem_of(out: &PlanOutcome) -> EditProblemCode {
    let v: &EditVerdict = out.verdicts.first().expect("one verdict");
    v.problem
        .as_ref()
        .unwrap_or_else(|| panic!("expected a problem, got ok: {v:?}"))
        .code
}

/// The replacement bytes of member `m` of the first planned run, as text.
pub(crate) fn replacement(out: &PlanOutcome, m: usize) -> String {
    let plan = ok_plan(out);
    let run = plan.runs.first().expect("a planned run");
    let s = run.splices.get(m).expect("member splice");
    String::from_utf8_lossy(&s.bytes).trim_start().to_string()
}

/// The decoded page content after the plan (all parts joined with `\n`).
pub(crate) fn after_text(out: &PlanOutcome) -> String {
    String::from_utf8_lossy(&ok_plan(out).expected_joined).into_owned()
}

pub(crate) fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
