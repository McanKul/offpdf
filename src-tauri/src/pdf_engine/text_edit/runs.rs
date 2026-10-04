//! Runs (SPEC §B.11): the editable or refused lines of a page. Every depth-0 show record gets its
//! own reasons (`runs/reasons.rs`); consecutive records join into one visual line (§A.1.1: same
//! baseline and direction, the full state equal except the font identity, identical or sibling
//! fonts, a gap within ±0.3 em, nothing painted between, the same first reason); each run gets its
//! units, synthetic spaces, text, caret offsets, ink box and extents (`runs/assemble.rs`), the
//! page-wide checks and a reading order (`order.rs`). Deterministic; page problems are data
//! (`page_reason`), never an `Err`.
//!
//! The run stage goes on charging the walk's page-model budget (`walker::budget`): its scratch
//! (reasons per record, groups, surfaces, the page-wide checks and reading order) and every run it
//! keeps, before each is made. Past the budget the page is `PAGE_TOO_COMPLEX` "page model size".

mod assemble;
mod forms;
pub(crate) mod reasons;
mod surface;

use crate::error::AppError;
use crate::pdf_engine::text_edit::content::{page_content, PageContent};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::fonts::{Code, TypingSurface};
use crate::pdf_engine::text_edit::geometry::{
    apply, cross, display_rotation, dot, sub, transform_rect, unit,
};
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::{
    JOIN_BASELINE_TOL_PT, JOIN_GAP_EM, PAGE_DECODE_BUDGET, RUNS_PER_PAGE_MAX, RUN_MEMBERS_MAX,
    STATE_EPSILON, XY_CUT_DEPTH_MAX,
};
use crate::pdf_engine::text_edit::order::{reading_order, RunBox};
use crate::pdf_engine::text_edit::reasons::{Face, TextReason};
use crate::pdf_engine::text_edit::snapshot::Fingerprint;
use crate::pdf_engine::text_edit::state::{same_state_except, StateField};
use crate::pdf_engine::text_edit::walker::budget::{content_bytes, ModelBudget};
use crate::pdf_engine::text_edit::walker::{
    refused_walk, walk_page, PageWalk, ShowRecord, Stop, WalkMode,
};
use reasons::{innermost_mcid, ReasonCtx};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use surface::Surfaces;

pub(crate) use forms::form_lines;
pub use surface::surface_of;

/// Scratch per run for the page-wide steps (duplicates, obstacles, run boxes and reading order:
/// a few indices and boxes each, plus one index per XY-cut level on the recursion path).
const RUN_SCRATCH_BYTES: usize = 256 + XY_CUT_DEPTH_MAX * 2 * size_of::<usize>();

#[derive(Debug, Clone)]
pub enum Unit {
    Glyph {
        member: usize,
        rec: usize,
        glyph: usize,
        code: Code,
        font_res: Option<Vec<u8>>,
        font_hash: u64,
        text: String,
        width1000: f64,
    },
    Kern {
        value: f64,
        src: KernSrc,
        synth_space: bool,
    },
}

