//! Check 5 of the re-walk (§B.14): every edited run draws exactly the expected glyphs — text
//! (equal to the planned text and to the text the user typed), codes and fonts (of the run's
//! typing surface or the requested face's, recomputed from the page, never taken from the plan),
//! drawable codes (B3) — with the expected size, spacing and
//! fill and no other state change, at the expected origins; the pen after the primary is the
//! original one, and every absorbed member draws nothing and keeps its travel.

use super::{decoded_text, dist, font_id, TextItem, VerifyFailure};
use crate::pdf_engine::text_edit::fonts::{face_surface, typing_surface};
use crate::pdf_engine::text_edit::limits::{DRIFT_TOLERANCE_PT, JOIN_BASELINE_TOL_PT};
use crate::pdf_engine::text_edit::rewrite::{ExpectedRun, PagePlan, RunPlan};
use crate::pdf_engine::text_edit::runs::TextRun;
use crate::pdf_engine::text_edit::state::{same_paint, same_state_except, StateField};
use crate::pdf_engine::text_edit::walker::{PageWalk, RecElem, ShowRecord};

/// Effective size of an edited glyph vs the requested one (pt).
const SIZE_TOL_PT: f64 = 0.01;

/// The before record of each member of `run`, found by span (id-free), with its index.
fn member_records<'w>(before: &'w PageWalk, run: &RunPlan) -> Option<Vec<(usize, &'w ShowRecord)>> {
    run.expected
        .member_spans
        .iter()
        .map(|span| {
            before
                .records
                .iter()
                .enumerate()
                .find(|(_, r)| r.span.as_ref() == Some(span))
        })
        .collect()
}

/// The fonts an edited glyph may use: the run's typing surface, or the face surface of the
/// requested face, recomputed from the page (not taken from the plan).
fn allowed_fonts(before: &PageWalk, primary: &ShowRecord, run: &RunPlan) -> Vec<(Vec<u8>, u64)> {
    let font = primary.before.text.font.as_ref();
    if font.is_some_and(|f| f.from_extgstate) {
        return font
            .map(|f| (Vec::new(), f.content_hash))
            .into_iter()
            .collect();
    }
    let surface = match (
        run.target.face_changed,
        run.target.face,
        primary.font.as_ref(),
    ) {
        (true, Some(face), Some(model)) => face_surface(&before.page_fonts, model, face),
        (true, _, _) => None,
        (false, _, _) => font
            .and_then(|f| f.resource.as_deref())
            .map(|res| typing_surface(&before.page_fonts, res)),
    };
    surface
        .map(|s| {
            s.fonts
                .iter()
                .map(|(n, m)| (n.clone(), m.content_hash))
                .collect()
        })
        .unwrap_or_default()
}

/// One run's view of the after walk.
struct Edited<'a> {
    exp: &'a ExpectedRun,
    run: &'a RunPlan,
    /// The before records of the members, primary first.
    members: Vec<(usize, &'a ShowRecord)>,
    /// Per member: the after record indices inside its replaced region.
    regions: Vec<&'a Vec<usize>>,
    /// The after records the primary's replacement emitted.
    recs: Vec<&'a ShowRecord>,
}

impl Edited<'_> {
    fn fail(&self, what: &'static str) -> VerifyFailure {
        VerifyFailure::EditedMismatch {
            run_id: self.exp.run_id.clone(),
            what,
        }
    }

    /// Every glyph the primary emitted, with its record.
    fn glyphs(&self) -> Vec<(&ShowRecord, usize)> {
        self.recs
            .iter()
            .flat_map(|r| (0..r.glyphs.len()).map(move |g| (*r, g)))
            .collect()
    }
}

