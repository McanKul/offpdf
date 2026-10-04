//! G-RENDER's neighbour ink check (review-final HIGH-1). G-RENDER accepts any change inside the
//! edited glyphs' masks, so when a changed line runs into the next table cell, the cell's own
//! pixels lie under those masks and nothing else sees them: a planner that also moved, deleted
//! or whitened the cell would pass. Inside the masks, a neighbour's ink may only gain ink (the new
//! glyphs drawn over or under it); a pixel of its box that held ink before (it differed from the
//! box's most common colour, its paper) and shows that paper after the edit lost it. More than
//! `RENDER_KEPT_PIXELS_MAX` such pixels fail A5 (an honest edit loses none; a 12 pt period is 5
//! pixels, review-verify HIGH-A). The old glyphs' ink boxes are left out (the
//! edit removes their ink there), and pixels outside the masks — or in a kept glyph's box outside
//! the edited glyphs' own boxes (review-verify HIGH-A) — are G-RENDER's.
//!
//! A neighbour re-coloured to another visible colour keeps its ink (this check cannot tell it from
//! new glyphs drawn over the cell in that colour); our own walk (A4 `STATE_CHANGED`) refuses it.

use super::{overlaps, pixel_masks, Layers, NearGlyphs, PixelBox, Raster, PT_PER_INCH};
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::limits::{RENDER_CHANNEL_TOL, RENDER_KEPT_PIXELS_MAX};
use crate::pdf_engine::text_edit::rewrite::ExpectedRun;
use std::collections::HashMap;

type Fail = (&'static str, String);

/// Work one check may do (pixels visited plus mask rows tested), per raster pixel: past it the
/// check fails closed (only pathological pages of overlapping words and glyphs come near it).
const WORK_PER_RASTER_PIXEL: usize = 4;

fn close(a: &[u8], b: &[u8]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| x.abs_diff(*y) <= RENDER_CHANNEL_TOL)
}

/// A text-frame rect (points, origin top left) in raster pixels, half-open and clamped.
fn frame_to_pixels(r: &[f64; 4], dpi: u32, w: usize, h: usize) -> Option<PixelBox> {
    let s = f64::from(dpi) / PT_PER_INCH;
    let px = |v: f64, max: usize| -> usize {
        if v.is_finite() && v > 0.0 {
            (v.min(max as f64)) as usize
        } else {
            0
        }
    };
    let b = PixelBox {
        x0: px((r[0] * s).floor(), w),
        y0: px((r[1] * s).floor(), h),
        x1: px((r[2] * s).ceil(), w),
        y1: px((r[3] * s).ceil(), h),
    };
    (b.x0 < b.x1 && b.y0 < b.y1).then_some(b)
}

/// The pixel at (x, y) of a raster `w` pixels wide.
fn pixel(r: &Raster, w: usize, x: usize, y: usize) -> &[u8] {
    let at = y.saturating_mul(w).saturating_add(x).saturating_mul(3);
    r.rgb.get(at..at.saturating_add(3)).unwrap_or_default()
}

/// The most common colour of `b` in `raster` (the paper the neighbour is printed on).
fn paper(raster: &Raster, w: usize, b: &PixelBox) -> [u8; 3] {
    let mut counts: HashMap<[u8; 3], usize> = HashMap::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            if let [red, green, blue] = pixel(raster, w, x, y) {
                *counts.entry([*red, *green, *blue]).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(c, n)| (*n, *c))
        .map_or([255; 3], |(c, _)| c)
}

/// Coverage layers of one neighbour box's rows.
const SLACK: usize = 0;
const KEPT: usize = 1;
const OWN: usize = 2;
const OLD: usize = 3;

/// The check (see the module doc). `neighbours`: text-frame boxes of the words set aside by
/// G-TEXT, as the source had them.
pub(super) fn check_neighbour_ink(
    (src, dst): (&Raster, &Raster),
    geom: &PageGeometry,
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    near: &NearGlyphs,
    neighbours: &[[f64; 4]],
    dpi: u32,
) -> Result<(), Fail> {
    let (w, h) = (src.w as usize, src.h as usize);
    if neighbours.is_empty() || dst.w != src.w || dst.h != src.h {
        return Ok(());
    }
    let per_edit = pixel_masks(edits, geom, dpi, w, h);
    let layers = Layers::new(edits, near, &per_edit, geom, dpi, (w, h));
    let masks: Vec<PixelBox> = per_edit.into_iter().flatten().collect();
    let mut budget = w.saturating_mul(h).saturating_mul(WORK_PER_RASTER_PIXEL);
    let too_much = || ("render", "too many neighbour words to compare".to_string());
    let mut lost = 0u64;
    // Per layer and row of a neighbour box: +1 where a box starts covering, −1 where it stops.
    let mut diff: [Vec<i64>; 4] = Default::default();
    for b in neighbours
        .iter()
        .filter_map(|r| frame_to_pixels(r, dpi, w, h))
    {
        let near_masks: Vec<&PixelBox> = masks.iter().filter(|m| overlaps(m, &b)).collect();
        if near_masks.is_empty() {
            continue;
        }
        let candidates = (layers.kept.len())
            .saturating_add(layers.own.len())
            .saturating_add(layers.old.len());
        budget = budget.checked_sub(candidates).ok_or_else(too_much)?;
        let layered: Vec<(usize, &PixelBox)> = (near_masks.into_iter().map(|m| (SLACK, m)))
            .chain(layers.kept.iter().map(|m| (KEPT, m)))
            .chain(layers.own.iter().map(|m| (OWN, m)))
            .chain(layers.old.iter().map(|m| (OLD, m)))
            .filter(|(_, m)| overlaps(m, &b))
            .collect();
        let (width, height) = (b.x1 - b.x0, b.y1 - b.y0);
        let work = width
            .saturating_mul(diff.len())
            .saturating_add(layered.len())
            .saturating_mul(height)
            .saturating_mul(2);
        budget = budget.checked_sub(work).ok_or_else(too_much)?;
        let ink = paper(src, w, &b);
        for y in b.y0..b.y1 {
            for d in diff.iter_mut() {
                d.clear();
                d.resize(width.saturating_add(1), 0);
            }
            for (layer, m) in layered.iter().filter(|(_, m)| m.y0 <= y && y < m.y1) {
                let from = m.x0.max(b.x0) - b.x0;
                let to = m.x1.min(b.x1).saturating_sub(b.x0);
                if let (true, Some(d)) = (from < to, diff.get_mut(*layer)) {
                    if let Some(v) = d.get_mut(from) {
                        *v = v.saturating_add(1);
                    }
                    if let Some(v) = d.get_mut(to) {
                        *v = v.saturating_sub(1);
                    }
                }
            }
            let mut cover = [0i64; 4];
            for dx in 0..width {
                for (c, d) in cover.iter_mut().zip(&diff) {
                    *c = c.saturating_add(d.get(dx).copied().unwrap_or(0));
                }
                let masked = cover[SLACK] > 0 && (cover[KEPT] <= 0 || cover[OWN] > 0);
                if !masked || cover[OLD] > 0 {
                    continue;
                }
                let x = b.x0 + dx;
                let (a, z) = (pixel(src, w, x, y), pixel(dst, w, x, y));
                if !close(a, &ink) && close(z, &ink) {
                    lost = lost.saturating_add(1);
                }
            }
        }
    }
    if lost > RENDER_KEPT_PIXELS_MAX {
        return Err((
            "render",
            format!(
                "next to the edited line: a neighbour lost its ink under the edit, pixels={lost}"
            ),
        ));
    }
    Ok(())
}