/// `Tj` = an original TJ number (its bytes are kept verbatim); `Gap` = a converted gap between
/// two joined members.
#[derive(Debug, Clone)]
pub enum KernSrc {
    Tj { span: Span },
    Gap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpaceMode {
    Glyph,
    Kern,
}

#[derive(Debug, Clone)]
pub struct TextRun {
    /// `t1:{fp}:{page}:{s0}-{e0}[,{s1}-{e1}...]` (joined-buffer spans of the members).
    pub id: String,
    /// Reading order in display space and the line cluster it belongs to.
    pub order: u32,
    pub line: u32,
    /// Record indices; `members[0]` is the primary.
    pub members: Vec<usize>,
    pub units: Vec<Unit>,
    pub text: String,
    /// chars + 1 user-space distances along `dir` from `origin`.
    pub caret_offsets: Vec<f64>,
    pub origin: (f64, f64),
    pub dir: (f64, f64),
    pub up: (f64, f64),
    /// User units from the baseline (rise included in the origin); `descent ≥ 0`.
    pub ascent: f64,
    pub descent: f64,
    /// `[x, y, w, h]` ink AABB, unrotated user space.
    pub rect: [f64; 4],
    /// Font resources, primary first (empty for an ExtGState font, whose surface is that font).
    /// Shared by every run with the same surface (one list per page and surface key).
    pub surface: Arc<[Vec<u8>]>,
    pub tfs: f64,
    pub effective_size: f64,
    pub tc: f64,
    pub tw: f64,
    pub th: f64,
    /// `|row 1 of Tm × CTM|`: user units per unscaled text-space unit along the baseline.
    pub text_to_user_x: f64,
    pub space_mode: SpaceMode,
    pub kern_space: f64,
    /// Ink extent along `dir` from the origin.
    pub original_extent: f64,
    /// Distance to the edge of (visible box ∩ clip rectangle) along `dir`.
    pub visible_extent: f64,
    /// Distance to the next run on the same line (band overlap > 10 %).
    pub next_obstacle: Option<f64>,
    /// `#rrggbb` when readable.
    pub fill_hex: Option<String>,
    pub face: Face,
    pub font_from_extgstate: bool,
    pub tr: i64,
    pub substituted: bool,
    /// `reasons[0]`.
    pub reason: Option<TextReason>,
    pub reasons: Vec<TextReason>,
}

pub struct PageModel {
    pub fingerprint: Fingerprint,
    pub page_index: u32,
    pub content: Arc<PageContent>,
    pub walk: Arc<PageWalk>,
    /// Non-blank runs, in reading order.
    pub runs: Vec<TextRun>,
    pub page_reason: Option<TextReason>,
    pub page_detail: Option<String>,
    /// Per walk record: the reason of the run it belongs to (blank runs included), or its own
    /// first reason when it draws no glyph.
    pub record_reason: Vec<Option<TextReason>>,
}

impl PageModel {
    pub fn run(&self, id: &str) -> Option<&TextRun> {
        self.runs.iter().find(|r| r.id == id)
    }

    /// The typing surface of `run`: its ExtGState font alone, or the primary's sibling group —
    /// the page fonts `run.surface` names, in its order (it holds no name for an ExtGState font).
    pub fn surface(&self, run: &TextRun) -> TypingSurface {
        let Some(primary) = run.members.first().and_then(|i| self.walk.records.get(*i)) else {
            return TypingSurface { fonts: Vec::new() };
        };
        if run.surface.is_empty() {
            return surface_of(&self.walk, primary);
        }
        let page_fonts = &self.walk.page_fonts;
        let fonts = run
            .surface
            .iter()
            .filter_map(|name| page_fonts.iter().find(|(n, _)| n == name).cloned())
            .collect();
        TypingSurface { fonts }
    }
}

/// The model of page `page_index` (`INVALID_PAGES` when there is none, `CANCELLED` when `cancel`
/// was set). Page problems are data: `page_reason` with no runs.
pub fn build_page_model(
    ctx: &SnapshotContext,
    page_index: u32,
    cancel: Option<&AtomicBool>,
) -> Result<PageModel, AppError> {
    let page_id = ctx.page_id(page_index)?;
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let (content, walk) = match page_content(ctx.doc(), page_id, &mut budget) {
        Ok(content) => {
            let walk = walk_page(ctx, page_index, &content, WalkMode::Edit, cancel);
            (content, walk)
        }
        Err(reason) => {
            let empty = PageContent {
                page_id,
                parts: Vec::new(),
                joined: Vec::new(),
                contents_array: None,
            };
            let walk = refused_walk(ctx, page_id, reason, "page content");
            (empty, walk)
        }
    };
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(AppError::cancelled());
    }
    Ok(model_of(ctx, page_index, content, walk))
}

