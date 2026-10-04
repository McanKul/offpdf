//! T3 tests (SPEC §E.3): WALK-01…27 (`tests_walk/walk.rs`, `walk2.rs`), GEO-01…15 (`geo.rs`),
//! RUN-01…22 (`runs.rs`, `runs2.rs`), the producer smoke tests (§E.2, `producers.rs`), the
//! walker fuzz (§B.21, `fuzz.rs`) and the review regressions on work and memory bounds and model
//! fidelity (`bounds.rs`, `bounds2.rs`, `bounds3.rs`, `bounds4.rs`, `pen.rs`), the page-model
//! budget against every amplification probe (`budget.rs`), and the engine fix pass of 2026-10-03
//! (`fixes.rs`: fonts, refused and dense pages, the shared Classify budget; `joins.rs`: content
//! part boundaries, inline-image ends). Shared helpers live here.

mod bounds;
mod bounds2;
mod bounds3;
mod bounds4;
mod budget;
mod fixes;
mod fuzz;
mod geo;
mod joins;
mod pen;
mod producers;
mod runs;
mod runs2;
mod walk;
mod walk2;

use crate::pdf_engine::text_edit::content::{page_content, PageContent};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel, TextRun};
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::walker::{walk_page, PageWalk, WalkMode};
use std::path::Path;

/// A context over `pdf` read by the production snapshot reader.
pub(super) fn ctx(pdf: Vec<u8>) -> SnapshotContext {
    let snap = snapshot_from_bytes(Path::new("walk-fixture.pdf"), pdf, None)
        .unwrap_or_else(|e| panic!("fixture must open: {e} {:?}", e.details));
    SnapshotContext::new(snap)
}

pub(super) fn model(ctx: &SnapshotContext, page: u32) -> PageModel {
    build_page_model(ctx, page, None).unwrap_or_else(|e| panic!("page model: {e}"))
}

/// The model of page 0 of `pdf`.
pub(super) fn model0(pdf: Vec<u8>) -> PageModel {
    model(&ctx(pdf), 0)
}

pub(super) fn content(ctx: &SnapshotContext, page: u32) -> PageContent {
    let id = ctx.page_id(page).expect("page id");
    page_content(ctx.doc(), id, &mut DecodeBudget::new(PAGE_DECODE_BUDGET))
        .unwrap_or_else(|r| panic!("page content: {r:?}"))
}

pub(super) fn walk(ctx: &SnapshotContext, page: u32, mode: WalkMode) -> PageWalk {
    let c = content(ctx, page);
    walk_page(ctx, page, &c, mode, None)
}

pub(super) fn texts(m: &PageModel) -> Vec<String> {
    m.runs.iter().map(|r| r.text.clone()).collect()
}

/// The run whose text is `text` (panics with the page's runs otherwise).
pub(super) fn run_with<'m>(m: &'m PageModel, text: &str) -> &'m TextRun {
    m.runs.iter().find(|r| r.text == text).unwrap_or_else(|| {
        panic!(
            "no run {text:?}; runs: {:?}; page_reason {:?} ({:?})",
            m.runs
                .iter()
                .map(|r| (r.text.clone(), r.reason))
                .collect::<Vec<_>>(),
            m.page_reason,
            m.page_detail
        )
    })
}

pub(super) fn reason_of(m: &PageModel, text: &str) -> Option<TextReason> {
    run_with(m, text).reason
}

pub(super) fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}
