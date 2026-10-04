//! One run from its joined records (SPEC §B.11 pipeline steps 3–5): units (glyphs, TJ kerns and
//! inter-member gaps converted to kerns), synthetic spaces (§A.3.4), text, caret offsets, ink box
//! and extents; then the page-wide run checks (DUPLICATE_TEXT, PER_GLYPH_TEXT) and the distance
//! to the next text on the same line.

use super::surface::{unit_heap, SurfaceInfo};
use super::{KernSrc, SpaceMode, TextRun, Unit};
use crate::pdf_engine::text_edit::geometry::{
    bbox_of, cross, dot, intersect, mul, ray_extent, sub, unit,
};
use crate::pdf_engine::text_edit::limits::{
    DEFAULT_KERN_SPACE, DUPLICATE_OVERLAP_SHARE, NEGLIGIBLE_KERN, PER_GLYPH_MIN_RUNS,
    PER_GLYPH_SHARE, SYNTH_SPACE_EM,
};
use crate::pdf_engine::text_edit::reasons::{Face, TextReason};
use crate::pdf_engine::text_edit::state::ClipState;
use crate::pdf_engine::text_edit::walker::budget::ModelBudget;
use crate::pdf_engine::text_edit::walker::{PageWalk, RecElem, ShowRecord, Stop};
use std::collections::{BTreeMap, HashMap};
use std::mem::size_of;
use std::sync::Arc;

const FALLBACK_ASCENT: f64 = 0.8;
const FALLBACK_DESCENT: f64 = -0.2;
/// Band overlap that makes another run "on the same line" (§B.11 `next_obstacle`).
const SAME_LINE_SHARE: f64 = 0.1;
/// Run pairs `next_obstacles` compares on one page. Past it, every run not yet measured gets an
/// obstacle at its own origin: a `NEXT_TEXT_OVERLAP` warning on any growth, never a missed one.
const OBSTACLE_PAIRS_MAX: usize = 8_000_000;

/// `|row 1|` and `|row 2|` of `Tm × CTM` at the record's start.
fn tm_ctm_norms(rec: &ShowRecord) -> (f64, f64) {
    let m = mul(&rec.tm_before, &rec.before.ctm);
    (m[0].hypot(m[1]), m[2].hypot(m[3]))
}

fn is_space(text: &str) -> bool {
    matches!(text, " " | "\u{a0}")
}