/// The model of a page already walked in `Edit` mode (also used to re-model edited content in the
/// same context). A model keeps no lexed ops (nothing reads them once the walk is done; they were
/// the largest part of a dense page, review T3-budget MEDIUM-3), and a page refused after its
/// walk keeps no walk either (MEDIUM-1): only its content, geometry and reason.
pub fn model_of(
    ctx: &SnapshotContext,
    page_index: u32,
    content: PageContent,
    mut walk: PageWalk,
) -> PageModel {
    walk.drop_ops();
    let out = build_runs(ctx, page_index, &content, &walk);
    if out.release_walk {
        walk.release();
    }
    // From here on the walk's charge is the whole model's (`PageWalk::model_bytes`).
    walk.model_bytes = out.held;
    PageModel {
        fingerprint: ctx.snap.fingerprint,
        page_index,
        content: Arc::new(content),
        walk: Arc::new(walk),
        runs: out.runs,
        page_reason: out.page_reason,
        page_detail: out.page_detail,
        record_reason: out.record_reason,
    }
}

struct RunsOut {
    held: usize,
    runs: Vec<TextRun>,
    page_reason: Option<TextReason>,
    page_detail: Option<String>,
    record_reason: Vec<Option<TextReason>>,
    /// The page was refused after its walk: the model keeps no walk.
    release_walk: bool,
}

impl RunsOut {
    /// A refused page: it keeps its content (`content_bytes`), no runs and no per-record reasons
    /// (its walk holds no record: refused at page level, or released by `model_of`).
    fn refused(content_bytes: usize, reason: TextReason, detail: String) -> RunsOut {
        RunsOut {
            held: content_bytes,
            runs: Vec::new(),
            page_reason: Some(reason),
            page_detail: Some(detail),
            record_reason: Vec::new(),
            release_walk: true,
        }
    }
}

fn build_runs(
    ctx: &SnapshotContext,
    page_index: u32,
    content: &PageContent,
    walk: &PageWalk,
) -> RunsOut {
    // A refused page's model keeps only its content (a walk refused at page level kept nothing
    // else; one refused later is released by `model_of`).
    let kept = content_bytes(content);
    if let Some(reason) = walk.page_reason {
        let detail = walk.page_detail.clone().unwrap_or_default();
        return RunsOut::refused(kept, reason, detail);
    }
    if let Err(reason) = ctx.kids() {
        return RunsOut::refused(kept, reason, "page tree".into());
    }
    let mut mem = ModelBudget::new(walk.model_bytes);
    runs_within(ctx, page_index, content, walk, &mut mem)
        .unwrap_or_else(|stop| RunsOut::refused(kept, stop.reason, stop.detail))
}

/// The run stage under the page-model budget `mem`.
fn runs_within(
    ctx: &SnapshotContext,
    page_index: u32,
    content: &PageContent,
    walk: &PageWalk,
    mem: &mut ModelBudget,
) -> Result<RunsOut, Stop> {
    let n = walk.records.len();
    let mut rc = ReasonCtx::new(ctx, content, walk);
    mem.scratch(n.saturating_mul(size_of::<Vec<TextReason>>()))?;
    let mut intrinsic: Vec<Vec<TextReason>> = Vec::with_capacity(n);
    for r in &walk.records {
        let reasons = rc.record_reasons(r);
        mem.scratch(reasons.capacity())?;
        intrinsic.push(reasons);
    }
    let mut surfaces = Surfaces::new(walk, mem)?;
    let groups = join_records(walk, &intrinsic, &mut surfaces, mem)?;
    if groups.len() > RUNS_PER_PAGE_MAX {
        return Err(Stop::new(TextReason::PageTooComplex, "runs per page"));
    }
    mem.hold(groups.len().saturating_mul(size_of::<TextRun>()))?;
    let mut all: Vec<TextRun> = Vec::with_capacity(groups.len());
    for g in &groups {
        let mut reasons: Vec<TextReason> = g
            .iter()
            .filter_map(|i| intrinsic.get(*i))
            .flatten()
            .copied()
            .collect();
        reasons.sort();
        reasons.dedup();
        reasons.shrink_to_fit();
        let Some(primary) = g.first().and_then(|i| walk.records.get(*i)) else {
            continue;
        };
        // Whether a surface types anything is decided once per surface (a page may hold
        // thousands of runs in one CJK font whose alphabet has tens of thousands of characters).
        let surface = surfaces.of(primary, mem)?;
        let mut run = assemble::assemble(walk, g, reasons, &surface, mem)?;
        if !surface.writable {
            assemble::add_reason(&mut run, TextReason::NoWritableGlyphs);
        }
        all.push(run);
    }
    drop(surfaces);
    mem.scratch(all.len().saturating_mul(RUN_SCRATCH_BYTES))?;
    assemble::mark_duplicates(&mut all);
    assemble::mark_per_glyph(&mut all);
    let fp = ctx.snap.fingerprint.to_string();
    mem.hold(n.saturating_mul(size_of::<Option<TextReason>>()))?;
    let mut record_reason: Vec<Option<TextReason>> =
        intrinsic.iter().map(|r| r.first().copied()).collect();
    drop(intrinsic);
    let all_slots = all.capacity().saturating_mul(size_of::<TextRun>());
    let mut listed = Vec::new();
    for mut run in all {
        run.id = run_id(&fp, page_index, walk, &run.members, mem)?;
        for m in &run.members {
            if let Some(slot) = record_reason.get_mut(*m) {
                *slot = run.reason;
            }
        }
        if !run.text.trim().is_empty() {
            mem.push(&mut listed, run)?;
        }
    }
    mem.unhold(all_slots);
    assemble::next_obstacles(&mut listed);
    let boxes: Vec<RunBox> = listed.iter().map(|r| run_box(walk, r)).collect();
    for (run, (order, line)) in listed
        .iter_mut()
        .zip(reading_order(ctx, walk.page_id, &boxes))
    {
        run.order = order;
        run.line = line;
    }
    listed.sort_by_key(|r| r.order);
    Ok(RunsOut {
        held: mem.held(),
        runs: listed,
        page_reason: None,
        page_detail: None,
        record_reason,
        release_walk: false,
    })
}

