//! Planner steps 6–8 (SPEC §B.12): the minimal diff and the new unit list. Unchanged leading and
//! trailing units keep their codes and the kerns between them; the middle is encoded through the
//! typing surface (run, then page code preferences, §A.3.2); a typed space is a glyph or, in kern
//! mode, a TJ number (§A.3.4). At the two boundaries a pair kern tuned for a glyph pair that no
//! longer exists is dropped; synthetic-space kerns count as " " and stay with the side that
//! matched them. Kerns before the first glyph and after the last one are not between glyphs and
//! are always kept (unless the whole line is removed or re-faced).

use super::{problem, NewUnit, StyleTarget};
use crate::pdf_engine::text_edit::encode::num;
use crate::pdf_engine::text_edit::fonts::{Code, TypingSurface};
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode as P};
use crate::pdf_engine::text_edit::runs::{PageModel, SpaceMode, TextRun, Unit};

/// Where a unit of the new list comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Prefix,
    Middle,
    Suffix,
}

/// The text a unit contributes when matching: a glyph's text, " " for a synthetic-space kern,
/// nothing for any other kern.
fn unit_text(u: &Unit) -> Option<&str> {
    match u {
        Unit::Glyph { text, .. } => Some(text.as_str()),
        Unit::Kern {
            synth_space: true, ..
        } => Some(" "),
        Unit::Kern { .. } => None,
    }
}

fn starts_with(hay: &[char], needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    (hay.len() >= n.len() && hay.get(..n.len()) == Some(n.as_slice())).then_some(n.len())
}

fn ends_with(hay: &[char], needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    let start = hay.len().checked_sub(n.len())?;
    (hay.get(start..) == Some(n.as_slice())).then_some(n.len())
}

/// `(prefix_end, prefix_chars, suffix_start, suffix_chars)`: units `..prefix_end` and
/// `suffix_start..` are kept; they match the first `prefix_chars` and last `suffix_chars` chars.
fn kept_ranges(units: &[Unit], chars: &[char]) -> (usize, usize, usize, usize) {
    let first_glyph = units
        .iter()
        .position(|u| matches!(u, Unit::Glyph { .. }))
        .unwrap_or(units.len());
    let last_glyph_end = units
        .iter()
        .rposition(|u| matches!(u, Unit::Glyph { .. }))
        .map_or(units.len(), |i| i + 1);
    // Prefix: leading kerns, then whole text units while they match.
    let mut prefix_end = first_glyph;
    let mut matched = 0usize;
    let mut i = first_glyph;
    while let Some(u) = units.get(i) {
        match unit_text(u) {
            None => i += 1,
            Some(t) => match chars.get(matched..).and_then(|rest| starts_with(rest, t)) {
                Some(n) => {
                    matched += n;
                    i += 1;
                    prefix_end = i;
                }
                None => break,
            },
        }
    }
    let prefix_chars = matched;
    // Suffix: trailing kerns, then whole text units from the end while they match.
    let mut suffix_start = last_glyph_end.max(prefix_end);
    let mut matched_s = 0usize;
    let mut k = last_glyph_end;
    while k > prefix_end {
        let Some(u) = units.get(k - 1) else { break };
        match unit_text(u) {
            None => k -= 1,
            Some(t) => {
                let room = chars.len().saturating_sub(prefix_chars + matched_s);
                let end = chars.len().saturating_sub(matched_s);
                let fits = t.chars().count() <= room;
                match chars.get(..end).and_then(|head| ends_with(head, t)) {
                    Some(n) if fits => {
                        matched_s += n;
                        k -= 1;
                        suffix_start = k;
                    }
                    _ => break,
                }
            }
        }
    }
    (prefix_end, prefix_chars, suffix_start, matched_s)
}