/// Builds the run of `members` (record indices, primary first) with its record-level reasons,
/// typed with `surface`; what it keeps is charged to `mem` before it is made.
pub(super) fn assemble(
    walk: &PageWalk,
    members: &[usize],
    reasons: Vec<TextReason>,
    surface: &SurfaceInfo,
    mem: &mut ModelBudget,
) -> Result<TextRun, Stop> {
    mem.scratch(members.len() * size_of::<&ShowRecord>())?;
    let recs: Vec<&ShowRecord> = members
        .iter()
        .filter_map(|i| walk.records.get(*i))
        .collect();
    mem.unscratch(members.len() * size_of::<&ShowRecord>());
    let Some(primary) = recs.first().copied() else {
        return Ok(empty_run(reasons));
    };
    let ttu = primary.text_to_user;
    let dir = unit((ttu[0], ttu[1])).unwrap_or((1.0, 0.0));
    let up = if cross(dir, (ttu[2], ttu[3])) >= 0.0 {
        (-dir.1, dir.0)
    } else {
        (dir.1, -dir.0)
    };
    let t = &primary.before.text;
    let (ttux, row2) = tm_ctm_norms(primary);
    let effective_size = t.tfs.abs() * row2;
    let origin = primary.pen_before;
    let units = build_units(&recs, members, dir, ttux, mem)?;
    let (text, caret_offsets) = text_and_carets(walk, primary, &units, origin, dir, ttux, mem)?;
    let corners: Vec<(f64, f64)> = recs
        .iter()
        .flat_map(|r| r.glyphs.iter())
        .flat_map(|g| [(g.bbox[0], g.bbox[1]), (g.bbox[2], g.bbox[3])])
        .collect();
    let ink = bbox_of(&corners).unwrap_or([origin.0, origin.1, origin.0, origin.1]);
    let (mut ascent, mut descent) = (0.0f64, 0.0f64);
    for r in &recs {
        let (a, d) = r
            .font
            .as_ref()
            .map_or((FALLBACK_ASCENT, FALLBACK_DESCENT), |f| {
                (f.ascent, f.descent)
            });
        ascent = ascent.max(a * effective_size);
        descent = descent.max(-d * effective_size);
    }
    let original_extent = recs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(move |g| (r, g)))
        .map(|(r, g)| {
            let w = g.width1000 / 1000.0 * (r.before.text.tfs * r.before.text.th * ttux).abs();
            dot(sub(g.origin, origin), dir) + w
        })
        .fold(0.0, f64::max);
    let visible = match &primary.before.clip {
        ClipState::Rect(r) => intersect(walk.geometry.visible, *r),
        _ => walk.geometry.visible,
    };
    let synth: Vec<f64> = units
        .iter()
        .filter_map(|u| match u {
            Unit::Kern {
                value,
                synth_space: true,
                ..
            } => Some(*value),
            _ => None,
        })
        .collect();
    let real_space = units
        .iter()
        .any(|u| matches!(u, Unit::Glyph { text, .. } if text == " "));
    let space_mode = if surface.has_space && (synth.is_empty() || real_space) {
        SpaceMode::Glyph
    } else {
        SpaceMode::Kern
    };
    let face = match primary
        .font
        .as_ref()
        .map_or((false, false), |m| (m.bold, m.italic))
    {
        (true, true) => Face::BoldItalic,
        (true, false) => Face::Bold,
        (false, true) => Face::Italic,
        (false, false) => Face::Regular,
    };
    let font_from_extgstate = t.font.as_ref().is_some_and(|f| f.from_extgstate);
    let fill_hex = primary.before.fill.hex();
    let small = reasons.capacity() + fill_hex.as_ref().map_or(0, String::capacity);
    mem.hold(small.saturating_add(members.len() * size_of::<usize>()))?;
    Ok(TextRun {
        id: String::new(),
        order: 0,
        line: 0,
        members: members.to_vec(),
        units,
        text,
        caret_offsets,
        origin,
        dir,
        up,
        ascent,
        descent,
        rect: [ink[0], ink[1], ink[2] - ink[0], ink[3] - ink[1]],
        surface: Arc::clone(&surface.names),
        tfs: t.tfs,
        effective_size,
        tc: t.tc,
        tw: t.tw,
        th: t.th,
        text_to_user_x: ttux,
        space_mode,
        kern_space: median(&synth).unwrap_or(DEFAULT_KERN_SPACE),
        original_extent,
        visible_extent: ray_extent(origin, dir, visible),
        next_obstacle: None,
        fill_hex,
        face,
        font_from_extgstate,
        tr: t.tr,
        substituted: recs
            .iter()
            .any(|r| r.font.as_ref().is_some_and(|f| f.substituted)),
        reason: reasons.first().copied(),
        reasons,
    })
}

fn empty_run(reasons: Vec<TextReason>) -> TextRun {
    TextRun {
        id: String::new(),
        order: 0,
        line: 0,
        members: Vec::new(),
        units: Vec::new(),
        text: String::new(),
        caret_offsets: vec![0.0],
        origin: (0.0, 0.0),
        dir: (1.0, 0.0),
        up: (0.0, 1.0),
        ascent: 0.0,
        descent: 0.0,
        rect: [0.0; 4],
        surface: Arc::from(Vec::new()),
        tfs: 0.0,
        effective_size: 0.0,
        tc: 0.0,
        tw: 0.0,
        th: 1.0,
        text_to_user_x: 0.0,
        space_mode: SpaceMode::Kern,
        kern_space: DEFAULT_KERN_SPACE,
        original_extent: 0.0,
        visible_extent: 0.0,
        next_obstacle: None,
        fill_hex: None,
        face: Face::Regular,
        font_from_extgstate: false,
        tr: 0,
        substituted: false,
        reason: reasons.first().copied(),
        reasons,
    }
}

