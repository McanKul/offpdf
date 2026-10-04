//! The reasons a single show record cannot be edited (SPEC §A.10 rows 1–28 that are properties of
//! one show op): nesting, content ownership, inline images, render mode, orientation as
//! displayed, clips, layers, masks, patterns, ActualText, fonts, decodability and scripts. Joins
//! use the first one (A.1.1: refused records only join records with the same reason); runs add
//! DUPLICATE_TEXT, PER_GLYPH_TEXT and NO_WRITABLE_GLYPHS afterwards.

use crate::pdf_engine::text_edit::content::{part_exclusive, KidsCounts, PageContent};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::fonts::encodings::{reading_reason, usable_text};
use crate::pdf_engine::text_edit::geometry::{
    bbox_of, classify_orientation, contains, display_rotation, mul, Matrix, Orientation,
};
use crate::pdf_engine::text_edit::limits::CLIP_CONTAIN_TOL_PT;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::{ClipState, OcState};
use crate::pdf_engine::text_edit::structure::PageStruct;
use crate::pdf_engine::text_edit::walker::show::width_unknown;
use crate::pdf_engine::text_edit::walker::{PageWalk, ShowRecord};

/// Per-page inputs shared by every record's checks.
pub(crate) struct ReasonCtx<'a> {
    ctx: &'a SnapshotContext,
    content: &'a PageContent,
    walk: &'a PageWalk,
    kids: Option<&'a KidsCounts>,
    rotation: Matrix,
    page_struct: PageStruct<'a>,
    exclusive: Vec<Option<bool>>,
}

impl<'a> ReasonCtx<'a> {
    pub(crate) fn new(
        ctx: &'a SnapshotContext,
        content: &'a PageContent,
        walk: &'a PageWalk,
    ) -> ReasonCtx<'a> {
        ReasonCtx {
            ctx,
            content,
            walk,
            kids: ctx.kids().ok(),
            rotation: display_rotation(walk.geometry.rotate),
            page_struct: PageStruct::of(ctx.doc(), walk.page_id),
            exclusive: vec![None; content.parts.len()],
        }
    }

    fn part_exclusive(&mut self, part: usize) -> bool {
        if let Some(Some(known)) = self.exclusive.get(part) {
            return *known;
        }
        let Some(kids) = self.kids else {
            return false;
        };
        let ok = part_exclusive(self.ctx.doc(), self.ctx.refs(), kids, self.content, part);
        if let Some(slot) = self.exclusive.get_mut(part) {
            *slot = Some(ok);
        }
        ok
    }

    /// Every reason of §A.10 that holds for `rec` on its own, in priority order.
    pub(crate) fn record_reasons(&mut self, rec: &ShowRecord) -> Vec<TextReason> {
        use TextReason as R;
        let mut out = Vec::new();
        if rec.depth > 0 {
            out.push(R::NestedForm);
        }
        if let Some(span) = &rec.span {
            match self.content.locate(span) {
                None => out.push(R::SplitContent),
                Some((part, _)) => {
                    if !self.part_exclusive(part) {
                        out.push(R::SharedContent);
                    }
                }
            }
        }
        if rec.after_unproven_inline_image {
            out.push(R::InlineImage);
        }
        let tr = rec.before.text.tr;
        match tr {
            3 => out.push(R::InvisibleText),
            4..=7 => out.push(R::TextClipMode),
            _ => {}
        }
        match classify_orientation(&mul(&rec.text_to_user, &self.rotation)) {
            Orientation::Upright => {}
            Orientation::ZeroSize => out.push(R::ZeroSize),
            Orientation::Rotated => out.push(R::RotatedText),
            Orientation::Mirrored => out.push(R::MirroredText),
            Orientation::Skewed => out.push(R::SkewedText),
        }
        if rec.font.as_ref().is_some_and(|f| f.vertical) {
            out.push(R::Vertical);
        }
        if self.clipped(rec) {
            out.push(R::Clipped);
        }
        if rec
            .before
            .marked
            .iter()
            .any(|m| matches!(m.oc, Some(OcState::Hidden | OcState::Unknown)))
        {
            out.push(R::OptionalContent);
        }
        if rec.before.gs.soft_mask {
            out.push(R::SoftMask);
        }
        let fills = matches!(tr, 0 | 2 | 4 | 6);
        let strokes = matches!(tr, 1 | 2 | 5 | 6);
        if (fills && rec.before.fill.pattern) || (strokes && rec.before.stroke.pattern) {
            out.push(R::Pattern);
        }
        if self.actual_text(rec) {
            out.push(R::ActualText);
        }
        match &rec.font {
            None => out.push(R::MissingFont),
            Some(f) => {
                if let Some(r) = f.refusal.filter(|r| !r.is_page_level()) {
                    out.push(r);
                }
                if rec.glyphs.iter().any(|g| width_unknown(f, g.code)) {
                    out.push(R::MissingWidths);
                }
            }
        }
        // Drawn where an earlier op of this text object left the pen after an advance the model
        // cannot know: the rest of the line is not where the model puts it.
        if rec.pen_unknown {
            out.push(R::MissingWidths);
        }
        let undecodable = rec.split_error
            || rec
                .glyphs
                .iter()
                .any(|g| g.text.as_deref().map_or(true, |t| !usable_text(t)));
        if undecodable {
            out.push(R::AmbiguousUnicode);
        }
        for g in &rec.glyphs {
            for ch in g.text.as_deref().unwrap_or_default().chars() {
                if let Some(r) = reading_reason(ch) {
                    out.push(r);
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The ink box must lie inside the visible box and the clip (§A.6); a complex clip refuses.
    fn clipped(&self, rec: &ShowRecord) -> bool {
        let corners: Vec<(f64, f64)> = rec
            .glyphs
            .iter()
            .flat_map(|g| [(g.bbox[0], g.bbox[1]), (g.bbox[2], g.bbox[3])])
            .collect();
        let Some(ink) = bbox_of(&corners) else {
            return false;
        };
        if !contains(self.walk.geometry.visible, ink, CLIP_CONTAIN_TOL_PT) {
            return true;
        }
        match &rec.before.clip {
            ClipState::None => false,
            ClipState::Rect(r) => !contains(*r, ink, CLIP_CONTAIN_TOL_PT),
            ClipState::Complex => true,
        }
    }

    /// `/ActualText` or `/E` on the marked-content stack, or on the structure element of the
    /// innermost MCID or one of its ancestors (D19). A broken structure tree counts as present.
    fn actual_text(&self, rec: &ShowRecord) -> bool {
        if rec.before.marked.iter().any(|m| m.actual_text) {
            return true;
        }
        if rec.depth > 0 {
            return false;
        }
        match innermost_mcid(rec) {
            Some(mcid) => !matches!(
                self.page_struct.actual_text(self.ctx.doc(), mcid),
                Ok(false)
            ),
            None => false,
        }
    }
}

/// The MCID of the innermost marked-content sequence that has one (`iter` is innermost first).
pub(crate) fn innermost_mcid(rec: &ShowRecord) -> Option<i64> {
    rec.before.marked.iter().find_map(|m| m.mcid)
}
