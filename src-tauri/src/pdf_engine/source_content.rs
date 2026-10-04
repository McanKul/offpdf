//! offpdf:forbidden-api-scan
//! Hardened read-only classifier (#33): bounded, fail-closed; the text half is wired to Edit
//! text through `inspect_text_page`.
//!
//! A thin adapter over `text_edit` (SPEC §C). The snapshot is one capped read, preflighted and
//! parsed from the bytes it hashed (`snapshot.rs`); the page model (`runs::build_page_model`)
//! brings strict geometry, the bounded lexer and decoders, the font layer, runs and their reason
//! codes; one more walk in `Classify` mode descends Form XObjects and lists image paints. Per page
//! it returns the run capabilities Edit text shows (`runs`) and the #33 occurrences — one per
//! show op and per image paint — with v2 locators:
//!
//! `v2:{fingerprint}:{page}:{t|i}:{path}:{ordinal}`, where `path` is `p{start}-{end}` (the op's
//! span in the page's joined content) or `x{obj}.{gen}>…:{start}-{end}` (the Form chain and the
//! span in the innermost Form's data), and `ordinal` is the occurrence's index on the page.
//!
//! A text occurrence carries the reason of the run that contains its record (Form text:
//! `NESTED_FORM`; a page refused `GEOMETRY`: `GEOMETRY`). Image occurrences use the image
//! priority of §A.10. `Supported` is never a Save affordance: Save re-plans and re-verifies from
//! the file.
//!
//! The Classify pass runs on the page-model budget its model left (`walker::budget`,
//! `PAGE_MODEL_BYTES_MAX`: the model's bytes are charged already, so the model and its Classify
//! pass hold no more than one page budget together, review T3-budget MEDIUM-2): past it the page
//! lists no occurrence (`occurrence_reason` `PAGE_TOO_COMPLEX`), never a partial list. A page that
//! paints no Form XObject is not walked again: its Edit walk is exactly what a Classify walk would
//! record, so the occurrences are built from the model's own walk.

use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::geometry::{contains, display_rotation, mul, Matrix};
use crate::pdf_engine::text_edit::limits::{AXIS_EPSILON_REL, CLIP_CONTAIN_TOL_PT};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::runs::reasons::ReasonCtx;
use crate::pdf_engine::text_edit::runs::{form_lines, PageModel, TextRun};
use crate::pdf_engine::text_edit::state::ClipState;
use crate::pdf_engine::text_edit::walker::budget::{map_entry, ModelBudget, MODEL_SIZE};
use crate::pdf_engine::text_edit::walker::{
    walk_page_within, PageWalk, PaintKind, PaintRecord, ShowRecord, Stop, WalkMode,
};
use lopdf::ObjectId;
use serde::Serialize;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};

/// Bytes of a locator besides its path: `v2:`, the fingerprint, the page, the kind and the
/// ordinal with their separators.
const LOCATOR_FIXED_BYTES: usize = 96;
/// Bytes of one Form in a path (`x{obj}.{gen}>`) and of the span (`:{start}-{end}`).
const PATH_FORM_BYTES: usize = 18;
const PATH_SPAN_BYTES: usize = 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceCapability {
    Supported,
    Unsupported,
}

