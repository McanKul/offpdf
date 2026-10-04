//! Phase B (SPEC §B.16.2, D29): on the final staged file, immediately before #34's
//! `validate_staged_pdf`. The expected content of each edited destination page must be found in
//! exactly one of two forms — its parts as a contiguous in-order run of the page's parts (later
//! passes may add parts around them), or qpdf's overlay wrapper: page content of `q cm Do Q` only,
//! one painted Form whose data holds `qpdf_join(expected_parts)` once (at its start or right after
//! a `\n`), with a composite matrix that is the identity within tolerance and a `/BBox` covering
//! the visible page (B1). The original edited parts must be nowhere (B2), and the page's depth-0
//! show records — walked through the wrapper when there is one — must be the proven ones (B3).
//! Any failure is `SOURCE_EDIT_GATE_FAILED`.

use super::join::join_is_neutral;
use super::originals::{find_up_to, holds_original};
use super::{reachable_forms, PageProof, RecordPrint};
use crate::pdf_engine::text_edit::content::{page_content, qpdf_join, PageContent};
use crate::pdf_engine::text_edit::context::{resolve, Lookup, Res, SnapshotContext};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::fonts::number_of;
use crate::pdf_engine::text_edit::geometry::{contains, mul, transform_rect, Matrix, IDENTITY};
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits, Operand, Operator};
use crate::pdf_engine::text_edit::limits::{
    DRIFT_TOLERANCE_PT, PAGE_CONTENT_MAX_DECODED, PAGE_DECODE_BUDGET, WRAPPER_MATRIX_EPSILON,
    WRAPPER_TRANSLATION_TOL_PT,
};
use crate::pdf_engine::text_edit::state::{same_state, ClipState, StateDigest};
use crate::pdf_engine::text_edit::walker::{walk_page, ShowRecord, WalkMode};
use lopdf::{Dictionary, Object, ObjectId};
use std::sync::atomic::AtomicBool;

/// The wrapper Form's `/BBox` must cover the visible box within this (pt).
const WRAPPER_BBOX_TOL_PT: f64 = 0.01;

/// Where the expected content was found.
enum Found {
    Parts,
    Wrapper { name: Vec<u8> },
}

/// Starting indices where the proof's expected parts appear as a contiguous run of `content`'s
/// parts (digests first, then the bytes).
fn parts_hits(content: &PageContent, proof: &PageProof) -> usize {
    let expected = &proof.expected_parts;
    let (n, k) = (content.parts.len(), expected.len());
    if k == 0 || k > n || proof.part_digests.len() != k {
        return 0;
    }
    (0..=n - k)
        .filter(|start| {
            expected
                .iter()
                .zip(&proof.part_digests)
                .enumerate()
                .all(|(j, (want, digest))| {
                    content.parts.get(start + j).map(|p| p.digest) == Some(*digest)
                        && content.part_bytes(start + j) == want.as_slice()
                })
        })
        .count()
}

/// Occurrences of `needle` in `hay` — counted up to 2, which is all B1 needs ("exactly once") —
/// and how many of those start at 0 or right after `\n`. Linear in `hay` (`find_up_to`); `Err`
/// (fail closed) when the search gives up.
fn occurrences(hay: &[u8], needle: &[u8]) -> Result<(usize, usize), String> {
    let hits = find_up_to(hay, needle, 2).map_err(|e| format!("too large to verify: {e}"))?;
    let anchored = hits
        .iter()
        .filter(|i| {
            **i == 0
                || i.checked_sub(1)
                    .and_then(|p| hay.get(p))
                    .is_some_and(|b| *b == b'\n')
        })
        .count();
    Ok((hits.len(), anchored))
}

fn form_matrix(doc: &lopdf::Document, dict: &Dictionary) -> Option<Matrix> {
    let Ok(raw) = dict.get(b"Matrix") else {
        return Some(IDENTITY);
    };
    let (_, Object::Array(items)) = resolve(doc, raw)? else {
        return None;
    };
    if items.len() != 6 {
        return None;
    }
    let mut m = IDENTITY;
    for (slot, item) in m.iter_mut().zip(items) {
        *slot = resolve(doc, item).and_then(|(_, o)| number_of(o))?;
    }
    Some(m)
}

fn form_bbox(doc: &lopdf::Document, dict: &Dictionary) -> Option<[f64; 4]> {
    let (_, Object::Array(items)) = resolve(doc, dict.get(b"BBox").ok()?)? else {
        return None;
    };
    let v: Vec<f64> = items
        .iter()
        .map(|i| resolve(doc, i).and_then(|(_, o)| number_of(o)))
        .collect::<Option<Vec<f64>>>()?;
    match v.as_slice() {
        [a, b, c, d] => Some([a.min(*c), b.min(*d), a.max(*c), b.max(*d)]),
        _ => None,
    }
}