/// Check 5 for one run.
pub(super) fn check_edited_run(
    before: &PageWalk,
    before_runs: &[TextRun],
    after: &PageWalk,
    plan: &PagePlan,
    edited: &[Vec<usize>],
    run: &RunPlan,
) -> Result<(), VerifyFailure> {
    let exp = &run.expected;
    let records_fail = || VerifyFailure::EditedMismatch {
        run_id: exp.run_id.clone(),
        what: "records",
    };
    let members = member_records(before, run).ok_or_else(records_fail)?;
    let known = before_runs.iter().any(|r| {
        r.reason.is_none()
            && r.members.len() == members.len()
            && r.members.iter().zip(&members).all(|(a, (b, _))| a == b)
    });
    if !known {
        return Err(records_fail());
    }
    let regions: Vec<&Vec<usize>> = run
        .splices
        .iter()
        .map(|s| {
            plan.splices
                .iter()
                .position(|x| x == s)
                .and_then(|k| edited.get(k))
                .ok_or_else(records_fail)
        })
        .collect::<Result<_, _>>()?;
    let primary_records: &[usize] = regions.first().copied().ok_or_else(records_fail)?;
    if exp.emitted_records.first() != Some(&primary_records.len()) {
        return Err(records_fail());
    }
    let recs = primary_records
        .iter()
        .filter_map(|j| after.records.get(*j))
        .collect();
    let e = Edited {
        exp,
        run,
        members,
        regions,
        recs,
    };
    check_glyphs(before, &e)?;
    check_state(&e, primary_records)?;
    check_origins(&e)?;
    check_absorbed(after, &e)
}

/// Text (with synthetic spaces), codes, fonts and drawable codes.
fn check_glyphs(before: &PageWalk, e: &Edited<'_>) -> Result<(), VerifyFailure> {
    let items = e.recs.iter().flat_map(|r| {
        r.elems.iter().map(move |el| match el {
            RecElem::Glyph(g) => TextItem::Glyph(
                r.glyphs
                    .get(*g)
                    .and_then(|x| x.text.as_deref())
                    .unwrap_or("\u{fffd}"),
            ),
            RecElem::Kern { value, .. } => TextItem::Kern(-value / 1000.0),
        })
    });
    let text = decoded_text(items);
    if text != e.exp.text {
        return Err(e.fail("text"));
    }
    let glyphs = e.glyphs();
    if glyphs.len() != e.exp.glyphs.len() {
        return Err(e.fail("codes"));
    }
    let primary = e
        .members
        .first()
        .map(|(_, r)| *r)
        .ok_or_else(|| e.fail("records"))?;
    let allowed = allowed_fonts(before, primary, e.run);
    for (k, ((rec, g), (font, hash, code))) in glyphs.iter().zip(&e.exp.glyphs).enumerate() {
        let x = rec.glyphs.get(*g).ok_or_else(|| e.fail("codes"))?;
        if x.code != *code {
            return Err(e.fail("codes"));
        }
        let id = font_id(rec);
        if id != (font.clone(), *hash) || !allowed.contains(&id) {
            return Err(e.fail("font"));
        }
        let new = e.exp.glyph_new.get(k).copied().unwrap_or(true);
        if new && !rec.font.as_ref().is_some_and(|m| m.drawable(x.code)) {
            return Err(e.fail("glyph"));
        }
    }
    // The planner's expectations are its own reading of its glyphs; the request is not.
    if text != e.exp.requested_text {
        return Err(e.fail("requested_text"));
    }
    Ok(())
}