/// Glyph units, TJ kerns and inter-member gaps (`n = −gap_ts·1000/Tfs` with
/// `gap_ts = gap_user / (Th · text_to_user_x)`; negligible ones omitted), then the
/// synthetic-space marks. Every unit's slot, text and font resource name are charged first (a
/// glyph unit copies its font's resource name: a long name times many glyphs is charged too).
fn build_units(
    recs: &[&ShowRecord],
    members: &[usize],
    dir: (f64, f64),
    ttux: f64,
    mem: &mut ModelBudget,
) -> Result<Vec<Unit>, Stop> {
    let mut slots = recs.len().saturating_sub(1);
    let mut heap = 0usize;
    for rec in recs {
        slots = slots.saturating_add(rec.elems.len());
        let name = rec
            .before
            .text
            .font
            .as_ref()
            .and_then(|f| f.resource.as_ref())
            .map_or(0, |r| r.len());
        for g in &rec.glyphs {
            let text = g.text.as_ref().map_or(3, String::len);
            heap = heap.saturating_add(text.saturating_add(name));
        }
    }
    mem.hold(slots.saturating_mul(size_of::<Unit>()).saturating_add(heap))?;
    let mut units = Vec::with_capacity(slots);
    let mut prev: Option<&ShowRecord> = None;
    for (k, (rec, idx)) in recs.iter().zip(members).enumerate() {
        let tfs = rec.before.text.tfs;
        let font = rec.before.text.font.as_ref();
        if let Some(p) = prev {
            let gap_user = dot(sub(rec.pen_before, p.pen_after), dir);
            let scale = rec.before.text.th * ttux;
            let n = if scale != 0.0 && tfs != 0.0 {
                -(gap_user / scale) * 1000.0 / tfs
            } else {
                0.0
            };
            if n.is_finite() && n.abs() >= NEGLIGIBLE_KERN {
                units.push(Unit::Kern {
                    value: n,
                    src: KernSrc::Gap,
                    synth_space: false,
                });
            }
        }
        prev = Some(rec);
        for elem in &rec.elems {
            match elem {
                RecElem::Glyph(gi) => {
                    let Some(g) = rec.glyphs.get(*gi) else {
                        continue;
                    };
                    units.push(Unit::Glyph {
                        member: k,
                        rec: *idx,
                        glyph: *gi,
                        code: g.code,
                        font_res: font.and_then(|f| f.resource.as_deref()).map(<[u8]>::to_vec),
                        font_hash: font.map_or(0, |f| f.content_hash),
                        text: g.text.clone().unwrap_or_else(|| "\u{fffd}".to_string()),
                        width1000: g.width1000,
                    });
                }
                RecElem::Kern { value, span } => units.push(Unit::Kern {
                    value: *value,
                    src: KernSrc::Tj { span: span.clone() },
                    synth_space: false,
                }),
            }
        }
    }
    mark_synthetic_spaces(&mut units);
    let kept: usize = units.iter().map(unit_heap).sum();
    mem.unhold(heap.saturating_sub(kept));
    Ok(units)
}

/// A TJ number or converted gap with `−n/1000 ≥ 0.2 em` between two non-space glyphs reads as
/// exactly one space (§A.3.4): in a sequence of kerns between two glyphs the sum decides, and the
/// largest one carries the space.
fn mark_synthetic_spaces(units: &mut [Unit]) {
    let glyph_at: Vec<usize> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| matches!(u, Unit::Glyph { .. }))
        .map(|(i, _)| i)
        .collect();
    for pair in glyph_at.windows(2) {
        let [a, b] = pair else { continue };
        let (a, b) = (*a, *b);
        let space_at = |i: usize| match units.get(i) {
            Some(Unit::Glyph { text, .. }) => is_space(text),
            _ => true,
        };
        if b <= a + 1 || space_at(a) || space_at(b) {
            continue;
        }
        let kerns: Vec<(usize, f64)> = (a + 1..b)
            .filter_map(|i| match units.get(i) {
                Some(Unit::Kern { value, .. }) => Some((i, *value)),
                _ => None,
            })
            .collect();
        let total: f64 = kerns.iter().map(|(_, v)| -v / 1000.0).sum();
        if total < SYNTH_SPACE_EM {
            continue;
        }
        let biggest = kerns
            .iter()
            .fold(None::<(usize, f64)>, |best, &(i, v)| match best {
                Some((_, bv)) if bv <= v => best,
                _ => Some((i, v)),
            });
        if let Some((i, _)) = biggest {
            if let Some(Unit::Kern { synth_space, .. }) = units.get_mut(i) {
                *synth_space = true;
            }
        }
    }
}

