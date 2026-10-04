//! Save integration (SPEC §B.18): text changes are written into per-source edited copies
//! **before** assembly, each proven by Phase A; the Edit PDF pipeline then runs unchanged on the
//! substituted paths, and Phase B proves the final staged file before #34's
//! `validate_staged_pdf` and the atomic rename (`edit_overlay::export_edit_pdf_with_check_exe`).
//!
//! Every error of this path passes through `reasons::save_failure`, so its suggestion ends with
//! "The original file was not changed." Save never uses the editor's cache: each edited source is
//! read again (one bounded read), its fingerprint must equal the one the edits were made on, and
//! qpdf only reads a copy written from those bytes. The only shared state is the memoised
//! `qpdf --check` result for identical bytes.

mod map;

use crate::error::AppError;
use crate::models::{JobUpdate, PageGroup};
use crate::pdf_engine::edit_overlay::{EditDocumentIn, EditObjectIn};
use crate::pdf_engine::text_edit::apply::{apply_update, updates_for_plan, write_update_json};
use crate::pdf_engine::text_edit::cache::file_name;
use crate::pdf_engine::text_edit::content::{page_content, qpdf_join, PageContent};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::engines::{
    qpdf_check_memo, qpdf_page_map, Engines, RunOpts, SourceCheck,
};
use crate::pdf_engine::text_edit::gate::join::join_is_neutral;
use crate::pdf_engine::text_edit::gate::{
    cancel_flag, verify_edited_copy, verify_final_output, BeforeModel, EditedPageInput, ModelPrint,
    PageProof, PhaseAInput, PhaseAReport, PopplerRef, TextWarning,
};
use crate::pdf_engine::text_edit::graph::{graph_digest, GraphDigest};
use crate::pdf_engine::text_edit::limits::{
    EDITS_PER_SAVE_MAX, EDIT_TEXT_CHARS_MAX, PAGE_DECODE_BUDGET, VERIFY_CAP_MARGIN_BYTES,
};
use crate::pdf_engine::text_edit::reasons::{
    self, save_failure, EditProblem, EditProblemCode, ProblemCtx, TextWarningCode,
};
use crate::pdf_engine::text_edit::rewrite::{plan_page, PagePlan, SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::snapshot::{check_page_map, read_snapshot};
use crate::pdf_engine::validate_output::{content_digest, ContentDigest};
use lopdf::{Document, ObjectId};
use map::{dest_pages, group_specs, key_of, SourceEdits};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One `sourceText` object of the export document.
#[derive(Debug, Clone)]
pub struct SourceTextSpec {
    /// Destination page (0-based, in the saved file).
    pub page_index: u32,
    /// Page of the source file (0-based) the edit was made on.
    pub source_page_index: u32,
    pub run_id: String,
    pub source_fingerprint: String,
    pub original_text: String,
    pub text: String,
    pub style: SourceTextStyleIn,
}

pub struct PreparedTextEdits {
    /// `groups` with every edited source replaced by its proven edited copy.
    pub content_groups: Vec<PageGroup>,
    /// Phase A proof of each edited destination page.
    pub proofs: Vec<(u32, PageProof)>,
    /// User sentences for the job message.
    pub warnings: Vec<String>,
    /// `read_verification_snapshot` cap of the final file: 2 × Σ distinct input sizes + margin.
    pub verify_cap: u64,
}

/// Progress steps shown on the job (`JobUpdate` step text).
pub const STEP_CHECKING: &str = "Checking text changes";
pub const STEP_APPLYING: &str = "Applying text changes";
pub const STEP_VERIFYING: &str = "Verifying text changes";

const WARNING_TEXT_MAX_CHARS: usize = 40;

/// The least `kept_models_budget` (a few ordinary page models are always kept).
const KEPT_MODELS_MIN: usize = 16 << 20;

fn too_many_edits(n: usize) -> AppError {
    AppError::new(
        "TOO_MANY_TEXT_EDITS",
        "Too many text changes",
        format!("This save has more than {EDITS_PER_SAVE_MAX} changed lines."),
    )
    .with_suggestion("Save in smaller batches.")
    .with_details(format!("text changes: {n}"))
}

fn redacted_page(dest: u32) -> AppError {
    let n = u64::from(dest) + 1;
    AppError::new(
        "TEXT_EDIT_ON_REDACTED_PAGE",
        "Redaction and text change on the same page",
        format!("Page {n} has both a redaction and a text change. Redaction turns the page into an image, so the text change would be lost."),
    )
    .with_suggestion("Remove the redaction or the text change on that page.")
    .with_details(format!("page={n}"))
}

fn problem_error(code: EditProblemCode, dest: u32, detail: &str) -> AppError {
    let p = EditProblem::new(code, Some(detail.to_string()));
    let ctx = ProblemCtx {
        page_number: Some(dest.saturating_add(1)),
        file_name: None,
        face: None,
        reason: None,
    };
    code.to_app_error(&p, &ctx)
}

/// The `sourceText` objects of an Edit PDF export document.
pub fn specs_of(doc: &EditDocumentIn) -> Vec<SourceTextSpec> {
    doc.objects
        .iter()
        .filter_map(|o| match o {
            EditObjectIn::SourceText {
                page_index,
                source_page_index,
                run_id,
                source_fingerprint,
                original_text,
                text,
                style,
                ..
            } => Some(SourceTextSpec {
                page_index: *page_index,
                source_page_index: *source_page_index,
                run_id: run_id.clone(),
                source_fingerprint: source_fingerprint.clone(),
                original_text: original_text.clone(),
                text: text.clone(),
                style: style.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// The engines of a Save with text changes, resolved before any work: qpdf ≥ 11
/// (`ENGINE_MISSING`) and both Poppler tools (`VERIFIER_MISSING`).
pub fn save_engines(qpdf: &Path, app: Option<&tauri::AppHandle>) -> Result<Engines, AppError> {
    let engines = Engines::for_export(qpdf, app).map_err(save_failure)?;
    #[cfg(test)]
    let engines = seams::engines(engines);
    Ok(engines)
}

/// The Edit PDF export's entry point: `None` when `doc` has no text change, else the engines and
/// the proven edited copies (progress steps go to the job when `app` is given).
pub fn prepare_save(
    doc: &EditDocumentIn,
    groups: &[PageGroup],
    work: &Path,
    qpdf: &Path,
    app: Option<&tauri::AppHandle>,
    job_id: &str,
    opts: &RunOpts<'_>,
) -> Result<Option<(Engines, PreparedTextEdits)>, AppError> {
    let specs = specs_of(doc);
    if specs.is_empty() {
        return Ok(None);
    }
    let engines = save_engines(qpdf, app)?;
    let mut progress = |step: &'static str| {
        if let Some(app) = app {
            use tauri::Emitter;
            let _ = app.emit("job:update", JobUpdate::new(job_id, "running", step));
        }
    };
    let prepared = prepare_text_sources(groups, &specs, work, &engines, opts, &mut progress)?;
    Ok(Some((engines, prepared)))
}

/// Digest of a page's content parts as qpdf joins them into its overlay Form (§B.18, the #34
/// `alt_content_digest`), decoded with the bounded text-edit decoder; `None` when that equals the
/// plain concatenation, when the join would change what the parts mean (`join_is_neutral`,
/// review-T5 H1: a part ending inside a comment or a string) or when the decoder refuses the
/// page — then only the plain digest is expected, as before.
pub fn qpdf_joined_digest(doc: &Document, page_id: ObjectId) -> Option<ContentDigest> {
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let page = page_content(doc, page_id, &mut budget).ok()?;
    let parts: Vec<&[u8]> = (0..page.parts.len()).map(|i| page.part_bytes(i)).collect();
    let joined = qpdf_join(&parts);
    let digest = content_digest(&joined);
    (digest != page.concat_digest() && join_is_neutral(&parts, &joined)).then_some(digest)
}

/// Request-level checks before any file is read (`validate_doc`): ≤ `EDITS_PER_SAVE_MAX`
/// (`TOO_MANY_TEXT_EDITS`), ≤ 1,000 characters (`TEXT_TOO_LONG`), one change per
/// (page, line) (`EDIT_CONFLICT`), no page with both a redaction and a text change
/// (`TEXT_EDIT_ON_REDACTED_PAGE`).
pub fn validate_specs(specs: &[SourceTextSpec], redact_pages: &[u32]) -> Result<(), AppError> {
    check_specs(specs, redact_pages).map_err(save_failure)
}

fn check_specs(specs: &[SourceTextSpec], redact_pages: &[u32]) -> Result<(), AppError> {
    if specs.len() > EDITS_PER_SAVE_MAX {
        return Err(too_many_edits(specs.len()));
    }
    let redacted: HashSet<u32> = redact_pages.iter().copied().collect();
    let mut seen: HashSet<(u32, &str)> = HashSet::new();
    for s in specs {
        if s.text.chars().count() > EDIT_TEXT_CHARS_MAX {
            return Err(problem_error(
                EditProblemCode::TextTooLong,
                s.page_index,
                &format!("{} characters", s.text.chars().count()),
            ));
        }
        if !seen.insert((s.page_index, s.run_id.as_str())) {
            return Err(problem_error(
                EditProblemCode::EditConflict,
                s.page_index,
                &format!("run {} is changed twice", s.run_id),
            ));
        }
        if redacted.contains(&s.page_index) {
            return Err(redacted_page(s.page_index));
        }
    }
    Ok(())
}

/// Writes and proves the edited copies (§B.18 steps 1–6). A no-op (empty `specs`, or only
/// changes equal to the original) returns `groups` unchanged.
pub fn prepare_text_sources(
    groups: &[PageGroup],
    specs: &[SourceTextSpec],
    work: &Path,
    engines: &Engines,
    opts: &RunOpts<'_>,
    progress: &mut dyn FnMut(&'static str),
) -> Result<PreparedTextEdits, AppError> {
    if specs.is_empty() {
        return Ok(PreparedTextEdits {
            content_groups: groups.to_vec(),
            proofs: Vec::new(),
            warnings: Vec::new(),
            verify_cap: 0,
        });
    }
    prepare(groups, specs, work, engines, opts, progress).map_err(save_failure)
}

/// Phase B on the final staged file, then #34's expected digest of each edited page must equal
/// the proof's independently planned digest (`snapshot_digests` = the #34 snapshot, by dest page).
pub fn verify_final(
    tmp: &Path,
    prepared: &PreparedTextEdits,
    snapshot_digests: &[ContentDigest],
    engines: &Engines,
    opts: &RunOpts<'_>,
) -> Result<(), AppError> {
    let expectations: Vec<(u32, &PageProof)> =
        prepared.proofs.iter().map(|(d, p)| (*d, p)).collect();
    verify_final_output(tmp, prepared.verify_cap, engines, &expectations, opts)
        .and_then(|()| {
            for (dest, proof) in &prepared.proofs {
                let planned = usize::try_from(*dest)
                    .ok()
                    .and_then(|i| snapshot_digests.get(i));
                if planned != Some(&proof.expected_page_digest) {
                    return Err(reasons::source_edit_gate_failed(&format!(
                        "phase=B check=34 page={} the #34 snapshot digest is not the planned page",
                        u64::from(*dest) + 1
                    )));
                }
            }
            Ok(())
        })
        .map_err(save_failure)
}

fn prepare(
    groups: &[PageGroup],
    specs: &[SourceTextSpec],
    work: &Path,
    engines: &Engines,
    opts: &RunOpts<'_>,
    progress: &mut dyn FnMut(&'static str),
) -> Result<PreparedTextEdits, AppError> {
    progress(STEP_CHECKING);
    let dest = dest_pages(groups, engines, opts)?;
    let sources = group_specs(groups, &dest, specs)?;
    let mut substitutes: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut proofs = Vec::new();
    let mut warnings = Vec::new();
    for (i, src) in sources.iter().enumerate() {
        let out = edit_source(i, src, work, engines, opts, progress)?;
        if let Some(edited) = out.edited {
            substitutes.insert(src.key.clone(), edited);
        }
        proofs.extend(out.proofs);
        warnings.extend(out.warnings);
    }
    let content_groups: Vec<PageGroup> = groups
        .iter()
        .map(|g| match substitutes.get(&key_of(&g.path)) {
            Some(edited) => PageGroup {
                path: edited.to_string_lossy().into_owned(),
                pages: g.pages.clone(),
            },
            None => g.clone(),
        })
        .collect();
    Ok(PreparedTextEdits {
        verify_cap: verify_cap(groups, &content_groups),
        content_groups,
        proofs,
        warnings,
    })
}

/// 2 × Σ over distinct inputs of max(original, edited copy) + `VERIFY_CAP_MARGIN_BYTES`.
fn verify_cap(groups: &[PageGroup], content_groups: &[PageGroup]) -> u64 {
    let size = |p: &str| std::fs::metadata(p).map_or(0, |m| m.len());
    let mut seen = HashSet::new();
    let total = groups
        .iter()
        .zip(content_groups)
        .filter(|(g, _)| seen.insert(g.path.as_str()))
        .map(|(g, c)| size(&g.path).max(size(&c.path)))
        .fold(0u64, u64::saturating_add);
    total
        .saturating_mul(2)
        .saturating_add(VERIFY_CAP_MARGIN_BYTES)
}

struct EditedSource {
    /// `None` when every change of this source was a no-op (nothing is written).
    edited: Option<PathBuf>,
    proofs: Vec<(u32, PageProof)>,
    warnings: Vec<String>,
}

/// One planned page of an edited source.
struct PlannedPage {
    dest: u32,
    /// The page's content in the source (the update's stream ids).
    content: Arc<PageContent>,
    /// Its model while the kept models stay within `kept_models_budget`; otherwise Phase A
    /// builds it again from the source context when it reaches the page (review-T4 M-1).
    model: Option<PageModel>,
    /// The model's print, which a model built again in Phase A must have (review-final LOW-5).
    print: ModelPrint,
    plan: PagePlan,
    /// The planner's warnings (`NEXT_TEXT_OVERLAP`).
    warnings: Vec<TextWarning>,
}

/// Bytes of page models one source's Save may keep from planning to Phase A: twice the file
/// (about what keeping its context and building each model again costs instead), at least
/// `KEPT_MODELS_MIN`. Past it, the source context is kept and no model: each is built again when
/// Phase A reaches its page, one at a time. No upper clamp: on a file over 80 MiB a 160 MiB cap
/// kept the context where the models cost less (review-final MEDIUM-1: 6.1 × file).
pub(super) fn kept_models_budget(file_len: u64) -> usize {
    usize::try_from(file_len.saturating_mul(2))
        .unwrap_or(usize::MAX)
        .max(KEPT_MODELS_MIN)
}

/// An edited source read again for the Save: its context, the copy qpdf reads, and the
/// benign `qpdf --check` warnings it already had.
struct CheckedSource {
    ctx: SnapshotContext,
    copy: PathBuf,
    benign: Vec<String>,
    page_count: u32,
    /// `read_verification_snapshot` cap of its edited copy: 2 × source size + margin.
    staged_cap: u64,
}

/// §B.18 step 3 for one source: fresh snapshot, fingerprint, copy, page map, check, plan,
/// graph digest (then the snapshot is dropped), qpdf update, Phase A.
fn edit_source(
    i: usize,
    src: &SourceEdits<'_>,
    work: &Path,
    engines: &Engines,
    opts: &RunOpts<'_>,
    progress: &mut dyn FnMut(&'static str),
) -> Result<EditedSource, AppError> {
    progress(STEP_CHECKING);
    let name = file_name(Path::new(&src.path));
    let checked = check_source(i, src, &name, work, engines, opts)?;
    let budget = kept_models_budget(checked.ctx.snap.fingerprint.len);
    let planned = plan_pages(&checked.ctx, src, &name, opts, budget)?;
    if planned.is_empty() {
        let _ = std::fs::remove_file(&checked.copy);
        return Ok(EditedSource {
            edited: None,
            proofs: Vec::new(),
            warnings: Vec::new(),
        });
    }
    progress(STEP_APPLYING);
    let CheckedSource {
        ctx,
        copy,
        benign,
        page_count,
        staged_cap,
    } = checked;
    let rebuild = planned.iter().any(|p| p.model.is_none());
    let written = write_edited(
        i, ctx, rebuild, &planned, &copy, &benign, work, engines, opts,
    )?;
    let (edited, before, ctx) = written;
    progress(STEP_VERIFYING);
    let a_work = work.join(format!("text-{i}-check"));
    std::fs::create_dir_all(&a_work)
        .map_err(|e| AppError::io("OffPDF could not create a work folder.", e))?;
    let pages = planned
        .iter()
        .map(|p| phase_a_page(p, ctx.as_ref(), &copy))
        .collect::<Result<Vec<_>, AppError>>()?;
    let input = PhaseAInput {
        before: &before,
        before_page_count: page_count,
        staged: &edited,
        staged_cap,
        pages,
        source_benign: &benign,
    };
    let report = verify_edited_copy(&input, engines, &a_work, opts)?;
    drop(input);
    drop(ctx);
    let _ = std::fs::remove_dir_all(&a_work);
    let _ = std::fs::remove_file(&copy);
    outcome(src, &planned, report, edited)
}

/// Phase A's input for one planned page: its kept model, or the source context to build it from.
fn phase_a_page<'a>(
    p: &'a PlannedPage,
    ctx: Option<&'a SnapshotContext>,
    copy: &Path,
) -> Result<EditedPageInput<'a>, AppError> {
    let page_index = p.plan.page_index;
    let model = match (&p.model, ctx) {
        (Some(m), _) => BeforeModel::Kept(m),
        (None, Some(ctx)) => BeforeModel::Build {
            ctx,
            page_index,
            print: &p.print,
        },
        (None, None) => {
            return Err(reasons::source_edit_gate_failed(
                "phase=A the source page model is gone",
            ))
        }
    };
    Ok(EditedPageInput {
        model,
        plan: &p.plan,
        input_page_index: page_index,
        input_render: PopplerRef {
            pdf: copy.to_path_buf(),
            page_1: page_index.saturating_add(1),
        },
    })
}

/// Reads the source again (never the editor's cache) and checks it like the editor did at open:
/// same fingerprint as the edits (`STALE`), lopdf/qpdf page-map agreement, `qpdf --check`
/// (memoised for these bytes; problems → `PDF_NEEDS_REPAIR`).
fn check_source(
    i: usize,
    src: &SourceEdits<'_>,
    name: &str,
    work: &Path,
    engines: &Engines,
    opts: &RunOpts<'_>,
) -> Result<CheckedSource, AppError> {
    let mut snap = read_snapshot(Path::new(&src.path))?;
    let fp = snap.fingerprint.to_string();
    let specs = src.pages.values().flat_map(|(_, specs)| specs);
    if specs.clone().any(|s| s.source_fingerprint != fp) {
        return Err(reasons::stale(name));
    }
    let copy = work.join(format!("text-{i}-source.pdf"));
    std::fs::write(&copy, snap.bytes.as_slice())
        .map_err(|e| AppError::io("OffPDF could not write a work file.", e))?;
    check_page_map(&snap, &qpdf_page_map(engines, &copy, opts)?)?;
    let benign = match qpdf_check_memo(engines, snap.fingerprint, &copy, opts)? {
        SourceCheck::Clean => Vec::new(),
        SourceCheck::Benign(lines) => lines,
        SourceCheck::Problems(lines) => return Err(reasons::pdf_needs_repair(&lines)),
    };
    // qpdf reads the copy from here on; the plan and Phase A read the parsed document.
    snap.release_bytes();
    Ok(CheckedSource {
        page_count: u32::try_from(snap.pages.len()).unwrap_or(u32::MAX),
        staged_cap: snap
            .fingerprint
            .len
            .saturating_mul(2)
            .saturating_add(VERIFY_CAP_MARGIN_BYTES),
        ctx: SnapshotContext::new(snap),
        copy,
        benign,
    })
}

/// The "before" graph digest of the source with the planned parts, then — the snapshot and its
/// document dropped (F14) unless `keep_ctx` (a page's model must be built again in Phase A) —
/// the qpdf update into `text-<i>-edited.pdf`.
#[allow(clippy::too_many_arguments)]
fn write_edited(
    i: usize,
    ctx: SnapshotContext,
    keep_ctx: bool,
    planned: &[PlannedPage],
    copy: &Path,
    benign: &[String],
    work: &Path,
    engines: &Engines,
    opts: &RunOpts<'_>,
) -> Result<(PathBuf, GraphDigest, Option<SnapshotContext>), AppError> {
    let mut replaced = HashMap::new();
    let mut updates = Vec::new();
    for p in planned {
        for u in updates_for_plan(&p.content, &p.plan)? {
            replaced.insert(u.object_id, u.decoded.clone());
            updates.push(u);
        }
    }
    let before = graph_digest(ctx.doc(), &replaced, cancel_flag(opts)).map_err(|m| {
        problem_error(
            EditProblemCode::EditVerifyFailed,
            planned.first().map_or(0, |p| p.dest),
            &format!("phase=A check=A2 source path={} what={}", m.path, m.what),
        )
    })?;
    let max_id = ctx.doc().max_id;
    drop(replaced);
    let ctx = keep_ctx.then_some(ctx);
    let update = work.join(format!("text-{i}-update.json"));
    write_update_json(&updates, max_id, &update)?;
    drop(updates);
    let edited = work.join(format!("text-{i}-edited.pdf"));
    apply_update(engines, copy, &update, &edited, benign, opts)?;
    let _ = std::fs::remove_file(&update);
    #[cfg(test)]
    seams::tamper(&edited);
    Ok((edited, before, ctx))
}

/// Proofs keyed by destination page and the warnings as job sentences. Every planned page has
/// exactly one proof, and every proof and warning is bound to a planned page explicitly
/// (`SOURCE_EDIT_GATE_FAILED` otherwise, review-T5 L2).
fn outcome(
    src: &SourceEdits<'_>,
    planned: &[PlannedPage],
    report: PhaseAReport,
    edited: PathBuf,
) -> Result<EditedSource, AppError> {
    let dest_of = |source_page: u32| {
        planned
            .iter()
            .find(|p| p.plan.page_index == source_page)
            .map(|p| p.dest)
            .ok_or_else(|| {
                reasons::source_edit_gate_failed(&format!(
                    "phase=A source page {} was not planned",
                    u64::from(source_page) + 1
                ))
            })
    };
    let texts: HashMap<&str, &str> = src
        .pages
        .values()
        .flat_map(|(_, specs)| specs)
        .map(|s| (s.run_id.as_str(), s.text.as_str()))
        .collect();
    let mut warnings = Vec::new();
    for w in planned
        .iter()
        .flat_map(|p| &p.warnings)
        .chain(&report.warnings)
    {
        warnings.extend(warning_sentence(w, dest_of(w.page_index)?, &texts));
    }
    let proofs = report
        .proofs
        .into_iter()
        .map(|proof| Ok((dest_of(proof.source_page_index)?, proof)))
        .collect::<Result<Vec<_>, AppError>>()?;
    let distinct: HashSet<u32> = proofs.iter().map(|(d, _)| *d).collect();
    if proofs.len() != planned.len() || distinct.len() != planned.len() {
        return Err(reasons::source_edit_gate_failed(&format!(
            "phase=A {} proofs for {} planned pages",
            proofs.len(),
            planned.len()
        )));
    }
    Ok(EditedSource {
        edited: Some(edited),
        proofs,
        warnings,
    })
}

/// Plans every edited page of the source; the first failed verdict is the save error (with the
/// destination page number). Pages whose changes are all no-ops are left out. Models are kept
/// when their bytes (`model_bytes`) all fit within `budget`; otherwise none is.
fn plan_pages(
    ctx: &SnapshotContext,
    src: &SourceEdits<'_>,
    name: &str,
    opts: &RunOpts<'_>,
    budget: usize,
) -> Result<Vec<PlannedPage>, AppError> {
    let mut planned = Vec::new();
    let mut kept = 0usize;
    for (source_page, (dest, specs)) in &src.pages {
        let model = build_page_model(ctx, *source_page, cancel_flag(opts))?;
        if let Some(e) = page_refusal(&model, dest.saturating_add(1)) {
            return Err(e);
        }
        let edits: Vec<TextEditIn> = specs
            .iter()
            .map(|s| TextEditIn {
                run_id: s.run_id.clone(),
                original_text: s.original_text.clone(),
                text: s.text.clone(),
                style: s.style.clone(),
            })
            .collect();
        let outcome = plan_page(ctx, &model, &edits)?;
        if let Some(p) = outcome.verdicts.iter().find_map(|v| v.problem.as_ref()) {
            let pctx = ProblemCtx {
                page_number: Some(dest.saturating_add(1)),
                file_name: Some(name),
                face: p.face,
                reason: p.reason,
            };
            return Err(p.code.to_app_error(p, &pctx));
        }
        if let Some(plan) = outcome.plan {
            let warnings = outcome
                .verdicts
                .iter()
                .flat_map(|v| {
                    v.warnings.iter().map(|code| TextWarning {
                        page_index: *source_page,
                        run_id: v.run_id.clone(),
                        code: *code,
                        detail: None,
                    })
                })
                .collect();
            let bytes = model.walk.model_bytes;
            let keep = kept.saturating_add(bytes) <= budget;
            if keep {
                kept = kept.saturating_add(bytes);
            }
            planned.push(PlannedPage {
                dest: *dest,
                print: ModelPrint::of(&model),
                content: Arc::clone(&model.content),
                model: keep.then_some(model),
                plan,
                warnings,
            });
        }
    }
    // Past the budget the source context stays for Phase A, and every page's model is built
    // again from it: models kept as well would only add to it (review-final MEDIUM-1).
    if planned.iter().all(|p| p.model.is_some()) {
        return Ok(planned);
    }
    Ok(planned
        .into_iter()
        .map(|p| PlannedPage { model: None, ..p })
        .collect())
}

/// The error of a page nothing can be changed on (its page-level reason code, with the model's
/// technical detail), or `None`.
pub(crate) fn page_refusal(model: &PageModel, page_number: u32) -> Option<AppError> {
    let e = model.page_reason?.to_app_error(Some(page_number));
    Some(match (&e.details, &model.page_detail) {
        (Some(base), Some(detail)) => {
            let details = format!("{base}; {detail}");
            e.with_details(details)
        }
        _ => e,
    })
}

fn short(text: &str) -> String {
    let mut out: String = text.chars().take(WARNING_TEXT_MAX_CHARS).collect();
    if text.chars().count() > WARNING_TEXT_MAX_CHARS {
        out.push('\u{2026}');
    }
    out
}

/// A Phase A warning as a sentence for the job message.
fn warning_sentence(w: &TextWarning, dest: u32, texts: &HashMap<&str, &str>) -> Option<String> {
    let n = u64::from(dest) + 1;
    let line = short(texts.get(w.run_id.as_str()).copied().unwrap_or_default());
    match w.code {
        TextWarningCode::NextTextOverlap => Some(format!(
            "Page {n}: the changed line \u{201c}{line}\u{201d} runs into the text that follows it."
        )),
        TextWarningCode::EditNotVisible => Some(format!(
            "Page {n}: the change to \u{201c}{line}\u{201d} doesn't change how the page looks. Something may be drawn over this line."
        )),
        TextWarningCode::PreviewUnavailable => None,
    }
}

/// Test seams (GATE-25 tamper hook, E2E-19 missing Poppler); absent from release builds.
#[cfg(test)]
pub(crate) mod seams {
    use crate::pdf_engine::text_edit::engines::Engines;
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};

    thread_local! {
        static TAMPER: Cell<Option<fn(&Path)>> = const { Cell::new(None) };
        static POPPLER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Runs on each edited copy right after `apply_update` (this thread only).
    pub(crate) fn set_tamper_hook(hook: Option<fn(&Path)>) {
        TAMPER.with(|t| t.set(hook));
    }

    pub(super) fn tamper(edited: &Path) {
        if let Some(hook) = TAMPER.with(Cell::get) {
            hook(edited);
        }
    }

    /// Replaces both Poppler tools of the save's engines (this thread only).
    pub(crate) fn set_poppler_override(path: Option<PathBuf>) {
        POPPLER.with(|p| *p.borrow_mut() = path);
    }

    pub(crate) fn engines(mut engines: Engines) -> Engines {
        if let Some(p) = POPPLER.with(|p| p.borrow().clone()) {
            engines.pdftoppm = p.clone();
            engines.pdftotext = p;
        }
        engines
    }
}

#[cfg(test)]
mod tests;
