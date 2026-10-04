//! The page-model byte budget (`limits::PAGE_MODEL_BYTES_MAX`, review T3 r1–r3): one tally of
//! the bytes a page model build (`walk_page`, then `build_runs`) holds at once, and — on the same
//! tally, the model's bytes already charged — the #33 Classify pass run for that model. Every
//! allocation that grows with the page is charged *before* it is made — lexed ops, records with
//! their glyphs, kerns and text, paints, state digests and the shared parts they hold, the font
//! models the walk keeps, page font names, the walk's memo maps, a Form's decoded content and ops
//! while it runs, runs with their units, text, carets and surfaces, and the run stage's scratch —
//! and the page is `PAGE_TOO_COMPLEX` "page model size" the moment the next charge would pass the
//! cap. A vector's growth also needs room for its old buffer, which lives until the copy is made.
//!
//! The per-element budgets (glyphs, kerns, characters, operand nodes) bound one kind of element
//! each; this one bounds their sum and every allocation of the same kind that no review has found
//! yet: a small file cannot make any charged path hold more than the cap, whatever it repeats.
//!
//! Charges use `approx_bytes`'s accounting (`runs/size.rs` calls the same helpers): the bytes each
//! allocation requests — vectors by capacity, each shared allocation once by address (`Shared`).
//! `held` is what the model keeps (`PageWalk::model_bytes` hands the walk's part to the run
//! stage); `scratch` lives only while a walk or the run stage runs (memo maps, estimated per entry
//! with `map_entry`; a Form's data and ops, released when it ends).
//!
//! Font models are charged once per allocation (`FontModel::approx_bytes`) when the walk loads
//! them: a model keeps its fonts alive in its records and page fonts however the snapshot's
//! `FontCache` evicts (review T3-budget HIGH-1), so they count in `model_bytes` and
//! `approx_bytes`, whichever pages share them.
//!
//! Not charged, and why it stays bounded: the joined content's decode (`PAGE_DECODE_BUDGET`,
//! before the walk; the content itself is charged), and a state part (verbatim op bytes, a name)
//! between the op that copies it and the first digest that holds it — the current state and its
//! `q` stack hold at most one copy of each op span they were set from, so never more than the
//! decoded content.

use super::{Stop, OPERAND_NODES_MAX};
use crate::pdf_engine::text_edit::content::{ContentPart, PageContent};
use crate::pdf_engine::text_edit::fonts::FontModel;
use crate::pdf_engine::text_edit::lexer::{
    lex_content, LexError, LexLimits, Op, Operand, OPERAND_NODES,
};
use crate::pdf_engine::text_edit::limits::model_bytes_max;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::{Bytes, ColorSpaceKind, MarkedNode, StateDigest};
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// `page_detail` of a page refused by the budget.
pub(crate) const MODEL_SIZE: &str = "page model size";

/// The two reference counts in front of every `Arc` allocation.
pub(crate) const ARC_HEADER: usize = 2 * size_of::<usize>();

/// The bytes an `Arc<[T]>` of `len` items of `item` bytes requests (header plus data, padded to
/// the header's alignment).
pub(crate) fn arc_slice(len: usize, item: usize) -> usize {
    let data = len.saturating_mul(item);
    ARC_HEADER.saturating_add(data.div_ceil(size_of::<usize>()) * size_of::<usize>())
}

/// What one more entry of a `HashMap<K, V>` / `HashSet<K>` may cost: key, value and control byte
/// for the 2 × 8/7 buckets per entry right after a growth, plus the table being replaced.
pub(crate) const fn map_entry<K, V>() -> usize {
    4 * (size_of::<K>() + size_of::<V>() + 1)
}

fn addr<T: ?Sized>(a: &Arc<T>) -> usize {
    Arc::as_ptr(a).cast::<u8>() as usize
}

/// Distinct digests and the shared allocations they hold, each counted once (by address: every
/// counted one is held by a record or paint that lives as long as the walk, so no two share an
/// address).
#[derive(Default)]
pub(crate) struct Shared {
    seen: HashSet<usize>,
    bytes: usize,
}

impl Shared {
    /// Bytes counted so far.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Adds `bytes` for the allocation at `at` unless it was counted already; true when new.
    fn once(&mut self, at: usize, bytes: usize) -> bool {
        let new = self.seen.insert(at);
        if new {
            self.bytes = self.bytes.saturating_add(bytes);
        }
        new
    }

