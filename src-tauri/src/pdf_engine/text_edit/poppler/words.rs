//! G-TEXT (SPEC §B.15): Poppler's words before and after the edit. Words outside the edited
//! bands must stay 1:1 (text and box within `WORD_BBOX_TOL_PT`). Inside a band, the words of the
//! line's other runs (its neighbours: table cells, tab stops) must still be extracted as the same
//! word at their place, or joined with the changed line's glyphs at their outer edge, and are set
//! aside; what remains must hold the new text, and the old text must be gone. Their boxes go on
//! to G-RENDER's ink check (`poppler/ink.rs`), which sees what the edit's masks hide.
//!
//! A word is a neighbour when it holds glyphs of runs no edit changes (`poppler/near.rs`, our
//! model only saying where to look), wherever it lies, else when its centre lies outside every
//! edited run's old box (+ `EDIT_BAND_PAD_PT`). A word Poppler joined across an edited run and
//! such glyphs ("Hello:" with a ":" in another colour) is split: the edited part is the run's, and
//! the kept glyphs at its start or end are a neighbour pinned at that outer edge (review-verify
//! HIGH-A).

use super::{dilate, union, user_rect_to_text_frame, NearGlyphs, Word};
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::limits::{EDIT_BAND_PAD_PT, WORD_BBOX_TOL_PT};
use crate::pdf_engine::text_edit::rewrite::ExpectedRun;

/// A G-TEXT failure: (check id, detail).
type Fail = (&'static str, String);

/// G-TEXT word matching: bucket entries one comparison may pass over, per word on the page (an
/// honest page's unchanged words have the same boxes on both sides, so a search stops at its
/// first look; only words of equal text a hair apart cost more), and at least this many in all.
const WORD_SCAN_PER_WORD: usize = 16;
const WORD_SCAN_MIN: usize = 4_096;
/// Compatibility folding for the word comparison (the NFKC cases pdftotext output needs: the
/// FB00–FB06 ligatures, no-break and other spaces, fullwidth ASCII), whitespace removed.
pub(crate) fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{fb00}' => out.push_str("ff"),
            '\u{fb01}' => out.push_str("fi"),
            '\u{fb02}' => out.push_str("fl"),
            '\u{fb03}' => out.push_str("ffi"),
            '\u{fb04}' => out.push_str("ffl"),
            '\u{fb05}' | '\u{fb06}' => out.push_str("st"),
            '\u{ff01}'..='\u{ff5e}' => {
                if let Some(a) = char::from_u32(u32::from(c) - 0xfee0) {
                    out.push(a);
                }
            }
            c if c.is_whitespace() || c == '\u{a0}' || c == '\u{2007}' || c == '\u{202f}' => {}
            c => out.push(c),
        }
    }
    out
}

fn count_of(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    hay.match_indices(needle).count()
}

fn in_band(w: &Word, band: (f64, f64)) -> bool {
    w.y0 < band.1 && band.0 < w.y1
}

fn same_word(a: &Word, b: &Word) -> bool {
    a.text == b.text
        && (a.x0 - b.x0).abs() <= WORD_BBOX_TOL_PT
        && (a.y0 - b.y0).abs() <= WORD_BBOX_TOL_PT
        && (a.x1 - b.x1).abs() <= WORD_BBOX_TOL_PT
        && (a.y1 - b.y1).abs() <= WORD_BBOX_TOL_PT
}

/// The bucket cell of a word coordinate: cells are two tolerances wide, so the values within
/// `WORD_BBOX_TOL_PT` of `v` lie in at most two cells (`cells`).
fn cell(v: f64) -> i64 {
    (v / (2.0 * WORD_BBOX_TOL_PT)).floor() as i64
}

fn cells(v: f64) -> std::ops::RangeInclusive<i64> {
    cell(v - WORD_BBOX_TOL_PT)..=cell(v + WORD_BBOX_TOL_PT)
}

