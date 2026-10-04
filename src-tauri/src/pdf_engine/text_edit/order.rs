//! Reading order (SPEC §B.11, D33): structure-tree order on tagged pages, XY-cut in display space
//! otherwise — all or nothing, never a mix. Sets `order` (rank) and `line` (line clusters counted
//! in that order) for every listed run, so the DOM order of the text layer is what a screen
//! reader reads.

use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::limits::{
    XY_CUT_COL_GAP_EM, XY_CUT_DEPTH_MAX, XY_CUT_ROW_GAP_EM,
};
use crate::pdf_engine::text_edit::structure::structure_mcids;
use lopdf::ObjectId;
use std::collections::HashMap;

/// Baselines within this share of the smaller effective size form one line.
const LINE_CLUSTER_SHARE: f64 = 0.5;
/// Smallest size used for gap thresholds (zero-size runs still terminate the cut).
const SIZE_FLOOR: f64 = 1e-6;

#[derive(Debug, Clone)]
pub struct RunBox {
    /// `[x0, y0, x1, y1]` after `/Rotate`, y down.
    pub display_rect: [f64; 4],
    pub baseline_y: f64,
    pub size: f64,
    pub mcid: Option<i64>,
}

/// `(order, line)` per run, in input order.
pub fn reading_order(ctx: &SnapshotContext, page_id: ObjectId, runs: &[RunBox]) -> Vec<(u32, u32)> {
    let xy = xy_cut_order(runs);
    let order = tagged_order(ctx, page_id, runs, &xy).unwrap_or(xy);
    let mut out = vec![(0u32, 0u32); runs.len()];
    let mut line = 0u32;
    let mut prev: Option<&RunBox> = None;
    for (rank, i) in order.iter().enumerate() {
        let Some(b) = runs.get(*i) else { continue };
        if prev.is_some_and(|p| !same_line(p, b)) {
            line = line.saturating_add(1);
        }
        if let Some(slot) = out.get_mut(*i) {
            *slot = (u32::try_from(rank).unwrap_or(u32::MAX), line);
        }
        prev = Some(b);
    }
    out
}

/// Next in reading order on the same line: close baselines and not moving back to the left.
fn same_line(a: &RunBox, b: &RunBox) -> bool {
    let tol = LINE_CLUSTER_SHARE * a.size.min(b.size);
    (a.baseline_y - b.baseline_y).abs() <= tol && b.display_rect[0] >= a.display_rect[0] - tol
}

/// Structure order when the catalog has a structure tree and every run carries an MCID the DFS
/// reaches (ties keep their XY-cut order); `None` otherwise.
fn tagged_order(
    ctx: &SnapshotContext,
    page_id: ObjectId,
    runs: &[RunBox],
    xy: &[usize],
) -> Option<Vec<usize>> {
    if runs.is_empty() || runs.iter().any(|r| r.mcid.is_none()) {
        return None;
    }
    let mcids = structure_mcids(ctx.doc(), page_id)?;
    let mut rank: HashMap<i64, usize> = HashMap::new();
    for (i, m) in mcids.iter().enumerate() {
        rank.entry(*m).or_insert(i);
    }
    let mut xy_pos = vec![0usize; runs.len()];
    for (pos, i) in xy.iter().enumerate() {
        if let Some(slot) = xy_pos.get_mut(*i) {
            *slot = pos;
        }
    }
    let mut keyed = runs
        .iter()
        .enumerate()
        .map(|(i, r)| Some((*rank.get(&r.mcid?)?, *xy_pos.get(i)?, i)))
        .collect::<Option<Vec<(usize, usize, usize)>>>()?;
    keyed.sort_unstable();
    Some(keyed.into_iter().map(|(_, _, i)| i).collect())
}

