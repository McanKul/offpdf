//! The source-edit publish gate (SPEC §B.16). Phase A proves each file qpdf wrote from an edit
//! plan (a Save's edited copy, or a preview page): A0 `qpdf --check` no worse than the source,
//! A1 lopdf and qpdf agree on the page map, A2 the whole object graph equals qpdf's input except
//! the edited content streams (`graph.rs`), A3 the edited page's parts are exactly the expected
//! bytes with the old content nowhere, A4 the re-walk (`verify.rs`), A5 Poppler's words and pixels
//! (`poppler.rs`). Phase B proves the final file after every later pass: the expected content is
//! present once, either as parts or inside qpdf's overlay wrapper Form (B1), the old content is
//! absent (B2), and the page's show records are exactly the proven ones (B3). Cover-and-overlay
//! fakes fail both phases (§B.16.2). No setting can skip a check (`seams` exist only in tests).

mod final_output;
pub(crate) mod join;
pub(crate) mod originals;

use crate::error::AppError;
use crate::pdf_engine::text_edit::apply::{benign_warnings, warnings_known};
use crate::pdf_engine::text_edit::content::{page_content, PageContent};
use crate::pdf_engine::text_edit::context::{Lookup, Res, SnapshotContext};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::engines::{qpdf_page_map, run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::fonts::Code;
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::graph::{graph_matches, GraphDigest};
use crate::pdf_engine::text_edit::limits::{
    FORM_DEPTH_MAX, GATE_DECODED_TOTAL, HEAD_TAIL_HASH_BYTES, PAGE_DECODE_BUDGET,
    STREAM_MAX_DECODED,
};
use crate::pdf_engine::text_edit::poppler::{
    check_independent, pdftotext_words, render_dpi, render_page, NearGlyphs, Raster, Word,
    RENDER_DPI_MIN,
};
use crate::pdf_engine::text_edit::reasons::{
    source_edit_gate_failed, EditProblem, EditProblemCode, ProblemCtx, TextWarningCode,
};
use crate::pdf_engine::text_edit::rewrite::PagePlan;
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::snapshot::{
    check_page_map, fnv1a_extend, fnv1a_u64, read_verification_snapshot,
};
use crate::pdf_engine::text_edit::state::StateDigest;
use crate::pdf_engine::text_edit::verify::walk_and_verify_with;
use crate::pdf_engine::text_edit::walker::{walk_page, PageWalk, ShowOp, ShowRecord, WalkMode};
use crate::pdf_engine::validate_output::{content_digest, ContentDigest};
use lopdf::{Object, ObjectId};
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub use originals::OriginalPart;

/// The qpdf input file and page Poppler reads for the "before" side of A5.
#[derive(Debug, Clone)]
pub struct PopplerRef {
    pub pdf: PathBuf,
    pub page_1: u32,
}

/// The model (walk + runs) of an edited page as qpdf's input has it: kept by the caller, or built
/// from the source context when Phase A reaches the page — a Save with large pages keeps the
/// source context instead of every page's model (review-T4 M-1). A model built again must have
/// the planned model's `print` (`STALE` otherwise, review-final LOW-5).
pub enum BeforeModel<'a> {
    Kept(&'a PageModel),
    Build {
        ctx: &'a SnapshotContext,
        page_index: u32,
        print: &'a ModelPrint,
    },
}

/// What identifies a page model: its page, content parts, size, records and runs (ids, texts).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelPrint {
    page_index: u32,
    parts: Vec<ContentDigest>,
    model_bytes: usize,
    records: usize,
    runs: u64,
}

impl ModelPrint {
    pub fn of(model: &PageModel) -> ModelPrint {
        let runs = model.runs.iter().fold(fnv1a_u64(b""), |h, r| {
            let h = fnv1a_extend(h, r.id.as_bytes());
            fnv1a_extend(fnv1a_extend(h, &[0]), r.text.as_bytes())
        });
        ModelPrint {
            page_index: model.page_index,
            parts: model.content.parts.iter().map(|p| p.digest).collect(),
            model_bytes: model.walk.model_bytes,
            records: model.walk.records.len(),
            runs,
        }
    }
}