fn moved(w: &Word) -> Fail {
    (
        "words",
        format!(
            "word {:?} at ({:.2}, {:.2}) moved or changed",
            w.text, w.x0, w.y0
        ),
    )
}

/// Matches words of `src` 1:1 to words of `dst` (text equal, every coordinate within
/// `WORD_BBOX_TOL_PT`). Returns the indices of the `dst` words left over, in order, and the
/// `src` words nothing matched. `dst` is bucketed by (text, x0 cell, y0 cell); a match is removed
/// from its bucket, and the entries all searches pass over are bounded (`WORD_SCAN_PER_WORD`), so
/// the comparison is linear in the word count however the words are ordered.
fn match_some<'a>(src: &[&'a Word], dst: &[&Word]) -> Result<(Vec<usize>, Vec<&'a Word>), Fail> {
    let mut buckets: std::collections::HashMap<(&str, i64, i64), Vec<usize>> =
        std::collections::HashMap::with_capacity(dst.len());
    for (i, w) in dst.iter().enumerate() {
        buckets
            .entry((w.text.as_str(), cell(w.x0), cell(w.y0)))
            .or_default()
            .push(i);
    }
    let limit = WORD_SCAN_PER_WORD
        .saturating_mul(src.len())
        .max(WORD_SCAN_MIN);
    let mut scanned = 0usize;
    let mut missed = Vec::new();
    'words: for w in src {
        for cx in cells(w.x0) {
            for cy in cells(w.y0) {
                let Some(list) = buckets.get_mut(&(w.text.as_str(), cx, cy)) else {
                    continue;
                };
                let mut found = None;
                for (pos, d) in list.iter().enumerate() {
                    scanned = scanned.saturating_add(1);
                    if scanned > limit {
                        return Err(("words", "too many overlapping words to compare".to_string()));
                    }
                    if dst.get(*d).is_some_and(|d| same_word(w, d)) {
                        found = Some(pos);
                        break;
                    }
                }
                if let Some(pos) = found {
                    list.swap_remove(pos);
                    continue 'words;
                }
            }
        }
        missed.push(*w);
    }
    let mut rest: Vec<usize> = buckets.into_values().flatten().collect();
    rest.sort_unstable();
    Ok((rest, missed))
}

/// Every word of `src` must match a word of `dst` 1:1.
fn match_words(src: &[&Word], dst: &[&Word]) -> Result<(), Fail> {
    match match_some(src, dst)?.1.first() {
        Some(w) => Err(moved(w)),
        None => Ok(()),
    }
}

/// The middle of a word's box lies in `r` (`[x0, y0, x1, y1]`, text frame).
fn centred_in(w: &Word, r: &[f64; 4]) -> bool {
    let (x, y) = ((w.x0 + w.x1) / 2.0, (w.y0 + w.y1) / 2.0);
    r[0] <= x && x <= r[2] && r[1] <= y && y <= r[3]
}

/// Which edges of a neighbour are its own (review-verify HIGH-A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// A whole source word: both.
    Whole,
    /// The kept glyphs a joined word starts with: its x0 only (x1 is the model's).
    Start,
    /// The kept glyphs a joined word ends with: its x1 only (x0 is the model's).
    End,
}

/// A neighbour of an edited line: a source word, or the part of one at an outer edge.
#[derive(Debug, Clone)]
struct Neighbour {
    word: Word,
    edge: Edge,
}

