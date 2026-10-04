//! The editor's width estimate (SPEC §A.8, §D.4) in Rust, test-only: the same model as the
//! frontend's `estimateDeltaPt`/`estimateCaretOffsets` (`src/lib/editor/sourceText.ts`),
//! golden-tested against `src/lib/editor/__fixtures__/text-measure-golden.json` (MEAS-01). Rust
//! production never estimates: verdicts carry the planned `delta_pt` and caret offsets.
//!
//! For a run and a requested style: `tf = Tfs × size / effective` (or `Tfs`), `k = Th ×
//! text_to_user_x`, `tc = spacing / k` (or `Tc`); per character: a kern-mode space advances
//! `−kern_space/1000 × tf`; any other character the first surface font that can type it
//! (`w/1000 × tf + tc`, plus `Tw` for a 0x20 space), else `MISSING_GLYPH_EM × tf + tc`; each
//! advance is clamped at 0 and scaled by `k`. The delta is relative: new text with the style
//! minus the current text without it.

use super::word_space;
use crate::pdf_engine::text_edit::fonts::{face_surface, FontModel};
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::runs::{SpaceMode, TextRun};
use std::sync::Arc;

/// Advance (em) of a character no surface font can draw, in the estimate only (same as the TS).
pub const MISSING_GLYPH_EM: f64 = 0.5;

/// The fonts that type `face` for `run`: its own surface for its own face, else the face surface
/// of the page fonts (empty when the page has none).
fn surface_for(
    run: &TextRun,
    fonts: &[(Vec<u8>, Arc<FontModel>)],
    face: Option<crate::pdf_engine::text_edit::reasons::Face>,
) -> Vec<Arc<FontModel>> {
    let by_name = |name: &[u8]| {
        fonts
            .iter()
            .find(|(n, _)| n.as_slice() == name)
            .map(|(_, m)| Arc::clone(m))
    };
    match face {
        Some(f) if f != run.face => run
            .surface
            .first()
            .and_then(|n| by_name(n))
            .and_then(|primary| face_surface(fonts, &primary, f))
            .map(|s| s.fonts.into_iter().map(|(_, m)| m).collect())
            .unwrap_or_default(),
        _ => run.surface.iter().filter_map(|n| by_name(n)).collect(),
    }
}

struct Model {
    tf: f64,
    tc: f64,
    k: f64,
    surface: Vec<(Vec<(char, f64)>, bool)>,
}

fn model_for(
    run: &TextRun,
    fonts: &[(Vec<u8>, Arc<FontModel>)],
    style: &SourceTextStyleIn,
) -> Model {
    let k_raw = run.th * run.text_to_user_x;
    let k = if k_raw.is_finite() { k_raw } else { 0.0 };
    let tf = match style.size_pt {
        Some(pt) if pt.is_finite() && run.effective_size > 0.0 => run.tfs * pt / run.effective_size,
        _ => run.tfs,
    };
    let tc = match style.letter_spacing_pt {
        Some(pt) if pt.is_finite() && k > 0.0 => pt / k,
        _ => run.tc,
    };
    let surface = surface_for(run, fonts, style.face)
        .iter()
        .map(|m| (m.alphabet(), word_space(m)))
        .collect();
    Model { tf, tc, k, surface }
}

fn advances(
    run: &TextRun,
    fonts: &[(Vec<u8>, Arc<FontModel>)],
    text: &str,
    style: &SourceTextStyleIn,
) -> Vec<f64> {
    let m = model_for(run, fonts, style);
    text.chars()
        .map(|ch| {
            let advance = if ch == ' ' && run.space_mode == SpaceMode::Kern {
                -run.kern_space / 1000.0 * m.tf
            } else {
                let hit = m.surface.iter().find_map(|(alphabet, ws)| {
                    alphabet
                        .binary_search_by(|(c, _)| c.cmp(&ch))
                        .ok()
                        .and_then(|i| alphabet.get(i))
                        .map(|(_, w)| (if w.is_finite() { *w } else { 0.0 }, *ws))
                });
                let em = hit.map_or(MISSING_GLYPH_EM, |(w, _)| w / 1000.0);
                let word = if ch == ' ' && hit.is_some_and(|(_, ws)| ws) {
                    run.tw
                } else {
                    0.0
                };
                em * m.tf + m.tc + word
            };
            if advance.is_finite() {
                advance.max(0.0) * m.k
            } else {
                0.0
            }
        })
        .collect()
}

/// Estimated width change in points (positive = wider) of `text` with `style` on `run`.
pub fn estimate_delta_pt(
    run: &TextRun,
    fonts: &[(Vec<u8>, Arc<FontModel>)],
    text: &str,
    style: &SourceTextStyleIn,
) -> f64 {
    let new: f64 = advances(run, fonts, text, style).iter().sum();
    let old: f64 = advances(run, fonts, &run.text, &SourceTextStyleIn::default())
        .iter()
        .sum();
    new - old
}

/// `chars + 1` caret distances along `dir` for `text`, same model as `estimate_delta_pt`.
pub fn estimate_caret_offsets(
    run: &TextRun,
    fonts: &[(Vec<u8>, Arc<FontModel>)],
    text: &str,
    style: &SourceTextStyleIn,
) -> Vec<f64> {
    let mut out = vec![0.0];
    let mut at = 0.0;
    for a in advances(run, fonts, text, style) {
        at += a;
        out.push(at);
    }
    out
}