impl<'a> From<&'a PageModel> for BeforeModel<'a> {
    fn from(model: &'a PageModel) -> BeforeModel<'a> {
        BeforeModel::Kept(model)
    }
}

pub struct EditedPageInput<'a> {
    pub model: BeforeModel<'a>,
    pub plan: &'a PagePlan,
    /// Page index in qpdf's input (= in its output: the update never moves pages).
    pub input_page_index: u32,
    pub input_render: PopplerRef,
}

pub struct PhaseAInput<'a> {
    /// Digest of qpdf's input (the source copy, or `p<n>.pdf` for a preview).
    pub before: &'a GraphDigest,
    pub before_page_count: u32,
    /// qpdf's output.
    pub staged: &'a Path,
    /// `read_verification_snapshot` cap (D31).
    pub staged_cap: u64,
    pub pages: Vec<EditedPageInput<'a>>,
    pub source_benign: &'a [String],
}

/// A depth-0 show record, id-free.
#[derive(Debug, Clone)]
pub struct RecordPrint {
    pub op: ShowOp,
    pub font_res: Option<Vec<u8>>,
    pub font_hash: u64,
    pub codes: Vec<Code>,
    pub text: String,
    pub origins: Vec<(f64, f64)>,
    pub pen_after: (f64, f64),
    /// Shared with the walk's records (one allocation per distinct state, not per record).
    pub state: Arc<StateDigest>,
}