/// The text of `d` without the neighbour `n`'s (`part`), when `d` is `n` joined by Poppler with
/// glyphs of the changed line — the only inexact form a whole neighbour may take (review-final
/// HIGH-1): `d` reads longer than `n`, lies on `n`'s line (y0 and y1 within `WORD_BBOX_TOL_PT`),
/// keeps `n`'s outer edge (x1 for a word ending with `n`, x0 for one starting with it, within the
/// same tolerance), and reaches past `n`'s inner edge into the changed line's new box (`new_box`,
/// text frame). A neighbour moved, relabelled or swallowed anywhere else is not accepted. A part
/// of a joined word (`edge` `Start`/`End`) has only that outer edge, and may also come back as a
/// word of its own (`d` reads exactly `part` there: the empty rest), or — when the new box holds
/// the whole part (a "." under a new, wider "e") — inside a word that spans its box, joined with
/// the new glyphs, whose part past the part's outer edge lies in the new box (`covered`).
fn joined_with_edit(
    d: &Word,
    text: &str,
    n: &Word,
    part: &str,
    new_box: &[f64; 4],
    edge: Edge,
) -> Option<String> {
    let tol = WORD_BBOX_TOL_PT;
    let near = |a: f64, b: f64| (a - b).abs() <= tol;
    if !near(d.y0, n.y0) || !near(d.y1, n.y1) {
        return None;
    }
    let at_end = edge != Edge::Start && near(d.x1, n.x1);
    let at_start = edge != Edge::End && near(d.x0, n.x0);
    if edge != Edge::Whole && (at_end || at_start) && text == part {
        return Some(String::new());
    }
    if text.len() <= part.len() {
        return None;
    }
    let in_new_box = |lo: f64, hi: f64| lo < new_box[2] && new_box[0] < hi;
    if at_end && d.x0 < n.x0 - tol && in_new_box(d.x0, n.x0) {
        if let Some(head) = text.strip_suffix(part) {
            return Some(head.to_string());
        }
    }
    if at_start && d.x1 > n.x1 + tol && in_new_box(n.x1, d.x1) {
        if let Some(tail) = text.strip_prefix(part) {
            return Some(tail.to_string());
        }
    }
    match edge {
        Edge::Whole => None,
        _ => covered(d, text, n, part, new_box, edge),
    }
}

/// A part of a joined word that the new glyphs cover: Poppler merges it into the new word, so its
/// outer edge is no longer a word's. Accepted inside `d` when the new box holds the part, `d`
/// spans the part's box, reaches past its inner edge into the new box, and ends past its outer
/// edge only within the new box; the part's text is taken out of `d`'s once (its last
/// occurrence for an end part, its first for a start part).
fn covered(
    d: &Word,
    text: &str,
    n: &Word,
    part: &str,
    new_box: &[f64; 4],
    edge: Edge,
) -> Option<String> {
    let tol = WORD_BBOX_TOL_PT;
    let inside = |lo: f64, hi: f64| new_box[0] - tol <= lo && hi <= new_box[2] + tol;
    if part.is_empty() || !inside(n.x0, n.x1) || d.x0 > n.x0 + tol || d.x1 < n.x1 - tol {
        return None;
    }
    let reaches = match edge {
        Edge::End => d.x0 < n.x0 - tol && inside(n.x1, d.x1),
        _ => d.x1 > n.x1 + tol && inside(d.x0, n.x0),
    };
    let at = match edge {
        Edge::End => text.rfind(part),
        _ => text.find(part),
    };
    let at = at.filter(|_| reaches && text.len() > part.len())?;
    let rest = [text.get(..at)?, text.get(at + part.len()..)?].concat();
    Some(rest)
}

/// Sets a band's neighbours aside. A whole word must be extracted after the edit as the same word
/// (text and box within `WORD_BBOX_TOL_PT`), and any neighbour may be joined with the changed
/// line's glyphs (`joined_with_edit`). Returns the band's other words (folded, in Poppler's
/// order), and the first neighbour not found (reported after G-RENDER, so a moved follower still
/// fails the pixels first, as IND-09 pins).
fn set_aside(
    neighbours: &[Neighbour],
    dst: &[&Word],
    new_box: &[f64; 4],
) -> Result<(Vec<String>, Option<Fail>), Fail> {
    let whole: Vec<&Word> = neighbours
        .iter()
        .filter(|n| n.edge == Edge::Whole)
        .map(|n| &n.word)
        .collect();
    let (rest, missed) = match_some(&whole, dst)?;
    let mut words: Vec<(&Word, String)> = rest
        .iter()
        .filter_map(|i| dst.get(*i))
        .map(|w| (*w, fold(&w.text)))
        .collect();
    let parts = neighbours.iter().filter(|n| n.edge != Edge::Whole);
    let unmatched =
        (missed.into_iter().map(|w| (w, Edge::Whole))).chain(parts.map(|n| (&n.word, n.edge)));
    let mut miss = None;
    for (n, edge) in unmatched {
        let part = fold(&n.text);
        let joined = words.iter_mut().find_map(|(d, text)| {
            joined_with_edit(d, text, n, &part, new_box, edge).map(|left| *text = left)
        });
        if joined.is_none() && miss.is_none() {
            let (check, detail) = moved(n);
            miss = Some((check, format!("next to the edited line: {detail}")));
        }
    }
    Ok((words.into_iter().map(|(_, t)| t).collect(), miss))
}

