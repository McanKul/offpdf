//! The glyphs our own model draws on and around the edited lines (review-verify HIGH-A). They only
//! tell A5 where to look harder; Poppler's words and pixels still decide.
//! - G-TEXT (`words.rs`) counts a Poppler word that holds glyphs of runs no edit changes as a
//!   neighbour wherever it lies: a narrow follower right after the edited run (a 7 pt footnote
//!   marker, a ":" in another colour) has its centre within `EDIT_BAND_PAD_PT` of the edited
//!   run's old box, which made it the run's own word. A word Poppler joined across the edited run
//!   and such a run ("Hello:") is split: the kept glyphs at either end are a neighbour pinned at
//!   that outer edge.
//! - G-RENDER (`poppler.rs`) and the ink check (`ink.rs`) take the kept glyphs' boxes out of the
//!   masks wherever the edited glyphs' own boxes (the new glyphs' masks, the old glyphs' ink
//!   boxes) do not cover them, so a follower's pixels must not change.

use super::{dilate, union, user_rect_to_text_frame, words::fold, PT_PER_INCH};
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::limits::{EDIT_BAND_PAD_PT, RENDER_MASK_PAD_PX};
use crate::pdf_engine::text_edit::rewrite::ExpectedRun;
use crate::pdf_engine::text_edit::runs::{PageModel, TextRun};
use std::collections::HashSet;

/// A glyph of a run no edit changes.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptGlyph {
    /// User space: the advance × the font's ascent/descent, rise included (the walk's box).
    pub rect: [f64; 4],
    /// Its text folded as G-TEXT folds Poppler's words (`None`: the font maps it to none).
    pub text: Option<String>,
}

/// What A5 learns from the model about the edited lines.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NearGlyphs {
    /// The glyphs of the runs no edit changes, in the rows the edits' bands and masks span
    /// (Poppler's frame). Records whose pen the model does not know are left out.
    pub kept: Vec<KeptGlyph>,
    /// The edited runs' old glyphs that read as something (user space, the walk's boxes): which
    /// words were theirs.
    pub edited: Vec<[f64; 4]>,
}

impl NearGlyphs {
    /// From `model`, the page as qpdf's input has it; `edited`, its runs the plan changes;
    /// `edits`, A5's per-edit expectations and old and new rects.
    pub fn of(
        model: &PageModel,
        edited: &[&TextRun],
        edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
        geom: &PageGeometry,
        dpi: u32,
    ) -> NearGlyphs {
        let members: HashSet<usize> = edited
            .iter()
            .flat_map(|r| r.members.iter().copied())
            .collect();
        let rows = rows(edits, geom, dpi);
        let in_rows = |r: &[f64; 4]| {
            let f = user_rect_to_text_frame(*r, geom);
            rows.iter().any(|(lo, hi)| f[1] < *hi && *lo < f[3])
        };
        let mut near = NearGlyphs::default();
        for (i, rec) in model.walk.records.iter().enumerate() {
            if members.contains(&i) {
                let inked = rec
                    .glyphs
                    .iter()
                    .filter(|g| g.text.as_deref().map(fold) != Some(String::new()));
                near.edited.extend(inked.map(|g| g.bbox));
            } else if !rec.pen_unknown {
                let kept = rec.glyphs.iter().filter(|g| in_rows(&g.bbox));
                near.kept.extend(kept.map(|g| KeptGlyph {
                    rect: g.bbox,
                    text: g.text.as_deref().map(fold),
                }));
            }
        }
        near
    }
}

/// Per edit, the rows (Poppler's frame, `(y0, y1)`) of its band and masks: the old and new rects
/// and every mask box, grown by `EDIT_BAND_PAD_PT` and then `RENDER_MASK_PAD_PX` at `dpi`.
fn rows(
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    geom: &PageGeometry,
    dpi: u32,
) -> Vec<(f64, f64)> {
    let slack = RENDER_MASK_PAD_PX as f64 * PT_PER_INCH / f64::from(dpi.max(1));
    edits
        .iter()
        .map(|(exp, old, new, _)| {
            let all = exp
                .mask_boxes
                .iter()
                .fold(union(*old, *new), |a, b| union(a, *b));
            let f = user_rect_to_text_frame(dilate(all, EDIT_BAND_PAD_PT), geom);
            (f[1] - slack, f[3] + slack)
        })
        .collect()
}