/// Unrotated user space: text = baseline origin (rise included), `w` = the real advance (Tc, Tw
/// and kerns included), `h` = the effective size; images = the painted unit square's AABB.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceOccurrence {
    pub page_index: u32,
    pub kind: SourceKind,
    pub rect: SourceRect,
    pub locator: String,
    pub capability: SourceCapability,
    pub reason: Option<TextReason>,
    /// The decoded text of a text occurrence (None when a glyph cannot be decoded, or an image).
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRunCapability {
    pub run_id: String,
    pub capability: SourceCapability,
    pub reason: Option<TextReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePageResult {
    pub page_index: u32,
    /// The page-level refusal of Edit text (every run refused with it; runs are not listed).
    pub page_reason: Option<TextReason>,
    /// One per model run, in reading order — drives the run DTO's `editable`/`reason`.
    pub runs: Vec<SourceRunCapability>,
    /// Per show op / image paint (the #33 contract; image rows are not shown in v0.4).
    pub occurrences: Vec<SourceOccurrence>,
    /// The `Classify` walk (Form XObjects included) was refused at page level although Edit
    /// text's own depth-0 walk was not: no occurrence is listed (never a partial list).
    pub occurrence_reason: Option<TextReason>,
    /// Text drawn through Form XObjects, as refused `NESTED_FORM` lines after `runs` in reading
    /// order (§A.6 "still shown"; the page model has no run for it). Empty when the page paints no
    /// Form, is refused, or its Classify pass is.
    pub form_lines: Vec<SourceFormLine>,
}

/// One refused line of Form text, with the geometry a run DTO shows (unrotated user space, as
/// `TextRun`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFormLine {
    /// `t1:{fp}:{page}:x{obj}.{gen}>…:{s0}-{e0}[,…]` (Form chain and spans in its data).
    pub id: String,
    pub order: u32,
    pub line: u32,
    pub text: String,
    /// `[x, y, w, h]` ink AABB.
    pub rect: [f64; 4],
    pub origin: (f64, f64),
    pub dir: (f64, f64),
    pub ascent: f64,
    pub descent: f64,
    /// chars + 1 distances along `dir` from `origin`.
    pub caret_offsets: Vec<f64>,
    /// Always `NESTED_FORM`.
    pub reason: TextReason,
    pub substituted: bool,
}

impl SourceFormLine {
    fn of(run: TextRun) -> SourceFormLine {
        SourceFormLine {
            id: run.id,
            order: run.order,
            line: run.line,
            text: run.text,
            rect: run.rect,
            origin: run.origin,
            dir: run.dir,
            ascent: run.ascent,
            descent: run.descent,
            caret_offsets: run.caret_offsets,
            reason: run.reason.unwrap_or(TextReason::NestedForm),
            substituted: run.substituted,
        }
    }
}

fn capability(reason: Option<TextReason>) -> SourceCapability {
    match reason {
        Some(_) => SourceCapability::Unsupported,
        None => SourceCapability::Supported,
    }
}

/// The #33 classification of one page of `ctx` from its Edit-text model. `cancel` stops the
/// `Classify` walk (it then lists no occurrence: `occurrence_reason` `PAGE_TOO_COMPLEX`).
pub fn classify_source_page(
    ctx: &SnapshotContext,
    model: &PageModel,
    cancel: Option<&AtomicBool>,
) -> SourcePageResult {
    let runs: Vec<SourceRunCapability> = model
        .runs
        .iter()
        .map(|r| SourceRunCapability {
            run_id: r.id.clone(),
            capability: capability(r.reason),
            reason: r.reason,
        })
        .collect();
    let run_bytes = runs.iter().map(|r| r.run_id.capacity()).sum::<usize>()
        + runs.capacity() * size_of::<SourceRunCapability>();
    let held = model.walk.model_bytes.saturating_add(run_bytes);
    // A model refused for size, at its walk or its run stage, already passed the page budget: the
    // Classify pass would charge the same and more (it also descends Forms), so it does not run.
    let over_budget = model.page_reason == Some(TextReason::PageTooComplex)
        && model.page_detail.as_deref() == Some(MODEL_SIZE);
    let painted_forms = model
        .walk
        .paints
        .iter()
        .any(|p| matches!(p.kind, PaintKind::FormXObject { .. }));
    let listed = if over_budget {
        Err(TextReason::PageTooComplex)
    } else if model.page_reason.is_none() && !painted_forms {
        // No walk to stop: a cancel set by now still lists nothing, as a cancelled walk would.
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(TextReason::PageTooComplex)
        } else {
            let mut mem = ModelBudget::new(held);
            list_occurrences(ctx, model, &model.walk, &mut mem).map(|o| (o, Vec::new()))
        }
    } else {
        let mut mem = ModelBudget::new(held);
        let walk = &model.walk;
        let fonts = walk.page_fonts.iter().map(|(_, f)| f);
        mem.fonts_held(fonts.chain(walk.records.iter().filter_map(|r| r.font.as_ref())));
        let mut walk = walk_page_within(ctx, &model.content, WalkMode::Classify, cancel, mem);
        walk.drop_ops();
        match walk.page_reason {
            Some(r) if r != TextReason::Geometry => Err(r),
            _ => {
                let mut mem = ModelBudget::new(walk.model_bytes);
                list_occurrences(ctx, model, &walk, &mut mem).map(|o| {
                    // Lines that do not fit the budget are left out (they are refused anyway).
                    let lines = match model.page_reason {
                        None => form_lines(model, &walk, &mut mem).unwrap_or_default(),
                        Some(_) => Vec::new(),
                    };
                    let lines = lines.into_iter().map(SourceFormLine::of).collect();
                    (o, lines)
                })
            }
        }
    };
    let (occurrences, form_lines, occurrence_reason) = match listed {
        Ok((list, lines)) => (list, lines, None),
        Err(reason) => (Vec::new(), Vec::new(), Some(reason)),
    };
    SourcePageResult {
        page_index: model.page_index,
        page_reason: model.page_reason,
        runs,
        occurrences,
        occurrence_reason,
        form_lines,
    }
}

