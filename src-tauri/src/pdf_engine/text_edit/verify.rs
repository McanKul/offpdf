//! The verification re-walk (SPEC §B.14): one function, `walk_and_verify_with`, shared by the plan
//! self-check, the preview and Save (Phase A check A4). It compares the walk of the page before
//! the edit with the walk of the content after it, id-free (D32), so it holds across qpdf's
//! renumbering:
//!
//! 1. the after page is not refused and the replacement grammar holds inside every splice;
//! 2. every record and paint outside the splices pairs by span (`SpanMap`) with one of the same
//!    operator, and the records inside the splices are exactly the emitted ones;
//! 3. unedited records keep op, codes, texts, font, full state and every glyph origin (0.01 pt);
//! 4. paints keep their id-free kind (name + deep hash) and full state (B15);
//! 5. edited runs draw exactly the expected glyphs (`verify/edited.rs`);
//! 6. the state in force after each replaced region — read by a probe `[]TJ` inserted right
//!    after it into a copy of the content, walked in the same context — equals the original state
//!    after that member, with the same text matrix and pen (B14).
//!
//! Spans are mapped through prefix sums and regions found by binary search, so a page of 250,000
//! ops with hundreds of edits is checked in O(n log k).

mod edited;

use crate::pdf_engine::text_edit::content::PageContent;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::encode::REPLACEMENT_OPERATORS;
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::{DRIFT_TOLERANCE_PT, STATE_EPSILON, SYNTH_SPACE_EM};
use crate::pdf_engine::text_edit::reasons::{EditProblemCode, TextReason};
use crate::pdf_engine::text_edit::rewrite::{PagePlan, Splice};
use crate::pdf_engine::text_edit::runs::TextRun;
use crate::pdf_engine::text_edit::state::same_state;
use crate::pdf_engine::text_edit::walker::{walk_page, PageWalk, ShowRecord, WalkMode};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

/// The probe inserted after each replaced region of a copy of the after content.
const PROBE: &[u8] = b"\n[]TJ\n";

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyFailure {
    PageRefused(TextReason),
    RecordUnpaired {
        index: usize,
    },
    RecordCount {
        expected: usize,
        found: usize,
    },
    PaintChanged {
        index: usize,
        field: &'static str,
    },
    Drift {
        record: usize,
        glyph: usize,
        pt: f64,
    },
    StateChanged {
        record: usize,
        field: &'static str,
    },
    /// `what`: text|codes|font|size|tc|fill|origin|pen|glyph|requested_text|absorbed|post_state|
    /// records
    EditedMismatch {
        run_id: String,
        what: &'static str,
    },
    ForbiddenOperator {
        op: &'static str,
    },
}

impl VerifyFailure {
    /// Drift → PEN_DRIFT; StateChanged/PaintChanged → STATE_CHANGED; the rest → EDIT_VERIFY_FAILED.
    pub fn problem_code(&self) -> EditProblemCode {
        match self {
            VerifyFailure::Drift { .. } => EditProblemCode::PenDrift,
            VerifyFailure::StateChanged { .. } | VerifyFailure::PaintChanged { .. } => {
                EditProblemCode::StateChanged
            }
            _ => EditProblemCode::EditVerifyFailed,
        }
    }
}

impl std::fmt::Display for VerifyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyFailure::PageRefused(r) => write!(f, "page_refused reason={}", r.as_str()),
            VerifyFailure::RecordUnpaired { index } => write!(f, "record_unpaired record={index}"),
            VerifyFailure::RecordCount { expected, found } => {
                write!(f, "record_count expected={expected} found={found}")
            }
            VerifyFailure::PaintChanged { index, field } => {
                write!(f, "paint_changed paint={index} field={field}")
            }
            VerifyFailure::Drift { record, glyph, pt } => {
                write!(f, "drift record={record} glyph={glyph} pt={pt:.4}")
            }
            VerifyFailure::StateChanged { record, field } => {
                write!(f, "state_changed record={record} field={field}")
            }
            VerifyFailure::EditedMismatch { run_id, what } => {
                write!(f, "edited_mismatch what={what} run={run_id}")
            }
            VerifyFailure::ForbiddenOperator { op } => write!(f, "forbidden_operator op={op}"),
        }
    }
}