    fn bytes_of(&mut self, b: &Bytes) {
        self.once(addr(b), arc_slice(b.len(), 1));
    }

    fn maybe(&mut self, b: &Option<Bytes>) {
        if let Some(b) = b {
            self.bytes_of(b);
        }
    }

    /// Counts the digest `d` and every shared part it holds that was not counted yet.
    pub(crate) fn add(&mut self, d: &Arc<StateDigest>) {
        if !self.once(addr(d), ARC_HEADER + size_of::<StateDigest>()) {
            return;
        }
        for p in [&d.fill, &d.stroke] {
            self.bytes = self
                .bytes
                .saturating_add(p.comps.capacity() * size_of::<f64>());
            self.maybe(&p.space_op);
            self.maybe(&p.color_op);
            if let ColorSpaceKind::Named(name, _) = &p.space {
                self.bytes_of(name);
            }
        }
        let g = &d.gs;
        self.bytes_of(&g.blend);
        self.bytes_of(&g.rendering_intent);
        self.once(addr(&g.dash.0), arc_slice(g.dash.0.len(), size_of::<f64>()));
        let other = arc_slice(g.other.len(), size_of::<(Bytes, u64)>());
        if self.once(addr(&g.other), other) {
            for (key, _) in g.other.iter() {
                self.bytes_of(key);
            }
        }
        if let Some(f) = &d.text.font {
            self.maybe(&f.resource);
            self.maybe(&f.tf_op);
        }
        self.maybe(&d.text.tc_src);
        self.maybe(&d.text.tw_src);
        // Nodes are shared from the top down: below the first one already counted, all are.
        for (at, entry) in d.marked.nodes() {
            if !self.once(at, ARC_HEADER + size_of::<MarkedNode>()) {
                break;
            }
            self.bytes_of(&entry.tag);
        }
    }

    fn entries(&self) -> usize {
        self.seen.len()
    }
}

/// One lexed op's heap: every operand node (arrays and dictionaries walked without recursion)
/// and their bytes, and an inline image's dictionary.
pub(crate) fn op_heap(op: &Op) -> usize {
    let mut total = op.operands.capacity() * size_of::<Operand>();
    let mut stack: Vec<&Operand> = Vec::new();
    let image = op.inline_image.iter().flat_map(|i| {
        let keys: usize = i.dict.iter().map(|(k, _)| k.capacity()).sum();
        let entries = i.dict.capacity() * size_of::<(Vec<u8>, Operand)>();
        std::iter::once(keys + entries)
    });
    total = image.fold(total, usize::saturating_add);
    let values = op.inline_image.iter().flat_map(|i| i.dict.iter());
    for o in op.operands.iter().chain(values.map(|(_, v)| v)) {
        total = total.saturating_add(operand_node(o, &mut stack));
        while let Some(inner) = stack.pop() {
            total = total.saturating_add(operand_node(inner, &mut stack));
        }
    }
    total
}

/// What a page's content costs the model that keeps it: the joined buffer and its parts.
pub(crate) fn content_bytes(content: &PageContent) -> usize {
    content
        .joined
        .capacity()
        .saturating_add(content.parts.capacity() * size_of::<ContentPart>())
}

/// The lexed ops of one stream: the vector and every op's heap.
pub(crate) fn ops_bytes(ops: &Vec<Op>) -> usize {
    ops.iter().map(op_heap).fold(
        ops.capacity().saturating_mul(size_of::<Op>()),
        usize::saturating_add,
    )
}

/// The heap bytes of one operand node; its children are pushed on `stack`.
fn operand_node<'o>(o: &'o Operand, stack: &mut Vec<&'o Operand>) -> usize {
    match o {
        Operand::Name { bytes, .. } | Operand::Str { bytes, .. } => bytes.capacity(),
        Operand::Array { items, .. } => {
            stack.extend(items.iter());
            items.capacity() * size_of::<Operand>()
        }
        Operand::Dict { entries, .. } => {
            let keys: usize = entries.iter().map(|(k, _)| k.capacity()).sum();
            stack.extend(entries.iter().map(|(_, v)| v));
            keys + entries.capacity() * size_of::<(Vec<u8>, Operand)>()
        }
        Operand::Number { .. } | Operand::Bool { .. } | Operand::Null { .. } => 0,
    }
}

