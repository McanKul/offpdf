//! T4 gate tests (SPEC §E.5, qpdf and Poppler required): GATE-01…08 and 11…13
//! (`tests_gate/plan_bugs.rs`: a planner bug written consistently, so A0–A3 pass and A4 must
//! catch it), GATE-09, 10 and 14…17 (`writer_bugs.rs`: qpdf wrote other bytes than planned),
//! GATE-18…22 and 24 (`cover.rs`: cover-and-overlay, raster and annotation fakes, and the #34
//! composition), GATE-23 and 26…29 (`collateral.rs`), PB-01…08 (`phase_b.rs`), the preview
//! core under the same Phase A (`preview.rs`), the gate's cost bounds and cancellation
//! (`bounds.rs`), and the regressions of the T4 review (`review_fixes.rs`). Each
//! case builds the honest plan, writes it through `apply_update`, runs `verify_edited_copy`
//! (which must pass), then produces a tampered file and asserts the failing check id in the
//! error details. Later checks are reached by skipping earlier ones through the `#[cfg(test)]`
//! seams, so every listed check is shown to fail on its own. Fakes are built with
//! `testkit/fakes.rs` and real qpdf only.

mod bounds;
mod collateral;
mod cover;
mod phase_b;
mod plan_bugs;
mod preview;
mod review_fixes;
mod writer_bugs;

use crate::error::AppError;
use crate::pdf_engine::text_edit::apply::{apply_update, updates_for_plan, write_update_json};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::{Engines, RunOpts};
use crate::pdf_engine::text_edit::gate::{
    test_seams, verify_edited_copy, verify_final_output, EditedPageInput, PageProof, PhaseAInput,
    PhaseAReport, PopplerRef,
};
use crate::pdf_engine::text_edit::graph::{graph_digest, GraphDigest};
use crate::pdf_engine::text_edit::limits::VERIFY_CAP_MARGIN_BYTES;
use crate::pdf_engine::text_edit::rewrite::{
    assemble_page_plan, plan_page, PagePlan, RunPlan, SourceTextStyleIn, TextEditIn,
};
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use lopdf::ObjectId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One edit of the run whose text is `old`.
pub(crate) struct Ed<'a> {
    pub old: &'a str,
    pub new: &'a str,
    pub style: SourceTextStyleIn,
}

pub(crate) fn ed<'a>(old: &'a str, new: &'a str) -> Ed<'a> {
    Ed {
        old,
        new,
        style: SourceTextStyleIn::default(),
    }
}

/// An honest edit written by qpdf and proven by Phase A, with everything a tampering test needs.
pub(crate) struct Honest {
    pub dir: Scratch,
    pub engines: Engines,
    pub ctx: SnapshotContext,
    pub model: PageModel,
    pub plan: PagePlan,
    pub page: u32,
    pub page_count: u32,
    /// qpdf's input (the snapshot bytes).
    pub source: PathBuf,
    /// qpdf's honest output.
    pub staged: PathBuf,
    pub digest: GraphDigest,
    pub report: PhaseAReport,
}

pub(crate) fn opts() -> RunOpts<'static> {
    RunOpts::default()
}