/// One element of a decoded show sequence.
pub(crate) enum TextItem<'a> {
    Glyph(&'a str),
    /// A kern in em (`−n/1000`).
    Kern(f64),
}

fn is_space(t: &str) -> bool {
    matches!(t, " " | "\u{a0}")
}

/// The text a reader of the records gets: glyph texts, plus one space where the kerns between two
/// non-space glyphs add up to at least `SYNTH_SPACE_EM` (§A.3.4).
pub(crate) fn decoded_text<'a>(items: impl Iterator<Item = TextItem<'a>>) -> String {
    let mut out = String::new();
    let mut prev_glyph: Option<&str> = None;
    let mut kerns = 0.0f64;
    let mut any_kern = false;
    for item in items {
        match item {
            TextItem::Kern(em) => {
                kerns += em;
                any_kern = true;
            }
            TextItem::Glyph(t) => {
                if let Some(p) = prev_glyph {
                    if any_kern && kerns >= SYNTH_SPACE_EM && !is_space(p) && !is_space(t) {
                        out.push(' ');
                    }
                }
                out.push_str(t);
                prev_glyph = Some(t);
                kerns = 0.0;
                any_kern = false;
            }
        }
    }
    out
}

/// Byte-length change of a splice.
fn growth(s: &Splice) -> i64 {
    let old = s.joined.end.saturating_sub(s.joined.start);
    i64::try_from(s.bytes.len())
        .unwrap_or(i64::MAX)
        .saturating_sub(i64::try_from(old).unwrap_or(i64::MAX))
}

fn shifted(v: usize, shift: i64) -> usize {
    usize::try_from(i64::try_from(v).unwrap_or(i64::MAX).saturating_add(shift)).unwrap_or(0)
}

/// `original` (a span of the content before the edit) in the content after the edit: shifted by
/// the net length change of the splices that end at or before it — prefix sums over the splices
/// (sorted by joined start, disjoint); `map_span` (test-only) is the one-span definition.
struct SpanMap {
    /// (joined end of splice k, total shift of splices 0..=k).
    ends: Vec<(usize, i64)>,
}

impl SpanMap {
    fn new(splices: &[Splice]) -> SpanMap {
        let mut total = 0i64;
        let ends = splices
            .iter()
            .map(|s| {
                total = total.saturating_add(growth(s));
                (s.joined.end, total)
            })
            .collect();
        SpanMap { ends }
    }

    fn map(&self, original: &Span) -> Span {
        let k = self.ends.partition_point(|(end, _)| *end <= original.start);
        let shift = k
            .checked_sub(1)
            .and_then(|i| self.ends.get(i))
            .map_or(0, |(_, s)| *s);
        shifted(original.start, shift)..shifted(original.end, shift)
    }
}

/// Each splice's region in the after content (`[start, end)`), in `plan.splices` order (sorted by
/// joined start, so the regions are sorted and disjoint too).
fn after_regions(splices: &[Splice]) -> Vec<Span> {
    let mut shift: i64 = 0;
    splices
        .iter()
        .map(|s| {
            let start = shifted(s.joined.start, shift);
            shift = shift.saturating_add(growth(s));
            start..start.saturating_add(s.bytes.len())
        })
        .collect()
}

fn inside(span: &Span, region: &Span) -> bool {
    span.start >= region.start && span.end <= region.end
}

fn overlaps(span: &Span, region: &Span) -> bool {
    span.start < region.end && region.start < span.end
}

/// The first of the sorted, disjoint `regions` that could overlap `span` (binary search).
fn region_near(regions: &[Span], span: &Span) -> Option<usize> {
    let k = regions.partition_point(|r| r.end <= span.start);
    regions
        .get(k)
        .is_some_and(|r| overlaps(span, r))
        .then_some(k)
}