/// Run and page code preferences (§A.3.2) for the surface fonts.
fn preferences(
    model: &PageModel,
    run: &TextRun,
    surface: &TypingSurface,
    wanted: &[char],
) -> (Vec<(usize, char, Code)>, Vec<(usize, char, Code)>) {
    let font_index = |res: Option<&[u8]>, hash: u64| {
        surface.fonts.iter().position(|(n, m)| {
            m.content_hash == hash && (n.as_slice() == res.unwrap_or_default() || n.is_empty())
        })
    };
    let single = |t: &str| {
        let mut it = t.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    };
    let mut prefer_run = Vec::new();
    for u in &run.units {
        if let Unit::Glyph {
            code,
            font_res,
            font_hash,
            text,
            ..
        } = u
        {
            if let (Some(ch), Some(idx)) =
                (single(text), font_index(font_res.as_deref(), *font_hash))
            {
                if wanted.contains(&ch) && !prefer_run.iter().any(|(_, c, _)| *c == ch) {
                    prefer_run.push((idx, ch, *code));
                }
            }
        }
    }
    let mut prefer_page: Vec<(usize, char, Code)> = Vec::new();
    let mut left: Vec<char> = wanted
        .iter()
        .copied()
        .filter(|c| !prefer_run.iter().any(|(_, r, _)| r == c))
        .collect();
    'records: for rec in &model.walk.records {
        if left.is_empty() {
            break;
        }
        let Some(font) = rec.before.text.font.as_ref() else {
            continue;
        };
        let Some(idx) = font_index(font.resource.as_deref(), font.content_hash) else {
            continue;
        };
        for g in &rec.glyphs {
            let Some(ch) = g.text.as_deref().and_then(single) else {
                continue;
            };
            if let Some(pos) = left.iter().position(|c| *c == ch) {
                prefer_page.push((idx, ch, g.code));
                left.swap_remove(pos);
                if left.is_empty() {
                    break 'records;
                }
            }
        }
    }
    (prefer_run, prefer_page)
}

/// §A.3.4 kern mode: the first space of the whole requested line that is not between two
/// non-space characters (leading, trailing or doubled). The whole line is checked, not only the
/// typed middle: a synthetic-space kern kept at either end no longer sits between two glyphs and
/// would not read back as a space (the frontend's `spaceProblem` applies the same rule).
fn misplaced_space(chars: &[char]) -> Option<usize> {
    let space_at = |i: Option<usize>| i.and_then(|i| chars.get(i)) == Some(&' ');
    (0..chars.len()).find(|&i| {
        space_at(Some(i)) && (i == 0 || i + 1 == chars.len() || space_at(i.checked_sub(1)))
    })
}

/// Steps 6–8: the new unit list (with each unit's region) for `text`.
pub(super) fn new_units(
    model: &PageModel,
    run: &TextRun,
    surface: &TypingSurface,
    target: &StyleTarget,
    text: &str,
) -> Result<Vec<(NewUnit, Region)>, EditProblem> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Ok(Vec::new());
    }
    let kern_mode = run.space_mode == SpaceMode::Kern;
    if let Some(i) = misplaced_space(&chars).filter(|_| kern_mode) {
        return Err(problem(
            P::SpaceNotWritable,
            format!("space at character {i}"),
        ));
    }
    let units = &run.units;
    let (prefix_end, prefix_chars, suffix_start, suffix_chars) = if target.face_changed {
        (0, 0, units.len(), 0)
    } else {
        kept_ranges(units, &chars)
    };
    let middle_end = chars.len().saturating_sub(suffix_chars);
    let wanted: &[char] = chars.get(prefix_chars..middle_end).unwrap_or_default();
    let (prefer_run, prefer_page) = preferences(model, run, surface, wanted);
    let kern_space = num(run.kern_space)?.1;
    let mut encoded = Vec::with_capacity(wanted.len());
    let mut missing: Vec<char> = Vec::new();
    for ch in wanted {
        if *ch == ' ' && kern_mode {
            encoded.push(NewUnit::KernSpace(kern_space));
            continue;
        }
        match surface.writer_for_with(*ch, &prefer_run, &prefer_page) {
            Some((font, code)) => encoded.push(NewUnit::Code { font, code }),
            None => {
                if !missing.contains(ch) {
                    missing.push(*ch);
                }
            }
        }
    }
    if !missing.is_empty() {
        let code = if target.face_changed {
            P::FaceUnavailable
        } else {
            P::GlyphMissing
        };
        let mut p = problem(code, "characters the font can't draw");
        p.chars = missing;
        p.face = target.face.filter(|_| target.face_changed);
        return Err(p);
    }
    let mut out: Vec<(NewUnit, Region)> = (0..prefix_end)
        .map(|i| (NewUnit::Kept(i), Region::Prefix))
        .collect();
    let middle_empty = encoded.is_empty();
    out.extend(encoded.into_iter().map(|u| (u, Region::Middle)));
    if middle_empty && !target.face_changed {
        // The two kept sides meet: kerns between them stay only when nothing was removed there.
        let between = units.get(prefix_end..suffix_start).unwrap_or_default();
        if between.iter().all(|u| unit_text(u).is_none()) {
            out.extend((prefix_end..suffix_start).map(|i| (NewUnit::Kept(i), Region::Prefix)));
        }
    }
    out.extend((suffix_start..units.len()).map(|i| (NewUnit::Kept(i), Region::Suffix)));
    Ok(out)
}