/// The page model and plan of `edits` on page `page` of `ctx` (all verdicts must be ok).
pub(crate) fn plan_of(ctx: &SnapshotContext, page: u32, edits: &[Ed<'_>]) -> (PageModel, PagePlan) {
    let model = build_page_model(ctx, page, None).unwrap_or_else(|e| panic!("model: {e}"));
    let ins: Vec<TextEditIn> = edits
        .iter()
        .map(|e| {
            let run = model
                .runs
                .iter()
                .find(|r| r.text == e.old)
                .unwrap_or_else(|| {
                    panic!(
                        "no run {:?} in {:?}",
                        e.old,
                        model
                            .runs
                            .iter()
                            .map(|r| (&r.text, r.reason))
                            .collect::<Vec<_>>()
                    )
                });
            TextEditIn {
                run_id: run.id.clone(),
                original_text: e.old.to_string(),
                text: e.new.to_string(),
                style: e.style.clone(),
            }
        })
        .collect();
    let out = plan_page(ctx, &model, &ins).unwrap_or_else(|e| panic!("plan: {e} {:?}", e.details));
    for v in &out.verdicts {
        assert!(v.problem.is_none(), "verdict: {:?}", v.problem);
    }
    let plan = out.plan.expect("a plan");
    (model, plan)
}

impl Honest {
    /// `None` (after printing `skip:`) when an engine is missing.
    pub(crate) fn new(test: &str, pdf: Vec<u8>, page: u32, edits: &[Ed<'_>]) -> Option<Honest> {
        let mut h = Self::unverified(test, pdf, page, edits)?;
        h.report = h.phase_a(&h.staged).unwrap_or_else(|e| {
            panic!(
                "{test}: the honest edit must pass Phase A: {e} {:?}",
                e.details
            )
        });
        Some(h)
    }

    /// The honest edit planned and written by qpdf, Phase A not run (`report` is empty).
    pub(crate) fn unverified(
        test: &str,
        pdf: Vec<u8>,
        page: u32,
        edits: &[Ed<'_>],
    ) -> Option<Honest> {
        let engines = engines_or_skip(test)?;
        let dir = Scratch::new(test);
        let source = dir.write("source.pdf", &pdf);
        let snap = snapshot_from_bytes(&source, pdf, None).unwrap_or_else(|e| panic!("{e}"));
        let page_count = snap.pages.len() as u32;
        let ctx = SnapshotContext::new(snap);
        let (model, plan) = plan_of(&ctx, page, edits);
        let staged = dir.path("edited.pdf");
        let digest = write_plan(&engines, &ctx, &model, &plan, &source, &staged);
        Some(Honest {
            dir,
            engines,
            ctx,
            model,
            plan,
            page,
            page_count,
            source,
            staged,
            digest,
            report: PhaseAReport {
                proofs: Vec::new(),
                warnings: Vec::new(),
            },
        })
    }

    pub(crate) fn qpdf(&self) -> &Path {
        &self.engines.qpdf
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.path(name)
    }

    /// Phase A of `staged` against this edit's plan and digest.
    pub(crate) fn phase_a(&self, staged: &Path) -> Result<PhaseAReport, AppError> {
        phase_a_with(self, &self.model, &self.plan, &self.digest, staged, &opts())
    }

    /// `phase_a` with the caller's run options (cancel flag, job handle).
    pub(crate) fn phase_a_opts(
        &self,
        staged: &Path,
        run_opts: &RunOpts<'_>,
    ) -> Result<PhaseAReport, AppError> {
        phase_a_with(
            self,
            &self.model,
            &self.plan,
            &self.digest,
            staged,
            run_opts,
        )
    }

    /// The proof of the honest edit (for Phase B).
    pub(crate) fn proof(&self) -> &PageProof {
        self.report.proofs.first().expect("a proof")
    }

    /// Phase B of `final_pdf` with the honest proof on destination page `dest`.
    pub(crate) fn phase_b(&self, final_pdf: &Path, dest: u32) -> Result<(), AppError> {
        let cap = 4 * std::fs::metadata(final_pdf).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES;
        verify_final_output(
            final_pdf,
            cap,
            &self.engines,
            &[(dest, self.proof())],
            &opts(),
        )
    }

    /// The source stream id of part `i` of the edited page.
    pub(crate) fn part_id(&self, i: usize) -> ObjectId {
        self.model.content.parts[i].stream_id
    }

    /// A planner bug: `tamper` rewrites the primary replacement of the first run; the bytes are
    /// re-assembled into a consistent plan, written by qpdf (to `name`) and returned with it.
    pub(crate) fn bad_plan(
        &self,
        name: &str,
        tamper: impl Fn(&str) -> String,
    ) -> (PagePlan, GraphDigest, PathBuf) {
        self.bad_run(name, |run| {
            let s = &mut run.splices[0];
            let text = tamper(&String::from_utf8_lossy(&s.bytes));
            s.bytes = text.into_bytes();
        })
    }

    /// A planner bug in the first run plan (bytes and expectations as `f` leaves them).
    pub(crate) fn bad_run(
        &self,
        name: &str,
        f: impl Fn(&mut RunPlan),
    ) -> (PagePlan, GraphDigest, PathBuf) {
        let mut runs: Vec<RunPlan> = self.plan.runs.clone();
        f(&mut runs[0]);
        let plan = assemble_page_plan(&self.model.content, self.page, runs);
        self.write_bad(name, plan)
    }

    /// Writes `plan` through qpdf to `name`; returns it with its digest.
    pub(crate) fn write_bad(&self, name: &str, plan: PagePlan) -> (PagePlan, GraphDigest, PathBuf) {
        let out = self.path(name);
        let digest = write_plan(
            &self.engines,
            &self.ctx,
            &self.model,
            &plan,
            &self.source,
            &out,
        );
        (plan, digest, out)
    }

    /// Phase A of a file written from another (tampered) plan.
    pub(crate) fn phase_a_of(
        &self,
        plan: &PagePlan,
        digest: &GraphDigest,
        staged: &Path,
    ) -> Result<PhaseAReport, AppError> {
        phase_a_with(self, &self.model, plan, digest, staged, &opts())
    }
}

/// `write_update_json` + `apply_update` of `plan`, and the "before" digest of qpdf's input.
pub(crate) fn write_plan(
    engines: &Engines,
    ctx: &SnapshotContext,
    model: &PageModel,
    plan: &PagePlan,
    source: &Path,
    out: &Path,
) -> GraphDigest {
    let updates = updates_for_plan(&model.content, plan).expect("updates");
    let update = out.with_extension("update.json");
    write_update_json(&updates, ctx.doc().max_id, &update).unwrap_or_else(|e| panic!("{e}"));
    apply_update(engines, source, &update, out, &[], &opts())
        .unwrap_or_else(|e| panic!("{e} {:?}", e.details));
    let replaced: HashMap<ObjectId, Vec<u8>> = updates
        .into_iter()
        .map(|u| (u.object_id, u.decoded))
        .collect();
    graph_digest(ctx.doc(), &replaced, None).unwrap_or_else(|m| panic!("digest: {m:?}"))
}

fn phase_a_with(
    h: &Honest,
    model: &PageModel,
    plan: &PagePlan,
    digest: &GraphDigest,
    staged: &Path,
    run_opts: &RunOpts<'_>,
) -> Result<PhaseAReport, AppError> {
    let cap = 2 * std::fs::metadata(&h.source).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES;
    let input = PhaseAInput {
        before: digest,
        before_page_count: h.page_count,
        staged,
        staged_cap: cap,
        pages: vec![EditedPageInput {
            model: model.into(),
            plan,
            input_page_index: h.page,
            input_render: PopplerRef {
                pdf: h.source.clone(),
                page_1: h.page + 1,
            },
        }],
        source_benign: &[],
    };
    verify_edited_copy(&input, &h.engines, h.dir.dir(), run_opts)
}

/// `r` must fail with `code` and its details must name `check` (and contain `what` when given).
pub(crate) fn fails_at<T>(r: Result<T, AppError>, code: &str, check: &str, what: &str, id: &str) {
    let e = match r {
        Ok(_) => panic!("{id}: expected a failure at {check}, the check passed"),
        Err(e) => e,
    };
    let details = e.details.clone().unwrap_or_default();
    assert_eq!(e.code, code, "{id}: code ({details})");
    assert!(
        details.contains(&format!("check={check}")),
        "{id}: expected check={check}, got {details}"
    );
    assert!(
        details.contains(what),
        "{id}: expected {what:?} in {details}"
    );
}

/// Runs `f` with the named checks skipped (test seam).
pub(crate) fn skipping<T>(checks: &[&str], f: impl FnOnce() -> T) -> T {
    let _guard = test_seams::skip(checks);
    f()
}