impl RecordPrint {
    pub fn of(rec: &ShowRecord) -> RecordPrint {
        let font = rec.before.text.font.as_ref();
        RecordPrint {
            op: rec.op,
            font_res: font.and_then(|f| f.resource.as_deref()).map(<[u8]>::to_vec),
            font_hash: font.map_or(0, |f| f.content_hash),
            codes: rec.glyphs.iter().map(|g| g.code).collect(),
            text: rec
                .glyphs
                .iter()
                .map(|g| g.text.as_deref().unwrap_or("\u{fffd}"))
                .collect(),
            origins: rec.glyphs.iter().map(|g| g.origin).collect(),
            pen_after: rec.pen_after,
            state: Arc::clone(&rec.before),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageProof {
    pub source_page_index: u32,
    /// Every part of the edited page, in order, after the edit.
    pub expected_parts: Vec<Vec<u8>>,
    /// Digests of `expected_parts` (B1 compares them before the bytes).
    pub part_digests: Vec<ContentDigest>,
    pub original_edited_digests: Vec<ContentDigest>,
    /// Rolling hashes of the same parts (B2 finds them inside a wrapper Form's data).
    pub original_edited_rolling: Vec<u64>,
    /// Concatenation of the expected parts without separator (#34 semantics).
    pub expected_page_digest: ContentDigest,
    /// Depth-0 show records of the edited page after the edit.
    pub records: Vec<RecordPrint>,
}

impl PageProof {
    /// The original edited parts as B2 searches for them.
    pub fn originals(&self) -> Vec<OriginalPart> {
        self.original_edited_digests
            .iter()
            .zip(&self.original_edited_rolling)
            .map(|(digest, rolling)| OriginalPart {
                digest: *digest,
                rolling: *rolling,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextWarning {
    pub page_index: u32,
    pub run_id: String,
    pub code: TextWarningCode,
    pub detail: Option<String>,
}

pub struct PhaseAReport {
    pub proofs: Vec<PageProof>,
    pub warnings: Vec<TextWarning>,
}

/// An `AppError` of Phase A: `code` with details `phase=A check=<id> page=<n> …`.
fn phase_a_error(code: EditProblemCode, check: &str, page: Option<u32>, detail: &str) -> AppError {
    let page_part = page.map_or(String::new(), |n| format!(" page={n}"));
    let p = EditProblem::new(
        code,
        Some(format!("phase=A check={check}{page_part} {detail}")),
    );
    let ctx = ProblemCtx {
        page_number: page,
        file_name: None,
        face: None,
        reason: None,
    };
    code.to_app_error(&p, &ctx)
}

fn a_fail(check: &str, page: Option<u32>, detail: &str) -> AppError {
    phase_a_error(EditProblemCode::EditVerifyFailed, check, page, detail)
}

/// Wraps an engine error of a check into `EDIT_VERIFY_FAILED` (cancel and missing tools pass).
fn engine_error(e: AppError, check: &str, page: Option<u32>) -> AppError {
    match e.code.as_str() {
        "CANCELLED" | "ENGINE_MISSING" | "VERIFIER_MISSING" => e,
        _ => a_fail(
            check,
            page,
            &format!("{}: {}", e.code, e.details.unwrap_or(e.message)),
        ),
    }
}

/// A0: `qpdf --check` exit 0, or exit 3 whose warnings the source already had.
fn check_a0(
    engines: &Engines,
    staged: &Path,
    source_benign: &[String],
    opts: &RunOpts<'_>,
) -> Result<(), AppError> {
    let args = [OsString::from("--check"), staged.as_os_str().to_os_string()];
    let out =
        run_tool(&engines.qpdf, &args, false, opts).map_err(|e| engine_error(e, "A0", None))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    match out.code {
        0 => Ok(()),
        3 => match benign_warnings(&out.stdout, &out.stderr) {
            Some(lines) if warnings_known(&lines, source_benign) => Ok(()),
            _ => Err(a_fail(
                "A0",
                None,
                &format!("qpdf --check: {} {}", out.stderr.trim(), stdout.trim()),
            )),
        },
        code => Err(a_fail(
            "A0",
            None,
            &format!(
                "qpdf --check exited with code {code}: {}",
                out.stderr.trim()
            ),
        )),
    }
}

/// The decoded data of every Form XObject reachable from the page's resources (depth ≤ 8).
pub(crate) fn reachable_forms(
    ctx: &SnapshotContext,
    page_id: ObjectId,
    budget: &mut DecodeBudget,
) -> Result<Vec<Vec<u8>>, String> {
    let doc = ctx.doc();
    let root = Res::of_page(doc, page_id).map_err(str::to_string)?;
    let mut out = Vec::new();
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut queue: VecDeque<(Res<'_>, usize)> = VecDeque::from([(root, 0)]);
    while let Some((res, depth)) = queue.pop_front() {
        let Some(dict) = res.dict else { continue };
        let Some((_, Object::Dictionary(xobjects))) = dict
            .get(b"XObject")
            .ok()
            .and_then(|x| crate::pdf_engine::text_edit::context::resolve(doc, x))
        else {
            continue;
        };
        for (name, _) in xobjects.iter() {
            let Lookup::Found(e) = res.entry(doc, b"XObject", name) else {
                continue;
            };
            let (Some(id), Object::Stream(s)) = (e.id, e.value) else {
                continue;
            };
            let is_form =
                s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Form");
            if !is_form || !seen.insert(id) {
                continue;
            }
            let data = decode_stream(s, STREAM_MAX_DECODED, budget).map_err(|e| e.to_string())?;
            out.push(data);
            if depth + 1 < FORM_DEPTH_MAX {
                let sub = match Res::of_form(doc, id, s) {
                    Some(Ok(r)) => r,
                    Some(Err(w)) => return Err(w.to_string()),
                    None => continue,
                };
                queue.push_back((sub, depth + 1));
            }
        }
    }
    Ok(out)
}

/// A3: the original edited parts are not attached anywhere on the page (as parts, or as Form
/// XObjects reachable from its resources), the part count is unchanged and every part is exactly
/// the expected bytes. Every finding is reported together.
fn check_a3(
    ctx: &SnapshotContext,
    page_id: ObjectId,
    content: &PageContent,
    plan: &PagePlan,
    originals: &[OriginalPart],
    page: u32,
) -> Result<(), AppError> {
    let mut problems: Vec<String> = Vec::new();
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let too_large = |e: &str| a_fail("A3", Some(page), &format!("too large to verify: {e}"));
    let forms = reachable_forms(ctx, page_id, &mut budget).map_err(|e| too_large(&e))?;
    let mut attached = content
        .parts
        .iter()
        .any(|p| originals.iter().any(|o| o.digest == p.digest));
    for f in &forms {
        attached = attached || originals::holds_original(f, originals).map_err(too_large)?;
    }
    if attached {
        problems.push("the original content is still attached".to_string());
    }
    if content.parts.len() != plan.expected_parts.len() {
        problems.push(format!(
            "part count {} (expected {})",
            content.parts.len(),
            plan.expected_parts.len()
        ));
    } else if let Some(i) = (0..content.parts.len())
        .find(|i| plan.expected_parts.get(*i).map(Vec::as_slice) != Some(content.part_bytes(*i)))
    {
        problems.push(format!("part {i} bytes differ"));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(a_fail("A3", Some(page), &problems.join("; ")))
    }
}

/// The original edited parts that no part of the expected page equals (a page may hold two
/// identical parts; the unedited twin is not "the original content").
pub(crate) fn original_parts(model: &PageModel, plan: &PagePlan) -> Vec<OriginalPart> {
    let expected: Vec<ContentDigest> = plan
        .expected_parts
        .iter()
        .map(|p| content_digest(p))
        .collect();
    plan.edited_parts
        .iter()
        .filter(|i| **i < model.content.parts.len())
        .map(|i| OriginalPart::of(model.content.part_bytes(*i)))
        .filter(|o| !expected.contains(&o.digest))
        .collect()
}

/// Cached "before" words and render of a qpdf input page (the preview re-renders the same
/// `p<n>.pdf` for every commit). Keyed by path, length, mtime, the FNV of the file's first and last
/// `HEAD_TAIL_HASH_BYTES` (a same-size rewrite under a coarse mtime is a miss), page and DPI.
type BeforeKey = (PathBuf, u64, Option<std::time::SystemTime>, u64, u32, u32);
type BeforeValue = Arc<(Vec<Word>, Raster)>;
static BEFORE_CACHE: Mutex<VecDeque<(BeforeKey, BeforeValue)>> = Mutex::new(VecDeque::new());
const BEFORE_CACHE_MAX: usize = 4;
/// Raster bytes the cache may hold in total.
const BEFORE_CACHE_BYTES_MAX: usize = 32 << 20;

/// FNV of the first and last `HEAD_TAIL_HASH_BYTES` of `path` (`None` when unreadable).
fn head_tail_hash(path: &Path, len: u64) -> Option<u64> {
    use std::io::{Seek, SeekFrom};
    let n = HEAD_TAIL_HASH_BYTES as u64;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = Vec::new();
    (&mut file).take(n).read_to_end(&mut head).ok()?;
    file.seek(SeekFrom::Start(len.saturating_sub(n))).ok()?;
    let mut tail = Vec::new();
    file.take(n).read_to_end(&mut tail).ok()?;
    Some(fnv1a_extend(fnv1a_u64(&head), &tail))
}

fn before_inputs(
    engines: &Engines,
    r: &PopplerRef,
    dpi: u32,
    work: &Path,
    opts: &RunOpts<'_>,
) -> Result<BeforeValue, AppError> {
    let meta = std::fs::metadata(&r.pdf).ok();
    let len = meta.as_ref().map_or(0, std::fs::Metadata::len);
    let key = head_tail_hash(&r.pdf, len).map(|hash| {
        let modified = meta.and_then(|m| m.modified().ok());
        (r.pdf.clone(), len, modified, hash, r.page_1, dpi)
    });
    let lock = || BEFORE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = &key {
        let hit = lock()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| Arc::clone(v));
        if let Some(hit) = hit {
            return Ok(hit);
        }
    }
    let words = pdftotext_words(engines, &r.pdf, r.page_1, opts)?;
    let raster = render_page(engines, &r.pdf, r.page_1, dpi, work, opts)?;
    let value: BeforeValue = Arc::new((words, raster));
    if let (Some(key), true) = (key, value.1.rgb.len() <= BEFORE_CACHE_BYTES_MAX) {
        let mut cache = lock();
        cache.retain(|(k, _)| *k != key);
        cache.push_front((key, Arc::clone(&value)));
        let mut total = 0usize;
        let keep = cache
            .iter()
            .take_while(|(_, v)| {
                total = total.saturating_add(v.1.rgb.len());
                total <= BEFORE_CACHE_BYTES_MAX
            })
            .count()
            .min(BEFORE_CACHE_MAX);
        cache.truncate(keep);
    }
    Ok(value)
}

/// A5 on one page: returns the `EDIT_NOT_VISIBLE` warnings.
#[allow(clippy::too_many_arguments)]
fn check_a5(
    engines: &Engines,
    page_in: &EditedPageInput<'_>,
    model: &PageModel,
    staged: &Path,
    after_geom: &PageGeometry,
    work: &Path,
    opts: &RunOpts<'_>,
    page: u32,
) -> Result<Vec<TextWarning>, AppError> {
    let Some(dpi) = render_dpi(after_geom) else {
        return Err(a_fail(
            "A5",
            Some(page),
            &format!(
                "check=render page={page} too large to verify: the page renders below \
                 {RENDER_DPI_MIN} DPI"
            ),
        ));
    };
    let before = before_inputs(engines, &page_in.input_render, dpi, work, opts)
        .map_err(|e| engine_error(e, "A5", Some(page)))?;
    let page_1 = page_in.input_page_index.saturating_add(1);
    let dst_words = pdftotext_words(engines, staged, page_1, opts)
        .map_err(|e| engine_error(e, "A5", Some(page)))?;
    let dst_raster = render_page(engines, staged, page_1, dpi, work, opts)
        .map_err(|e| engine_error(e, "A5", Some(page)))?;
    let (mut edits, mut olds) = (Vec::new(), Vec::new());
    for run in &page_in.plan.runs {
        let old = model
            .runs
            .iter()
            .find(|r| {
                r.members
                    .iter()
                    .filter_map(|m| model.walk.records.get(*m))
                    .filter_map(|x| x.span.clone())
                    .eq(run.expected.member_spans.iter().cloned())
            })
            .ok_or_else(|| a_fail("A5", Some(page), "edited run not on the page"))?;
        let r = old.rect;
        let n = run.new_rect;
        edits.push((
            run.expected.clone(),
            [r[0], r[1], r[0] + r[2], r[1] + r[3]],
            [n[0], n[1], n[0] + n[2], n[1] + n[3]],
            old.text.clone(),
        ));
        olds.push(old);
    }
    let near = NearGlyphs::of(model, &olds, &edits, after_geom, dpi);
    let report = check_independent(
        &before.0,
        &dst_words,
        &before.1,
        &dst_raster,
        after_geom,
        &edits,
        &near,
        dpi,
    )
    .map_err(|(check, detail)| {
        a_fail(
            "A5",
            Some(page),
            &format!("check={check} page={page} {detail}"),
        )
    })?;
    Ok(report
        .edit_visible
        .iter()
        .zip(&page_in.plan.runs)
        .filter(|(v, _)| !**v)
        .map(|(_, run)| TextWarning {
            page_index: model.page_index,
            run_id: run.run_id.clone(),
            code: TextWarningCode::EditNotVisible,
            detail: Some("no pixel of the edited line changed".to_string()),
        })
        .collect())
}

/// The cancel flag the in-process checks (graph traversal, walks) poll: the caller's flag, else
/// the job handle's.
pub(crate) fn cancel_flag<'a>(opts: &RunOpts<'a>) -> Option<&'a AtomicBool> {
    opts.cancel.or_else(|| opts.handle.map(|h| &h.cancelled))
}

pub(crate) fn is_cancelled(opts: &RunOpts<'_>) -> bool {
    opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst))
        || opts.handle.is_some_and(|h| h.is_cancelled())
}

/// A check that failed while the job was being cancelled failed because of the cancel (a walk
/// stopped early, a traversal gave up): the result is `CANCELLED`, never a verification code.
fn or_cancelled<T>(r: Result<T, AppError>, opts: &RunOpts<'_>) -> Result<T, AppError> {
    match r {
        Err(_) if is_cancelled(opts) => Err(AppError::cancelled()),
        other => other,
    }
}

/// Phase A on one edited copy (or preview file). The caller deletes its work files on failure.
/// A failure while the job is cancelled is `CANCELLED`.
pub fn verify_edited_copy(
    input: &PhaseAInput<'_>,
    engines: &Engines,
    work: &Path,
    opts: &RunOpts<'_>,
) -> Result<PhaseAReport, AppError> {
    or_cancelled(phase_a(input, engines, work, opts), opts)
}

fn phase_a(
    input: &PhaseAInput<'_>,
    engines: &Engines,
    work: &Path,
    opts: &RunOpts<'_>,
) -> Result<PhaseAReport, AppError> {
    if !seams::skipped("A0") {
        check_a0(engines, input.staged, input.source_benign, opts)?;
    }
    let mut snap = read_verification_snapshot(input.staged, input.staged_cap).map_err(|e| {
        a_fail(
            "A1",
            None,
            &format!("read: {}", e.details.unwrap_or(e.message)),
        )
    })?;
    if !seams::skipped("A1") {
        let pages =
            qpdf_page_map(engines, input.staged, opts).map_err(|e| engine_error(e, "A1", None))?;
        check_page_map(&snap, &pages)
            .map_err(|e| a_fail("A1", None, &e.details.unwrap_or(e.message)))?;
        if snap.pages.len() as u64 != u64::from(input.before_page_count) {
            return Err(a_fail(
                "A1",
                None,
                &format!(
                    "page count {} (expected {})",
                    snap.pages.len(),
                    input.before_page_count
                ),
            ));
        }
    }
    snap.release_bytes();
    let ctx = SnapshotContext::new(snap);
    if !seams::skipped("A2") {
        let mut budget =
            DecodeBudget::new(usize::try_from(GATE_DECODED_TOTAL).unwrap_or(usize::MAX));
        graph_matches(ctx.doc(), input.before, &mut budget, cancel_flag(opts)).map_err(|m| {
            let lead = if m.what == "budget" {
                "too large to verify "
            } else {
                ""
            };
            a_fail(
                "A2",
                None,
                &format!("{lead}path={} what={}", m.path, m.what),
            )
        })?;
    }
    let mut proofs = Vec::new();
    let mut warnings = Vec::new();
    for page_in in &input.pages {
        let (proof, mut w) = verify_page_a(&ctx, page_in, input.staged, engines, work, opts)?;
        proofs.push(proof);
        warnings.append(&mut w);
    }
    Ok(PhaseAReport { proofs, warnings })
}

/// A3–A5 on one page and its proof.
fn verify_page_a(
    ctx: &SnapshotContext,
    page_in: &EditedPageInput<'_>,
    staged: &Path,
    engines: &Engines,
    work: &Path,
    opts: &RunOpts<'_>,
) -> Result<(PageProof, Vec<TextWarning>), AppError> {
    let built;
    let model = match &page_in.model {
        BeforeModel::Kept(m) => *m,
        BeforeModel::Build {
            ctx,
            page_index,
            print,
        } => {
            built = build_page_model(ctx, *page_index, cancel_flag(opts))?;
            if ModelPrint::of(&built) != **print {
                let page = Some(page_index.saturating_add(1));
                let detail = "the source page model built again differs from the planned one";
                return Err(phase_a_error(EditProblemCode::Stale, "A4", page, detail));
            }
            &built
        }
    };
    let page = model.page_index.saturating_add(1);
    let plan = page_in.plan;
    let page_id = ctx
        .page_id(page_in.input_page_index)
        .map_err(|_| a_fail("A1", Some(page), "edited page missing"))?;
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let content = page_content(ctx.doc(), page_id, &mut budget)
        .map_err(|r| a_fail("A3", Some(page), &format!("page content: {}", r.as_str())))?;
    let originals = original_parts(model, plan);
    if !seams::skipped("A3") {
        check_a3(ctx, page_id, &content, plan, &originals, page)?;
    }
    // The proof keeps the after walk's depth-0 records as prints, taken before the probe walk.
    let prints = |walk: PageWalk| -> (PageGeometry, Vec<RecordPrint>) {
        let records = walk.records.iter().filter(|r| r.depth == 0);
        (
            walk.geometry.clone(),
            records.map(RecordPrint::of).collect(),
        )
    };
    let (geometry, records) = if seams::skipped("A4") {
        let walk = walk_page(
            ctx,
            page_in.input_page_index,
            &content,
            WalkMode::Edit,
            cancel_flag(opts),
        );
        prints(walk)
    } else {
        let walks = (&content, &*model.walk, &model.runs[..]);
        let index = page_in.input_page_index;
        walk_and_verify_with(ctx, index, walks, plan, cancel_flag(opts), prints)
            .map_err(|f| phase_a_error(f.problem_code(), "A4", Some(page), &f.to_string()))?
    };
    drop(content);
    let warnings = if seams::skipped("A5") {
        Vec::new()
    } else {
        check_a5(engines, page_in, model, staged, &geometry, work, opts, page)?
    };
    let proof = PageProof {
        source_page_index: model.page_index,
        part_digests: plan
            .expected_parts
            .iter()
            .map(|p| content_digest(p))
            .collect(),
        expected_parts: plan.expected_parts.clone(),
        original_edited_digests: originals.iter().map(|o| o.digest).collect(),
        original_edited_rolling: originals.iter().map(|o| o.rolling).collect(),
        expected_page_digest: plan.expected_page_digest,
        records,
    };
    Ok((proof, warnings))
}

/// Phase B (§B.16.2) on the final staged file, immediately before #34's `validate_staged_pdf`:
/// B0 (bounded read with no policy refusals — D31 — and the page map), then B1–B3 per edited
/// destination page (`gate/final_output.rs`). Any failure → `SOURCE_EDIT_GATE_FAILED`, or
/// `CANCELLED` while the job is being cancelled.
pub fn verify_final_output(
    staged: &Path,
    cap: u64,
    engines: &Engines,
    expectations: &[(u32, &PageProof)],
    opts: &RunOpts<'_>,
) -> Result<(), AppError> {
    or_cancelled(phase_b(staged, cap, engines, expectations, opts), opts)
}

fn phase_b(
    staged: &Path,
    cap: u64,
    engines: &Engines,
    expectations: &[(u32, &PageProof)],
    opts: &RunOpts<'_>,
) -> Result<(), AppError> {
    let fail = |detail: &str| source_edit_gate_failed(&format!("phase=B {detail}"));
    let detail = |e: AppError| e.details.unwrap_or(e.message);
    let mut snap = read_verification_snapshot(staged, cap)
        .map_err(|e| fail(&format!("check=B0 read: {}", detail(e))))?;
    let pages = qpdf_page_map(engines, staged, opts).map_err(|e| match e.code.as_str() {
        "CANCELLED" | "ENGINE_MISSING" => e,
        _ => fail(&format!("check=B0 {}", detail(e))),
    })?;
    check_page_map(&snap, &pages).map_err(|e| fail(&format!("check=B0 {}", detail(e))))?;
    snap.release_bytes();
    let ctx = SnapshotContext::new(snap);
    for (dest, proof) in expectations {
        final_output::check_page(&ctx, *dest, proof, cancel_flag(opts))
            .map_err(|e| fail(&format!("page={} {e}", dest.saturating_add(1))))?;
    }
    Ok(())
}

/// Test seams: skip named gate checks on this thread (IND-02/03/04, the GATE "must fail at"
/// matrix). Production builds have no way to skip a check.
mod seams {
    pub(super) fn skipped(check: &str) -> bool {
        #[cfg(test)]
        {
            if super::test_seams::skipped(check) {
                return true;
            }
        }
        let _ = check;
        false
    }
}

#[cfg(test)]
pub(crate) mod test_seams {
    use std::cell::RefCell;

    thread_local! {
        static SKIPPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn skipped(check: &str) -> bool {
        SKIPPED.with(|s| s.borrow().iter().any(|c| c == check))
    }

    /// Skips `checks` (e.g. `["A2", "A3"]`, `["B1"]`) on this thread until the guard drops.
    pub(crate) fn skip(checks: &[&str]) -> SkipGuard {
        SKIPPED.with(|s| *s.borrow_mut() = checks.iter().map(|c| c.to_string()).collect());
        SkipGuard
    }

    pub(crate) struct SkipGuard;

    impl Drop for SkipGuard {
        fn drop(&mut self) {
            SKIPPED.with(|s| s.borrow_mut().clear());
        }
    }
}
