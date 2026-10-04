//! Approximate heap size of a page model (SPEC §B.19 caches, §H R20), counted from what the
//! `PageModel` holds: the oracle the page-model budget is tested against (`approx_bytes ≤
//! walk.model_bytes` on every page the tests build). Production sizes models by
//! `walk.model_bytes`, the budget's own O(1) tally (fix pass 2026-10-03). A model is linear in its
//! page's ops (records, paints, runs), but one hostile page at `PAGE_OPS_MAX` could hold hundreds
//! of MiB, which a count of models alone cannot see.
//!
//! Counted, as the bytes each allocation requests: every vector's capacity, every lexed operand
//! node and its bytes (a model keeps none), the joined content, each distinct state digest once
//! (consecutive records and paints share one) and, once per allocation, every shared part a digest
//! holds (verbatim op bytes, names, the dash array, the unmodelled ExtGState keys and their list,
//! the nodes of the marked-content stack and their tags) — a page whose state changes on every op
//! holds one of these per op — each run surface once (runs share them), and each distinct font
//! model the page fonts and records keep alive (`FontModel::approx_bytes`, once per allocation:
//! the model keeps them however the snapshot's font cache evicts). Not counted: the allocator's
//! own rounding and bookkeeping.
//!
//! The page-model budget charges the same bytes as they are allocated (`walker::budget`, whose
//! helpers this module uses), so `approx_bytes` never exceeds what the build was allowed to hold.

use super::surface::unit_heap;
use super::{PageModel, TextRun, Unit};
use crate::pdf_engine::text_edit::fonts::FontModel;
use crate::pdf_engine::text_edit::lexer::{Op, Span};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::walker::budget::{arc_slice, content_bytes, op_heap, Shared};
use crate::pdf_engine::text_edit::walker::{GlyphRec, PaintKind, PaintRecord, RecElem, ShowRecord};
use lopdf::ObjectId;
use std::mem::size_of;
use std::sync::Arc;

impl PageModel {
    /// Approximate bytes this model keeps on the heap (see the module comment).
    pub fn approx_bytes(&self) -> usize {
        let walk = &self.walk;
        let mut digests = Shared::default();
        let records: usize = walk
            .records
            .iter()
            .map(|r| record_bytes(r, &mut digests))
            .sum();
        let paints: usize = walk
            .paints
            .iter()
            .map(|p| paint_bytes(p, &mut digests))
            .sum();
        let ops: usize = walk.ops.iter().map(op_heap).sum();
        let runs: usize = self.runs.iter().map(|r| run_bytes(r, &mut digests)).sum();
        let vectors = walk.records.capacity() * size_of::<ShowRecord>()
            + walk.paints.capacity() * size_of::<PaintRecord>()
            + walk.ops.capacity() * size_of::<Op>()
            + self.runs.capacity() * size_of::<TextRun>()
            + walk.page_fonts.capacity() * size_of::<(Vec<u8>, usize)>();
        let content = content_bytes(&self.content);
        let fonts: usize = walk
            .page_fonts
            .iter()
            .map(|(name, model)| name.capacity() + font_bytes(model, &mut digests))
            .sum::<usize>()
            + walk
                .records
                .iter()
                .filter_map(|r| r.font.as_ref())
                .map(|model| font_bytes(model, &mut digests))
                .sum::<usize>();
        let per_record = self.record_reason.capacity() * size_of::<Option<TextReason>>();
        [
            vectors,
            records,
            paints,
            digests.bytes(),
            ops,
            runs,
            content,
            fonts,
            per_record,
        ]
        .iter()
        .fold(0usize, |sum, b| sum.saturating_add(*b))
    }
}

/// A font model's bytes, the first time its allocation is seen.
fn font_bytes(model: &Arc<FontModel>, shared: &mut Shared) -> usize {
    if shared.arc(model, 0) {
        model.approx_bytes()
    } else {
        0
    }
}

fn record_bytes(r: &ShowRecord, digests: &mut Shared) -> usize {
    digests.add(&r.before);
    digests.add(&r.after);
    let texts: usize = r
        .glyphs
        .iter()
        .map(|g| g.text.as_ref().map_or(0, String::capacity))
        .sum();
    r.form_chain.capacity() * size_of::<ObjectId>()
        + r.operand_spans.capacity() * size_of::<Span>()
        + r.glyphs.capacity() * size_of::<GlyphRec>()
        + texts
        + r.elems.capacity() * size_of::<RecElem>()
}

fn paint_bytes(p: &PaintRecord, digests: &mut Shared) -> usize {
    digests.add(&p.state);
    let name = match &p.kind {
        PaintKind::Shading { name, .. }
        | PaintKind::ImageXObject { name, .. }
        | PaintKind::FormXObject { name, .. } => name.capacity(),
        PaintKind::Path(_) | PaintKind::InlineImage { .. } => 0,
    };
    p.form_chain.capacity() * size_of::<ObjectId>() + name
}

/// One run's heap; its surface (shared by the runs that have it) is counted in `shared`, once
/// per allocation.
fn run_bytes(r: &TextRun, shared: &mut Shared) -> usize {
    let units: usize = r.units.iter().map(unit_heap).sum();
    let names: usize = r.surface.iter().map(Vec::capacity).sum();
    let list = arc_slice(r.surface.len(), size_of::<Vec<u8>>());
    shared.arc(&r.surface, list.saturating_add(names));
    r.id.capacity()
        + r.text.capacity()
        + r.members.capacity() * size_of::<usize>()
        + r.units.capacity() * size_of::<Unit>()
        + units
        + r.caret_offsets.capacity() * size_of::<f64>()
        + r.reasons.capacity() * size_of::<TextReason>()
        + r.fill_hex.as_ref().map_or(0, String::capacity)
}
