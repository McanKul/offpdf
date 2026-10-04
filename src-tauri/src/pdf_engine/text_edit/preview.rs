//! Preview of a patched page (SPEC §B.17): the same path as Save — `plan_page`, a qpdf update,
//! Phase A — on a one-page extraction of the source snapshot, so what the editor shows after a
//! commit is what Save will write (D9, D30).
//!
//! The caller (the command layer) first waits for the source's background `qpdf --check` and
//! passes its benign warnings as `source_benign` (a non-benign result is `PDF_NEEDS_REPAIR` there).
//! Files in `cache_dir` (`<temp>/textedit/<fingerprint>/`) are written once, atomically (a temp
//! name, then a rename; when the final name already exists the temp file is dropped and the
//! existing one, whose content is determined by the fingerprint, is used), so concurrent calls
//! never read a partial file. qpdf only ever reads the snapshot's bytes (`source.pdf`), never the
//! user's file. Each preview works in its own nonce directory, removed at the end.

use crate::error::AppError;
use crate::pdf_engine::text_edit::apply::{apply_update, updates_for_plan, write_update_json};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::{qpdf_page_map, run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::gate::{
    cancel_flag, is_cancelled, verify_edited_copy, EditedPageInput, PhaseAInput, PopplerRef,
    TextWarning,
};
use crate::pdf_engine::text_edit::graph::graph_digest;
use crate::pdf_engine::text_edit::limits::{PREVIEW_PDF_MAX_BYTES, VERIFY_CAP_MARGIN_BYTES};
use crate::pdf_engine::text_edit::reasons::{EditProblem, TextWarningCode};
use crate::pdf_engine::text_edit::rewrite::{plan_page, EditVerdict, PagePlan, TextEditIn};
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::snapshot::{check_page_map, read_verification_snapshot};
use crate::pdf_engine::validate_output::ContentDigest;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct PreviewResult {
    /// The patched one-page PDF, or `None` (a failed verdict, a page problem, or unavailable).
    pub pdf: Option<Vec<u8>>,
    pub verdicts: Vec<EditVerdict>,
    pub page_problem: Option<EditProblem>,
    pub warnings: Vec<TextWarning>,
}

static NONCE: AtomicUsize = AtomicUsize::new(0);

fn nonce() -> String {
    let n = NONCE.fetch_add(1, Ordering::SeqCst);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{}-{n}-{t:x}", std::process::id())
}

fn io(context: &str) -> impl Fn(std::io::Error) -> AppError + '_ {
    move |e| AppError::io(context, e)
}

/// Moves `tmp` to `dest` unless `dest` already exists (then `tmp` is removed and `dest` used).
fn publish_once(tmp: &Path, dest: &Path) -> Result<(), AppError> {
    if dest.is_file() {
        let _ = std::fs::remove_file(tmp);
        return Ok(());
    }
    match std::fs::rename(tmp, dest) {
        Ok(()) => Ok(()),
        Err(_) if dest.is_file() => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(tmp);
            Err(AppError::io("OffPDF could not write a preview file.", e))
        }
    }
}

/// Writes `bytes` to `dest` once (temp name + rename).
pub(crate) fn write_once(dest: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if dest.is_file() {
        return Ok(());
    }
    let tmp = dest.with_extension(format!("{}.tmp", nonce()));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        // A failed write can leave a partial temp file behind.
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io("OffPDF could not write a preview file.", e));
    }
    publish_once(&tmp, dest)
}

