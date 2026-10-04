//! Graphics state, its id-free digest and the comparisons joins and verification use (SPEC §B.10,
//! D32, D35). Identities are resource names plus content hashes, never lopdf object ids, so a
//! digest taken on qpdf's output (which renumbers every object) compares with the source's.
//!
//! Every variable-size part of the state (verbatim op bytes, names, the dash array, unmodelled
//! ExtGState keys) is shared through an `Arc`, and the marked-content stack is a persistent list
//! (`marked::MarkedStack`): a digest is taken for every show op and paint, so cloning one must
//! cost the same whatever the content holds. Whole digests are shared too:
//! `StateDigest::is_digest_of` tells in O(1) whether the last digest still describes the state,
//! so a run of records under one state holds a single digest.

mod marked;

pub use marked::{MarkedNode, MarkedStack};

use crate::pdf_engine::text_edit::fonts::{FontKey, FontModel};
use crate::pdf_engine::text_edit::geometry::{Matrix, IDENTITY};
use crate::pdf_engine::text_edit::limits::{COLOR_EPSILON, STATE_EPSILON};
use std::collections::HashSet;
use std::sync::Arc;

/// Shared, immutable bytes (verbatim op spans, names). Cloning never copies them.
pub type Bytes = Arc<[u8]>;

/// What a paint looks like, for display only ("Original colour", `fill_hex`).
#[derive(Debug, Clone, PartialEq)]
pub enum ColorEffect {
    Rgb([f64; 3]),
    Unreadable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorSpaceKind {
    /// Never set in this page (the initial DeviceGray black).
    Default,
    DeviceGray,
    DeviceRgb,
    DeviceCmyk,
    /// A `/ColorSpace` resource: its name and the deep hash of what it names.
    Named(Bytes, u64),
    Pattern,
}

/// A fill or stroke paint. `space_op`/`color_op` are the verbatim bytes of the ops that set it
/// (the whole op span), used for verbatim restores (B14). `pattern_hash` is the deep hash of the
/// `/Pattern` resource an `scn`/`SCN` names (its name is in `color_op`), so a pattern swapped
/// behind the same name compares unequal.
#[derive(Debug, Clone, PartialEq)]
pub struct Paint {
    pub space_op: Option<Bytes>,
    pub color_op: Option<Bytes>,
    pub space: ColorSpaceKind,
    pub comps: Vec<f64>,
    pub effect: ColorEffect,
    pub pattern: bool,
    pub pattern_hash: Option<u64>,
}

impl Paint {
    /// The initial paint: DeviceGray black, never set.
    pub fn initial() -> Paint {
        Paint {
            space_op: None,
            color_op: None,
            space: ColorSpaceKind::Default,
            comps: vec![0.0],
            effect: ColorEffect::Rgb([0.0; 3]),
            pattern: false,
            pattern_hash: None,
        }
    }

    /// `#rrggbb` when the effect is readable.
    pub fn hex(&self) -> Option<String> {
        let ColorEffect::Rgb(rgb) = &self.effect else {
            return None;
        };
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(format!(
            "#{:02x}{:02x}{:02x}",
            byte(rgb[0]),
            byte(rgb[1]),
            byte(rgb[2])
        ))
    }
}

/// The RGB effect of components in a device space; `Unreadable` for anything else.
pub fn color_effect(space: &ColorSpaceKind, comps: &[f64]) -> ColorEffect {
    let ok = comps.iter().all(|c| c.is_finite());
    match (space, comps) {
        (ColorSpaceKind::Default | ColorSpaceKind::DeviceGray, [g]) if ok => {
            ColorEffect::Rgb([*g, *g, *g])
        }
        (ColorSpaceKind::DeviceRgb, [r, g, b]) if ok => ColorEffect::Rgb([*r, *g, *b]),
        (ColorSpaceKind::DeviceCmyk, [c, m, y, k]) if ok => {
            let inv = |v: f64| (1.0 - v.clamp(0.0, 1.0)) * (1.0 - k.clamp(0.0, 1.0));
            ColorEffect::Rgb([inv(*c), inv(*m), inv(*y)])
        }
        _ => ColorEffect::Unreadable,
    }
}

/// The clip in force: none, an intersection of axis-aligned rectangles (`[x0, y0, x1, y1]`, user
/// space), or anything else.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipState {
    None,
    Rect([f64; 4]),
    Complex,
}

