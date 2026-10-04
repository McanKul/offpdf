//! Strict page geometry and text orientation (SPEC §A.5, §B.10).
//!
//! Matrices use the PDF row-vector convention: `mul(a, b)` is "a × b" (apply `a` first, then
//! `b`), so a glyph's rendering matrix is `mul(&mul(&text_params, &tm), &ctm)`. Page boxes are
//! read by an own parser that refuses everything a viewer would have to guess (`crop.rs` falls
//! back silently and is only used by the parity test GEO-11).

use crate::pdf_engine::text_edit::fonts::number_of;
use crate::pdf_engine::text_edit::limits::{AXIS_EPSILON_REL, PAGE_TREE_DEPTH_MAX, SHEAR_MAX};
use crate::pdf_engine::text_edit::reasons::TextReason;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

pub type Matrix = [f64; 6];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// References followed for one box or number value.
const REF_HOPS_MAX: usize = 16;
/// Smallest visible side (Crop ∩ Media) in points.
const VISIBLE_SIDE_MIN_PT: f64 = 1.0;

/// PDF "a × b": the transformation that applies `a`, then `b`.
pub fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let [a0, a1, a2, a3, a4, a5] = *a;
    let [b0, b1, b2, b3, b4, b5] = *b;
    [
        a0 * b0 + a1 * b2,
        a0 * b1 + a1 * b3,
        a2 * b0 + a3 * b2,
        a2 * b1 + a3 * b3,
        a4 * b0 + a5 * b2 + b4,
        a4 * b1 + a5 * b3 + b5,
    ]
}

/// The point `(x, y)` mapped by `m`.
pub fn apply(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

/// The vector `(x, y)` mapped by the linear part of `m`.
pub fn apply_linear(m: &Matrix, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2], x * m[1] + y * m[3])
}

pub fn is_finite(m: &Matrix) -> bool {
    m.iter().all(|v| v.is_finite())
}

/// A translation by `(tx, ty)`.
pub fn translate(tx: f64, ty: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

/// R(0/90/180/270) of §A.5: clockwise display rotation in the row-vector convention. Any other
/// value is treated as 0 (callers only pass normalised `/Rotate` values).
pub fn display_rotation(rotate: i64) -> Matrix {
    match rotate.rem_euclid(360) {
        90 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        270 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        _ => IDENTITY,
    }
}

/// Page boxes as `[x0, y0, x1, y1]` (normalised, default user space).
#[derive(Debug, Clone, PartialEq)]
pub struct PageGeometry {
    pub media: [f64; 4],
    pub crop: [f64; 4],
    pub visible: [f64; 4],
    pub rotate: i64,
    pub user_unit: f64,
}

impl PageGeometry {
    /// Geometry used by the classifier on a page refused `GEOMETRY` (§B.10): no box, so nothing is
    /// "outside the page"; occurrences carry the page reason instead.
    pub fn unbounded(rotate: i64) -> PageGeometry {
        let all = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
        PageGeometry {
            media: all,
            crop: all,
            visible: all,
            rotate,
            user_unit: 1.0,
        }
    }
}

/// The page's geometry, read strictly (§A.5): the inherited `/MediaBox` must exist and be four
/// finite numbers with a positive area; an inherited `/CropBox` likewise when present; Crop ∩ Media
/// must be at least 1 pt in both directions; `/Rotate` (inherited) an Integer multiple of 90;
/// `/UserUnit` (page only) absent or exactly the number 1. Anything else is `GEOMETRY`.
pub fn page_geometry(doc: &Document, page_id: ObjectId) -> Result<PageGeometry, TextReason> {
    let bad = TextReason::Geometry;
    let chain = page_chain(doc, page_id).ok_or(bad)?;
    let page = chain.first().copied().ok_or(bad)?;
    let media = inherited(&chain, b"MediaBox")
        .ok_or(bad)
        .and_then(|o| read_box(doc, o).ok_or(bad))?;
    let crop = match inherited(&chain, b"CropBox") {
        None => media,
        Some(o) => read_box(doc, o).ok_or(bad)?,
    };
    let visible = [
        crop[0].max(media[0]),
        crop[1].max(media[1]),
        crop[2].min(media[2]),
        crop[3].min(media[3]),
    ];
    if visible[2] - visible[0] < VISIBLE_SIDE_MIN_PT
        || visible[3] - visible[1] < VISIBLE_SIDE_MIN_PT
    {
        return Err(bad);
    }
    let rotate = match inherited(&chain, b"Rotate") {
        None => 0,
        Some(o) => match resolve(doc, o) {
            Some(Object::Integer(r)) if r.rem_euclid(90) == 0 => r.rem_euclid(360),
            _ => return Err(bad),
        },
    };
    let user_unit = match page.get(b"UserUnit").ok() {
        None => 1.0,
        Some(o) => match resolve(doc, o).and_then(number_of) {
            Some(u) if u == 1.0 => 1.0,
            _ => return Err(bad),
        },
    };
    Ok(PageGeometry {
        media,
        crop,
        visible,
        rotate,
        user_unit,
    })
}

/// The page dictionary and its ancestors (nearest first); `None` on a dangling or cyclic chain.
fn page_chain(doc: &Document, page_id: ObjectId) -> Option<Vec<&Dictionary>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut cur = Some(page_id);
    while let Some(id) = cur {
        if !seen.insert(id) || out.len() > PAGE_TREE_DEPTH_MAX {
            return None;
        }
        let dict = match doc.objects.get(&id)? {
            Object::Dictionary(d) => d,
            _ => return None,
        };
        out.push(dict);
        cur = match dict.get(b"Parent").ok() {
            None => None,
            Some(Object::Reference(p)) => Some(*p),
            Some(_) => return None,
        };
    }
    Some(out)
}