pub(crate) fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// The font a record is drawn with, id-free: (resource name, content hash).
pub(crate) fn font_id(rec: &ShowRecord) -> (Vec<u8>, u64) {
    match rec.before.text.font.as_ref() {
        Some(f) => (
            f.resource
                .as_deref()
                .map(<[u8]>::to_vec)
                .unwrap_or_default(),
            f.content_hash,
        ),
        None => (Vec::new(), 0),
    }
}

/// The member splices (those of the plan's runs) as `(after insertion point, run, member)`.
fn member_ends(plan: &PagePlan, regions: &[Span]) -> Vec<(usize, usize, usize)> {
    let index: HashMap<usize, usize> = plan
        .splices
        .iter()
        .enumerate()
        .map(|(k, s)| (s.joined.start, k))
        .collect();
    let mut out = Vec::new();
    for (r, run) in plan.runs.iter().enumerate() {
        for (m, s) in run.splices.iter().enumerate() {
            if let Some(region) = index.get(&s.joined.start).and_then(|k| regions.get(*k)) {
                out.push((region.end, r, m));
            }
        }
    }
    out.sort_unstable();
    out
}

/// A copy of `after` with a probe `[]TJ` after every member splice, and each probe's op start
/// (joined coordinates of the copy) with its (run, member).
fn probe_content(
    after: &PageContent,
    plan: &PagePlan,
) -> (PageContent, Vec<(usize, usize, usize)>) {
    let regions = after_regions(&plan.splices);
    let ends = member_ends(plan, &regions);
    let mut per_part: HashMap<usize, Vec<usize>> = HashMap::new();
    for (pos, _, _) in &ends {
        let part = after
            .parts
            .iter()
            .position(|p| *pos >= p.start && p.start.checked_add(p.len).is_some_and(|e| *pos <= e));
        if let Some(part) = part {
            let local = pos.saturating_sub(after.parts.get(part).map_or(0, |p| p.start));
            per_part.entry(part).or_default().push(local);
        }
    }
    let mut replaced = Vec::new();
    for (part, mut locals) in per_part {
        locals.sort_unstable();
        let mut bytes = after.part_bytes(part).to_vec();
        for local in locals.iter().rev() {
            if *local <= bytes.len() {
                bytes.splice(*local..*local, PROBE.iter().copied());
            }
        }
        replaced.push((part, bytes));
    }
    let probes = ends
        .iter()
        .enumerate()
        .map(|(i, (pos, r, m))| {
            let before = i.saturating_mul(PROBE.len()).saturating_add(1);
            (pos.saturating_add(before), *r, *m)
        })
        .collect();
    (after.with_replaced_parts(&replaced), probes)
}

/// Walks the after content (and its probe copy) in `ctx` and runs checks 1–6, keeping only what
/// `keep` takes from the after walk. Every caller — the plan self-check (in the source context),
/// the preview and Save (in the context of qpdf's output) — goes through here. The after walk is handed to `keep` and dropped once checks 1–5 pass, before
/// the probe walk, so two walks never coexist (review-T4 M-1).
pub fn walk_and_verify_with<T>(
    ctx: &SnapshotContext,
    page_index: u32,
    (after_content, before, before_runs): (&PageContent, &PageWalk, &[TextRun]),
    plan: &PagePlan,
    cancel: Option<&AtomicBool>,
    keep: impl FnOnce(PageWalk) -> T,
) -> Result<T, VerifyFailure> {
    let after = walk_page(ctx, page_index, after_content, WalkMode::Edit, cancel);
    check_page(before, before_runs, &after, plan)?;
    let kept = keep(after);
    let (probe_copy, probes) = probe_content(after_content, plan);
    let probe = walk_page(ctx, page_index, &probe_copy, WalkMode::Edit, cancel);
    drop(probe_copy);
    check_post_state(before, &probe, &probes, plan)?;
    Ok(kept)
}

