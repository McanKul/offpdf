//! Lines drawn through Form XObjects (SPEC §A.6: "Text in a Form XObject (any depth) → Refused,
//! still shown, `NESTED_FORM`"; live check B1). A page model walks depth 0 only, so Form text
//! has no run; the Classify walk descends Forms, and its depth > 0 records become refused lines:
//! consecutive records of one Form on one baseline (the §A.1.1 geometry: same direction and size,
//! baseline within `JOIN_BASELINE_TOL_PT`, gap within `JOIN_GAP_EM`, nothing painted between)
//! join, each line is assembled like a run (`assemble`, typed with no surface) and carries
//! `NESTED_FORM` only. They follow the page's runs in reading order. Everything they keep is
//! charged to the Classify pass's budget.

use super::assemble::assemble;
use super::surface::SurfaceInfo;
use super::{PageModel, TextRun};
use crate::pdf_engine::text_edit::geometry::{cross, dot, sub, unit};
use crate::pdf_engine::text_edit::limits::{
    JOIN_BASELINE_TOL_PT, JOIN_GAP_EM, RUNS_PER_PAGE_MAX, RUN_MEMBERS_MAX, STATE_EPSILON,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::walker::budget::ModelBudget;
use crate::pdf_engine::text_edit::walker::{PageWalk, ShowRecord, Stop};
use std::mem::size_of;

/// Bytes of a Form line id besides its spans: `t1:`, the fingerprint, the page and separators.
const ID_FIXED_BYTES: usize = 64;
/// Bytes per Form in an id (`x{obj}.{gen}>`) and per member span (`{start}-{end},`).
const ID_FORM_BYTES: usize = 24;
const ID_SPAN_BYTES: usize = 42;

/// The refused lines `walk` (a Classify walk of `model`'s page) draws through Forms, numbered
/// after the model's runs; none when they would pass `RUNS_PER_PAGE_MAX` together.
pub(crate) fn form_lines(
    model: &PageModel,
    walk: &PageWalk,
    mem: &mut ModelBudget,
) -> Result<Vec<TextRun>, Stop> {
    let groups = form_groups(walk, mem)?;
    if groups.len().saturating_add(model.runs.len()) > RUNS_PER_PAGE_MAX {
        return Ok(Vec::new());
    }
    let order = model.runs.iter().map(|r| r.order + 1).max().unwrap_or(0);
    let line = model.runs.iter().map(|r| r.line + 1).max().unwrap_or(0);
    let none = SurfaceInfo::none();
    let fp = model.fingerprint.to_string();
    mem.hold(groups.len().saturating_mul(size_of::<TextRun>()))?;
    let mut lines = Vec::with_capacity(groups.len());
    for g in &groups {
        let mut run = assemble(walk, g, vec![TextReason::NestedForm], &none, mem)?;
        if run.text.trim().is_empty() {
            continue;
        }
        let k = u32::try_from(lines.len()).unwrap_or(u32::MAX);
        run.order = order.saturating_add(k);
        run.line = line.saturating_add(k);
        run.id = line_id(&fp, model.page_index, walk, g, mem)?;
        lines.push(run);
    }
    Ok(lines)
}

/// Consecutive depth > 0 records with glyphs that read as one line.
fn form_groups(walk: &PageWalk, mem: &mut ModelBudget) -> Result<Vec<Vec<usize>>, Stop> {
    let n = walk.records.iter().filter(|r| r.depth > 0).count();
    mem.scratch(n.saturating_mul(size_of::<Vec<usize>>() + 2 * size_of::<usize>()))?;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    for (i, rec) in walk.records.iter().enumerate() {
        if rec.depth == 0 || rec.glyphs.is_empty() {
            if !cur.is_empty() {
                groups.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let joins = cur.last().is_some_and(|last| {
            cur.len() < RUN_MEMBERS_MAX
                && *last + 1 == i
                && walk
                    .records
                    .get(*last)
                    .is_some_and(|a| one_line(walk, a, rec))
        });
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

/// `b` continues `a`'s line: the same Form, nothing painted between, the same text direction and
/// size, `b`'s pen on `a`'s baseline and its first glyph within `JOIN_GAP_EM` of `a`'s end.
fn one_line(walk: &PageWalk, a: &ShowRecord, b: &ShowRecord) -> bool {
    let next_paint = walk.paints.partition_point(|p| p.seq <= a.seq);
    let painted_between = walk.paints.get(next_paint).is_some_and(|p| p.seq < b.seq);
    if a.form_chain != b.form_chain || painted_between {
        return false;
    }
    let (ta, tb) = (a.text_to_user, b.text_to_user);
    let same_linear = ta
        .iter()
        .zip(&tb)
        .take(4)
        .all(|(x, y)| (x - y).abs() <= STATE_EPSILON * 1f64.max(x.abs()).max(y.abs()));
    let Some(dir) = unit((ta[0], ta[1])) else {
        return false;
    };
    let effective = ta[2].hypot(ta[3]);
    let b_origin = b.glyphs.first().map_or(b.pen_before, |g| g.origin);
    same_linear
        && cross(sub(b.pen_before, a.pen_before), dir).abs() <= JOIN_BASELINE_TOL_PT
        && dot(sub(b_origin, a.pen_after), dir).abs() <= JOIN_GAP_EM * effective
}

/// `t1:{fp}:{page}:x{obj}.{gen}>…:{s0}-{e0}[,{s1}-{e1}…]`: the Form chain and the members' spans
/// in the innermost Form's data (unique on the page; a run id never starts with `x` there).
fn line_id(
    fp: &str,
    page_index: u32,
    walk: &PageWalk,
    members: &[usize],
    mem: &mut ModelBudget,
) -> Result<String, Stop> {
    let first = members.first().and_then(|i| walk.records.get(*i));
    let chain = first.map_or(0, |r| r.form_chain.len());
    let bound = (ID_FIXED_BYTES + fp.len())
        .saturating_add(chain.saturating_mul(ID_FORM_BYTES))
        .saturating_add(members.len().saturating_mul(ID_SPAN_BYTES));
    mem.hold(bound)?;
    let forms: Vec<String> = first
        .map(|r| {
            r.form_chain
                .iter()
                .map(|(o, g)| format!("x{o}.{g}"))
                .collect()
        })
        .unwrap_or_default();
    let spans: Vec<String> = members
        .iter()
        .filter_map(|i| walk.records.get(*i))
        .map(|r| format!("{}-{}", r.local_span.start, r.local_span.end))
        .collect();
    let id = format!(
        "t1:{fp}:{page_index}:{}:{}",
        forms.join(">"),
        spans.join(",")
    );
    mem.unhold(bound.saturating_sub(id.capacity()));
    Ok(id)
}