/// The occurrences of `walk` under `mem` (which has charged the model and the walk).
fn list_occurrences(
    ctx: &SnapshotContext,
    model: &PageModel,
    walk: &PageWalk,
    mem: &mut ModelBudget,
) -> Result<Vec<SourceOccurrence>, TextReason> {
    occurrences(ctx, model, walk, mem).map_err(|stop| stop.reason)
}

struct Item {
    seq: u32,
    kind: SourceKind,
    rect: SourceRect,
    path: String,
    reason: Option<TextReason>,
    text: Option<String>,
}

/// Bytes a path string may take (`path`).
fn path_bound(chain: &[ObjectId]) -> usize {
    chain.len().saturating_mul(PATH_FORM_BYTES) + PATH_SPAN_BYTES
}

/// The occurrences of `walk`, each charged to `mem` (with the scratch that sorts them) before it
/// is made.
fn occurrences(
    ctx: &SnapshotContext,
    model: &PageModel,
    walk: &PageWalk,
    mem: &mut ModelBudget,
) -> Result<Vec<SourceOccurrence>, Stop> {
    let records = model.walk.records.len();
    let listed = walk.records.len().saturating_add(walk.paints.len());
    mem.scratch(
        records.saturating_mul(map_entry::<(usize, usize), usize>())
            + walk.paints.len() * map_entry::<ObjectId, usize>()
            + listed.saturating_mul(size_of::<Item>()),
    )?;
    mem.hold(listed.saturating_mul(size_of::<SourceOccurrence>()))?;
    let by_span: HashMap<(usize, usize), usize> = model
        .walk
        .records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.span.as_ref().map(|s| ((s.start, s.end), i)))
        .collect();
    let mut rc = ReasonCtx::new(ctx, &model.content, walk);
    let mut items: Vec<Item> = Vec::with_capacity(listed);
    for rec in &walk.records {
        let reason = if rec.depth > 0 {
            Some(TextReason::NestedForm)
        } else if let Some(page) = walk.page_reason.or(model.page_reason) {
            Some(page)
        } else {
            match rec
                .span
                .as_ref()
                .and_then(|s| by_span.get(&(s.start, s.end)))
            {
                Some(i) => model.record_reason.get(*i).copied().flatten(),
                None => rc.record_reasons(rec).first().copied(),
            }
        };
        // The text is collected by appending (room for up to twice its bytes).
        let text: usize = rec
            .glyphs
            .iter()
            .filter_map(|g| g.text.as_ref())
            .map(String::len)
            .sum();
        mem.hold(2 * text + 2 * path_bound(&rec.form_chain) + LOCATOR_FIXED_BYTES)?;
        items.push(Item {
            seq: rec.seq,
            kind: SourceKind::Text,
            rect: text_rect(rec),
            path: path(rec.depth, &rec.form_chain, &rec.local_span),
            reason,
            text: rec.glyphs.iter().map(|g| g.text.as_deref()).collect(),
        });
    }
    let refs = ctx.refs();
    let mut paints_of: HashMap<ObjectId, usize> = HashMap::new();
    for p in &walk.paints {
        if let Some(x) = p.xobject {
            *paints_of.entry(x.id).or_default() += 1;
        }
    }
    let rotation = display_rotation(walk.geometry.rotate);
    for p in &walk.paints {
        if !matches!(
            p.kind,
            PaintKind::ImageXObject { .. } | PaintKind::InlineImage { .. }
        ) {
            continue;
        }
        let shared = p.xobject.is_some_and(|x| {
            refs.count(x.id) != 1 || x.shared_path || paints_of.get(&x.id).copied() != Some(1)
        });
        let bbox = p.bbox.unwrap_or_default();
        mem.hold(path_bound(&p.form_chain) * 2 + LOCATOR_FIXED_BYTES)?;
        items.push(Item {
            seq: p.seq,
            kind: SourceKind::Image,
            rect: SourceRect {
                x: bbox[0],
                y: bbox[1],
                w: (bbox[2] - bbox[0]).max(0.01),
                h: (bbox[3] - bbox[1]).max(0.01),
            },
            path: path(p.depth, &p.form_chain, &p.local_span),
            reason: image_reason(walk, p, shared, &rotation),
            text: None,
        });
    }
    items.sort_by_key(|i| i.seq);
    let fp = ctx.snap.fingerprint;
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(ordinal, i)| {
            let k = match i.kind {
                SourceKind::Text => 't',
                SourceKind::Image => 'i',
            };
            SourceOccurrence {
                page_index: model.page_index,
                kind: i.kind,
                rect: i.rect,
                locator: format!("v2:{fp}:{}:{k}:{}:{ordinal}", model.page_index, i.path),
                capability: capability(i.reason),
                reason: i.reason,
                text: i.text,
            }
        })
        .collect())
}