/// The font in force. `resource` + `content_hash` are the identity; the lopdf `FontKey` is kept
/// outside the digest (on `GState::font_ref` and the record) because qpdf renumbers objects.
#[derive(Debug, Clone, PartialEq)]
pub struct FontUse {
    pub resource: Option<Bytes>,
    pub content_hash: u64,
    pub from_extgstate: bool,
    pub tf_op: Option<Bytes>, // verbatim "/F1 12 Tf" op bytes
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextParams {
    pub font: Option<FontUse>,
    pub tfs: f64,
    pub tc: f64,
    pub tc_src: Option<Bytes>,
    pub tw: f64,
    pub tw_src: Option<Bytes>,
    pub th: f64,
    pub tl: f64,
    pub tr: i64,
    pub ts: f64,
}

impl Default for TextParams {
    fn default() -> Self {
        TextParams {
            font: None,
            tfs: 0.0,
            tc: 0.0,
            tc_src: None,
            tw: 0.0,
            tw_src: None,
            th: 1.0,
            tl: 0.0,
            tr: 0,
            ts: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GsEffects {
    pub ca: f64,
    pub ca_stroke: f64,
    pub blend: Bytes,
    pub soft_mask: bool,
    pub overprint: (bool, bool, i64),
    pub line_width: f64,
    pub line_cap: i64,
    pub line_join: i64,
    pub miter_limit: f64,
    pub dash: (Arc<[f64]>, f64),
    pub rendering_intent: Bytes,
    pub flatness: f64,
    pub stroke_adjust: bool,
    /// ExtGState keys outside the modelled list that affect rendering (`/TR`, `/HT`, `/BG`, …),
    /// with the canonical hash of their value; sorted by key, one entry per key, last one wins.
    pub other: Arc<[(Bytes, u64)]>,
}

impl Default for GsEffects {
    fn default() -> Self {
        GsEffects {
            ca: 1.0,
            ca_stroke: 1.0,
            blend: Arc::from(&b"Normal"[..]),
            soft_mask: false,
            overprint: (false, false, 0),
            line_width: 1.0,
            line_cap: 0,
            line_join: 0,
            miter_limit: 10.0,
            dash: (Arc::from(Vec::new()), 0.0),
            rendering_intent: Arc::from(&b"RelativeColorimetric"[..]),
            flatness: 1.0,
            stroke_adjust: false,
            other: Arc::from(Vec::new()),
        }
    }
}

impl GsEffects {
    /// The unmodelled keys of one ExtGState as `set_others` takes them: sorted by key, one entry
    /// per key.
    pub fn sorted_others(mut entries: Vec<(Bytes, u64)>) -> Arc<[(Bytes, u64)]> {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|later, earlier| later.0 == earlier.0);
        entries.into()
    }

    /// Records the unmodelled keys of one ExtGState (`sorted_others`; each replaces an earlier
    /// value of the same key) with one merge, O(k + m). When every key is already in force with
    /// the same hash nothing changes and the shared list in force is kept, so re-applying one
    /// ExtGState never allocates a new list (nor, through `Exec::digest`, a new digest). A merged
    /// list equal to one in `interned` (the walk's lists so far) is that list, so a page that
    /// alternates between ExtGStates holds one list per distinct state, not one per `gs`. True
    /// when a new list was interned (the caller's budget keeps it).
    pub fn set_others(
        &mut self,
        entries: &[(Bytes, u64)],
        interned: &mut HashSet<Arc<[(Bytes, u64)]>>,
    ) -> bool {
        let unchanged = entries.iter().all(|(key, hash)| {
            self.other
                .binary_search_by(|(k, _)| k.cmp(key))
                .ok()
                .and_then(|i| self.other.get(i))
                .is_some_and(|(_, h)| h == hash)
        });
        if unchanged {
            return false;
        }
        let mut merged = Vec::with_capacity(self.other.len().saturating_add(entries.len()));
        let mut old = self.other.iter().peekable();
        let mut new = entries.iter().peekable();
        loop {
            let take_old = match (old.peek(), new.peek()) {
                (Some(o), Some(n)) => match o.0.cmp(&n.0) {
                    std::cmp::Ordering::Less => true,
                    std::cmp::Ordering::Equal => {
                        old.next();
                        false
                    }
                    std::cmp::Ordering::Greater => false,
                },
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let next = if take_old { old.next() } else { new.next() };
            merged.extend(next.cloned());
        }
        if let Some(known) = interned.get(merged.as_slice()) {
            self.other = Arc::clone(known);
            return false;
        }
        let list: Arc<[(Bytes, u64)]> = merged.into();
        interned.insert(Arc::clone(&list));
        self.other = list;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedEntry {
    pub tag: Bytes,
    pub mcid: Option<i64>,
    pub actual_text: bool,
    pub oc: Option<OcState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcState {
    Visible,
    Hidden,
    Unknown,
}

/// The graphics state (saved by `q`, restored by `Q`). `font_ref` is the lopdf-side handle of the
/// font in force (key + loaded model) and is never part of a digest.
#[derive(Debug, Clone)]
pub struct GState {
    pub ctm: Matrix,
    pub clip: ClipState,
    pub fill: Paint,
    pub stroke: Paint,
    pub gs: GsEffects,
    pub text: TextParams,
    pub font_ref: Option<(FontKey, Arc<FontModel>)>,
}

impl GState {
    pub fn initial(ctm: Matrix) -> GState {
        GState {
            ctm,
            clip: ClipState::None,
            fill: Paint::initial(),
            stroke: Paint::initial(),
            gs: GsEffects::default(),
            text: TextParams::default(),
            font_ref: None,
        }
    }

    /// O(1) in the size of the content: every variable-size part is shared.
    pub fn digest(&self, marked: &MarkedStack) -> StateDigest {
        StateDigest {
            ctm: self.ctm,
            clip: self.clip.clone(),
            fill: self.fill.clone(),
            stroke: self.stroke.clone(),
            gs: self.gs.clone(),
            text: self.text.clone(),
            marked: marked.clone(),
        }
    }
}

impl Default for GState {
    fn default() -> Self {
        GState::initial(IDENTITY)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateDigest {
    pub ctm: Matrix,
    pub clip: ClipState,
    pub fill: Paint,
    pub stroke: Paint,
    pub gs: GsEffects,
    pub text: TextParams,
    /// The marked-content stack (`iter` gives the innermost entry first).
    pub marked: MarkedStack,
}

impl StateDigest {
    /// Whether this digest is exactly what `gs.digest(marked)` would return, judged in O(1):
    /// every shared part must be the very same allocation and every number equal bit for bit. A
    /// state re-set to an equal value under a new allocation only loses sharing, never
    /// correctness. Lets consecutive records and paints share one digest (`Exec::digest`).
    pub fn is_digest_of(&self, gs: &GState, marked: &MarkedStack) -> bool {
        let StateDigest {
            ctm,
            clip,
            fill,
            stroke,
            gs: effects,
            text,
            marked: stack,
        } = self;
        same_bits(ctm, &gs.ctm)
            && clip.identical(&gs.clip)
            && fill.identical(&gs.fill)
            && stroke.identical(&gs.stroke)
            && effects.identical(&gs.gs)
            && text.identical(&gs.text)
            && stack.same(marked)
    }
}

/// Equality that never walks a shared part (`StateDigest::is_digest_of`). The destructuring
/// patterns are exhaustive, so a new state field cannot be left out.
trait Identical {
    fn identical(&self, other: &Self) -> bool;
}

fn same_bits(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn same_f(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

fn same_alloc<T: ?Sized>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

impl Identical for ClipState {
    fn identical(&self, other: &Self) -> bool {
        match (self, other) {
            (ClipState::None, ClipState::None) | (ClipState::Complex, ClipState::Complex) => true,
            (ClipState::Rect(a), ClipState::Rect(b)) => same_bits(a, b),
            _ => false,
        }
    }
}

impl Identical for ColorSpaceKind {
    fn identical(&self, other: &Self) -> bool {
        use ColorSpaceKind as K;
        match (self, other) {
            (K::Named(a, h), K::Named(b, k)) => Arc::ptr_eq(a, b) && h == k,
            (K::Default, K::Default)
            | (K::DeviceGray, K::DeviceGray)
            | (K::DeviceRgb, K::DeviceRgb)
            | (K::DeviceCmyk, K::DeviceCmyk)
            | (K::Pattern, K::Pattern) => true,
            _ => false,
        }
    }
}

impl Identical for Paint {
    fn identical(&self, o: &Self) -> bool {
        let Paint {
            space_op,
            color_op,
            space,
            comps,
            effect,
            pattern,
            pattern_hash,
        } = self;
        let same_effect = match (effect, &o.effect) {
            (ColorEffect::Rgb(a), ColorEffect::Rgb(b)) => same_bits(a, b),
            (ColorEffect::Unreadable, ColorEffect::Unreadable) => true,
            _ => false,
        };
        same_alloc(space_op, &o.space_op)
            && same_alloc(color_op, &o.color_op)
            && space.identical(&o.space)
            && same_bits(comps, &o.comps)
            && same_effect
            && *pattern == o.pattern
            && *pattern_hash == o.pattern_hash
    }
}

impl Identical for GsEffects {
    fn identical(&self, o: &Self) -> bool {
        let GsEffects {
            ca,
            ca_stroke,
            blend,
            soft_mask,
            overprint,
            line_width,
            line_cap,
            line_join,
            miter_limit,
            dash,
            rendering_intent,
            flatness,
            stroke_adjust,
            other,
        } = self;
        same_f(*ca, o.ca)
            && same_f(*ca_stroke, o.ca_stroke)
            && Arc::ptr_eq(blend, &o.blend)
            && *soft_mask == o.soft_mask
            && *overprint == o.overprint
            && same_f(*line_width, o.line_width)
            && *line_cap == o.line_cap
            && *line_join == o.line_join
            && same_f(*miter_limit, o.miter_limit)
            && Arc::ptr_eq(&dash.0, &o.dash.0)
            && same_f(dash.1, o.dash.1)
            && Arc::ptr_eq(rendering_intent, &o.rendering_intent)
            && same_f(*flatness, o.flatness)
            && *stroke_adjust == o.stroke_adjust
            && Arc::ptr_eq(other, &o.other)
    }
}

impl Identical for TextParams {
    fn identical(&self, o: &Self) -> bool {
        let TextParams {
            font,
            tfs,
            tc,
            tc_src,
            tw,
            tw_src,
            th,
            tl,
            tr,
            ts,
        } = self;
        let same_font = match (font, &o.font) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                let FontUse {
                    resource,
                    content_hash,
                    from_extgstate,
                    tf_op,
                } = a;
                same_alloc(resource, &b.resource)
                    && *content_hash == b.content_hash
                    && *from_extgstate == b.from_extgstate
                    && same_alloc(tf_op, &b.tf_op)
            }
            _ => false,
        };
        same_font
            && same_f(*tfs, o.tfs)
            && same_f(*tc, o.tc)
            && same_alloc(tc_src, &o.tc_src)
            && same_f(*tw, o.tw)
            && same_alloc(tw_src, &o.tw_src)
            && same_f(*th, o.th)
            && same_f(*tl, o.tl)
            && *tr == o.tr
            && same_f(*ts, o.ts)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateField {
    Font,
    Tfs,
    Tc,
    Fill,
}

fn close(a: f64, b: f64, eps: f64) -> bool {
    a == b || (a - b).abs() <= eps * 1f64.max(a.abs()).max(b.abs())
}

fn comps_close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= COLOR_EPSILON)
}

fn initial_black(p: &Paint) -> bool {
    match (&p.space, p.comps.as_slice()) {
        (ColorSpaceKind::DeviceGray, [g]) => g.abs() <= COLOR_EPSILON,
        (ColorSpaceKind::DeviceRgb, [r, g, b]) => {
            [r, g, b].iter().all(|v| v.abs() <= COLOR_EPSILON)
        }
        (ColorSpaceKind::DeviceCmyk, [c, m, y, k]) => {
            [c, m, y].iter().all(|v| v.abs() <= COLOR_EPSILON) && (k - 1.0).abs() <= COLOR_EPSILON
        }
        _ => false,
    }
}

/// Equal iff (a) the verbatim op bytes and the colour space are equal, or (b) same
/// `ColorSpaceKind` (Named: same name and hash) and components within COLOR_EPSILON, or (c) one
/// side is `Default` (never set) and the other is DeviceGray 0, DeviceRGB 0 0 0 or DeviceCMYK
/// 0 0 0 1. Pattern paints are equal only with identical colour ops (the pattern name) and the
/// same deep hash of the pattern they name. Other cross-family values are unequal (D35).
pub fn same_paint(a: &Paint, b: &Paint) -> bool {
    if a.pattern || b.pattern {
        return a.pattern == b.pattern
            && a.space == b.space
            && a.color_op == b.color_op
            && a.pattern_hash == b.pattern_hash;
    }
    let verbatim = (a.space_op.is_some() || a.color_op.is_some())
        && a.space_op == b.space_op
        && a.color_op == b.color_op;
    if a.space == b.space && (verbatim || comps_close(&a.comps, &b.comps)) {
        return true;
    }
    match (&a.space, &b.space) {
        (ColorSpaceKind::Default, _) => comps_close(&a.comps, &[0.0]) && initial_black(b),
        (_, ColorSpaceKind::Default) => comps_close(&b.comps, &[0.0]) && initial_black(a),
        _ => false,
    }
}

fn same_font(a: &Option<FontUse>, b: &Option<FontUse>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.resource == y.resource
                && x.content_hash == y.content_hash
                && x.from_extgstate == y.from_extgstate
        }
        _ => false,
    }
}

fn same_clip(a: &ClipState, b: &ClipState) -> bool {
    match (a, b) {
        (ClipState::None, ClipState::None) | (ClipState::Complex, ClipState::Complex) => true,
        (ClipState::Rect(x), ClipState::Rect(y)) => {
            x.iter().zip(y).all(|(p, q)| close(*p, *q, STATE_EPSILON))
        }
        _ => false,
    }
}

/// Dash arrays: the same shared array (the common case) without walking it.
fn same_dash(a: &(Arc<[f64]>, f64), b: &(Arc<[f64]>, f64)) -> bool {
    a.1 == b.1 && (Arc::ptr_eq(&a.0, &b.0) || a.0 == b.0)
}

fn same_gs(a: &GsEffects, b: &GsEffects) -> Result<(), &'static str> {
    let checks: [(bool, &'static str); 14] = [
        (a.ca == b.ca, "ca"),
        (a.ca_stroke == b.ca_stroke, "CA"),
        (a.blend == b.blend, "blend"),
        (a.soft_mask == b.soft_mask, "soft_mask"),
        (a.overprint == b.overprint, "overprint"),
        (a.line_width == b.line_width, "line_width"),
        (a.line_cap == b.line_cap, "line_cap"),
        (a.line_join == b.line_join, "line_join"),
        (a.miter_limit == b.miter_limit, "miter_limit"),
        (same_dash(&a.dash, &b.dash), "dash"),
        (a.rendering_intent == b.rendering_intent, "rendering_intent"),
        (a.flatness == b.flatness, "flatness"),
        (a.stroke_adjust == b.stroke_adjust, "stroke_adjust"),
        (a.other == b.other, "extgstate"),
    ];
    match checks.iter().find(|(ok, _)| !ok) {
        Some((_, field)) => Err(field),
        None => Ok(()),
    }
}

/// Field name on mismatch. Compares every field of the digest: CTM within STATE_EPSILON, clip,
/// fill and stroke via `same_paint`, all `GsEffects`, text params by value (sources ignored),
/// marked stack.
pub fn same_state(a: &StateDigest, b: &StateDigest) -> Result<(), &'static str> {
    same_state_except(a, b, &[])
}

/// As `same_state` but skipping the listed fields (joins skip the font identity; edited glyphs
/// skip only the fields their style change targets).
pub fn same_state_except(
    a: &StateDigest,
    b: &StateDigest,
    skip: &[StateField],
) -> Result<(), &'static str> {
    let skipped = |f: StateField| skip.contains(&f);
    if !a
        .ctm
        .iter()
        .zip(&b.ctm)
        .all(|(p, q)| close(*p, *q, STATE_EPSILON))
    {
        return Err("ctm");
    }
    if !same_clip(&a.clip, &b.clip) {
        return Err("clip");
    }
    if !skipped(StateField::Fill) && !same_paint(&a.fill, &b.fill) {
        return Err("fill");
    }
    if !same_paint(&a.stroke, &b.stroke) {
        return Err("stroke");
    }
    same_gs(&a.gs, &b.gs)?;
    let (x, y) = (&a.text, &b.text);
    if !skipped(StateField::Font) && !same_font(&x.font, &y.font) {
        return Err("font");
    }
    let text: [(bool, &'static str); 7] = [
        (skipped(StateField::Tfs) || x.tfs == y.tfs, "tfs"),
        (skipped(StateField::Tc) || x.tc == y.tc, "tc"),
        (x.tw == y.tw, "tw"),
        (x.th == y.th, "th"),
        (x.tl == y.tl, "tl"),
        (x.tr == y.tr, "tr"),
        (x.ts == y.ts, "ts"),
    ];
    if let Some((_, field)) = text.iter().find(|(ok, _)| !ok) {
        return Err(field);
    }
    if a.marked != b.marked {
        return Err("marked");
    }
    Ok(())
}
