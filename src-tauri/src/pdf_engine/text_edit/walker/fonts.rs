//! The walk's fonts (SPEC §B.9, §B.10): loaded once per walk through the snapshot's
//! `FontCache`, charged to the page-model budget once per model (`budget::ModelBudget::font`:
//! a page model keeps its fonts alive in its records and page fonts however the cache evicts,
//! review T3-budget HIGH-1), and the depth-0 page fonts in first-use order, then the unused ones.

use super::budget::map_entry;
use super::{show, Stop, Walked, Walker};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::fonts::{FontKey, FontModel};
use crate::pdf_engine::text_edit::limits::{
    FONTS_PER_PAGE_MAX, PAGE_DECODE_BUDGET, PAGE_UNUSED_FONT_BYTES_MAX,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use lopdf::Dictionary;
use std::mem::size_of;
use std::sync::Arc;

/// The unused page fonts may take at most this share of what the walk left (1 / n), so the run
/// stage keeps room.
const UNUSED_FONT_SHARE: usize = 4;

impl<'a> Walker<'a> {
    /// The font of `key`, loaded once per walk and charged to the page budget (held: the model
    /// keeps it in its records and page fonts however the snapshot's `FontCache` evicts). A load
    /// that ran out of the decode budget, or a model that does not fit the page budget, refuses
    /// the page `PAGE_TOO_COMPLEX` (review T3-budget HIGH-1).
    pub(crate) fn load_font(
        &mut self,
        key: FontKey,
        dict: &'a Dictionary,
    ) -> Result<Arc<FontModel>, Stop> {
        if let Some(model) = self.fonts.get(&key) {
            return Ok(Arc::clone(model));
        }
        if self.fonts.len() >= FONTS_PER_PAGE_MAX {
            return Err(Stop::new(TextReason::PageTooComplex, "fonts per page"));
        }
        self.mem
            .scratch(map_entry::<FontKey, Arc<FontModel>>() + map_entry::<FontKey, bool>())?;
        let model = self
            .ctx
            .fonts
            .get_or_load(self.doc, key, dict, &mut self.budget);
        if model.refusal == Some(TextReason::PageTooComplex) {
            return Err(Stop::new(TextReason::PageTooComplex, "font decode budget"));
        }
        self.mem.font(&model)?;
        self.pen_proofs
            .insert(key, show::advance_proven(self.doc, dict, &model));
        self.fonts.insert(key, Arc::clone(&model));
        Ok(model)
    }

    /// Records a font of the depth-0 resources in first-use order (its name kept twice: in the
    /// page fonts and in the walk's name set).
    pub(crate) fn note_root_font(&mut self, name: &[u8], model: &Arc<FontModel>) -> Walked {
        if !self.root_font_names.contains(name) {
            self.mem.hold(name.len())?;
            self.mem.scratch(name.len() + map_entry::<Vec<u8>, ()>())?;
            self.mem.grow(&mut self.root_fonts, 1)?;
            self.root_font_names.insert(name.to_vec());
            self.root_fonts.push((name.to_vec(), Arc::clone(model)));
        }
        Ok(())
    }

    /// Appends the unused fonts of the depth-0 resources (siblings for faces and joins). They are
    /// loaded under a decode budget of their own (`PAGE_DECODE_BUDGET`, shared by all of them),
    /// and their models are charged to the page budget within an allowance
    /// (`PAGE_UNUSED_FONT_BYTES_MAX`, and `1 / UNUSED_FONT_SHARE` of what the walk left): a font that does not fit is left out — one less sibling to type with — instead of
    /// refusing the page, so a document-wide `/Resources` full of large programs never refuses
    /// the text a page actually draws.
    pub(super) fn finish_page_fonts(&mut self) -> Walked {
        let res = self.root_res;
        if res.font_count(self.doc) > FONTS_PER_PAGE_MAX {
            return Err(Stop::new(TextReason::PageTooComplex, "fonts per page"));
        }
        // Every name is copied once more before the used ones are skipped.
        let names = res.font_name_bytes(self.doc);
        self.mem
            .hold(names.saturating_add(res.font_count(self.doc) * size_of::<FontKey>()))?;
        let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
        let allowance = PAGE_UNUSED_FONT_BYTES_MAX.min(self.mem.left() / UNUSED_FONT_SHARE);
        let charged = self.mem.font_bytes();
        for (name, key, dict) in res.all_fonts(self.doc) {
            if self.root_font_names.contains(&name) {
                continue;
            }
            let model = match self.fonts.get(&key) {
                Some(model) => Arc::clone(model),
                None => self.ctx.fonts.get_or_load(self.doc, key, dict, &mut budget),
            };
            let left = allowance.saturating_sub(self.mem.font_bytes().saturating_sub(charged));
            if model.refusal == Some(TextReason::PageTooComplex)
                || !self.mem.font_within(&model, left)
            {
                continue;
            }
            self.mem.push(&mut self.root_fonts, (name, model))?;
        }
        Ok(())
    }
}