/// Baseline origin (rise included), the advance length and the effective size (D15).
fn text_rect(rec: &ShowRecord) -> SourceRect {
    let m = mul(&rec.tm_before, &rec.before.ctm);
    let effective = rec.before.text.tfs.abs() * m[2].hypot(m[3]);
    let advance = (rec.pen_after.0 - rec.pen_before.0).hypot(rec.pen_after.1 - rec.pen_before.1);
    SourceRect {
        x: rec.pen_before.0,
        y: rec.pen_before.1,
        w: advance.max(0.01),
        h: effective.max(0.01),
    }
}

/// `p{start}-{end}` at depth 0, else `x{obj}.{gen}>…:{start}-{end}`.
fn path(depth: u8, chain: &[ObjectId], span: &std::ops::Range<usize>) -> String {
    if depth == 0 {
        return format!("p{}-{}", span.start, span.end);
    }
    let forms: Vec<String> = chain.iter().map(|(o, g)| format!("x{o}.{g}")).collect();
    format!("{}:{}-{}", forms.join(">"), span.start, span.end)
}

/// Image priority (§A.10): INLINE_IMAGE, NESTED_FORM, CLIPPED, PATTERN, MASKED_IMAGE,
/// SHARED_XOBJECT, TRANSFORMED_IMAGE, GEOMETRY.
fn image_reason(
    walk: &PageWalk,
    p: &PaintRecord,
    shared: bool,
    rotation: &Matrix,
) -> Option<TextReason> {
    use TextReason as R;
    if matches!(p.kind, PaintKind::InlineImage { .. }) {
        return Some(R::InlineImage);
    }
    if p.depth > 0 {
        return Some(R::NestedForm);
    }
    let bbox = p.bbox.unwrap_or_default();
    let clipped = !contains(walk.geometry.visible, bbox, CLIP_CONTAIN_TOL_PT)
        || match &p.state.clip {
            ClipState::None => false,
            ClipState::Rect(r) => !contains(*r, bbox, CLIP_CONTAIN_TOL_PT),
            ClipState::Complex => true,
        };
    if clipped {
        return Some(R::Clipped);
    }
    if p.state.fill.pattern || p.state.stroke.pattern {
        return Some(R::Pattern);
    }
    if p.masked {
        return Some(R::MaskedImage);
    }
    if shared {
        return Some(R::SharedXobject);
    }
    let [a, b, c, d, _, _] = mul(&p.state.ctm, rotation);
    let tol = AXIS_EPSILON_REL * a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if !(b.abs() <= tol && c.abs() <= tol && a > 0.0 && d > 0.0) {
        return Some(R::TransformedImage);
    }
    walk.page_reason
}