/// Checks 1–5.
fn check_page(
    before: &PageWalk,
    before_runs: &[TextRun],
    after: &PageWalk,
    plan: &PagePlan,
) -> Result<(), VerifyFailure> {
    if let Some(r) = after.page_reason {
        return Err(VerifyFailure::PageRefused(r));
    }
    let regions = after_regions(&plan.splices);
    if !seams::grammar_skipped() {
        check_grammar(after, &regions)?;
    }
    let map = SpanMap::new(&plan.splices);
    let edited = pair_records(before, after, plan, &regions, &map)?;
    // A splice that belongs to no run may hold no show op.
    for (k, s) in plan.splices.iter().enumerate() {
        let member = plan.runs.iter().any(|r| r.splices.contains(s));
        if let (false, Some(&j)) = (member, edited.get(k).and_then(|e| e.first())) {
            return Err(VerifyFailure::RecordUnpaired { index: j });
        }
    }
    check_paints(before, after, &regions, &map)?;
    for run in &plan.runs {
        edited::check_edited_run(before, before_runs, after, plan, &edited, run)?;
    }
    Ok(())
}

/// Check 1: inside each splice only replacement operators, never an op crossing its boundary.
fn check_grammar(after: &PageWalk, regions: &[Span]) -> Result<(), VerifyFailure> {
    for op in &after.ops {
        let Some(k) = region_near(regions, &op.span) else {
            continue;
        };
        let inside_one = regions.get(k).is_some_and(|r| inside(&op.span, r));
        if !inside_one {
            return Err(VerifyFailure::ForbiddenOperator {
                op: "splice boundary",
            });
        }
        if !REPLACEMENT_OPERATORS.contains(&op.operator) {
            return Err(VerifyFailure::ForbiddenOperator {
                op: op.operator.as_str(),
            });
        }
    }
    Ok(())
}

/// Check 2 and 3: pairs every unedited record and compares it; returns, per splice index, the
/// after records inside its region (in order).
fn pair_records(
    before: &PageWalk,
    after: &PageWalk,
    plan: &PagePlan,
    regions: &[Span],
    map: &SpanMap,
) -> Result<Vec<Vec<usize>>, VerifyFailure> {
    let mut by_span: HashMap<(usize, usize), usize> = HashMap::new();
    let mut edited: Vec<Vec<usize>> = vec![Vec::new(); regions.len()];
    for (j, rec) in after.records.iter().enumerate() {
        let Some(span) = rec.span.as_ref() else {
            return Err(VerifyFailure::RecordUnpaired { index: j });
        };
        match region_near(regions, span)
            .filter(|k| regions.get(*k).is_some_and(|r| inside(span, r)))
        {
            Some(k) => {
                if let Some(list) = edited.get_mut(k) {
                    list.push(j);
                }
            }
            None => {
                by_span.insert((span.start, span.end), j);
            }
        }
    }
    let originals: Vec<Span> = plan.splices.iter().map(|s| s.joined.clone()).collect();
    let mut paired: Vec<bool> = vec![false; after.records.len()];
    for (i, rec) in before.records.iter().enumerate() {
        let Some(span) = rec.span.as_ref() else {
            return Err(VerifyFailure::RecordUnpaired { index: i });
        };
        let replaced = region_near(&originals, span)
            .is_some_and(|k| originals.get(k).is_some_and(|r| inside(span, r)));
        if replaced {
            continue;
        }
        let mapped = map.map(span);
        let Some((j, a)) = by_span
            .get(&(mapped.start, mapped.end))
            .and_then(|j| Some((*j, after.records.get(*j)?)))
        else {
            return Err(VerifyFailure::RecordUnpaired { index: i });
        };
        compare_unedited(rec, a, j)?;
        if let Some(slot) = paired.get_mut(j) {
            *slot = true;
        }
    }
    let stray = by_span
        .values()
        .copied()
        .filter(|j| !paired.get(*j).copied().unwrap_or(false))
        .min();
    if let Some(j) = stray {
        return Err(VerifyFailure::RecordUnpaired { index: j });
    }
    let emitted: usize = edited.iter().map(Vec::len).sum();
    let count = paired.iter().filter(|p| **p).count();
    if count.saturating_add(emitted) != after.records.len() {
        return Err(VerifyFailure::RecordCount {
            expected: count.saturating_add(emitted),
            found: after.records.len(),
        });
    }
    Ok(edited)
}