/// The wrapper form (B1): `(name, form data)` of the one painted Form that holds the expected
/// content, after checking the page shape, the composite matrix and the `/BBox`.
fn wrapper_hit(
    ctx: &SnapshotContext,
    page_id: ObjectId,
    content: &PageContent,
    joined: &[u8],
    visible: [f64; 4],
) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
    let Ok(ops) = lex_content(&content.joined, &LexLimits::page(), None) else {
        return Ok(None);
    };
    let shape = !ops.is_empty()
        && ops.iter().all(|o| {
            matches!(
                o.operator,
                Operator::q | Operator::Q | Operator::cm | Operator::Do
            )
        });
    if !shape {
        return Ok(None);
    }
    let doc = ctx.doc();
    let res = Res::of_page(doc, page_id).map_err(str::to_string)?;
    let mut stack: Vec<Matrix> = Vec::new();
    let mut ctm = IDENTITY;
    let mut hits: Vec<(Vec<u8>, Vec<u8>, Matrix, ObjectId)> = Vec::new();
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    for op in &ops {
        match op.operator {
            Operator::q => stack.push(ctm),
            Operator::Q => ctm = stack.pop().unwrap_or(ctm),
            Operator::cm => {
                let n: Vec<f64> = op.operands.iter().filter_map(Operand::as_number).collect();
                let m: Matrix = match n.as_slice() {
                    [a, b, c, d, e, f] => [*a, *b, *c, *d, *e, *f],
                    _ => return Err("wrapper cm".to_string()),
                };
                ctm = mul(&m, &ctm);
            }
            Operator::Do => {
                let name = op
                    .operands
                    .first()
                    .and_then(Operand::as_name)
                    .unwrap_or_default();
                let Lookup::Found(e) = res.entry(doc, b"XObject", name) else {
                    return Err("wrapper XObject".to_string());
                };
                let (Some(id), Object::Stream(s)) = (e.id, e.value) else {
                    return Err("wrapper XObject".to_string());
                };
                let is_form =
                    s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Form");
                if !is_form {
                    continue;
                }
                let data = decode_stream(s, PAGE_CONTENT_MAX_DECODED, &mut budget)
                    .map_err(|e| format!("wrapper data: {e}"))?;
                let (all, anchored) = occurrences(&data, joined)?;
                if all == 0 {
                    continue;
                }
                if all != 1 {
                    return Err("expected content repeated in the wrapper".to_string());
                }
                if anchored != 1 {
                    return Err(
                        "expected content not at a part boundary in the wrapper".to_string()
                    );
                }
                let matrix = form_matrix(doc, &s.dict).ok_or("wrapper /Matrix")?;
                hits.push((name.to_vec(), data, mul(&matrix, &ctm), id));
            }
            _ => {}
        }
    }
    let [(name, data, m, id)] = hits.as_slice() else {
        if hits.is_empty() {
            return Ok(None);
        }
        return Err("the expected content is painted more than once".to_string());
    };
    let painted = ops
        .iter()
        .filter(|o| {
            o.operator == Operator::Do
                && o.operands.first().and_then(Operand::as_name) == Some(name.as_slice())
        })
        .count();
    if painted != 1 {
        return Err("the wrapper Form is painted more than once".to_string());
    }
    let linear_ok = (m[0] - 1.0).abs() <= WRAPPER_MATRIX_EPSILON
        && m[1].abs() <= WRAPPER_MATRIX_EPSILON
        && m[2].abs() <= WRAPPER_MATRIX_EPSILON
        && (m[3] - 1.0).abs() <= WRAPPER_MATRIX_EPSILON;
    let shift_ok =
        m[4].abs() <= WRAPPER_TRANSLATION_TOL_PT && m[5].abs() <= WRAPPER_TRANSLATION_TOL_PT;
    if !linear_ok || !shift_ok {
        return Err("wrapper matrix is not the identity".to_string());
    }
    let Some(Object::Stream(s)) = doc.objects.get(id) else {
        return Err("wrapper XObject".to_string());
    };
    let bbox = form_bbox(doc, &s.dict).ok_or("wrapper /BBox")?;
    if !contains(transform_rect(m, bbox), visible, WRAPPER_BBOX_TOL_PT) {
        return Err("wrapper /BBox cuts the visible page".to_string());
    }
    Ok(Some((name.clone(), data.clone())))
}

/// The digest of the after state with the wrapper's transform factored out: CTM and clip within
/// the wrapper tolerances of the proof's are taken as the proof's.
fn factored(after: &StateDigest, proof: &StateDigest) -> StateDigest {
    let mut d = after.clone();
    let ctm_close = after
        .ctm
        .iter()
        .zip(&proof.ctm)
        .enumerate()
        .all(|(i, (a, b))| {
            let tol = if i < 4 {
                WRAPPER_MATRIX_EPSILON * 1f64.max(a.abs()).max(b.abs())
            } else {
                WRAPPER_TRANSLATION_TOL_PT
            };
            (a - b).abs() <= tol
        });
    if ctm_close {
        d.ctm = proof.ctm;
    }
    if let (ClipState::Rect(a), ClipState::Rect(b)) = (&after.clip, &proof.clip) {
        if a.iter()
            .zip(b)
            .all(|(x, y)| (x - y).abs() <= DRIFT_TOLERANCE_PT)
        {
            d.clip = proof.clip.clone();
        }
    }
    d
}