/// A glyph of our model (`NearGlyphs`) in Poppler's frame.
#[derive(Debug)]
struct FrameGlyph {
    /// The centre.
    cx: f64,
    cy: f64,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
    /// `None` for an edited run's old glyph; a kept glyph's folded text (`Some(None)`: unknown).
    kept: Option<Option<String>>,
}

/// The model's glyphs by the x of their centres; a kept glyph that reads as nothing (a space) is
/// left out.
fn frame_glyphs(near: &NearGlyphs, geom: &PageGeometry) -> Vec<FrameGlyph> {
    let edited = near.edited.iter().map(|r| (r, None));
    let kept = (near.kept.iter())
        .filter(|k| k.text.as_deref() != Some(""))
        .map(|k| (&k.rect, Some(k.text.clone())));
    let mut out: Vec<FrameGlyph> = edited
        .chain(kept)
        .map(|(r, kept)| {
            let f = user_rect_to_text_frame(*r, geom);
            FrameGlyph {
                cx: (f[0] + f[2]) / 2.0,
                cy: (f[1] + f[3]) / 2.0,
                x0: f[0],
                x1: f[2],
                y0: f[1],
                y1: f[3],
                kept,
            }
        })
        .filter(|g| g.cx.is_finite() && g.cy.is_finite())
        .collect();
    out.sort_by(|a, b| a.cx.total_cmp(&b.cx));
    out
}

/// The glyphs of `w`, by x: a glyph's centre lies in `w`'s box and `w`'s middle height within
/// the glyph's (both within `WORD_BBOX_TOL_PT`; a 9 pt stamp drawn over a 26 pt title is not the
/// title's). Each glyph looked at costs one unit of `budget`.
fn glyphs_in<'g>(
    glyphs: &'g [FrameGlyph],
    w: &Word,
    budget: &mut usize,
) -> Result<Vec<&'g FrameGlyph>, Fail> {
    let tol = WORD_BBOX_TOL_PT;
    let from = glyphs.partition_point(|g| g.cx < w.x0 - tol);
    let mut out = Vec::new();
    for g in glyphs.get(from..).unwrap_or_default() {
        if g.cx > w.x1 + tol {
            break;
        }
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| ("words", "too many overlapping words to compare".to_string()))?;
        let middle = (w.y0 + w.y1) / 2.0;
        let own = g.y0 - tol <= middle && middle <= g.y1 + tol;
        if own && w.y0 - tol <= g.cy && g.cy <= w.y1 + tol {
            out.push(g);
        }
    }
    Ok(out)
}