/// `t1:{fp}:{page}:{s0}-{e0}[,…]`, charged (an upper bound, then what it holds) before it is
/// built: two 20-digit numbers and two separators per member.
fn run_id(
    fp: &str,
    page_index: u32,
    walk: &PageWalk,
    members: &[usize],
    mem: &mut ModelBudget,
) -> Result<String, Stop> {
    let bound = members
        .len()
        .saturating_mul(42)
        .saturating_add(fp.len() + 16);
    mem.hold(bound)?;
    let spans: Vec<String> = members
        .iter()
        .map(
            |m| match walk.records.get(*m).and_then(|r| r.span.as_ref()) {
                Some(s) => format!("{}-{}", s.start, s.end),
                None => "?".to_string(),
            },
        )
        .collect();
    let id = format!("t1:{fp}:{page_index}:{}", spans.join(","));
    mem.unhold(bound.saturating_sub(id.capacity()));
    Ok(id)
}

/// The run's ink box and baseline in display space (after `/Rotate`, y down).
fn run_box(walk: &PageWalk, run: &TextRun) -> RunBox {
    let r = display_rotation(walk.geometry.rotate);
    let [x, y, w, h] = run.rect;
    let d = transform_rect(&r, [x, y, x + w, y + h]);
    let o = apply(&r, run.origin.0, run.origin.1);
    RunBox {
        display_rect: [d[0], -d[3], d[2], -d[1]],
        baseline_y: -o.1,
        size: run.effective_size,
        mcid: run
            .members
            .first()
            .and_then(|i| walk.records.get(*i))
            .and_then(innermost_mcid),
    }
}