/// B3: the depth-0 show records equal the proof one-to-one.
fn same_records(records: &[&ShowRecord], proof: &[RecordPrint]) -> Result<(), String> {
    if records.len() != proof.len() {
        return Err(format!(
            "show records {} (expected {})",
            records.len(),
            proof.len()
        ));
    }
    for (i, (r, p)) in records.iter().zip(proof).enumerate() {
        let now = RecordPrint::of(r);
        if now.op != p.op
            || now.font_res != p.font_res
            || now.font_hash != p.font_hash
            || now.codes != p.codes
            || now.text != p.text
        {
            return Err(format!("record {i} differs"));
        }
        let far =
            |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).hypot(a.1 - b.1) > DRIFT_TOLERANCE_PT;
        let moved = now.origins.len() != p.origins.len()
            || now.origins.iter().zip(&p.origins).any(|(a, b)| far(a, b))
            || far(&now.pen_after, &p.pen_after);
        if moved {
            return Err(format!("record {i} moved"));
        }
        same_state(&factored(&now.state, &p.state), &p.state)
            .map_err(|field| format!("record {i} state {field}"))?;
    }
    Ok(())
}

/// B1–B3 on one destination page.
pub(super) fn check_page(
    ctx: &SnapshotContext,
    dest: u32,
    proof: &PageProof,
    cancel: Option<&AtomicBool>,
) -> Result<(), String> {
    let skip_b1 = super::seams::skipped("B1");
    let page_id = ctx.page_id(dest).map_err(|_| "page missing".to_string())?;
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let content = page_content(ctx.doc(), page_id, &mut budget)
        .map_err(|r| format!("content {}", r.as_str()))?;
    let visible = crate::pdf_engine::text_edit::geometry::page_geometry(ctx.doc(), page_id)
        .map_err(|r| format!("geometry {}", r.as_str()))?
        .visible;
    let parts: Vec<&[u8]> = proof.expected_parts.iter().map(Vec::as_slice).collect();
    let joined = qpdf_join(&parts);
    let in_parts = parts_hits(&content, proof);
    let wrapper = match wrapper_hit(ctx, page_id, &content, &joined, visible) {
        // qpdf's join must not change what the expected parts mean (review-T5 H1).
        Ok(Some(_)) if !skip_b1 && !join_is_neutral(&parts, &joined) => {
            return Err("check=B1 qpdf's overlay join changes the edited content".to_string())
        }
        Ok(w) => w,
        Err(e) if !skip_b1 => return Err(format!("check=B1 {e}")),
        Err(_) => None,
    };
    let found = match (in_parts, &wrapper) {
        (1, None) => Some(Found::Parts),
        (0, Some((name, _))) => Some(Found::Wrapper { name: name.clone() }),
        _ => None,
    };
    if found.is_none() && !skip_b1 {
        return Err(if in_parts == 0 && wrapper.is_none() {
            "check=B1 the expected content is not on the page".to_string()
        } else {
            "check=B1 the expected content is on the page more than once".to_string()
        });
    }
    if !super::seams::skipped("B2") {
        // The parts by digest; every reachable Form (the wrapper is one) whole or as a segment.
        let originals = proof.originals();
        let too_large = |e: &str| format!("check=B2 too large to verify: {e}");
        let mut found = content
            .parts
            .iter()
            .any(|p| originals.iter().any(|o| o.digest == p.digest));
        let forms = reachable_forms(ctx, page_id, &mut DecodeBudget::new(PAGE_DECODE_BUDGET))
            .map_err(|e| too_large(&e))?;
        for f in &forms {
            found = found || holds_original(f, &originals).map_err(too_large)?;
        }
        if found {
            return Err("check=B2 the original content is still on the page".to_string());
        }
    }
    if !super::seams::skipped("B3") {
        // With B1 skipped by a test and nothing found, every record at any depth is compared.
        let mode = match found {
            Some(Found::Parts) => WalkMode::Edit,
            Some(Found::Wrapper { name }) => WalkMode::Wrapped { name },
            None => WalkMode::Classify,
        };
        let walk = walk_page(ctx, dest, &content, mode, cancel);
        if let Some(r) = walk.page_reason {
            return Err(format!(
                "check=B3 walk refused {} {}",
                r.as_str(),
                walk.page_detail.unwrap_or_default()
            ));
        }
        let records: Vec<&ShowRecord> = walk.records.iter().collect();
        same_records(&records, &proof.records).map_err(|e| format!("check=B3 {e}"))?;
    }
    Ok(())
}
