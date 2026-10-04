//! Planner steps 11–14 (SPEC §B.12): the replacement bytes of every member's show op, the grammar
//! check on each (before any IO), the splices and the expected content parts.
//!
//! Primary: `prefix + set + body + restore`. Prefix keeps a `'` (`T*`) or `"` (`aw Tw ac Tc T*`,
//! verbatim) op's positioning; set writes the new `Tc`/fill; the body is one `TJ` per segment of
//! glyphs in one font, a `Tf` before a segment whose font differs from the font in force or when
//! the size changed, kept TJ numbers as their original bytes, the compensation after the last
//! segment; restore writes back, verbatim, exactly what changed: fill, `Tc`, then `Tf`. Every
//! absorbed member draws nothing and keeps its pen travel: `prefix + [<> n] TJ`.

use super::layout::{Laid, Planned};
use super::{problem, ExpectedRun, PagePlan, RunPlan, Splice, StyleTarget};
use crate::pdf_engine::text_edit::content::PageContent;
use crate::pdf_engine::text_edit::encode::{
    check_replacement_grammar, hex_codes, needs_leading_space,
};
use crate::pdf_engine::text_edit::fonts::Code;
use crate::pdf_engine::text_edit::lexer::{is_delimiter, is_whitespace};
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode as P, TextReason};
use crate::pdf_engine::text_edit::runs::{PageModel, TextRun};
use crate::pdf_engine::text_edit::state::{color_effect, ColorSpaceKind, Paint};
use crate::pdf_engine::text_edit::walker::{ShowOp, ShowRecord};
use crate::pdf_engine::validate_output::content_digest;
use std::sync::Arc;