/// The run text (synthetic spaces as " ") and its chars+1 caret offsets along `dir` from the
/// origin, made monotonic; both are sized (and charged) exactly before they are filled.
fn text_and_carets(
    walk: &PageWalk,
    primary: &ShowRecord,
    units: &[Unit],
    origin: (f64, f64),
    dir: (f64, f64),
    ttux: f64,
    mem: &mut ModelBudget,
) -> Result<(String, Vec<f64>), Stop> {
    let (tfs, th) = (primary.before.text.tfs, primary.before.text.th);
    let (mut bytes, mut chars) = (0usize, 1usize);
    for u in units {
        let (b, c) = match u {
            Unit::Glyph { text: t, .. } => (t.len(), t.chars().count()),
            Unit::Kern {
                synth_space: true, ..
            } => (1, 1),
            Unit::Kern { .. } => (0, 0),
        };
        bytes = bytes.saturating_add(b);
        chars = chars.saturating_add(c);
    }
    mem.hold(bytes.saturating_add(chars.saturating_mul(size_of::<f64>())))?;
    let mut text = String::with_capacity(bytes);
    let mut offsets = Vec::with_capacity(chars);
    let mut end = 0.0f64;
    let mut pen = 0.0f64;
    for u in units {
        match u {
            Unit::Glyph {
                rec,
                glyph,
                text: t,
                ..
            } => {
                let Some(g) = walk.records.get(*rec).and_then(|r| r.glyphs.get(*glyph)) else {
                    continue;
                };
                let start = dot(sub(g.origin, origin), dir);
                let adv = dot(g.advance_user, dir);
                let n = t.chars().count();
                for j in 0..n {
                    offsets.push(start + adv * j as f64 / n as f64);
                }
                text.push_str(t);
                pen = start + adv;
                end = pen;
            }
            Unit::Kern {
                value,
                synth_space,
                src,
            } => {
                let adv = -value / 1000.0 * tfs * th * ttux;
                if *synth_space {
                    offsets.push(pen);
                    text.push(' ');
                    pen += adv;
                    end = pen;
                } else if matches!(src, KernSrc::Tj { .. }) {
                    pen += adv;
                }
            }
        }
    }
    offsets.push(end);
    let mut max = f64::NEG_INFINITY;
    for o in &mut offsets {
        max = max.max(*o);
        *o = max;
    }
    Ok((text, offsets))
}

fn median(values: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied()
}

fn area(r: [f64; 4]) -> f64 {
    r[2].max(0.0) * r[3].max(0.0)
}

fn overlap_area(a: [f64; 4], b: [f64; 4]) -> f64 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    w.max(0.0) * h.max(0.0)
}