/// A drift of more than `DRIFT_TOLERANCE_PT` between two positions.
fn drift(a: (f64, f64), b: (f64, f64), record: usize, glyph: usize) -> Result<(), VerifyFailure> {
    let d = dist(a, b);
    if d > DRIFT_TOLERANCE_PT || !d.is_finite() {
        return Err(VerifyFailure::Drift {
            record,
            glyph,
            pt: d,
        });
    }
    Ok(())
}

fn compare_unedited(b: &ShowRecord, a: &ShowRecord, j: usize) -> Result<(), VerifyFailure> {
    let same_glyphs = b.glyphs.len() == a.glyphs.len()
        && b.glyphs
            .iter()
            .zip(&a.glyphs)
            .all(|(x, y)| x.code == y.code && x.text == y.text);
    if b.op != a.op || !same_glyphs {
        return Err(VerifyFailure::RecordUnpaired { index: j });
    }
    if font_id(b) != font_id(a) {
        return Err(VerifyFailure::StateChanged {
            record: j,
            field: "font",
        });
    }
    same_state(&b.before, &a.before)
        .and_then(|()| same_state(&b.after, &a.after))
        .map_err(|field| VerifyFailure::StateChanged { record: j, field })?;
    for (g, (x, y)) in b.glyphs.iter().zip(&a.glyphs).enumerate() {
        drift(x.origin, y.origin, j, g)?;
    }
    drift(b.pen_before, a.pen_before, j, a.glyphs.len())?;
    drift(
        b.pen_after,
        a.pen_after,
        j,
        a.glyphs.len().saturating_add(1),
    )
}

fn unpaired_paint(index: usize) -> VerifyFailure {
    VerifyFailure::PaintChanged {
        index,
        field: "unpaired",
    }
}

/// Check 2 (paints) and 4: same count, paired by span, same id-free kind and full state.
fn check_paints(
    before: &PageWalk,
    after: &PageWalk,
    regions: &[Span],
    map: &SpanMap,
) -> Result<(), VerifyFailure> {
    if before.paints.len() != after.paints.len() {
        return Err(VerifyFailure::RecordCount {
            expected: before.paints.len(),
            found: after.paints.len(),
        });
    }
    let mut by_span: HashMap<(usize, usize), usize> = HashMap::new();
    for (j, p) in after.paints.iter().enumerate() {
        match p.span.as_ref() {
            Some(s) if region_near(regions, s).is_none() => {
                by_span.insert((s.start, s.end), j);
            }
            _ => return Err(unpaired_paint(j)),
        }
    }
    for (i, p) in before.paints.iter().enumerate() {
        let span = p.span.as_ref().ok_or_else(|| unpaired_paint(i))?;
        let mapped = map.map(span);
        let a = by_span
            .get(&(mapped.start, mapped.end))
            .and_then(|j| after.paints.get(*j))
            .ok_or_else(|| unpaired_paint(i))?;
        if p.kind != a.kind {
            return Err(VerifyFailure::PaintChanged {
                index: i,
                field: "kind",
            });
        }
        same_state(&p.state, &a.state)
            .map_err(|field| VerifyFailure::PaintChanged { index: i, field })?;
    }
    Ok(())
}