fn inherited<'a>(chain: &[&'a Dictionary], key: &[u8]) -> Option<&'a Object> {
    chain.iter().find_map(|d| d.get(key).ok())
}

/// Follows references (≤ 16 hops); `None` when one dangles.
fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    let mut obj = obj;
    for _ in 0..REF_HOPS_MAX {
        match obj {
            Object::Reference(id) => obj = doc.objects.get(id)?,
            other => return Some(other),
        }
    }
    None
}

/// Four finite numbers, normalised, with a positive area.
fn read_box(doc: &Document, obj: &Object) -> Option<[f64; 4]> {
    let items = match resolve(doc, obj)? {
        Object::Array(items) if items.len() == 4 => items,
        _ => return None,
    };
    let mut v = [0.0; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = resolve(doc, item).and_then(number_of)?;
    }
    let b = [
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ];
    (b[2] > b[0] && b[3] > b[1]).then_some(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Upright,
    ZeroSize,
    Rotated,
    Mirrored,
    Skewed,
}

/// §A.5 table for the composite `T = [Tfs·Th 0 0 Tfs 0 Ts] × Tm × CTM × R(rotate)`. `s` is the
/// largest absolute linear coefficient (the spec's `max(|a|,|d|)` extended to `|b|`, `|c|` so that
/// off-axis tolerances scale too); a rotation needs `a ≈ d` and `b ≈ −c` (signed).
pub fn classify_orientation(t: &Matrix) -> Orientation {
    let [a, b, c, d, _, _] = *t;
    if ![a, b, c, d].iter().all(|v| v.is_finite()) {
        return Orientation::Skewed;
    }
    let s = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if (a * d - b * c).abs() < 1e-12 * (s * s).max(1.0) {
        return Orientation::ZeroSize;
    }
    let tol = AXIS_EPSILON_REL * s;
    if b.abs() <= tol && c.abs() <= SHEAR_MAX * d.abs() && a > 0.0 && d > 0.0 {
        return Orientation::Upright;
    }
    if b.abs() <= tol && c.abs() <= tol {
        if a < 0.0 && d < 0.0 {
            return Orientation::Rotated;
        }
        if (a < 0.0) != (d < 0.0) {
            return Orientation::Mirrored;
        }
    }
    if (a - d).abs() <= tol && (b + c).abs() <= tol {
        return Orientation::Rotated;
    }
    Orientation::Skewed
}

/// Distance from `origin` along the unit vector `dir` to the edge of `rect` (`[x0, y0, x1, y1]`);
/// 0 when the origin is outside or the ray misses it.
pub fn ray_extent(origin: (f64, f64), dir: (f64, f64), rect: [f64; 4]) -> f64 {
    let mut t_min = f64::NEG_INFINITY;
    let mut t_max = f64::INFINITY;
    for (o, d, lo, hi) in [
        (origin.0, dir.0, rect[0], rect[2]),
        (origin.1, dir.1, rect[1], rect[3]),
    ] {
        if d.abs() < 1e-12 {
            if o < lo || o > hi {
                return 0.0;
            }
            continue;
        }
        let (t0, t1) = ((lo - o) / d, (hi - o) / d);
        t_min = t_min.max(t0.min(t1));
        t_max = t_max.min(t0.max(t1));
    }
    if t_min > 1e-9 || t_max < 0.0 || t_min > t_max || !t_max.is_finite() {
        return 0.0;
    }
    t_max
}

/// Axis-aligned bounding box of `points`; `None` when empty.
pub fn bbox_of(points: &[(f64, f64)]) -> Option<[f64; 4]> {
    let mut it = points.iter();
    let &(x, y) = it.next()?;
    Some(it.fold([x, y, x, y], |b, &(x, y)| {
        [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
    }))
}

/// AABB of `rect` (`[x0, y0, x1, y1]`) mapped by `m`.
pub fn transform_rect(m: &Matrix, rect: [f64; 4]) -> [f64; 4] {
    let corners = [
        apply(m, rect[0], rect[1]),
        apply(m, rect[2], rect[1]),
        apply(m, rect[0], rect[3]),
        apply(m, rect[2], rect[3]),
    ];
    bbox_of(&corners).unwrap_or(rect)
}

/// Whether `m` maps axis-aligned rectangles to axis-aligned rectangles (no shear or off-axis
/// rotation other than multiples of 90°).
pub fn axis_aligned(m: &Matrix) -> bool {
    let s = m[0].abs().max(m[1].abs()).max(m[2].abs()).max(m[3].abs());
    let tol = AXIS_EPSILON_REL * s;
    (m[1].abs() <= tol && m[2].abs() <= tol) || (m[0].abs() <= tol && m[3].abs() <= tol)
}

/// Intersection of two `[x0, y0, x1, y1]` rectangles (possibly empty: x1 < x0 kept as zero size).
pub fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    [r[0], r[1], r[2].max(r[0]), r[3].max(r[1])]
}

/// Whether `inner` lies inside `outer` within `tol` on every side.
pub fn contains(outer: [f64; 4], inner: [f64; 4], tol: f64) -> bool {
    inner[0] >= outer[0] - tol
        && inner[1] >= outer[1] - tol
        && inner[2] <= outer[2] + tol
        && inner[3] <= outer[3] + tol
}

/// Unit vector of `v`; `None` when its length is not positive and finite.
pub fn unit(v: (f64, f64)) -> Option<(f64, f64)> {
    let n = v.0.hypot(v.1);
    (n.is_finite() && n > 1e-12).then(|| (v.0 / n, v.1 / n))
}

pub fn dot(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.0 + a.1 * b.1
}

pub fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

pub fn sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}