/// Document-wide classification (tests and the corpus): every page through
/// `classify_source_page`; the first page-level refusal fails the whole call (its details name
/// the page) — a page refused `GEOMETRY` is not an error, its occurrences carry the reason.
#[cfg(test)]
pub fn classify_source_content(
    path: &std::path::Path,
) -> Result<Vec<SourceOccurrence>, crate::error::AppError> {
    use crate::pdf_engine::text_edit::runs::build_page_model;
    use crate::pdf_engine::text_edit::snapshot::read_snapshot;
    let ctx = SnapshotContext::new(read_snapshot(path)?);
    let mut out = Vec::new();
    for page in 0..ctx.snap.pages.len() {
        let page = u32::try_from(page).unwrap_or(u32::MAX);
        let result = classify_source_page(&ctx, &build_page_model(&ctx, page, None)?, None);
        page_refusal(&result)?;
        out.extend(result.occurrences);
    }
    Ok(out)
}

/// The occurrence a v2 locator names, from one fresh read of `path`: a fingerprint that differs
/// (or an occurrence that no longer exists) is `STALE`; only the locator's page is walked.
#[cfg(test)]
pub fn resolve_source_locator(
    path: &std::path::Path,
    locator: &str,
) -> Result<SourceOccurrence, crate::error::AppError> {
    use crate::pdf_engine::text_edit::runs::build_page_model;
    use crate::pdf_engine::text_edit::snapshot::{fnv1a_u64, snapshot_from_bytes, Fingerprint};
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stale = || crate::pdf_engine::text_edit::reasons::stale(&name);
    let mut fields = locator.split(':');
    let (Some("v2"), Some(fp), Some(page)) = (fields.next(), fields.next(), fields.next()) else {
        return Err(stale());
    };
    let page: u32 = page.parse().map_err(|_| stale())?;
    let (bytes, modified) = read_once(path)?;
    let fingerprint = Fingerprint {
        len: bytes.len() as u64,
        fnv: fnv1a_u64(&bytes),
    };
    if fingerprint.to_string() != fp {
        return Err(stale());
    }
    let ctx = SnapshotContext::new(snapshot_from_bytes(path, bytes, modified)?);
    let result = classify_source_page(&ctx, &build_page_model(&ctx, page, None)?, None);
    page_refusal(&result)?;
    result
        .occurrences
        .into_iter()
        .find(|o| o.locator == locator)
        .ok_or_else(stale)
}

#[cfg(test)]
fn page_refusal(result: &SourcePageResult) -> Result<(), crate::error::AppError> {
    let reason = result
        .page_reason
        .filter(|r| *r != TextReason::Geometry)
        .or(result.occurrence_reason);
    match reason {
        None => Ok(()),
        Some(r) => {
            let page = result.page_index.saturating_add(1);
            Err(r
                .to_app_error(Some(page))
                .with_details(format!("page {page}: {}", r.as_str())))
        }
    }
}

/// The one capped read of `path` (same rules as `read_snapshot`).
#[cfg(test)]
fn read_once(
    path: &std::path::Path,
) -> Result<(Vec<u8>, Option<std::time::SystemTime>), crate::error::AppError> {
    use crate::error::AppError;
    use crate::pdf_engine::text_edit::{limits, reasons};
    use std::io::Read;
    if !path.is_file() {
        return Err(AppError::invalid_pdf(&path.to_string_lossy()));
    }
    let cap = limits::file_cap();
    let meta = std::fs::metadata(path).map_err(|e| AppError::io("Could not read the PDF.", e))?;
    if meta.len() > cap {
        return Err(reasons::file_too_large());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(cap.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|e| AppError::io("Could not read the PDF.", e))?;
    if bytes.len() as u64 > cap {
        return Err(reasons::file_too_large());
    }
    Ok((bytes, meta.modified().ok()))
}