/// Size, spacing and fill as requested; every other state field as the primary had it.
fn check_state(e: &Edited<'_>, primary_records: &[usize]) -> Result<(), VerifyFailure> {
    let primary = e
        .members
        .first()
        .map(|(_, r)| *r)
        .ok_or_else(|| e.fail("records"))?;
    let primary_font = font_id(primary);
    let t = &e.run.target;
    for (j, rec) in e.recs.iter().enumerate() {
        if rec.glyphs.is_empty() {
            continue;
        }
        let text = &rec.before.text;
        let effective = rec.text_to_user[2].hypot(rec.text_to_user[3]);
        if text.tfs != e.exp.tfs || (effective - e.exp.effective_size).abs() > SIZE_TOL_PT {
            return Err(e.fail("size"));
        }
        if text.tc != e.exp.tc {
            return Err(e.fail("tc"));
        }
        if !same_paint(&rec.before.fill, &e.exp.fill) {
            return Err(e.fail("fill"));
        }
        let mut targeted = Vec::new();
        if t.size_changed {
            targeted.push(StateField::Tfs);
        }
        if t.fill_changed {
            targeted.push(StateField::Fill);
        }
        if t.tc_changed {
            targeted.push(StateField::Tc);
        }
        if t.face_changed || font_id(rec) != primary_font {
            targeted.push(StateField::Font);
        }
        same_state_except(&primary.before, &rec.before, &targeted).map_err(|field| {
            VerifyFailure::StateChanged {
                record: primary_records.get(j).copied().unwrap_or(0),
                field,
            }
        })?;
    }
    Ok(())
}

/// The new layout's origins, the kept glyphs at their original places (shifted by the width
/// change when no style moved them), and the pen after the primary.
fn check_origins(e: &Edited<'_>) -> Result<(), VerifyFailure> {
    let exp = e.exp;
    if let Some(first) = e.recs.first() {
        if dist(first.pen_before, exp.origin) > DRIFT_TOLERANCE_PT {
            return Err(e.fail("origin"));
        }
    }
    let t = &e.run.target;
    let unstyled = !t.size_changed && !t.tc_changed && !t.face_changed;
    let suffix_from = exp.glyphs.len().saturating_sub(exp.suffix_glyphs);
    for (k, (rec, g)) in e.glyphs().into_iter().enumerate() {
        let x = rec.glyphs.get(g).ok_or_else(|| e.fail("origin"))?;
        let want = exp
            .glyph_origins
            .get(k)
            .copied()
            .ok_or_else(|| e.fail("origin"))?;
        if dist(x.origin, want) > DRIFT_TOLERANCE_PT {
            return Err(e.fail("origin"));
        }
        let middle = k >= exp.prefix_glyphs && k < suffix_from;
        let Some(Some((member, gi))) = exp.kept_from.get(k) else {
            continue;
        };
        if !unstyled || middle {
            continue;
        }
        let orig = e
            .members
            .get(*member)
            .and_then(|(_, r)| r.glyphs.get(*gi))
            .map(|o| o.origin)
            .ok_or_else(|| e.fail("origin"))?;
        let shifted = k >= suffix_from && !exp.unshifted_from.is_some_and(|u| k >= u);
        let expected = if shifted {
            (orig.0 + exp.shift_user.0, orig.1 + exp.shift_user.1)
        } else {
            orig
        };
        if dist(x.origin, expected) > DRIFT_TOLERANCE_PT + JOIN_BASELINE_TOL_PT {
            return Err(e.fail("origin"));
        }
    }
    let pen = e
        .recs
        .last()
        .map(|r| r.pen_after)
        .ok_or_else(|| e.fail("pen"))?;
    if dist(pen, exp.primary_pen_after) > DRIFT_TOLERANCE_PT {
        return Err(e.fail("pen"));
    }
    Ok(())
}

/// Absorbed members: one show op drawing nothing, with the original pen travel.
fn check_absorbed(after: &PageWalk, e: &Edited<'_>) -> Result<(), VerifyFailure> {
    for (m, region) in e.regions.iter().enumerate().skip(1) {
        let ok = region.len() == 1
            && e.exp.emitted_records.get(m) == Some(&1)
            && region
                .first()
                .and_then(|j| after.records.get(*j))
                .is_some_and(|r| {
                    r.glyphs.is_empty()
                        && e.exp
                            .member_pen_after
                            .get(m)
                            .is_some_and(|p| dist(r.pen_after, *p) <= DRIFT_TOLERANCE_PT)
                });
        if !ok {
            return Err(e.fail("absorbed"));
        }
    }
    Ok(())
}
