//! The edit planner (SPEC §B.12): from a page model and the requested edits to exact replacement
//! bytes for the show ops of each edited run, the expected content parts, and the expectations
//! verification checks the re-walk against. A minimal diff keeps unchanged leading and trailing
//! glyphs with their codes and kerns; the pen after the run is compensated so every follower stays
//! where it was; style changes are written as `Tf`/`Tc`/`rg` and restored verbatim.
//!
//! Steps 1–4 (`rewrite/style.rs`): resolve, validate, normalise and check availability. Steps 5–8
//! (`rewrite/diff.rs`): targets, minimal diff, encoding, the new unit list. Steps 9–10
//! (`rewrite/layout.rs`): advances, column gaps, compensation, fit. Steps 11–14
//! (`rewrite/bytes.rs`): bytes, grammar, splices, expected parts. Step 15: the self-check through
//! `verify::walk_and_verify` before any IO; a failure is an internal `EDIT_VERIFY_FAILED` and qpdf
//! is never invoked.

mod bytes;
mod cff_scale;
mod diff;
mod layout;
mod masks;
mod style;

pub(crate) use style::letter_spacing_pt;

use crate::error::AppError;
use crate::pdf_engine::text_edit::content::PageContent;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::fonts::{Code, FontModel, TypingSurface};
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::EDITS_PER_PAGE_MAX;
use crate::pdf_engine::text_edit::reasons::{
    EditProblem, EditProblemCode, Face, ProblemCtx, TextWarningCode,
};
use crate::pdf_engine::text_edit::runs::PageModel;
use crate::pdf_engine::text_edit::state::Paint;
use crate::pdf_engine::text_edit::verify;
use crate::pdf_engine::validate_output::ContentDigest;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) use bytes::assemble_page_plan;

// Edit input types are defined here; dto.rs re-exports them and edit_overlay.rs uses
// `text_edit::rewrite::SourceTextStyleIn`.

/// Requested style, each field only when it differs from the run's current value.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTextStyleIn {
    pub size_pt: Option<f64>,
    pub face: Option<Face>,
    pub fill: Option<String>,
    pub letter_spacing_pt: Option<f64>,
}

impl SourceTextStyleIn {
    pub fn is_empty(&self) -> bool {
        self.size_pt.is_none()
            && self.face.is_none()
            && self.fill.is_none()
            && self.letter_spacing_pt.is_none()
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEditIn {
    pub run_id: String,
    pub original_text: String,
    pub text: String,
    #[serde(default)]
    pub style: SourceTextStyleIn,
}

/// What the replacement sets. Numbers are the values read back from what is written.
#[derive(Clone)]
pub struct StyleTarget {
    pub tfs: f64,
    /// The `Tf` operand to write: the original token when the size is unchanged.
    pub tfs_token: Vec<u8>,
    pub tc: f64,
    pub fill: Option<[f64; 3]>,
    pub face_surface: Option<TypingSurface>,
    pub size_changed: bool,
    pub tc_changed: bool,
    pub fill_changed: bool,
    pub face_changed: bool,
    /// The requested face (verification recomputes the allowed fonts from it).
    pub face: Option<Face>,
    /// The requested effective size in points (the original one when unchanged).
    pub effective_size: f64,
}

impl std::fmt::Debug for StyleTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StyleTarget")
            .field("tfs", &self.tfs)
            .field("tc", &self.tc)
            .field("fill", &self.fill)
            .field("size_changed", &self.size_changed)
            .field("tc_changed", &self.tc_changed)
            .field("fill_changed", &self.fill_changed)
            .field("face_changed", &self.face_changed)
            .field("face", &self.face)
            .finish()
    }
}