/// One source word of an edited band: the text it gives the edited run (folded) and the
/// neighbours it holds. `inside`: our glyphs in it, by x; `in_a_run`: its centre lies in an edited
/// run's padded old box. A word that cannot be split cleanly (kept glyphs between edited ones,
/// texts that do not match Poppler's) is a whole neighbour: it must come back unchanged (fail
/// closed).
fn classify(w: &Word, inside: &[&FrameGlyph], in_a_run: bool) -> (Option<String>, Vec<Neighbour>) {
    let whole = || {
        let word = w.clone();
        (
            None,
            vec![Neighbour {
                word,
                edge: Edge::Whole,
            }],
        )
    };
    let Some(first) = inside.iter().position(|g| g.kept.is_none()) else {
        return match (inside.is_empty(), in_a_run) {
            (true, true) => (Some(fold(&w.text)), Vec::new()),
            _ => whole(),
        };
    };
    if inside.iter().all(|g| g.kept.is_none()) {
        return (Some(fold(&w.text)), Vec::new());
    }
    let last = inside
        .iter()
        .rposition(|g| g.kept.is_none())
        .unwrap_or(first);
    let (start, mid, end) = (
        inside.get(..first).unwrap_or_default(),
        inside.get(first..=last).unwrap_or_default(),
        inside.get(last + 1..).unwrap_or_default(),
    );
    let text_of = |gs: &[&FrameGlyph]| -> Option<String> {
        gs.iter()
            .map(|g| g.kept.clone().flatten())
            .collect::<Option<Vec<String>>>()
            .map(|v| v.concat())
    };
    let (Some(a), Some(z), false) = (
        text_of(start),
        text_of(end),
        mid.iter().any(|g| g.kept.is_some()),
    ) else {
        return whole();
    };
    let folded = fold(&w.text);
    let head = folded
        .strip_prefix(a.as_str())
        .and_then(|r| r.strip_suffix(z.as_str()));
    let Some(head) = head.filter(|h| !h.is_empty()) else {
        return whole();
    };
    let part = |text: String, x0: f64, x1: f64, edge: Edge| Neighbour {
        word: Word {
            text,
            x0,
            y0: w.y0,
            x1,
            y1: w.y1,
        },
        edge,
    };
    let mut parts = Vec::new();
    if let Some(x1) = start.iter().map(|g| g.x1).reduce(f64::max) {
        parts.push(part(a, w.x0, x1, Edge::Start));
    }
    if let Some(x0) = end.iter().map(|g| g.x0).reduce(f64::min) {
        parts.push(part(z, x0, w.x1, Edge::End));
    }
    (Some(head.to_string()), parts)
}

/// What G-TEXT leaves for later: a neighbour that is gone (reported after G-RENDER), and the
/// boxes of every neighbour of an edited line (text frame) for G-RENDER's ink check.
pub(super) struct WordsOutcome {
    pub gone: Option<Fail>,
    pub neighbours: Vec<[f64; 4]>,
}

/// G-TEXT. `near`: our model's glyphs around the edits (where a word is a neighbour).
pub(super) fn check_words(
    src: &[Word],
    dst: &[Word],
    geom: &PageGeometry,
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    near: &NearGlyphs,
) -> Result<WordsOutcome, Fail> {
    let bands: Vec<(f64, f64)> = edits
        .iter()
        .map(|(_, old, new, _)| {
            let f = user_rect_to_text_frame(dilate(union(*old, *new), EDIT_BAND_PAD_PT), geom);
            (f[1], f[3])
        })
        .collect();
    let outside = |w: &&Word| !bands.iter().any(|b| in_band(w, *b));
    let src_out: Vec<&Word> = src.iter().filter(outside).collect();
    let dst_out: Vec<&Word> = dst.iter().filter(outside).collect();
    if src_out.len() != dst_out.len() {
        return Err((
            "words",
            format!(
                "words outside the edited lines: {} before, {} after",
                src_out.len(),
                dst_out.len()
            ),
        ));
    }
    match_words(&src_out, &dst_out)?;
    // Where each edited run was (its old box, padded): a word there before the edit is the run's.
    let runs: Vec<[f64; 4]> = edits
        .iter()
        .map(|(_, old, _, _)| user_rect_to_text_frame(dilate(*old, EDIT_BAND_PAD_PT), geom))
        .collect();
    let in_a_run = |w: &Word| runs.iter().any(|r| centred_in(w, r));
    let glyphs = frame_glyphs(near, geom);
    let mut budget = WORD_SCAN_PER_WORD
        .saturating_mul(src.len().saturating_add(glyphs.len()))
        .max(WORD_SCAN_MIN);
    let mut gone = None;
    let mut neighbour_boxes = Vec::new();
    for ((exp, _, new_rect, old), band) in edits.iter().zip(&bands) {
        let src_band: Vec<&Word> = src.iter().filter(|w| in_band(w, *band)).collect();
        let dst_band: Vec<&Word> = dst.iter().filter(|w| in_band(w, *band)).collect();
        // Neighbours on the edited line (table cells, tab stops) are set aside: Poppler orders a
        // line's words by position, so a changed line that now runs into them would interleave
        // with them (review-T5 live B2: an overlap is a warning, not a refusal).
        let mut before: Vec<String> = Vec::new();
        let mut neighbours: Vec<Neighbour> = Vec::new();
        for w in &src_band {
            let inside = glyphs_in(&glyphs, w, &mut budget)?;
            let (own, parts) = classify(w, &inside, in_a_run(w));
            before.extend(own);
            neighbours.extend(parts);
        }
        let new_box = user_rect_to_text_frame(dilate(*new_rect, EDIT_BAND_PAD_PT), geom);
        let (after, miss) = set_aside(&neighbours, &dst_band, &new_box)?;
        neighbour_boxes.extend(neighbours.iter().map(|n| {
            let w = &n.word;
            [w.x0, w.y0, w.x1, w.y1]
        }));
        // A neighbour that moved stays among the line's words, between the new ones: it is the
        // reason the new text does not read (review-final HIGH-1).
        band_text(exp, old, &before, &after).map_err(|f| {
            match (&miss, f.1.starts_with(NEW_TEXT)) {
                (Some(m), true) => m.clone(),
                _ => f,
            }
        })?;
        gone = gone.or(miss);
    }
    Ok(WordsOutcome {
        gone,
        neighbours: neighbour_boxes,
    })
}

