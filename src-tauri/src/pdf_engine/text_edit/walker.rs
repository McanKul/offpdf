//! Page walker (SPEC §B.10): interprets one page's lexed content with the full graphics state and
//! records every show op (`ShowRecord`) and every paint (`PaintRecord`) in paint order, with
//! id-free state digests (D32).
//!
//! Modes: `Edit` walks depth 0 only (a Form `Do` is a paint); `Classify` descends Forms (depth
//! ≤ 8, cycle check, `/Matrix`, `/BBox` as a clip) for the #33 classifier; `Wrapped` (Phase B)
//! follows exactly the one depth-0 `Do` of qpdf's overlay wrapper Form and treats its content as
//! depth 0. Every page-level problem (lexer, decode, budgets, `q` overflow, geometry,
//! unresolvable references, non-finite numbers) becomes `page_reason` with empty records — never a
//! partial list. `walk_page` never fails.
//!
//! Work and memory are bounded per walk: every op costs O(its own operands) — a path op checks
//! only the points it adds, a `Tf` finds its root font by name, an optional-content group is a
//! binary search in lists read once per snapshot (`structure::OcConfig`), and other lookups that
//! would repeat per op are memoised. Records and paints drawn under an unchanged state share one
//! digest (`Exec::digest`), and digests share their variable-size parts (`state`); deep hashes are
//! memoised per object and charged to the walk's budgets (`hash`). What a walk keeps per element
//! is budgeted per page, not only per op: glyphs (`GLYPHS_PER_PAGE_MAX`), TJ kerns
//! (`KERNS_PER_PAGE_MAX`), characters of glyph text (`TEXT_CHARS_PER_PAGE_MAX`) and lexed operand
//! nodes alive at once (`OPERAND_NODES_MAX`). Above them, every allocation that grows with the
//! page is charged to one byte budget before it is made (`budget::ModelBudget`,
//! `PAGE_MODEL_BYTES_MAX`), so no small page can build a model of GiBs, whatever it repeats.

pub(crate) mod budget;
mod fonts;
mod gfx;
pub(crate) mod hash;
pub(crate) mod show;
mod xobj;