/// A PDF name token for a resource name (`#xx` for anything not a plain regular character).
pub(crate) fn name_token(name: &[u8]) -> String {
    let mut out = String::with_capacity(name.len() + 1);
    out.push('/');
    for &b in name {
        let plain =
            (0x21..=0x7e).contains(&b) && b != b'#' && !is_delimiter(b) && !is_whitespace(b);
        if plain {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

fn joined_text(content: &PageContent, span: &std::ops::Range<usize>) -> String {
    String::from_utf8_lossy(content.joined.get(span.clone()).unwrap_or_default()).into_owned()
}

/// The positioning a show op does before drawing, verbatim.
fn op_prefix(content: &PageContent, rec: &ShowRecord) -> Vec<String> {
    match rec.op {
        ShowOp::Tj | ShowOp::TJ => Vec::new(),
        ShowOp::Quote => vec!["T*".to_string()],
        ShowOp::DoubleQuote => {
            let operand = |i: usize| {
                rec.operand_spans
                    .get(i)
                    .map(|s| joined_text(content, s))
                    .unwrap_or_default()
            };
            vec![
                format!("{} Tw", operand(0)),
                format!("{} Tc", operand(1)),
                "T*".to_string(),
            ]
        }
    }
}

/// One TJ segment: its font resource and its elements.
struct Segment {
    font: Vec<u8>,
    elements: Vec<String>,
    codes: Vec<Code>,
    has_glyph: bool,
}

impl Segment {
    fn flush_codes(&mut self) {
        if !self.codes.is_empty() {
            self.elements.push(hex_codes(&self.codes));
            self.codes.clear();
        }
    }
}

/// Splits the new units into TJ segments (kerns attach to the preceding segment; leading kerns
/// to the first one).
fn segments(units: &[Planned], content: &PageContent) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for u in units {
        match u {
            Planned::Glyph { code, font_res, .. } => {
                let same = segs.last().is_some_and(|s| s.font == *font_res);
                if !same {
                    if let Some(last) = segs.last_mut() {
                        last.flush_codes();
                    }
                    segs.push(Segment {
                        font: font_res.clone(),
                        elements: std::mem::take(&mut pending),
                        codes: Vec::new(),
                        has_glyph: true,
                    });
                }
                if let Some(seg) = segs.last_mut() {
                    seg.codes.push(*code);
                }
            }
            Planned::Kern {
                written, original, ..
            } => {
                let token = match (written, original) {
                    (Some(s), _) => s.clone(),
                    (None, Some(span)) => joined_text(content, span),
                    (None, None) => continue,
                };
                match segs.last_mut() {
                    Some(seg) => {
                        seg.flush_codes();
                        seg.elements.push(token);
                    }
                    None => pending.push(token),
                }
            }
        }
    }
    if let Some(last) = segs.last_mut() {
        last.flush_codes();
    }
    if segs.is_empty() {
        segs.push(Segment {
            font: Vec::new(),
            elements: pending,
            codes: Vec::new(),
            has_glyph: false,
        });
    }
    segs
}

/// The primary's replacement (`prefix + set + body + restore`) and its TJ count.
fn primary_bytes(
    content: &PageContent,
    primary: &ShowRecord,
    target: &StyleTarget,
    laid: &Laid,
) -> Result<(String, usize), EditProblem> {
    let internal = |what: &str| problem(P::EditVerifyFailed, what.to_string());
    let mut tokens = op_prefix(content, primary);
    let fmt = |v: f64| crate::pdf_engine::text_edit::encode::fmt_num(v);
    if target.tc_changed {
        tokens.push(format!("{} Tc", fmt(target.tc)?));
    }
    if let (true, Some(rgb)) = (target.fill_changed, target.fill) {
        tokens.push(format!(
            "{} {} {} rg",
            fmt(rgb[0])?,
            fmt(rgb[1])?,
            fmt(rgb[2])?
        ));
    }
    let original_font: Vec<u8> = primary
        .before
        .text
        .font
        .as_ref()
        .and_then(|f| f.resource.as_deref())
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    let units: Vec<Planned> = laid.units.iter().map(|p| p.unit.clone()).collect();
    let mut segs = segments(&units, content);
    let tf_token = String::from_utf8_lossy(&target.tfs_token).into_owned();
    let mut in_force = original_font.clone();
    let seg_count = segs.len();
    if let (Some(n_c), Some(last)) = (&laid.compensation, segs.last_mut()) {
        last.elements.push(n_c.clone());
    }
    for seg in &segs {
        let switch = seg.has_glyph && !seg.font.is_empty() && seg.font != in_force;
        if switch || (seg.has_glyph && target.size_changed) {
            if seg.font.is_empty() || tf_token.is_empty() {
                return Err(internal("a font switch without a Tf font"));
            }
            tokens.push(format!("{} {tf_token} Tf", name_token(&seg.font)));
            in_force = seg.font.clone();
        }
        let elements = if seg.elements.is_empty() {
            "<>".to_string()
        } else if seg.has_glyph {
            seg.elements.join(" ")
        } else {
            format!("<> {}", seg.elements.join(" "))
        };
        tokens.push(format!("[{elements}] TJ"));
    }
    let after = &primary.after;
    if target.fill_changed {
        tokens.extend(fill_restore(&after.fill)?);
    }
    if target.tc_changed {
        tokens.push(match &after.text.tc_src {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => "0 Tc".to_string(),
        });
    }
    if in_force != original_font || target.size_changed {
        let tf_op = after
            .text
            .font
            .as_ref()
            .and_then(|f| f.tf_op.as_deref())
            .ok_or_else(|| internal("no Tf to restore"))?;
        tokens.push(String::from_utf8_lossy(tf_op).into_owned());
    }
    Ok((tokens.join(" "), seg_count))
}

/// The ops that put the fill back after the new `rg`: the verbatim colour-space and colour ops in
/// force after the original op, or `0 g` when the page never set a fill. An `sc`/`scn` written
/// without its own `cs` (the space came from the initial DeviceGray or from an earlier `g`/`rg`/
/// `k`) gets that device space's `cs` first: after the new `rg` it would otherwise apply to
/// DeviceRGB.
fn fill_restore(fill: &Paint) -> Result<Vec<String>, EditProblem> {
    let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let (space_op, color_op) = (fill.space_op.as_deref(), fill.color_op.as_deref());
    if space_op.is_none() && color_op.is_none() {
        return Ok(vec!["0 g".to_string()]);
    }
    let mut out = Vec::with_capacity(3);
    let bare_sc = color_op.is_some_and(|op| {
        matches!(
            op.split(|b| is_whitespace(*b))
                .filter(|t| !t.is_empty())
                .next_back(),
            Some(b"sc" | b"scn")
        )
    });
    if space_op.is_none() && bare_sc {
        let device = match fill.space {
            ColorSpaceKind::Default | ColorSpaceKind::DeviceGray => "/DeviceGray cs",
            ColorSpaceKind::DeviceRgb => "/DeviceRGB cs",
            ColorSpaceKind::DeviceCmyk => "/DeviceCMYK cs",
            ColorSpaceKind::Named(..) | ColorSpaceKind::Pattern => {
                return Err(problem(
                    P::EditVerifyFailed,
                    "a colour op without its colour space",
                ))
            }
        };
        out.push(device.to_string());
    }
    out.extend(space_op.map(text));
    out.extend(color_op.map(text));
    Ok(out)
}

/// An absorbed member's replacement: its positioning, then nothing drawn and the same pen travel.
fn absorbed_bytes(content: &PageContent, rec: &ShowRecord, number: Option<&String>) -> String {
    let mut tokens = op_prefix(content, rec);
    tokens.push(match number {
        Some(n) => format!("[<> {n}] TJ"),
        None => "[<>] TJ".to_string(),
    });
    tokens.join(" ")
}

/// The splice of `rec`'s op span with `text` (a space is prepended when the first token could
/// fuse with the byte before it).
fn splice_of(content: &PageContent, rec: &ShowRecord, text: String) -> Result<Splice, EditProblem> {
    let span = rec
        .span
        .clone()
        .ok_or_else(|| problem(P::EditVerifyFailed, "show op without a span"))?;
    let Some((part, local)) = content.locate(&span) else {
        let mut p = problem(P::TextEditRefused, "show op straddles two content parts");
        p.reason = Some(TextReason::SplitContent);
        return Err(p);
    };
    let prev = local
        .start
        .checked_sub(1)
        .and_then(|i| content.part_bytes(part).get(i).copied());
    let mut bytes = text.into_bytes();
    if needs_leading_space(prev, &bytes) {
        bytes.insert(0, b' ');
    }
    Ok(Splice {
        part,
        local,
        joined: span,
        bytes,
    })
}

/// The expected fill of the edited glyphs.
fn expected_fill(primary: &ShowRecord, target: &StyleTarget) -> Result<Paint, EditProblem> {
    let Some(rgb) = target.fill.filter(|_| target.fill_changed) else {
        return Ok(primary.before.fill.clone());
    };
    let fmt = crate::pdf_engine::text_edit::encode::fmt_num;
    let op = format!("{} {} {} rg", fmt(rgb[0])?, fmt(rgb[1])?, fmt(rgb[2])?);
    Ok(Paint {
        space_op: None,
        color_op: Some(Arc::from(op.as_bytes())),
        effect: color_effect(&ColorSpaceKind::DeviceRgb, &rgb),
        space: ColorSpaceKind::DeviceRgb,
        comps: rgb.to_vec(),
        pattern: false,
        pattern_hash: None,
    })
}

/// Steps 11–14 for one run.
pub(super) fn run_plan(
    model: &PageModel,
    run: &TextRun,
    requested_text: &str,
    target: StyleTarget,
    laid: Laid,
) -> Result<RunPlan, EditProblem> {
    let content = &model.content;
    let records: Vec<&ShowRecord> = run
        .members
        .iter()
        .filter_map(|i| model.walk.records.get(*i))
        .collect();
    let Some(primary) = records.first().copied() else {
        return Err(problem(P::EditVerifyFailed, "run without members"));
    };
    let grammar = |bytes: &str, tj: usize| {
        check_replacement_grammar(bytes.as_bytes(), tj)
            .map_err(|op| problem(P::EditVerifyFailed, format!("replacement grammar: {op}")))
    };
    let (primary_text, seg_count) = primary_bytes(content, primary, &target, &laid)?;
    grammar(&primary_text, seg_count)?;
    let mut splices = vec![splice_of(content, primary, primary_text)?];
    let mut emitted_records = vec![seg_count];
    for (m, rec) in records.iter().enumerate().skip(1) {
        let text = absorbed_bytes(
            content,
            rec,
            laid.absorbed_numbers.get(m).and_then(Option::as_ref),
        );
        grammar(&text, 1)?;
        splices.push(splice_of(content, rec, text)?);
        emitted_records.push(1);
    }
    let mut glyphs = Vec::new();
    let mut glyph_new = Vec::new();
    let mut kept_from = Vec::new();
    for p in &laid.units {
        if let Planned::Glyph {
            code,
            font_res,
            font_hash,
            kept,
            ..
        } = &p.unit
        {
            glyphs.push((font_res.clone(), *font_hash, *code));
            glyph_new.push(kept.is_none());
            kept_from.push(kept.map(|(_, member, gi)| (member, gi)));
        }
    }
    let expected = ExpectedRun {
        run_id: run.id.clone(),
        text: laid.text.clone(),
        glyphs,
        prefix_glyphs: laid.prefix_glyphs,
        suffix_glyphs: laid.suffix_glyphs,
        shift_user: laid.shift_user,
        unshifted_from: laid.unshifted_from,
        origin: run.origin,
        primary_pen_after: primary.pen_after,
        member_pen_after: records.iter().map(|r| r.pen_after).collect(),
        tfs: target.tfs,
        effective_size: target.effective_size,
        tc: target.tc,
        fill: expected_fill(primary, &target)?,
        emitted_records,
        member_spans: records.iter().filter_map(|r| r.span.clone()).collect(),
        glyph_origins: laid.glyph_origins.clone(),
        glyph_new,
        kept_from,
        mask_boxes: laid.mask_boxes.clone(),
        old_ink_boxes: laid.old_ink_boxes.clone(),
        glyphs_changed: laid.glyphs_changed,
        requested_text: requested_text.to_string(),
    };
    Ok(RunPlan {
        run_id: run.id.clone(),
        target,
        splices,
        expected,
        new_rect: laid.new_rect,
    })
}

/// Recomputes `expected_parts`, `edited_parts`, `expected_joined` and `expected_page_digest`
/// from `plan.splices` (sorted by joined start). Splices are applied back to front per part.
pub(crate) fn rebuild_parts(content: &PageContent, plan: &mut PagePlan) {
    plan.splices.sort_by_key(|s| s.joined.start);
    let mut parts: Vec<Vec<u8>> = (0..content.parts.len())
        .map(|i| content.part_bytes(i).to_vec())
        .collect();
    let mut edited: Vec<usize> = Vec::new();
    for s in plan.splices.iter().rev() {
        let Some(part) = parts.get_mut(s.part) else {
            continue;
        };
        if s.local.start <= s.local.end && s.local.end <= part.len() {
            part.splice(s.local.clone(), s.bytes.iter().copied());
            if !edited.contains(&s.part) {
                edited.push(s.part);
            }
        }
    }
    edited.sort_unstable();
    let mut joined = Vec::new();
    let mut concat = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            joined.push(b'\n');
        }
        joined.extend_from_slice(p);
        concat.extend_from_slice(p);
    }
    plan.expected_page_digest = content_digest(&concat);
    plan.expected_parts = parts;
    plan.edited_parts = edited;
    plan.expected_joined = joined;
}

/// The page plan of `runs` over `content`.
pub(crate) fn assemble_page_plan(
    content: &PageContent,
    page_index: u32,
    runs: Vec<RunPlan>,
) -> PagePlan {
    let splices = runs.iter().flat_map(|r| r.splices.clone()).collect();
    let mut plan = PagePlan {
        page_index,
        runs,
        splices,
        expected_parts: Vec::new(),
        edited_parts: Vec::new(),
        expected_joined: Vec::new(),
        expected_page_digest: content_digest(b""),
    };
    rebuild_parts(content, &mut plan);
    plan
}