/// The start of the "new text not extracted" failure.
const NEW_TEXT: &str = "new text";

/// One band after its neighbours are set aside: the new text is extracted and the old text gone.
fn band_text(
    exp: &ExpectedRun,
    old: &str,
    before: &[String],
    after: &[String],
) -> Result<(), Fail> {
    let (before_text, after_text) = (before.concat(), after.concat());
    // The request, not the planner's reading of its glyphs (§B.15 "each non-empty new text").
    let new = fold(&exp.requested_text);
    if !new.is_empty() && !after_text.contains(&new) {
        return Err((
            "words",
            format!("{NEW_TEXT} {:?} not extracted", exp.requested_text),
        ));
    }
    let old_folded = fold(old);
    if !old_folded.is_empty() && !new.contains(&old_folded) {
        let (b, a) = (
            count_of(&before_text, &old_folded),
            count_of(&after_text, &old_folded),
        );
        if a + 1 > b {
            return Err(("words", format!("old text {old_folded:?} still extracted")));
        }
    }
    removed_words_gone(old, &exp.requested_text, before, after)
}

/// Every word the edit removed (a word of the old text the new text has fewer of) must be
/// extracted that many times fewer, when Poppler extracted it as a word before. Poppler drops
/// text drawn twice at almost one position and orders words by position, so a fake that keeps
/// the old text under new text can defeat the concatenated-text test above, not this one.
fn removed_words_gone(
    old: &str,
    new: &str,
    before: &[String],
    after: &[String],
) -> Result<(), Fail> {
    let words = |t: &str| -> Vec<String> {
        t.split_whitespace()
            .map(fold)
            .filter(|w| !w.is_empty())
            .collect()
    };
    let count = |list: &[String], w: &str| list.iter().filter(|x| x.as_str() == w).count();
    let (old_words, new_words) = (words(old), words(new));
    let mut seen: Vec<&String> = Vec::new();
    for w in &old_words {
        if seen.contains(&w) {
            continue;
        }
        seen.push(w);
        let removed = count(&old_words, w).saturating_sub(count(&new_words, w));
        let had = count(before, w);
        if removed == 0 || had < removed {
            continue;
        }
        if count(after, w) > had - removed {
            return Err(("words", format!("removed word {w:?} still extracted")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