/// `p<n>.pdf`: `qpdf --empty --remove-unreferenced-resources=no --pages source.pdf <n> -- out`
/// (shared inherited resources stay whole, D30), written once.
fn extract_page(
    engines: &Engines,
    source: &Path,
    page_1: u32,
    dest: &Path,
    opts: &RunOpts<'_>,
) -> Result<(), AppError> {
    if dest.is_file() {
        return Ok(());
    }
    let tmp = dest.with_extension(format!("{}.tmp", nonce()));
    let args = [
        OsString::from("--empty"),
        OsString::from("--remove-unreferenced-resources=no"),
        OsString::from("--pages"),
        source.as_os_str().to_os_string(),
        OsString::from(page_1.to_string()),
        OsString::from("--"),
        tmp.as_os_str().to_os_string(),
    ];
    let out = match run_tool(&engines.qpdf, &args, false, opts) {
        Ok(out) => out,
        Err(e) => {
            // A cancelled or timed-out qpdf can leave a partial output behind.
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if out.code != 0 && out.code != 3 {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::engine_failed(format!(
            "qpdf page extraction exited with code {}: {}",
            out.code,
            out.stderr.trim()
        )));
    }
    publish_once(&tmp, dest)
}

fn verdict_warnings(page_index: u32, verdicts: &[EditVerdict]) -> Vec<TextWarning> {
    verdicts
        .iter()
        .flat_map(|v| {
            v.warnings.iter().map(move |code| TextWarning {
                page_index,
                run_id: v.run_id.clone(),
                code: *code,
                detail: None,
            })
        })
        .collect()
}

fn unavailable(page_index: u32, detail: &str) -> TextWarning {
    TextWarning {
        page_index,
        run_id: String::new(),
        code: TextWarningCode::PreviewUnavailable,
        detail: Some(detail.to_string()),
    }
}

/// An error of the verification path as a page problem (the editor keeps its last good page).
fn page_problem_of(e: &AppError) -> Option<EditProblem> {
    let code = match e.code.as_str() {
        "PEN_DRIFT" => crate::pdf_engine::text_edit::reasons::EditProblemCode::PenDrift,
        "STATE_CHANGED" => crate::pdf_engine::text_edit::reasons::EditProblemCode::StateChanged,
        "EDIT_VERIFY_FAILED" => {
            crate::pdf_engine::text_edit::reasons::EditProblemCode::EditVerifyFailed
        }
        _ => return None,
    };
    Some(EditProblem::new(code, e.details.clone()))
}

/// What `same_page` compares of the source page: its parts (by digest, in order) and its page
/// fonts (name, content hash). Kept instead of the source model, which the preview releases once
/// it has planned (review-final MEDIUM-3).
struct SourcePage {
    page_index: u32,
    parts: Vec<ContentDigest>,
    fonts: Vec<(Vec<u8>, u64)>,
}

impl SourcePage {
    fn of(model: &PageModel) -> SourcePage {
        SourcePage {
            page_index: model.page_index,
            parts: model.content.parts.iter().map(|p| p.digest).collect(),
            fonts: model
                .walk
                .page_fonts
                .iter()
                .map(|(n, m)| (n.clone(), m.content_hash))
                .collect(),
        }
    }
}

/// Whether `p<n>.pdf`'s page is the source page: the same parts in order (by digest) and every
/// page font name resolving to the same content hash (§B.17 step 3).
fn same_page(source: &SourcePage, extracted: &PageModel) -> bool {
    let fonts: HashMap<&[u8], u64> = extracted
        .walk
        .page_fonts
        .iter()
        .map(|(n, m)| (n.as_slice(), m.content_hash))
        .collect();
    let parts: Vec<ContentDigest> = extracted.content.parts.iter().map(|p| p.digest).collect();
    extracted.page_reason.is_none()
        && source.parts == parts
        && source
            .fonts
            .iter()
            .all(|(n, h)| fonts.get(n.as_slice()) == Some(h))
}

/// Reads a file of at most `cap` bytes.
fn read_capped(path: &Path, cap: u64) -> Result<Option<Vec<u8>>, AppError> {
    let file = std::fs::File::open(path).map_err(io("OffPDF could not read the preview."))?;
    let mut data = Vec::new();
    file.take(cap.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(io("OffPDF could not read the preview."))?;
    Ok((data.len() as u64 <= cap).then_some(data))
}

/// §B.17: plan, patch the one-page extraction with qpdf, prove it with Phase A, return its bytes.
/// A preview interrupted by a cancel is `CANCELLED` (never a page problem). The source `model` is
/// released once the edit is planned: when the caller holds no other reference (the service hands
/// over a large model, `TextEditCache::page_to_release`), it is freed before the extracted page's
/// model is built (review-final MEDIUM-3).
pub fn preview_page(
    ctx: &SnapshotContext,
    model: Arc<PageModel>,
    edits: &[TextEditIn],
    cache_dir: &Path,
    engines: &Engines,
    source_benign: &[String],
    opts: &RunOpts<'_>,
) -> Result<PreviewResult, AppError> {
    let page_index = model.page_index;
    let outcome = plan_page(ctx, &model, edits)?;
    let source_page = SourcePage::of(&model);
    drop(model);
    let mut warnings = verdict_warnings(page_index, &outcome.verdicts);
    let Some(plan) = outcome.plan else {
        return Ok(PreviewResult {
            pdf: None,
            verdicts: outcome.verdicts,
            page_problem: None,
            warnings,
        });
    };
    std::fs::create_dir_all(cache_dir).map_err(io("OffPDF could not create a preview folder."))?;
    let source = cache_dir.join("source.pdf");
    write_once(&source, &ctx.snap.bytes)?;
    let page_1 = page_index.saturating_add(1);
    let extracted = cache_dir.join(format!("p{page_1}.pdf"));
    extract_page(engines, &source, page_1, &extracted, opts)?;
    let work = cache_dir.join(format!("preview-{}", nonce()));
    std::fs::create_dir_all(&work).map_err(io("OffPDF could not create a preview folder."))?;
    let result = patch_and_prove(
        ctx,
        &source_page,
        &plan,
        &extracted,
        &work,
        engines,
        source_benign,
        opts,
    );
    let _ = std::fs::remove_dir_all(&work);
    if is_cancelled(opts) {
        return Err(AppError::cancelled());
    }
    let (pdf, page_problem, mut more) = result?;
    warnings.append(&mut more);
    Ok(PreviewResult {
        pdf,
        verdicts: outcome.verdicts,
        page_problem,
        warnings,
    })
}

type Patched = (Option<Vec<u8>>, Option<EditProblem>, Vec<TextWarning>);

/// Steps 3–6 inside the nonce directory.
#[allow(clippy::too_many_arguments)]
fn patch_and_prove(
    ctx: &SnapshotContext,
    source_page: &SourcePage,
    plan: &PagePlan,
    extracted: &Path,
    work: &Path,
    engines: &Engines,
    source_benign: &[String],
    opts: &RunOpts<'_>,
) -> Result<Patched, AppError> {
    let page_index = source_page.page_index;
    let cap = ctx
        .snap
        .fingerprint
        .len
        .saturating_mul(2)
        .saturating_add(VERIFY_CAP_MARGIN_BYTES);
    let gone = |detail: &str| Ok((None, None, vec![unavailable(page_index, detail)]));
    let Ok(snap) = read_verification_snapshot(extracted, cap) else {
        return gone("the page could not be extracted");
    };
    let Ok(pages) = qpdf_page_map(engines, extracted, opts) else {
        return gone("the extracted page could not be read");
    };
    if check_page_map(&snap, &pages).is_err() {
        return gone("the extracted page reads differently");
    }
    let p_ctx = SnapshotContext::new(snap);
    let p_model = build_page_model(&p_ctx, 0, cancel_flag(opts))?;
    if !same_page(source_page, &p_model) {
        return gone("the extracted page differs from the source page");
    }
    let updates = match updates_for_plan(&p_model.content, plan) {
        Ok(u) => u,
        Err(e) => {
            return match page_problem_of(&e) {
                Some(p) => Ok((None, Some(p), Vec::new())),
                None => Err(e),
            }
        }
    };
    let update = work.join("update.json");
    write_update_json(&updates, p_ctx.doc().max_id, &update)?;
    let staged = work.join("preview.pdf");
    if let Err(e) = apply_update(engines, extracted, &update, &staged, source_benign, opts) {
        return match page_problem_of(&e) {
            Some(p) => Ok((None, Some(p), Vec::new())),
            None => Err(e),
        };
    }
    let replaced: HashMap<lopdf::ObjectId, Vec<u8>> = updates
        .into_iter()
        .map(|u| (u.object_id, u.decoded))
        .collect();
    let before = match graph_digest(p_ctx.doc(), &replaced, cancel_flag(opts)) {
        Ok(d) => d,
        Err(m) => {
            let detail = format!("phase=A check=A2 path={} what={}", m.path, m.what);
            return Ok((
                None,
                Some(EditProblem::new(
                    crate::pdf_engine::text_edit::reasons::EditProblemCode::EditVerifyFailed,
                    Some(detail),
                )),
                Vec::new(),
            ));
        }
    };
    let input = PhaseAInput {
        before: &before,
        before_page_count: 1,
        staged: &staged,
        staged_cap: cap,
        pages: vec![EditedPageInput {
            model: (&p_model).into(),
            plan,
            input_page_index: 0,
            input_render: PopplerRef {
                pdf: extracted.to_path_buf(),
                page_1: 1,
            },
        }],
        source_benign,
    };
    let report = match verify_edited_copy(&input, engines, work, opts) {
        Ok(r) => r,
        Err(e) => {
            return match page_problem_of(&e) {
                Some(p) => Ok((None, Some(p), Vec::new())),
                None => Err(e),
            }
        }
    };
    let mut warnings: Vec<TextWarning> = report
        .warnings
        .into_iter()
        .map(|w| TextWarning { page_index, ..w })
        .collect();
    match read_capped(&staged, PREVIEW_PDF_MAX_BYTES)? {
        Some(bytes) => Ok((Some(bytes), None, warnings)),
        None => {
            warnings.push(unavailable(page_index, "the preview is too large"));
            Ok((None, None, warnings))
        }
    }
}

/// `<temp root>/textedit/<fingerprint>/`.
pub fn cache_dir_for(temp_root: &Path, fingerprint: &str) -> PathBuf {
    temp_root.join("textedit").join(fingerprint)
}