/// Check 6: the state after each replaced region (probe record) equals the original state after
/// that member, with the same text matrix (linear part) and pen.
fn check_post_state(
    before: &PageWalk,
    probe: &PageWalk,
    probes: &[(usize, usize, usize)],
    plan: &PagePlan,
) -> Result<(), VerifyFailure> {
    if let Some(r) = probe.page_reason {
        return Err(VerifyFailure::PageRefused(r));
    }
    let by_start: HashMap<usize, usize> = probe
        .records
        .iter()
        .enumerate()
        .filter_map(|(i, x)| Some((x.span.as_ref()?.start, i)))
        .collect();
    let before_by_span: HashMap<(usize, usize), &ShowRecord> = before
        .records
        .iter()
        .filter_map(|x| Some(((x.span.as_ref()?.start, x.span.as_ref()?.end), x)))
        .collect();
    for (start, r, m) in probes {
        let Some(run) = plan.runs.get(*r) else {
            continue;
        };
        let fail = || VerifyFailure::EditedMismatch {
            run_id: run.expected.run_id.clone(),
            what: "post_state",
        };
        let (index, rec) = by_start
            .get(start)
            .and_then(|i| Some((*i, probe.records.get(*i)?)))
            .ok_or_else(fail)?;
        let original = run
            .expected
            .member_spans
            .get(*m)
            .and_then(|s| before_by_span.get(&(s.start, s.end)))
            .ok_or_else(fail)?;
        same_state(&original.after, &rec.before).map_err(|field| VerifyFailure::StateChanged {
            record: index,
            field,
        })?;
        let linear_same = original
            .tm_after
            .iter()
            .zip(&rec.tm_before)
            .take(4)
            .all(|(a, b)| (a - b).abs() <= STATE_EPSILON * 1f64.max(a.abs()).max(b.abs()));
        if !linear_same {
            return Err(VerifyFailure::StateChanged {
                record: index,
                field: "tm",
            });
        }
        drift(original.pen_after, rec.pen_before, index, 0)?;
    }
    Ok(())
}

/// Test seam: skip the replacement-grammar re-check (GATE-07/08 "grammar bypassed"). Production
/// builds have no way to skip it.
mod seams {
    pub(super) fn grammar_skipped() -> bool {
        #[cfg(test)]
        {
            if super::test_seams::grammar_skipped() {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
pub(crate) mod test_seams {
    use std::cell::Cell;

    thread_local! {
        static SKIP_GRAMMAR: Cell<bool> = const { Cell::new(false) };
    }

    pub(crate) fn grammar_skipped() -> bool {
        SKIP_GRAMMAR.with(Cell::get)
    }

    /// Skips the grammar re-check on this thread until the guard is dropped.
    pub(crate) fn skip_grammar() -> GrammarGuard {
        SKIP_GRAMMAR.with(|c| c.set(true));
        GrammarGuard
    }

    pub(crate) struct GrammarGuard;

    impl Drop for GrammarGuard {
        fn drop(&mut self) {
            SKIP_GRAMMAR.with(|c| c.set(false));
        }
    }
}

/// `walk_and_verify_with` keeping the whole after walk (tests).
#[cfg(test)]
pub fn walk_and_verify(
    ctx: &SnapshotContext,
    page_index: u32,
    after_content: &PageContent,
    before: &PageWalk,
    before_runs: &[TextRun],
    plan: &PagePlan,
    cancel: Option<&AtomicBool>,
) -> Result<PageWalk, VerifyFailure> {
    let walks = (after_content, before, before_runs);
    walk_and_verify_with(ctx, page_index, walks, plan, cancel, |walk| walk)
}

/// The definition `SpanMap` implements for many spans: `original` shifted by the net length
/// change of the splices that end at or before it.
#[cfg(test)]
pub fn map_span(splices: &[Splice], original: &Span) -> Span {
    let shift: i64 = splices
        .iter()
        .filter(|s| s.joined.end <= original.start)
        .map(growth)
        .sum();
    shifted(original.start, shift)..shifted(original.end, shift)
}

#[cfg(test)]
mod tests {
    use super::{map_span, SpanMap};
    use crate::pdf_engine::text_edit::rewrite::Splice;

    /// review-T5 H2: production pairs spans with `SpanMap`; it must equal `map_span` (which
    /// VER-04 pins) on any sorted, disjoint splices and any span, including spans touching a
    /// splice end.
    #[test]
    fn span_map_equals_map_span_on_random_splices() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..2_000 {
            let mut at = 0;
            let mut splices = Vec::new();
            for _ in 0..next(6) {
                let start = at + next(20);
                let end = start + next(12);
                splices.push(Splice {
                    part: 0,
                    local: start..end,
                    joined: start..end,
                    bytes: vec![b'x'; next(25)],
                });
                at = end;
            }
            let map = SpanMap::new(&splices);
            for _ in 0..40 {
                let start = next(at + 30);
                let span = start..start + next(10);
                assert_eq!(
                    map.map(&span),
                    map_span(&splices, &span),
                    "{span:?} {splices:?}"
                );
            }
        }
    }
}