/// DUPLICATE_TEXT (§A.6): two runs with the same trimmed text whose ink boxes overlap by at
/// least half of the smaller one are both refused.
///
/// Per text, runs are swept by `x`; memory is one flag per run. Once a run is marked it is only
/// compared with runs not marked yet, found through `Unmarked` (a stack of identical shadows costs
/// O(n), not O(n²)).
pub(super) fn mark_duplicates(runs: &mut [TextRun]) {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in runs.iter().enumerate() {
        let key = r.text.trim();
        if !key.is_empty() {
            groups.entry(key).or_default().push(i);
        }
    }
    let mut marked = vec![false; runs.len()];
    for mut idx in groups.into_values() {
        let x_of = |i: &usize| runs.get(*i).map_or(0.0, |r| r.rect[0]);
        idx.sort_by(|a, b| x_of(a).total_cmp(&x_of(b)));
        let mut unmarked = Unmarked::new(idx.len());
        for (k, &i) in idx.iter().enumerate() {
            let Some(a) = runs.get(i) else { continue };
            let mut a_marked = marked.get(i).copied().unwrap_or(true);
            let mut p = if a_marked {
                unmarked.find(k + 1)
            } else {
                k + 1
            };
            while let Some((&j, b)) = idx.get(p).and_then(|j| Some((j, runs.get(*j)?))) {
                if b.rect[0] > a.rect[0] + a.rect[2] {
                    break; // sorted by x: no later run overlaps `a`
                }
                let smaller = area(a.rect).min(area(b.rect));
                let shared = overlap_area(a.rect, b.rect);
                if smaller > 0.0 && shared >= DUPLICATE_OVERLAP_SHARE * smaller {
                    for (slot, pos) in [(i, k), (j, p)] {
                        if let Some(m) = marked.get_mut(slot) {
                            *m = true;
                        }
                        unmarked.mark(pos);
                    }
                    a_marked = true;
                }
                p = if a_marked {
                    unmarked.find(p + 1)
                } else {
                    p + 1
                };
            }
        }
    }
    for (r, hit) in runs.iter_mut().zip(marked) {
        if hit {
            add_reason(r, TextReason::DuplicateText);
        }
    }
}

/// The next unmarked position at or after `p` in one sorted group (`len` when none): a
/// path-compressed successor list, `next[p] == p` while `p` is unmarked.
struct Unmarked {
    next: Vec<usize>,
}

impl Unmarked {
    fn new(len: usize) -> Unmarked {
        Unmarked {
            next: (0..=len).collect(),
        }
    }

    fn mark(&mut self, p: usize) {
        if let Some(slot) = self.next.get_mut(p) {
            *slot = p.saturating_add(1);
        }
    }

    fn find(&mut self, p: usize) -> usize {
        let end = self.next.len().saturating_sub(1);
        let mut root = p.min(end);
        while let Some(&n) = self.next.get(root) {
            if n == root || n > end {
                break;
            }
            root = n;
        }
        let mut cur = p.min(end);
        while cur != root {
            let Some(slot) = self.next.get_mut(cur) else {
                break;
            };
            let n = *slot;
            *slot = root;
            cur = n;
        }
        root
    }
}

/// PER_GLYPH_TEXT (§A.1.2): with ≥ 12 editable non-blank runs of which ≥ 80 % are one code
/// point long, those one-code-point runs are refused.
pub(super) fn mark_per_glyph(runs: &mut [TextRun]) {
    let editable: Vec<usize> = runs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.reasons.is_empty() && !r.text.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    let single: Vec<usize> = editable
        .iter()
        .copied()
        .filter(|i| runs.get(*i).is_some_and(|r| r.text.chars().count() == 1))
        .collect();
    let share = single.len() as f64 / editable.len().max(1) as f64;
    if editable.len() >= PER_GLYPH_MIN_RUNS && share >= PER_GLYPH_SHARE {
        for i in single {
            if let Some(r) = runs.get_mut(i) {
                add_reason(r, TextReason::PerGlyphText);
            }
        }
    }
}

pub(super) fn add_reason(run: &mut TextRun, reason: TextReason) {
    if !run.reasons.contains(&reason) {
        run.reasons.push(reason);
        run.reasons.sort();
    }
    run.reason = run.reasons.first().copied();
}