impl Clone for TypingSurface {
    fn clone(&self) -> Self {
        TypingSurface {
            fonts: self
                .fonts
                .iter()
                .map(|(n, m)| (n.clone(), Arc::clone(m)))
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum NewUnit {
    /// A unit of the run kept as it is (index in `TextRun::units`).
    Kept(usize),
    /// A newly encoded glyph (`font` indexes the typing surface used).
    Code { font: usize, code: Code },
    /// A typed space written as a TJ number (§A.3.4 kern mode).
    KernSpace(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Splice {
    pub part: usize,
    pub local: Span,
    pub joined: Span,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ExpectedRun {
    pub run_id: String,
    /// The text the planned glyphs decode to (synthetic spaces as " ").
    pub text: String,
    /// Font resource (empty for an ExtGState font), font content hash and code of every glyph.
    pub glyphs: Vec<(Vec<u8>, u64, Code)>,
    pub prefix_glyphs: usize,
    pub suffix_glyphs: usize,
    /// Displacement of the first kept suffix glyph (`Δ·dir`).
    pub shift_user: (f64, f64),
    /// Suffix glyph index (in `glyphs`) from which the displacement is 0 (a column gap absorbed Δ).
    pub unshifted_from: Option<usize>,
    pub origin: (f64, f64),
    pub primary_pen_after: (f64, f64),
    pub member_pen_after: Vec<(f64, f64)>,
    pub tfs: f64,
    pub effective_size: f64,
    pub tc: f64,
    pub fill: Paint,
    /// Per member: the number of show ops its replacement emits.
    pub emitted_records: Vec<usize>,
    // ---- additions (T4, see DEVIATIONS) ----
    /// The joined-buffer spans of the members, primary first: the id-free identity of the run.
    pub member_spans: Vec<Span>,
    /// Expected user-space origin of every glyph (the new layout, from the written numbers).
    pub glyph_origins: Vec<(f64, f64)>,
    /// The glyph was newly encoded (its code must be drawable, B3).
    pub glyph_new: Vec<bool>,
    /// A kept glyph's (member, glyph index in that member's record).
    pub kept_from: Vec<Option<(usize, usize)>>,
    /// User-space boxes covering every old and new glyph (A5 render masks): one per new glyph,
    /// in `glyph_origins` order, then the old glyphs' (`new_glyph_masks`).
    pub mask_boxes: Vec<[f64; 4]>,
    /// User-space boxes of the old glyphs' own ink, as tight as known (the program's outline
    /// box, else the advance; `rewrite/masks.rs` `ink_box`): A5 checks the pixels of the glyphs no
    /// edit changes outside them and the new glyphs' masks (review-verify HIGH-A).
    pub old_ink_boxes: Vec<[f64; 4]>,
    /// The non-space glyph sequence changed (A5 `EDIT_NOT_VISIBLE`).
    pub glyphs_changed: bool,
    /// The text the user asked for, copied from the request (never derived from the planned
    /// glyphs): A4 and A5 compare what the file reads back with it.
    pub requested_text: String,
}

impl ExpectedRun {
    /// The new glyphs' masks: the first `glyph_origins.len()` of `mask_boxes`.
    pub fn new_glyph_masks(&self) -> &[[f64; 4]] {
        self.mask_boxes
            .get(..self.glyph_origins.len())
            .unwrap_or(&self.mask_boxes)
    }
}

#[derive(Debug, Clone)]
pub struct RunPlan {
    pub run_id: String,
    pub target: StyleTarget,
    /// One per member, primary first.
    pub splices: Vec<Splice>,
    pub expected: ExpectedRun,
    pub new_rect: [f64; 4],
}

#[derive(Debug, Clone)]
pub struct EditVerdict {
    pub run_id: String,
    pub problem: Option<EditProblem>,
    pub delta_pt: f64,
    pub new_rect: Option<[f64; 4]>,
    pub caret_offsets: Option<Vec<f64>>,
    pub warnings: Vec<TextWarningCode>,
}

impl EditVerdict {
    fn failed(run_id: &str, problem: EditProblem) -> EditVerdict {
        EditVerdict {
            run_id: run_id.to_string(),
            problem: Some(problem),
            delta_pt: 0.0,
            new_rect: None,
            caret_offsets: None,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PagePlan {
    pub page_index: u32,
    pub runs: Vec<RunPlan>,
    /// Sorted by joined start.
    pub splices: Vec<Splice>,
    pub expected_parts: Vec<Vec<u8>>,
    pub edited_parts: Vec<usize>,
    pub expected_joined: Vec<u8>,
    pub expected_page_digest: ContentDigest,
}

impl PagePlan {
    /// The page content after the edit, in the same parts as `content`.
    pub fn expected_content(&self, content: &PageContent) -> PageContent {
        let replaced: Vec<(usize, Vec<u8>)> = self
            .edited_parts
            .iter()
            .filter_map(|i| Some((*i, self.expected_parts.get(*i)?.clone())))
            .collect();
        content.with_replaced_parts(&replaced)
    }
}

/// `plan` is `None` iff any verdict has a problem or every edit is a no-op.
pub struct PlanOutcome {
    pub verdicts: Vec<EditVerdict>,
    pub plan: Option<PagePlan>,
}

/// A problem with a code only (and a technical detail).
pub(crate) fn problem(code: EditProblemCode, detail: impl Into<String>) -> EditProblem {
    EditProblem::new(code, Some(detail.into()))
}

/// `BAD_EDIT`: a malformed request (not a user problem).
pub(crate) fn bad_edit(detail: &str) -> AppError {
    AppError::new(
        "BAD_EDIT",
        "Unsupported edit data",
        "A text change has a style value OffPDF can't apply.",
    )
    .with_details(detail.to_string())
}

fn too_many_edits(n: usize) -> AppError {
    AppError::new(
        "TOO_MANY_TEXT_EDITS",
        "Too many text changes",
        format!("This save has more than {EDITS_PER_PAGE_MAX} changed lines on one page."),
    )
    .with_suggestion("Save in smaller batches.")
    .with_details(format!("edits on one page: {n}"))
}

/// Plans every edit of one page (verdicts in request order), then self-checks the whole page plan
/// by re-walking the expected content in the same context. `AppError` only for a malformed
/// request (`BAD_EDIT`, `TOO_MANY_TEXT_EDITS`) or an internal self-check failure
/// (`EDIT_VERIFY_FAILED`).
pub fn plan_page(
    ctx: &SnapshotContext,
    model: &PageModel,
    edits: &[TextEditIn],
) -> Result<PlanOutcome, AppError> {
    if edits.len() > EDITS_PER_PAGE_MAX {
        return Err(too_many_edits(edits.len()));
    }
    for e in edits {
        style::validate_style(&e.style)?;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in edits {
        *counts.entry(e.run_id.as_str()).or_insert(0) += 1;
    }
    let mut verdicts = Vec::with_capacity(edits.len());
    let mut runs: Vec<RunPlan> = Vec::new();
    let mut masks = masks::MaskFonts::new(ctx.doc());
    for e in edits {
        if counts.get(e.run_id.as_str()).copied().unwrap_or(0) > 1 {
            verdicts.push(EditVerdict::failed(
                &e.run_id,
                problem(
                    EditProblemCode::EditConflict,
                    "the same line is changed twice",
                ),
            ));
            continue;
        }
        match plan_edit(model, e, &mut masks) {
            Ok((verdict, Some(run_plan))) => {
                let overlaps = runs.iter().flat_map(|r| &r.splices).any(|s| {
                    run_plan
                        .splices
                        .iter()
                        .any(|t| t.joined.start < s.joined.end && s.joined.start < t.joined.end)
                });
                if overlaps {
                    verdicts.push(EditVerdict::failed(
                        &e.run_id,
                        problem(EditProblemCode::EditConflict, "overlapping replacements"),
                    ));
                } else {
                    verdicts.push(verdict);
                    runs.push(run_plan);
                }
            }
            Ok((verdict, None)) => verdicts.push(verdict),
            Err(p) => verdicts.push(EditVerdict::failed(&e.run_id, p)),
        }
    }
    if verdicts.iter().any(|v| v.problem.is_some()) || runs.is_empty() {
        return Ok(PlanOutcome {
            verdicts,
            plan: None,
        });
    }
    let plan = assemble_page_plan(&model.content, model.page_index, runs);
    self_check(ctx, model, &plan)?;
    Ok(PlanOutcome {
        verdicts,
        plan: Some(plan),
    })
}

/// Step 15: the expected content re-walked in the same context must pass `verify_page`.
fn self_check(ctx: &SnapshotContext, model: &PageModel, plan: &PagePlan) -> Result<(), AppError> {
    let after = plan.expected_content(&model.content);
    let walks = (&after, &*model.walk, &model.runs[..]);
    verify::walk_and_verify_with(ctx, model.page_index, walks, plan, None, drop).map_err(
        |failure| {
            let p = problem(
                EditProblemCode::EditVerifyFailed,
                format!("self-check: {failure}"),
            );
            let ctx = ProblemCtx {
                page_number: Some(model.page_index.saturating_add(1)),
                file_name: None,
                face: None,
                reason: None,
            };
            EditProblemCode::EditVerifyFailed.to_app_error(&p, &ctx)
        },
    )
}

/// One edit: `(verdict, plan)`; the plan is `None` for a no-op.
fn plan_edit(
    model: &PageModel,
    edit: &TextEditIn,
    masks: &mut masks::MaskFonts<'_>,
) -> Result<(EditVerdict, Option<RunPlan>), EditProblem> {
    let run = style::resolve(model, edit)?;
    style::validate_text(&edit.text)?;
    let primary = run
        .members
        .first()
        .and_then(|i| model.walk.records.get(*i))
        .ok_or_else(|| problem(EditProblemCode::EditVerifyFailed, "run without members"))?;
    let primary_model: Option<Arc<FontModel>> = primary.font.clone();
    let mut wanted = style::normalise(run, &edit.style);
    if edit.text.is_empty() {
        // Removing a line draws nothing: there is no style to apply.
        wanted = SourceTextStyleIn::default();
    }
    if edit.text == run.text && wanted.is_empty() {
        return Ok((
            EditVerdict {
                run_id: run.id.clone(),
                problem: None,
                delta_pt: 0.0,
                new_rect: Some(run.rect),
                caret_offsets: Some(run.caret_offsets.clone()),
                warnings: Vec::new(),
            },
            None,
        ));
    }
    let face_surface = style::availability(
        run,
        primary_model.as_deref(),
        &model.walk.page_fonts,
        &wanted,
        &edit.text,
    )?;
    let target = style::targets(primary, run, &wanted, face_surface)?;
    let surface = match &target.face_surface {
        Some(s) => s.clone(),
        None => model.surface(run),
    };
    let new_units = diff::new_units(model, run, &surface, &target, &edit.text)?;
    let laid = layout::lay_out(masks, model, run, &surface, &target, &new_units)?;
    if laid.text != edit.text {
        // Every later check compares the file with the request, not with the planner's reading
        // of its own glyphs; a plan that would not read back as typed never leaves here.
        return Err(problem(
            EditProblemCode::EditVerifyFailed,
            "the new line would not read back as typed",
        ));
    }
    let verdict = EditVerdict {
        run_id: run.id.clone(),
        problem: None,
        delta_pt: laid.delta_pt,
        new_rect: Some(laid.new_rect),
        caret_offsets: Some(laid.caret_offsets.clone()),
        warnings: laid.warnings.clone(),
    };
    drop(new_units);
    let run_plan = bytes::run_plan(model, run, &edit.text, target, laid)?;
    Ok((verdict, Some(run_plan)))
}