/// XY-cut (D33): at each node first a horizontal cut at the widest empty band spanning the node
/// (≥ `XY_CUT_ROW_GAP_EM` × the node's median size), else a vertical cut at the widest empty
/// gutter (≥ `XY_CUT_COL_GAP_EM` × median size); leaves are sorted into lines by baseline, then
/// left to right. Depth ≤ `XY_CUT_DEPTH_MAX`.
pub fn xy_cut_order(runs: &[RunBox]) -> Vec<usize> {
    let mut out = Vec::with_capacity(runs.len());
    cut(runs, (0..runs.len()).collect(), 0, &mut out);
    out
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

fn cut(runs: &[RunBox], idx: Vec<usize>, depth: usize, out: &mut Vec<usize>) {
    if idx.len() > 1 && depth < XY_CUT_DEPTH_MAX {
        let size = median_size(runs, &idx).max(SIZE_FLOOR);
        for (axis, share) in [(Axis::Y, XY_CUT_ROW_GAP_EM), (Axis::X, XY_CUT_COL_GAP_EM)] {
            if let Some((first, second)) = split(runs, &idx, axis, share * size) {
                cut(runs, first, depth + 1, out);
                cut(runs, second, depth + 1, out);
                return;
            }
        }
    }
    leaf(runs, idx, out);
}

fn median_size(runs: &[RunBox], idx: &[usize]) -> f64 {
    let mut sizes: Vec<f64> = idx
        .iter()
        .filter_map(|i| runs.get(*i).map(|r| r.size))
        .filter(|s| s.is_finite())
        .collect();
    sizes.sort_by(|a, b| a.total_cmp(b));
    sizes.get(sizes.len() / 2).copied().unwrap_or(0.0)
}

fn interval(r: &RunBox, axis: Axis) -> (f64, f64) {
    match axis {
        Axis::X => (r.display_rect[0], r.display_rect[2]),
        Axis::Y => (r.display_rect[1], r.display_rect[3]),
    }
}

/// Splits at the widest empty band on `axis` that is at least `min_gap` wide.
fn split(
    runs: &[RunBox],
    idx: &[usize],
    axis: Axis,
    min_gap: f64,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut iv: Vec<(f64, f64, usize)> = idx
        .iter()
        .filter_map(|i| {
            runs.get(*i).map(|r| {
                let (lo, hi) = interval(r, axis);
                (lo, hi, *i)
            })
        })
        .collect();
    iv.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reach = f64::NEG_INFINITY;
    let mut best: Option<(f64, f64)> = None;
    for (k, (lo, hi, _)) in iv.iter().enumerate() {
        if k > 0 {
            let gap = lo - reach;
            if gap > 0.0 && gap >= min_gap && best.map_or(true, |(g, _)| gap > g) {
                best = Some((gap, reach));
            }
        }
        reach = reach.max(*hi);
    }
    let (_, at) = best?;
    let (first, second): (Vec<(f64, f64, usize)>, Vec<(f64, f64, usize)>) =
        iv.into_iter().partition(|(lo, _, _)| *lo <= at);
    Some((
        first.into_iter().map(|x| x.2).collect(),
        second.into_iter().map(|x| x.2).collect(),
    ))
}

/// Lines by baseline (clustered within half the smaller size), each left to right.
fn leaf(runs: &[RunBox], mut idx: Vec<usize>, out: &mut Vec<usize>) {
    let base = |i: &usize| runs.get(*i).map_or(0.0, |r| r.baseline_y);
    idx.sort_by(|a, b| base(a).total_cmp(&base(b)));
    let mut lines: Vec<Vec<usize>> = Vec::new();
    for i in idx {
        let Some(r) = runs.get(i) else { continue };
        let joins = lines
            .last()
            .and_then(|l| l.first())
            .and_then(|f| runs.get(*f))
            .is_some_and(|f| {
                (f.baseline_y - r.baseline_y).abs() <= LINE_CLUSTER_SHARE * f.size.min(r.size)
            });
        match lines.last_mut() {
            Some(line) if joins => line.push(i),
            _ => lines.push(vec![i]),
        }
    }
    let left = |i: &usize| runs.get(*i).map_or(0.0, |r| r.display_rect[0]);
    for mut line in lines {
        line.sort_by(|a, b| left(a).total_cmp(&left(b)));
        out.extend(line);
    }
}