/// `next_obstacle` (§B.11): the distance along `dir` to the nearest other run with the same
/// direction that starts ahead of this run's origin and whose band (descent..ascent across the
/// baseline) overlaps this run's band by more than 10 %.
///
/// Runs are bucketed by direction, then by band height (powers of two), each class sorted by its
/// position across the baseline: a run only looks at the runs of each class whose band can reach
/// its own, so one tall title does not widen every other run's search. At most
/// `OBSTACLE_PAIRS_MAX` pairs are compared per page (in a fixed order); a run whose search the
/// cap cuts short gets an obstacle at its own origin (conservative: a warning on any growth).
pub(super) fn next_obstacles(runs: &mut [TextRun]) {
    let dir_key = |r: &TextRun| {
        (
            (r.dir.0 * 1e4).round() as i64,
            (r.dir.1 * 1e4).round() as i64,
        )
    };
    let mut buckets: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, r) in runs.iter().enumerate() {
        buckets.entry(dir_key(r)).or_default().push(i);
    }
    let mut left = OBSTACLE_PAIRS_MAX;
    let mut found: Vec<(usize, f64)> = Vec::new();
    for idx in buckets.values() {
        let Some(up) = idx.first().and_then(|i| runs.get(*i)).map(|r| r.up) else {
            continue;
        };
        let perp = |i: usize| runs.get(i).map_or(0.0, |r| dot(r.origin, up));
        let classes = band_classes(runs, idx, &perp);
        let mut order = idx.clone();
        order.sort_by(|a, b| perp(*a).total_cmp(&perp(*b)).then(a.cmp(b)));
        for i in order {
            let Some(a) = runs.get(i) else { continue };
            let pa = perp(i);
            let (a_lo, a_hi) = (pa - a.descent, pa + a.ascent);
            let band = (a_hi - a_lo).max(1e-9);
            let mut best: Option<f64> = None;
            let mut cut = false;
            for class in classes.values() {
                // `b` can overlap only if its baseline lies within this window.
                let lo = a_lo - class.ascent;
                let hi = a_hi + class.descent;
                let start = class.sorted.partition_point(|j| perp(*j) < lo);
                for &j in class.sorted.iter().skip(start) {
                    let pb = perp(j);
                    if pb > hi {
                        break;
                    }
                    let Some(next) = left.checked_sub(1) else {
                        cut = true;
                        break;
                    };
                    left = next;
                    let Some(b) = runs.get(j).filter(|_| j != i) else {
                        continue;
                    };
                    let shared = a_hi.min(pb + b.ascent) - a_lo.max(pb - b.descent);
                    let ahead = dot(sub(b.origin, a.origin), a.dir);
                    if shared > SAME_LINE_SHARE * band && ahead > 1e-6 {
                        best = Some(best.map_or(ahead, |x: f64| x.min(ahead)));
                    }
                }
            }
            match (cut, best) {
                (true, _) => found.push((i, 0.0)),
                (false, Some(d)) => found.push((i, d)),
                (false, None) => {}
            }
        }
    }
    for (i, d) in found {
        if let Some(r) = runs.get_mut(i) {
            r.next_obstacle = Some(d);
        }
    }
}

/// The runs of one direction bucket grouped by band height (`floor(log2(height))`): each class
/// sorted by `perp`, with the largest ascent and descent of its members.
struct BandClass {
    sorted: Vec<usize>,
    ascent: f64,
    descent: f64,
}

fn band_classes(
    runs: &[TextRun],
    idx: &[usize],
    perp: &impl Fn(usize) -> f64,
) -> BTreeMap<i32, BandClass> {
    let mut classes: BTreeMap<i32, BandClass> = BTreeMap::new();
    for &i in idx {
        let Some(r) = runs.get(i) else { continue };
        let height = (r.ascent + r.descent).max(1e-9);
        let key = if height.is_finite() {
            height.log2().floor().clamp(-64.0, 64.0) as i32
        } else {
            64
        };
        let class = classes.entry(key).or_insert(BandClass {
            sorted: Vec::new(),
            ascent: 0.0,
            descent: 0.0,
        });
        class.sorted.push(i);
        class.ascent = class.ascent.max(r.ascent);
        class.descent = class.descent.max(r.descent);
    }
    for class in classes.values_mut() {
        class
            .sorted
            .sort_by(|a, b| perp(*a).total_cmp(&perp(*b)).then(a.cmp(b)));
    }
    classes
}