/// Consecutive depth-0 records with glyphs that join (§A.1.1); a glyph-less show op or a depth > 0
/// record ends the line. Sibling groups come from the page's shared surfaces (`Surfaces`, built
/// once per resource: a line may alternate between many fonts).
fn join_records(
    walk: &PageWalk,
    intrinsic: &[Vec<TextReason>],
    surfaces: &mut Surfaces<'_>,
    mem: &mut ModelBudget,
) -> Result<Vec<Vec<usize>>, Stop> {
    // At most one group per record, each member in exactly one group (a group's room may double).
    let n = walk.records.len();
    let groups_bytes = n.saturating_mul(size_of::<Vec<usize>>() + 2 * size_of::<usize>());
    mem.scratch(groups_bytes.saturating_add(walk.paints.len() * size_of::<u32>()))?;
    let paint_seqs: Vec<u32> = walk.paints.iter().map(|p| p.seq).collect();
    let mut groups = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for (i, rec) in walk.records.iter().enumerate() {
        if rec.depth != 0 || rec.glyphs.is_empty() {
            if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let joins = match cur.last() {
            Some(&last) if cur.len() < RUN_MEMBERS_MAX => {
                joinable(walk, intrinsic, &paint_seqs, surfaces, mem, last, i)?
            }
            _ => false,
        };
        if !joins && !cur.is_empty() {
            groups.push(std::mem::take(&mut cur));
        }
        cur.push(i);
    }
    if !cur.is_empty() {
        groups.push(cur);
    }
    Ok(groups)
}

#[allow(clippy::too_many_arguments)]
fn joinable(
    walk: &PageWalk,
    intrinsic: &[Vec<TextReason>],
    paint_seqs: &[u32],
    surfaces: &mut Surfaces<'_>,
    mem: &mut ModelBudget,
    a: usize,
    b: usize,
) -> Result<bool, Stop> {
    let (Some(ra), Some(rb)) = (walk.records.get(a), walk.records.get(b)) else {
        return Ok(false);
    };
    let first = |i: usize| intrinsic.get(i).and_then(|r| r.first().copied());
    let actual = |r: &ShowRecord, i: usize| {
        r.before.marked.iter().any(|m| m.actual_text)
            || intrinsic
                .get(i)
                .is_some_and(|v| v.contains(&TextReason::ActualText))
    };
    if b != a + 1 || first(a) != first(b) || actual(ra, a) || actual(rb, b) {
        return Ok(false);
    }
    let next_paint = paint_seqs.partition_point(|s| *s <= ra.seq);
    if paint_seqs.get(next_paint).is_some_and(|s| *s < rb.seq) {
        return Ok(false);
    }
    if same_state_except(&ra.before, &rb.before, &[StateField::Font]).is_err()
        || !fonts_join(surfaces, mem, ra, rb)?
    {
        return Ok(false);
    }
    let (ta, tb) = (ra.text_to_user, rb.text_to_user);
    let same_linear = ta
        .iter()
        .zip(&tb)
        .take(4)
        .all(|(x, y)| (x - y).abs() <= STATE_EPSILON * 1f64.max(x.abs()).max(y.abs()));
    let Some(dir) = unit((ta[0], ta[1])) else {
        return Ok(false);
    };
    let effective = ta[2].hypot(ta[3]);
    // The gap runs to `b`'s first glyph: kerns that open a TJ count (a TJ that starts with a
    // column-wide kern is the next column, not the same line).
    let b_origin = rb.glyphs.first().map_or(rb.pen_before, |g| g.origin);
    Ok(same_linear
        && cross(sub(rb.pen_before, ra.pen_before), dir).abs() <= JOIN_BASELINE_TOL_PT
        && dot(sub(b_origin, ra.pen_after), dir).abs() <= JOIN_GAP_EM * effective)
}

/// Identical fonts or siblings (§A.1.1); an ExtGState font joins only the same ExtGState font.
fn fonts_join(
    surfaces: &mut Surfaces<'_>,
    mem: &mut ModelBudget,
    ra: &ShowRecord,
    rb: &ShowRecord,
) -> Result<bool, Stop> {
    let (Some(x), Some(y)) = (&ra.before.text.font, &rb.before.text.font) else {
        return Ok(false);
    };
    if x.from_extgstate || y.from_extgstate {
        return Ok(x.from_extgstate
            && y.from_extgstate
            && x.content_hash == y.content_hash
            && ra.font_key == rb.font_key);
    }
    if x.resource == y.resource && x.content_hash == y.content_hash {
        return Ok(true);
    }
    let (Some(_), Some(yr)) = (&x.resource, &y.resource) else {
        return Ok(false);
    };
    surfaces.joins(ra, yr, y.content_hash, mem)
}

/// `PageModel::approx_bytes`: the test oracle the page-model budget is checked against
/// (`approx_bytes ≤ walk.model_bytes`); production sizes models by `walk.model_bytes`.
#[cfg(test)]
mod size;