use crate::pdf_engine::text_edit::content::{check_part_joins, PageContent};
use crate::pdf_engine::text_edit::context::{resolve, Res, SnapshotContext};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::fonts::{Code, FontKey, FontModel};
use crate::pdf_engine::text_edit::geometry::{page_geometry, Matrix, PageGeometry, IDENTITY};
use crate::pdf_engine::text_edit::lexer::{Op, Operator, Span};
use crate::pdf_engine::text_edit::limits::{
    GLYPHS_PER_PAGE_MAX, LEX_CANCEL_EVERY_OPS, PAGE_DECODE_BUDGET, PAGE_OPS_MAX, Q_DEPTH_MAX,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::{Bytes, GState, MarkedStack, OcState, StateDigest};
use budget::{content_bytes, lex_ops, map_entry, ModelBudget};
use lopdf::{Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// TJ numbers per page. Each one is kept as a kern element of its record and a unit of its run
/// (§B.2 counts only glyphs): a page of kern-only TJ arrays would otherwise turn 50 KB of
/// deflated content into a model of GiBs (review T3 r2 HIGH-1).
pub(crate) const KERNS_PER_PAGE_MAX: usize = GLYPHS_PER_PAGE_MAX;
/// Characters of glyph text per page. A ToUnicode destination may give one glyph 256 of them,
/// each kept in the glyph, its run unit, the run text and a caret offset (review T3 r2 HIGH-2).
pub(crate) const TEXT_CHARS_PER_PAGE_MAX: usize = 2 * GLYPHS_PER_PAGE_MAX;
/// Lexed operand nodes alive at once in one walk (`limits::OPERAND_NODES_MAX`, also the lexer's
/// own cap): 48 MiB of `[1 1 … 1] 0 d` would otherwise build 24 M nodes (≈ 1.2 GiB).
pub(crate) use crate::pdf_engine::text_edit::limits::OPERAND_NODES_MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowOp {
    Tj,
    TJ,
    Quote,
    DoubleQuote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkMode {
    Edit,
    Classify,
    Wrapped { name: Vec<u8> },
}

#[derive(Debug, Clone)]
pub struct GlyphRec {
    pub code: Code,
    pub text: Option<String>,
    pub origin: (f64, f64),
    /// User-space displacement of the pen for this glyph (incl. Tc, Tw, Th).
    pub advance_user: (f64, f64),
    pub width1000: f64,
    /// Ink box from the font's ascent/descent (rise included), user-space AABB `[x0, y0, x1, y1]`.
    pub bbox: [f64; 4],
}

#[derive(Debug, Clone)]
pub enum RecElem {
    Glyph(usize),
    Kern { value: f64, span: Span },
}

#[derive(Debug, Clone)]
pub struct ShowRecord {
    pub seq: u32,
    pub depth: u8,
    pub form_chain: Vec<ObjectId>,
    pub op: ShowOp,
    /// The op span in the joined buffer (depth-0 page content only).
    pub span: Option<Span>,
    /// The op span in the buffer that was lexed (joined buffer, or the Form's data).
    pub local_span: Span,
    pub operand_spans: Vec<Span>,
    /// The state the glyphs are drawn with (for `"` after its `aw Tw ac Tc` side effects).
    /// Shared: consecutive records (and paints) drawn under an unchanged state hold one digest.
    pub before: Arc<StateDigest>,
    pub after: Arc<StateDigest>,
    pub tm_before: Matrix,
    pub tm_after: Matrix,
    pub glyphs: Vec<GlyphRec>,
    pub elems: Vec<RecElem>,
    /// User space, rise included (= the first glyph's origin when the op starts with a glyph).
    pub pen_before: (f64, f64),
    pub pen_after: (f64, f64),
    /// Σ(w0·Tfs + Tc + Tw·[word space]) + Σ(−n/1000·Tfs), Th excluded.
    pub advance_ts: f64,
    /// `[Tfs·Th 0 0 Tfs 0 Ts] × Tm × CTM` at the first glyph.
    pub text_to_user: Matrix,
    pub after_unproven_inline_image: bool,
    /// Lookup handles, never compared across files.
    pub font_key: Option<FontKey>,
    pub font: Option<Arc<FontModel>>,
    /// A 2-byte font was shown an odd number of bytes.
    pub split_error: bool,
    /// The op is drawn where an earlier show op of the same text object left the pen, and that
    /// op advanced by an amount the model cannot know (an odd-length 2-byte string, a missing
    /// font, a code without a width, a glyph of a font whose widths are not proven to be the
    /// viewers' — `show::advance_proven`), or after a `Q` inside the text object restored a
    /// state saved under another text matrix (`Exec::tm_unsettled`): viewers place this text
    /// elsewhere (`MISSING_WIDTHS`).
    pub pen_unknown: bool,
}

/// Id-free (D32): resource names plus deep hashes.
#[derive(Debug, Clone, PartialEq)]
pub enum PaintKind {
    Path(Operator),
    Shading { name: Vec<u8>, hash: u64 },
    ImageXObject { name: Vec<u8>, hash: u64 },
    InlineImage { hash: u64 },
    FormXObject { name: Vec<u8>, hash: u64 },
}

/// The XObject behind a `Do` (classifier only): its id and whether it was reached through a
/// resource dictionary other pages can share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XObjectUse {
    pub id: ObjectId,
    pub shared_path: bool,
}

#[derive(Debug, Clone)]
pub struct PaintRecord {
    pub seq: u32,
    pub depth: u8,
    pub kind: PaintKind,
    pub span: Option<Span>,
    pub state: Arc<StateDigest>,
    pub bbox: Option<[f64; 4]>,
    pub masked: bool,
    pub form_chain: Vec<ObjectId>,
    pub local_span: Span,
    pub xobject: Option<XObjectUse>,
}

pub struct PageWalk {
    pub page_id: ObjectId,
    pub geometry: PageGeometry,
    /// The page's lexed ops (empty in a page model: `runs::model_of` drops them, see
    /// `ops_bytes`).
    pub ops: Vec<Op>,
    pub records: Vec<ShowRecord>,
    pub paints: Vec<PaintRecord>,
    /// The fonts of the depth-0 resources, first-use order, then the unused ones.
    pub page_fonts: Vec<(Vec<u8>, Arc<FontModel>)>,
    pub page_reason: Option<TextReason>,
    /// Technical detail of `page_reason` (byte offset, check name).
    pub page_detail: Option<String>,
    /// Bytes of the page-model budget charged for what is kept: by `walk_page`, the content it
    /// walked and the walk (0 when refused), which the run stage goes on from; once
    /// `runs::model_of` has built a model on the walk, the whole model's (its runs and the font
    /// models it keeps included — each distinct model once, `FontModel::approx_bytes`, a font
    /// shared by several pages charged by each — its ops not). Never less than that model's
    /// `approx_bytes`, never more than `PAGE_MODEL_BYTES_MAX`.
    pub model_bytes: usize,
    /// The part of `model_bytes` that is `ops` (released when a model drops them).
    pub ops_bytes: usize,
}

impl PageWalk {
    fn empty(page_id: ObjectId, geometry: PageGeometry) -> PageWalk {
        PageWalk {
            page_id,
            geometry,
            ops: Vec::new(),
            records: Vec::new(),
            paints: Vec::new(),
            page_fonts: Vec::new(),
            page_reason: None,
            page_detail: None,
            model_bytes: 0,
            ops_bytes: 0,
        }
    }

    fn refused(self, reason: TextReason, detail: impl Into<String>) -> PageWalk {
        PageWalk {
            page_reason: Some(reason),
            page_detail: Some(detail.into()),
            ..PageWalk::empty(self.page_id, self.geometry)
        }
    }

    /// Releases the walk itself (ops, records, paints and page fonts), keeping its page, geometry
    /// and reason: a model refused after the walk keeps nothing it cannot use (review T3-budget
    /// MEDIUM-1). Returns the bytes released from `model_bytes`.
    pub(crate) fn release(&mut self) -> usize {
        let kept = PageWalk {
            page_reason: self.page_reason,
            page_detail: self.page_detail.take(),
            ..PageWalk::empty(self.page_id, self.geometry.clone())
        };
        let released = self.model_bytes;
        *self = kept;
        released
    }

    /// Drops the lexed ops (nothing reads a model's ops) and their charge.
    pub(crate) fn drop_ops(&mut self) {
        self.ops = Vec::new();
        self.model_bytes = self.model_bytes.saturating_sub(self.ops_bytes);
        self.ops_bytes = 0;
    }
}

/// A page-level refusal raised while walking.
#[derive(Debug, Clone)]
pub(crate) struct Stop {
    pub reason: TextReason,
    pub detail: String,
}

impl Stop {
    pub(crate) fn new(reason: TextReason, detail: impl Into<String>) -> Stop {
        Stop {
            reason,
            detail: detail.into(),
        }
    }

    pub(crate) fn malformed(detail: impl Into<String>) -> Stop {
        Stop::new(TextReason::MalformedContent, detail)
    }
}

pub(crate) type Walked = Result<(), Stop>;

/// Walks one page (`_page_index` names it for the caller only). Never fails: problems are
/// `page_reason`.
pub fn walk_page(
    ctx: &SnapshotContext,
    _page_index: u32,
    content: &PageContent,
    mode: WalkMode,
    cancel: Option<&AtomicBool>,
) -> PageWalk {
    let mem = ModelBudget::new(content_bytes(content));
    walk_page_within(ctx, content, mode, cancel, mem)
}

/// `walk_page` under `mem`, which has already charged `content` and whatever else stays alive
/// while the walk runs (a Classify pass runs on the budget its page model left, review
/// T3-budget MEDIUM-2).
pub(crate) fn walk_page_within(
    ctx: &SnapshotContext,
    content: &PageContent,
    mode: WalkMode,
    cancel: Option<&AtomicBool>,
    mut mem: ModelBudget,
) -> PageWalk {
    let doc = ctx.doc();
    let page_id = content.page_id;
    let (geometry, geometry_reason) = match page_geometry(doc, page_id) {
        Ok(g) => (g, None),
        Err(reason) => (
            PageGeometry::unbounded(lenient_rotation(doc, page_id)),
            Some(reason),
        ),
    };
    let walk = PageWalk::empty(page_id, geometry.clone());
    if let (Some(reason), false) = (geometry_reason, mode == WalkMode::Classify) {
        return walk.refused(reason, "page geometry");
    }
    // The page's lexed ops are charged first; then every part boundary must read alike in every
    // viewer (`content::joins`).
    let lexed = lex_ops(&content.joined, PAGE_OPS_MAX, 0, &mut mem, cancel, true, "");
    let (ops, nodes, ops_bytes) = match lexed {
        Ok(l) => (l.ops, l.nodes, l.bytes),
        Err(stop) => return walk.refused(stop.reason, stop.detail),
    };
    if let Err(at) = check_part_joins(content) {
        let detail = format!("content parts joined inside a token or comment at byte {at}");
        return walk.refused(TextReason::MalformedContent, detail);
    }
    let res = match Res::of_page(doc, page_id) {
        Ok(r) => r,
        Err(what) => return walk.refused(TextReason::MalformedContent, what),
    };
    let mut w = Walker {
        ctx,
        doc,
        mode: mode.clone(),
        cancel,
        geometry,
        budget: DecodeBudget::new(PAGE_DECODE_BUDGET.saturating_sub(content.joined.len())),
        hash_budget: DecodeBudget::new(PAGE_DECODE_BUDGET),
        hash_steps: 0,
        hash_raw_left: ctx.snap.file_len(),
        records: Vec::new(),
        paints: Vec::new(),
        root_fonts: Vec::new(),
        root_font_names: HashSet::new(),
        root_res: res,
        seq: 0,
        ops_left: PAGE_OPS_MAX.saturating_sub(ops.len()),
        glyphs: 0,
        kerns: 0,
        text_chars: 0,
        operand_nodes: nodes,
        form_paints: 0,
        hashes: HashMap::new(),
        direct_hashes: HashMap::new(),
        oc_states: HashMap::new(),
        gs_others: HashMap::new(),
        others_interned: HashSet::new(),
        gs_last: None,
        fonts: HashMap::new(),
        pen_proofs: HashMap::new(),
        mem,
    };
    let frame = Frame {
        bytes: &content.joined,
        res,
        depth: 0,
        chain: Vec::new(),
        joined: true,
        root: true,
    };
    let result = match &mode {
        WalkMode::Wrapped { name } => w.walk_wrapped(&frame, &ops, name, page_id),
        _ => {
            let mut ex = Exec::new(GState::initial(IDENTITY), MarkedStack::default());
            w.run(&frame, &mut ex, &ops)
        }
    }
    .and_then(|()| w.finish_page_fonts());
    match result {
        Err(stop) => walk.refused(stop.reason, stop.detail),
        Ok(()) => PageWalk {
            ops,
            records: w.records,
            paints: w.paints,
            page_fonts: w.root_fonts,
            page_reason: geometry_reason,
            page_detail: geometry_reason.map(|_| "page geometry".to_string()),
            model_bytes: w.mem.held(),
            ops_bytes,
            ..walk
        },
    }
}

/// A walk refused before it started (e.g. the page content could not be decoded).
pub(crate) fn refused_walk(
    ctx: &SnapshotContext,
    page_id: ObjectId,
    reason: TextReason,
    detail: &str,
) -> PageWalk {
    let doc = ctx.doc();
    let geometry = page_geometry(doc, page_id)
        .unwrap_or_else(|_| PageGeometry::unbounded(lenient_rotation(doc, page_id)));
    PageWalk::empty(page_id, geometry).refused(reason, detail)
}

/// `/Rotate` of a page whose geometry was refused, read leniently (classifier display only).
fn lenient_rotation(doc: &Document, page_id: ObjectId) -> i64 {
    let mut cur = Some(page_id);
    for _ in 0..crate::pdf_engine::text_edit::limits::PAGE_TREE_DEPTH_MAX {
        let Some(Object::Dictionary(d)) = cur.and_then(|id| doc.objects.get(&id)) else {
            return 0;
        };
        if let Some((_, Object::Integer(r))) = d.get(b"Rotate").ok().and_then(|o| resolve(doc, o)) {
            return if r.rem_euclid(90) == 0 {
                r.rem_euclid(360)
            } else {
                0
            };
        }
        cur = d.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    0
}

/// One content stream being executed: the buffer its ops were lexed from and its resources.
pub(crate) struct Frame<'f, 'a> {
    pub bytes: &'f [u8],
    pub res: Res<'a>,
    pub depth: u8,
    pub chain: Vec<ObjectId>,
    /// Spans are joined-buffer spans (page content).
    pub joined: bool,
    /// Depth-0 content whose `/Font` entries are the page fonts.
    pub root: bool,
}

impl Frame<'_, '_> {
    /// The verbatim bytes of `op` (its whole span), shared.
    pub(crate) fn op_bytes(&self, op: &Op) -> Bytes {
        Arc::from(self.bytes.get(op.span.clone()).unwrap_or_default())
    }

    pub(crate) fn span_bytes(&self, span: &Span) -> &[u8] {
        self.bytes.get(span.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PathState {
    /// Bounding box of every point added so far (a path keeps no point list: an unpainted path of
    /// 250,000 ops costs nothing).
    pub bbox: Option<[f64; 4]>,
    pub re_count: usize,
    pub re_rect: Option<[f64; 4]>,
    pub re_axis: bool,
    pub other: bool,
}

/// What `q` saves: the graphics state, `Tm`, `Tlm` and `Exec::tm_base`.
pub(crate) type Saved = (GState, Matrix, Matrix, Matrix);

/// Execution state of one content stream.
pub(crate) struct Exec {
    pub gs: GState,
    /// `q` saves the graphics state with the text matrices in force (`Q` compares them).
    pub stack: Vec<Saved>,
    /// Shared with the digests taken under it (a persistent list: push and pop are O(1)).
    pub marked: MarkedStack,
    pub marked_base: usize,
    pub in_text: bool,
    pub tm: Matrix,
    pub tlm: Matrix,
    /// The matrix the last `Tm` set (the identity at `BT`): poppler's text matrix, on which its
    /// line start (moved by `Td`, kept by `Q`) is based.
    pub tm_base: Matrix,
    pub text_clip: bool,
    pub path: PathState,
    pub clip_pending: bool,
    /// `tm` carries an advance the model cannot know (cleared by `BT` and every line move).
    pub pen_unknown: bool,
    /// A `Q` inside this text object restored a state saved under other text matrices, or under
    /// another `tm_base` (review T3 r3 LOW-1). `q`/`Q` are not allowed in a text object (ISO
    /// 32000-1 §8.2) and viewers differ: the model and poppler keep the text position (poppler
    /// restores `Tm` but not the line start), pdf.js restores both. Only `Tm` or `BT` settle it; a
    /// `Td` is relative to the disputed line start.
    pub tm_unsettled: bool,
    /// The last digest taken, handed out again while the state is unchanged.
    digest_memo: Option<Arc<StateDigest>>,
}

impl Exec {
    pub(crate) fn new(gs: GState, marked: MarkedStack) -> Exec {
        Exec {
            gs,
            stack: Vec::new(),
            marked_base: marked.len(),
            marked,
            in_text: false,
            tm: IDENTITY,
            tlm: IDENTITY,
            tm_base: IDENTITY,
            text_clip: false,
            path: PathState::default(),
            clip_pending: false,
            pen_unknown: false,
            tm_unsettled: false,
            digest_memo: None,
        }
    }

    /// The digest of the current state, shared with the previous one when nothing changed (an
    /// O(1) identity check, `StateDigest::is_digest_of`): a page of 250,000 show ops under one
    /// state holds one digest, not 500,000.
    /// A marked-content stack equal by value to the last digest's (an `EMC` then a `BMC` of the
    /// same tag makes a new top node) is replaced by that digest's stack first, so a page that
    /// reopens a stack around every op still shares one digest.
    pub(crate) fn digest(&mut self) -> Arc<StateDigest> {
        if let Some(memo) = &self.digest_memo {
            if !memo.marked.same(&self.marked) && memo.marked == self.marked {
                self.marked = memo.marked.clone();
            }
            if memo.is_digest_of(&self.gs, &self.marked) {
                return Arc::clone(memo);
            }
        }
        let digest = Arc::new(self.gs.digest(&self.marked));
        self.digest_memo = Some(Arc::clone(&digest));
        digest
    }
}

pub(crate) struct Walker<'a> {
    pub ctx: &'a SnapshotContext,
    pub doc: &'a Document,
    pub mode: WalkMode,
    pub cancel: Option<&'a AtomicBool>,
    pub geometry: PageGeometry,
    /// Content, Form and font decoding (§B.2 PAGE_DECODE_BUDGET).
    pub budget: DecodeBudget,
    /// Decoding for deep hashes of XObjects and colour spaces (same size, kept apart so a large
    /// image never refuses the page's text). A failed decode is charged as if it ran to its cap.
    pub hash_budget: DecodeBudget,
    /// Objects visited by all deep hashes of this walk (`hash::HASH_STEPS_MAX`).
    pub hash_steps: usize,
    /// Raw (undecoded) stream bytes the deep hashes of this walk may still read: the file's size
    /// (each stream is hashed once per walk; objects that alias one byte range cannot multiply it).
    pub hash_raw_left: usize,
    pub records: Vec<ShowRecord>,
    pub paints: Vec<PaintRecord>,
    pub root_fonts: Vec<(Vec<u8>, Arc<FontModel>)>,
    /// The names in `root_fonts` (a page may switch fonts on every op).
    pub root_font_names: HashSet<Vec<u8>>,
    pub root_res: Res<'a>,
    pub seq: u32,
    pub ops_left: usize,
    pub glyphs: usize,
    /// TJ numbers shown so far (`KERNS_PER_PAGE_MAX`).
    pub kerns: usize,
    /// Characters of glyph text so far (`TEXT_CHARS_PER_PAGE_MAX`).
    pub text_chars: usize,
    /// Lexed operand nodes alive now: the page's, plus those of the Forms being run
    /// (`OPERAND_NODES_MAX`).
    pub operand_nodes: usize,
    pub form_paints: usize,
    pub hashes: HashMap<ObjectId, u64>,
    /// Deep hashes of direct values, by address (the document is borrowed for the whole walk,
    /// so an address names one value).
    pub direct_hashes: HashMap<usize, u64>,
    /// Optional-content state of each `/OC` group named by a `BDC`, once per walk.
    pub oc_states: HashMap<ObjectId, OcState>,
    /// The unmodelled keys of each ExtGState dictionary a `gs` applied (sorted, one per key),
    /// by address like `direct_hashes`: re-applying one dictionary reuses its list, and a merge
    /// that changes nothing keeps the shared list in force (`GsEffects::set_others`).
    pub gs_others: HashMap<usize, Others>,
    /// Every merged list of unmodelled keys this walk has put in force, by value.
    pub others_interned: HashSet<Others>,
    /// The last `gs`: its dictionary's key list and the list it put in force. Re-applying that
    /// dictionary to that list changes nothing, so it costs O(1) (review T3 r3 MEDIUM-2).
    pub gs_last: Option<(Others, Others)>,
    pub fonts: HashMap<FontKey, Arc<FontModel>>,
    /// Per loaded font: whether viewers advance by the model's widths (`show::advance_proven`).
    pub pen_proofs: HashMap<FontKey, bool>,
    /// The page-model byte budget of this walk.
    pub mem: ModelBudget,
}

/// A shared list of unmodelled ExtGState keys (`GsEffects::other`).
pub(crate) type Others = Arc<[(Bytes, u64)]>;

impl<'a> Walker<'a> {
    pub(crate) fn next_seq(&mut self) -> Result<u32, Stop> {
        let s = self.seq;
        self.seq = self
            .seq
            .checked_add(1)
            .ok_or_else(|| Stop::new(TextReason::PageTooComplex, "paint order"))?;
        Ok(s)
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// `Exec::digest`, charged to the budget (the digest and its parts not counted yet).
    pub(crate) fn digest(&mut self, ex: &mut Exec) -> Result<Arc<StateDigest>, Stop> {
        let d = ex.digest();
        self.mem.digest(&d)?;
        Ok(d)
    }

    pub(crate) fn run(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, ops: &[Op]) -> Walked {
        for (i, op) in ops.iter().enumerate() {
            if i % LEX_CANCEL_EVERY_OPS == 0 && self.cancelled() {
                return Err(Stop::new(TextReason::PageTooComplex, "cancelled"));
            }
            self.op(frame, ex, op)?;
        }
        Ok(())
    }

    fn op(&mut self, frame: &Frame<'_, 'a>, ex: &mut Exec, op: &Op) -> Walked {
        use Operator as O;
        match op.operator {
            O::q => {
                if ex.stack.len() >= Q_DEPTH_MAX {
                    return Err(Stop::malformed(format!(
                        "q nesting deeper than {Q_DEPTH_MAX} at byte {}",
                        op.op_span.start
                    )));
                }
                ex.stack.push((ex.gs.clone(), ex.tm, ex.tlm, ex.tm_base));
            }
            O::Q => {
                if let Some((prev, tm, tlm, base)) = ex.stack.pop() {
                    ex.gs = prev;
                    let same = same_bits(&tm, &ex.tm)
                        && same_bits(&tlm, &ex.tlm)
                        && same_bits(&base, &ex.tm_base);
                    if ex.in_text && !same {
                        ex.tm_unsettled = true;
                    }
                    ex.tm_base = base;
                }
            }
            O::cm => gfx::concat(ex, op)?,
            O::w | O::J | O::j | O::M | O::d | O::ri | O::i => gfx::line_param(ex, op)?,
            O::gs => self.ext_gstate(frame, ex, op)?,
            O::m | O::l | O::c | O::v | O::y | O::h | O::re => gfx::path(ex, op)?,
            O::S | O::s | O::f | O::F | O::fStar | O::B | O::BStar | O::b | O::bStar | O::n => {
                self.paint_path(frame, ex, op)?
            }
            O::W | O::WStar => ex.clip_pending = true,
            O::CS
            | O::cs
            | O::SC
            | O::sc
            | O::SCN
            | O::scn
            | O::G
            | O::g
            | O::RG
            | O::rg
            | O::K
            | O::k => self.color(frame, ex, op)?,
            O::BT => {
                if ex.in_text {
                    return Err(Stop::malformed(format!(
                        "BT inside a text object at byte {}",
                        op.op_span.start
                    )));
                }
                ex.in_text = true;
                ex.tm = IDENTITY;
                ex.tlm = IDENTITY;
                ex.tm_base = IDENTITY;
                ex.text_clip = false;
                ex.pen_unknown = false;
                ex.tm_unsettled = false;
            }
            O::ET => {
                if ex.in_text && ex.text_clip {
                    ex.gs.clip = crate::pdf_engine::text_edit::state::ClipState::Complex;
                }
                ex.in_text = false;
                ex.text_clip = false;
            }
            O::Tc | O::Tw | O::Tz | O::TL | O::Tf | O::Tr | O::Ts => {
                self.text_state(frame, ex, op)?
            }
            O::Td | O::TD | O::Tm | O::TStar => show::position(ex, op)?,
            O::Tj | O::TJ | O::Quote | O::DoubleQuote => self.show(frame, ex, op)?,
            O::Do => self.do_xobject(frame, ex, op)?,
            O::BI => self.inline_image(frame, ex, op)?,
            O::sh => self.shading(frame, ex, op)?,
            O::BMC | O::BDC | O::EMC | O::MP | O::DP => self.marked(frame, ex, op)?,
            O::d0 | O::d1 | O::BX | O::EX | O::EI | O::ID | O::Unknown => {}
        }
        Ok(())
    }

    /// The state of the optional-content group `props` names (`OcConfig::state`, whose lists are
    /// read once per snapshot), once per group and walk: a page may open the same layer in every
    /// `BDC`.
    pub(crate) fn oc_state(
        &mut self,
        id: Option<ObjectId>,
        props: &Object,
    ) -> Result<OcState, Stop> {
        let config = self.ctx.oc_config();
        let Some(id) = id else {
            return Ok(config.state(self.doc, props));
        };
        if let Some(state) = self.oc_states.get(&id) {
            return Ok(*state);
        }
        let state = config.state(self.doc, &Object::Reference(id));
        self.mem.scratch(map_entry::<ObjectId, OcState>())?;
        self.oc_states.insert(id, state);
        Ok(state)
    }
}

/// Bit-for-bit equal matrices (a `q … Q` that moved nothing leaves them untouched).
fn same_bits(a: &Matrix, b: &Matrix) -> bool {
    a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}