/// Lexed operand nodes of `ops`: every number, name, string, array and dictionary, nested ones
/// and an inline image's dictionary values included (counted without recursion).
pub(crate) fn operand_nodes(ops: &[Op]) -> usize {
    let mut stack: Vec<&Operand> = Vec::new();
    let mut n = 0usize;
    for op in ops {
        stack.extend(op.operands.iter());
        stack.extend(
            op.inline_image
                .iter()
                .flat_map(|i| i.dict.iter().map(|(_, v)| v)),
        );
        while let Some(o) = stack.pop() {
            n = n.saturating_add(1);
            match o {
                Operand::Array { items, .. } => stack.extend(items.iter()),
                Operand::Dict { entries, .. } => stack.extend(entries.iter().map(|(_, v)| v)),
                _ => {}
            }
        }
    }
    n
}

/// One stream's lexed ops, their operand nodes and the bytes charged for them.
pub(crate) struct Lexed {
    pub ops: Vec<Op>,
    pub nodes: usize,
    pub bytes: usize,
}

/// Lexes one stream under the walk's budgets. The lexer stops at the operand nodes the walk may
/// still hold — `OPERAND_NODES_MAX` alive at once (`alive` are already held) and what the byte
/// budget has left, at `size_of::<Operand>()` each — and the ops' bytes are charged before they
/// are kept (`keep`: the page's ops, held) or run (a Form's, scratch the caller releases). Lexer
/// errors keep their reason, with `prefix` before the detail; the node cap reads "content
/// operands", or "page model size" when the byte budget was the tighter bound.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lex_ops(
    bytes: &[u8],
    ops_max: usize,
    alive: usize,
    mem: &mut ModelBudget,
    cancel: Option<&AtomicBool>,
    keep: bool,
    prefix: &str,
) -> Result<Lexed, Stop> {
    let by_count = OPERAND_NODES_MAX.saturating_sub(alive);
    let by_bytes = mem.left() / size_of::<Operand>();
    let limits = LexLimits {
        ops_max,
        operand_nodes_max: by_count.min(by_bytes),
        ..LexLimits::page()
    };
    let too_many = |by_budget: bool| {
        let what = if by_budget {
            MODEL_SIZE
        } else {
            "content operands"
        };
        Stop::new(TextReason::PageTooComplex, what)
    };
    let ops = lex_content(bytes, &limits, cancel).map_err(|e| match e {
        LexError::TooComplex { what } if what == OPERAND_NODES => too_many(by_bytes < by_count),
        e => Stop::new(e.page_reason(), format!("{prefix}{e}")),
    })?;
    let nodes = operand_nodes(&ops);
    if nodes > by_count {
        return Err(too_many(false));
    }
    let size = ops_bytes(&ops);
    if keep {
        mem.hold(size)?;
    } else {
        mem.scratch(size)?;
    }
    Ok(Lexed {
        ops,
        nodes,
        bytes: size,
    })
}

/// The byte budget of one page model build or one Classify pass (see the module comment).
pub(crate) struct ModelBudget {
    max: usize,
    held: usize,
    scratch: usize,
    shared: Shared,
    /// The part of `held` that is font models.
    fonts: usize,
}

impl ModelBudget {
    /// A budget of `model_bytes_max()` with `held` bytes already charged (the content, for a new
    /// walk; the walk's, for the run stage; the model's, for its Classify pass).
    pub(crate) fn new(held: usize) -> ModelBudget {
        ModelBudget {
            max: model_bytes_max(),
            held,
            scratch: 0,
            shared: Shared::default(),
            fonts: 0,
        }
    }

    /// Notes font models charged already (held by the model a Classify pass is for, alive until
    /// it ends): loading one of them again costs nothing more.
    pub(crate) fn fonts_held<'f>(&mut self, fonts: impl Iterator<Item = &'f Arc<FontModel>>) {
        for font in fonts {
            self.shared.seen.insert(addr(font));
        }
    }

    /// Font-model bytes charged by this budget.
    pub(crate) fn font_bytes(&self) -> usize {
        self.fonts
    }

    /// Charges the font model `font` (held), once per allocation.
    pub(crate) fn font(&mut self, font: &Arc<FontModel>) -> Result<(), Stop> {
        if self.shared.seen.contains(&addr(font)) {
            return Ok(());
        }
        let bytes = font.approx_bytes();
        let entry = map_entry::<usize, ()>();
        self.check(bytes.saturating_add(entry))?;
        self.hold(bytes)?;
        self.scratch(entry)?;
        self.shared.seen.insert(addr(font));
        self.fonts = self.fonts.saturating_add(bytes);
        Ok(())
    }

    /// `font` for a model the page can do without: charged when it fits `allowance` and the
    /// budget (or was charged already); false when it is to be left out.
    pub(crate) fn font_within(&mut self, font: &Arc<FontModel>, allowance: usize) -> bool {
        if self.shared.seen.contains(&addr(font)) {
            return true;
        }
        font.approx_bytes() <= allowance && self.font(font).is_ok()
    }

    /// Bytes the model keeps so far.
    pub(crate) fn held(&self) -> usize {
        self.held
    }

    /// Bytes that may still be charged.
    pub(crate) fn left(&self) -> usize {
        self.max
            .saturating_sub(self.held.saturating_add(self.scratch))
    }

    fn check(&self, more: usize) -> Result<(), Stop> {
        if more > self.left() {
            return Err(Stop::new(TextReason::PageTooComplex, MODEL_SIZE));
        }
        Ok(())
    }

    /// Charges `bytes` the model will keep.
    pub(crate) fn hold(&mut self, bytes: usize) -> Result<(), Stop> {
        self.check(bytes)?;
        self.held = self.held.saturating_add(bytes);
        Ok(())
    }

    /// Releases held bytes that were charged but not kept (spare capacity given back).
    pub(crate) fn unhold(&mut self, bytes: usize) {
        self.held = self.held.saturating_sub(bytes);
    }

    /// Charges `bytes` that live only while this walk or run stage runs.
    pub(crate) fn scratch(&mut self, bytes: usize) -> Result<(), Stop> {
        self.check(bytes)?;
        self.scratch = self.scratch.saturating_add(bytes);
        Ok(())
    }

    /// Releases scratch bytes that were freed.
    pub(crate) fn unscratch(&mut self, bytes: usize) {
        self.scratch = self.scratch.saturating_sub(bytes);
    }

    /// Charges what the digest `d` adds to the model: itself and its shared parts, each once,
    /// plus one dedupe entry per new allocation (scratch).
    pub(crate) fn digest(&mut self, d: &Arc<StateDigest>) -> Result<(), Stop> {
        let (before, entries) = (self.shared.bytes(), self.shared.entries());
        self.shared.add(d);
        let new = self.shared.bytes().saturating_sub(before);
        let seen = self.shared.entries().saturating_sub(entries);
        self.hold(new)?;
        self.scratch(seen.saturating_mul(map_entry::<usize, ()>()))
    }

    /// Makes room for `more` items in `v`, charging the growth (held) before it is allocated.
    /// Grows by doubling like `Vec::push`, so a vector costs what an unbudgeted one would. The old
    /// buffer lives until its items are moved, so the budget must also have room for it then
    /// (review T3-budget LOW-1: a doubling of the records vector briefly holds both).
    pub(crate) fn grow<T>(&mut self, v: &mut Vec<T>, more: usize) -> Result<(), Stop> {
        let need = v.len().saturating_add(more);
        let cap = v.capacity();
        if need <= cap {
            return Ok(());
        }
        let new_cap = need.max(cap.saturating_mul(2)).max(4);
        let extra = new_cap.saturating_sub(cap).saturating_mul(size_of::<T>());
        self.check(extra.saturating_add(cap.saturating_mul(size_of::<T>())))?;
        self.hold(extra)?;
        v.reserve_exact(new_cap.saturating_sub(v.len()));
        Ok(())
    }

    /// Pushes `x` onto `v` after charging any growth.
    pub(crate) fn push<T>(&mut self, v: &mut Vec<T>, x: T) -> Result<(), Stop> {
        self.grow(v, 1)?;
        v.push(x);
        Ok(())
    }

    /// Gives `v`'s spare capacity back (held) once it is complete.
    pub(crate) fn fit<T>(&mut self, v: &mut Vec<T>) {
        let spare = v.capacity().saturating_sub(v.len());
        v.shrink_to_fit();
        self.unhold(spare.saturating_mul(size_of::<T>()));
    }
}

#[cfg(test)]
impl Shared {
    /// Counts a shared allocation of `bytes` at `a` once (`PageModel::approx_bytes`).
    pub(crate) fn arc<T: ?Sized>(&mut self, a: &Arc<T>, bytes: usize) -> bool {
        self.once(addr(a), bytes)
    }
}
