//! Moving one directly painted image occurrence (issue #129).
//!
//! The image's `Do` bytes are never touched: a `q 1 0 0 1 dx dy cm` is inserted right before it
//! and a `Q` right after it. `cm` pre-multiplies the CTM, so the unit square the image paints
//! moves by exactly `(dx, dy)` in unrotated user space. The XObject resource stays untouched, so
//! other occurrences of the same image stay where they are. A shared content stream, an inline
//! image, an image inside a Form, a masked or non-axis-aligned image, a complex clip and a
//! destination outside the visible page are refused with a reason instead of guessed at.

use super::{
    problem, EditProblemCode, EditVerdict, ImageEditIn, ImagePlan, RunPlan, Splice,
};
use crate::pdf_engine::text_edit::content::{part_exclusive, KidsCounts, RefCounts};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::encode::fmt_precise;
use crate::pdf_engine::text_edit::geometry::{
    axis_aligned, contains, display_rotation, mul, transform_rect,
};
use crate::pdf_engine::text_edit::lexer::Span;
use crate::pdf_engine::text_edit::limits::CLIP_CONTAIN_TOL_PT;
use crate::pdf_engine::text_edit::reasons::{EditProblem, TextReason};
use crate::pdf_engine::text_edit::runs::PageModel;
use crate::pdf_engine::text_edit::state::ClipState;
use crate::pdf_engine::text_edit::walker::{PaintKind, PaintRecord};
use crate::pdf_engine::text_edit::geometry::PageGeometry;

const UNIT_SQUARE: [f64; 4] = [0.0, 0.0, 1.0, 1.0];

fn refused(locator: &str, code: EditProblemCode, detail: impl Into<String>) -> EditVerdict {
    EditVerdict::failed(locator, problem(code, detail))
}

fn refused_reason(locator: &str, reason: TextReason, detail: &str) -> EditVerdict {
    let mut p = EditProblem::new(
        EditProblemCode::TextEditRefused,
        Some(format!("{}: {detail}", reason.as_str())),
    );
    p.reason = Some(reason);
    EditVerdict::failed(locator, p)
}

/// `v2:{fingerprint}:{page}:i:p{start}-{end}:{ordinal}` of a depth-0 occurrence.
struct Locator {
    fingerprint: String,
    page: u32,
    span: Span,
}

fn parse(locator: &str) -> Option<Locator> {
    let fields: Vec<&str> = locator.split(':').collect();
    // A Form path (`x…:{start}-{end}`) has more fields: handled as a nested-form refusal.
    if fields.len() != 6 || fields[0] != "v2" || fields[3] != "i" {
        return None;
    }
    let path = fields[4].strip_prefix('p')?;
    let (start, end) = path.split_once('-')?;
    Some(Locator {
        fingerprint: fields[1].to_string(),
        page: fields[2].parse().ok()?,
        span: start.parse().ok()?..end.parse().ok()?,
    })
}

fn same_box(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(&b).all(|(x, y)| x == y)
}

/// The image's paint record and its depth-0 span, or the verdict that refuses the locator.
fn resolve<'a>(
    ctx: &SnapshotContext,
    model: &'a PageModel,
    edit: &ImageEditIn,
) -> Result<(&'a PaintRecord, Locator), EditVerdict> {
    let locator = edit.locator.trim();
    let Some(parsed) = parse(locator) else {
        return Err(refused_reason(
            locator,
            TextReason::NestedForm,
            "only images painted directly by the page can be moved",
        ));
    };
    if parsed.fingerprint != ctx.snap.fingerprint.sha256_hex() || parsed.page != model.page_index {
        return Err(refused(
            locator,
            EditProblemCode::Stale,
            "the page or file changed since it was read",
        ));
    }
    let Some(paint) = model
        .walk
        .paints
        .iter()
        .find(|p| p.span.as_ref() == Some(&parsed.span))
    else {
        return Err(refused(
            locator,
            EditProblemCode::Stale,
            "the image no longer exists on this page",
        ));
    };
    match &paint.kind {
        PaintKind::ImageXObject { .. } => {}
        PaintKind::InlineImage { .. } => {
            return Err(refused_reason(
                locator,
                TextReason::InlineImage,
                "inline images cannot be moved",
            ))
        }
        _ => {
            return Err(refused_reason(
                locator,
                TextReason::NestedForm,
                "only image XObjects can be moved",
            ))
        }
    }
    if paint.depth > 0 {
        return Err(refused_reason(
            locator,
            TextReason::NestedForm,
            "images inside reused blocks cannot be moved",
        ));
    }
    Ok((paint, parsed))
}

/// The reason the image cannot be moved even though it is a directly painted image (mirrors the
/// classifier's image priority, except that `SHARED_XOBJECT` is allowed: the resource is not
/// changed, only this occurrence's content stream).
fn move_refusal(model: &PageModel, p: &PaintRecord, geometry: &PageGeometry) -> Option<TextReason> {
    use TextReason as R;
    if matches!(p.kind, PaintKind::InlineImage { .. }) {
        return Some(R::InlineImage);
    }
    if p.depth > 0 {
        return Some(R::NestedForm);
    }
    let bbox = p.bbox.unwrap_or_default();
    let clipped = !contains(geometry.visible, bbox, CLIP_CONTAIN_TOL_PT)
        || match &p.state.clip {
            ClipState::None => false,
            ClipState::Rect(r) => !contains(*r, bbox, CLIP_CONTAIN_TOL_PT),
            ClipState::Complex => true,
        };
    if clipped {
        return Some(R::Clipped);
    }
    if p.state.fill.pattern || p.state.stroke.pattern {
        return Some(R::Pattern);
    }
    if p.masked {
        return Some(R::MaskedImage);
    }
    let rotation = display_rotation(geometry.rotate);
    if !axis_aligned(&mul(&p.state.ctm, &rotation)) {
        return Some(R::TransformedImage);
    }
    model.page_reason
}

/// A page's content parts are only safe to edit when this page owns them.
fn exclusive_part(ctx: &SnapshotContext, model: &PageModel, span: &Span) -> bool {
    let Some((part, _)) = model.content.locate(span) else {
        return false;
    };
    let refs = RefCounts::of(ctx.doc());
    let Ok(kids) = KidsCounts::of(ctx.doc()) else {
        return false;
    };
    part_exclusive(ctx.doc(), &refs, &kids, &model.content, part)
}

fn rect_of(bbox: [f64; 4]) -> [f64; 4] {
    [
        bbox[0],
        bbox[1],
        (bbox[2] - bbox[0]).max(0.0),
        (bbox[3] - bbox[1]).max(0.0),
    ]
}

/// Plans one image move. `taken` holds the moves already accepted (their spans must not touch
/// each other) and `runs` the accepted text plans (a move may not land inside a text splice).
pub(crate) fn plan_image(
    ctx: &SnapshotContext,
    model: &PageModel,
    edit: &ImageEditIn,
    runs: &[RunPlan],
    taken: &[ImagePlan],
) -> (EditVerdict, Option<ImagePlan>) {
    let locator = edit.locator.trim().to_string();
    let (paint, parsed) = match resolve(ctx, model, edit) {
        Ok(v) => v,
        Err(v) => return (v, None),
    };
    if let Some(reason) = move_refusal(model, paint, &model.walk.geometry) {
        return (
            refused_reason(&locator, reason, "this image cannot be moved"),
            None,
        );
    }
    if !exclusive_part(ctx, model, &parsed.span) {
        return (
            refused_reason(
                &locator,
                TextReason::SharedContent,
                "the page content is shared with another page",
            ),
            None,
        );
    }
    let Some((part, local)) = model.content.locate(&parsed.span) else {
        return (
            refused_reason(
                &locator,
                TextReason::SplitContent,
                "the image crosses a content part boundary",
            ),
            None,
        );
    };
    let move_by = edit.move_by;
    if move_by.is_zero() {
        let bbox = paint.bbox.unwrap_or_default();
        return (
            EditVerdict {
                run_id: locator,
                problem: None,
                delta_pt: 0.0,
                new_rect: Some(rect_of(bbox)),
                caret_offsets: None,
                warnings: Vec::new(),
            },
            None,
        );
    }
    for run in runs {
        for s in &run.splices {
            if parsed.span.start < s.joined.end && s.joined.start < parsed.span.end {
                return (
                    refused(
                        &locator,
                        EditProblemCode::EditConflict,
                        "the image is inside a changed line",
                    ),
                    None,
                );
            }
        }
    }
    for other in taken {
        if parsed.span.start == other.span.end || other.span.start == parsed.span.end {
            continue;
        }
        if parsed.span.start < other.span.end && other.span.start < parsed.span.end {
            return (
                refused(
                    &locator,
                    EditProblemCode::EditConflict,
                    "the same image is moved twice",
                ),
                None,
            );
        }
    }
    let old_ctm = paint.state.ctm;
    // `cm` pre-multiplies the CTM, so its operands are scaled by the image's own matrix. Invert
    // the linear part to turn the requested page-space delta into the operands that produce it.
    let det = old_ctm[0] * old_ctm[3] - old_ctm[1] * old_ctm[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return (
            refused_reason(
                &locator,
                TextReason::TransformedImage,
                "the image has no invertible placement",
            ),
            None,
        );
    }
    let translate = [
        (old_ctm[3] * move_by.x - old_ctm[2] * move_by.y) / det,
        (-old_ctm[1] * move_by.x + old_ctm[0] * move_by.y) / det,
    ];
    let new_ctm = mul(&[1.0, 0.0, 0.0, 1.0, translate[0], translate[1]], &old_ctm);
    let old_bbox = paint.bbox.unwrap_or_else(|| transform_rect(&old_ctm, UNIT_SQUARE));
    let new_bbox = transform_rect(&new_ctm, UNIT_SQUARE);
    if !contains(
        model.walk.geometry.visible,
        new_bbox,
        CLIP_CONTAIN_TOL_PT,
    ) {
        return (
            refused(
                &locator,
                EditProblemCode::TextOutsideVisibleArea,
                "the image would move outside the visible page",
            ),
            None,
        );
    }
    let plan = ImagePlan {
        locator: locator.clone(),
        span: parsed.span,
        part,
        local_prefix: local.start..local.start,
        local_suffix: local.end..local.end,
        old_ctm,
        new_ctm,
        translate,
        new_bbox,
    };
    if same_box(old_bbox, plan.new_bbox) && old_ctm == new_ctm {
        // A degenerate CTM can map a translation to no movement; that is not a move.
        return (
            EditVerdict {
                run_id: locator,
                problem: None,
                delta_pt: 0.0,
                new_rect: Some(rect_of(plan.new_bbox)),
                caret_offsets: None,
                warnings: Vec::new(),
            },
            None,
        );
    }
    (
        EditVerdict {
            run_id: locator,
            problem: None,
            delta_pt: move_by.x.hypot(move_by.y),
            new_rect: Some(rect_of(new_bbox)),
            caret_offsets: None,
            warnings: Vec::new(),
        },
        Some(plan),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_engine::source_content::classify_source_page;
    use crate::pdf_engine::text_edit::rewrite::{plan_page_with_images, MoveIn, PlanOutcome};
    use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec};
    use crate::pdf_engine::text_edit::tests_plan::{ctx, model};

    const IMG_DICT: &str = "/Type /XObject /Subtype /Image /Width 2 /Height 2 \
                            /ColorSpace /DeviceRGB /BitsPerComponent 8";
    const PIXELS: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];
    const PLACE: &[u8] = b"q 40 0 0 40 72 400 cm /Im0 Do Q";

    fn page_with_image(content: &[u8], masked: bool) -> Vec<u8> {
        let mut d = DocBuilder::new();
        let dict = if masked {
            let smask = d.b.add_stream(
                "/Type /XObject /Subtype /Image /Width 2 /Height 2 \
                 /ColorSpace /DeviceGray /BitsPerComponent 8",
                &[0, 255, 0, 255],
            );
            format!("{IMG_DICT} /SMask {smask} 0 R")
        } else {
            IMG_DICT.to_string()
        };
        let img = d.b.add_stream(&dict, PIXELS);
        d.page(PageSpec::new(
            content,
            &format!("/XObject << /Im0 {img} 0 R >>"),
        ));
        d.build()
    }

    /// The first depth-0 paint span of an image XObject (or of an inline image).
    fn span_of(model: &PageModel, inline: bool) -> Span {
        model
            .walk
            .paints
            .iter()
            .find(|p| match (&p.kind, inline) {
                (PaintKind::ImageXObject { .. }, false) => true,
                (PaintKind::InlineImage { .. }, true) => true,
                _ => false,
            })
            .and_then(|p| p.span.clone())
            .expect("image paint")
    }

    fn locator(c: &SnapshotContext, m: &PageModel, span: &Span) -> String {
        format!(
            "v2:{}:{}:i:p{}-{}:0",
            c.snap.fingerprint.sha256_hex(),
            m.page_index,
            span.start,
            span.end
        )
    }

    fn plan(c: &SnapshotContext, m: &PageModel, span: Span, dx: f64, dy: f64) -> PlanOutcome {
        plan_page_with_images(
            c,
            m,
            &[],
            &[ImageEditIn {
                locator: locator(c, m, &span),
                move_by: MoveIn { x: dx, y: dy },
            }],
        )
        .expect("plan")
    }

    #[test]
    fn moves_a_direct_image_by_the_requested_delta() {
        let c = ctx(page_with_image(PLACE, false));
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 10.0, 20.0);
        assert_eq!(out.verdicts.len(), 1);
        assert!(
            out.verdicts[0].problem.is_none(),
            "{:?}",
            out.verdicts[0].problem
        );
        let p = out.plan.as_ref().expect("plan");
        assert_eq!(p.images.len(), 1);
        let after = String::from_utf8_lossy(&p.expected_joined);
        assert!(after.contains("1 0 0 1 0.25 0.5 cm"), "{after}");
        assert_eq!(after.matches("/Im0 Do").count(), 1);
        assert!(
            after.contains("q 40 0 0 40 72 400 cm"),
            "the original placement survives: {after}"
        );
        assert_eq!(p.images[0].new_ctm[4], 82.0);
        assert_eq!(p.images[0].new_ctm[5], 420.0);
        assert_eq!(p.images[0].new_bbox, [82.0, 420.0, 122.0, 460.0]);
    }

    #[test]
    fn one_moved_occurrence_leaves_a_shared_image_alone() {
        let content = b"q 40 0 0 40 72 400 cm /Im0 Do Q \
                        q 40 0 0 40 72 500 cm /Im0 Do Q";
        let c = ctx(page_with_image(content, false));
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 0.0, 50.0);
        assert!(out.verdicts[0].problem.is_none(), "{:?}", out.verdicts);
        let p = out.plan.as_ref().expect("plan");
        let after = String::from_utf8_lossy(&p.expected_joined);
        assert_eq!(after.matches("1 0 0 1 0 1.25 cm").count(), 1, "{after}");
        assert_eq!(after.matches("/Im0 Do").count(), 2, "{after}");
        assert!(
            after.contains("q 40 0 0 40 72 500 cm /Im0 Do Q"),
            "the other occurrence is untouched: {after}"
        );
    }

    #[test]
    fn a_rotated_page_moves_in_unrotated_user_space() {
        let mut d = DocBuilder::new();
        let img = d.b.add_stream(IMG_DICT, PIXELS);
        d.page(
            PageSpec::new(PLACE, &format!("/XObject << /Im0 {img} 0 R >>"))
                .with("/Rotate 90"),
        );
        let c = ctx(d.build());
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 10.0, 20.0);
        assert!(out.verdicts[0].problem.is_none(), "{:?}", out.verdicts);
        let p = out.plan.as_ref().expect("plan");
        assert_eq!(p.images[0].new_ctm[4], 82.0);
        assert_eq!(p.images[0].new_ctm[5], 420.0);
    }

    #[test]
    fn refuses_a_move_outside_the_visible_page() {
        let c = ctx(page_with_image(PLACE, false));
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 600.0, 0.0);
        let p = out.verdicts[0].problem.as_ref().expect("refused");
        assert_eq!(p.code, EditProblemCode::TextOutsideVisibleArea);
        assert!(out.plan.is_none());
    }

    #[test]
    fn refuses_an_inline_image() {
        let mut d = DocBuilder::new();
        d.page(PageSpec::new(
            b"BI /W 2 /H 2 /BPC 8 /CS /RGB /F /AHx ID 00112233445566778899AABB> EI",
            "",
        ));
        let c = ctx(d.build());
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, true), 10.0, 0.0);
        let p = out.verdicts[0].problem.as_ref().expect("refused");
        assert_eq!(p.code, EditProblemCode::TextEditRefused);
        assert_eq!(p.reason, Some(TextReason::InlineImage));
        assert!(out.plan.is_none());
    }

    #[test]
    fn refuses_a_masked_image() {
        let c = ctx(page_with_image(PLACE, true));
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 10.0, 0.0);
        let p = out.verdicts[0].problem.as_ref().expect("refused");
        assert_eq!(p.reason, Some(TextReason::MaskedImage));
        assert!(out.plan.is_none());
    }

    #[test]
    fn the_same_image_cannot_move_twice() {
        let c = ctx(page_with_image(PLACE, false));
        let m = model(&c, 0);
        let loc = locator(&c, &m, &span_of(&m, false));
        let edit = || ImageEditIn {
            locator: loc.clone(),
            move_by: MoveIn { x: 1.0, y: 0.0 },
        };
        let out = plan_page_with_images(&c, &m, &[], &[edit(), edit()]).expect("plan");
        assert!(out.plan.is_none());
        assert_eq!(out.verdicts.len(), 2);
        for v in &out.verdicts {
            let p = v.problem.as_ref().expect("refused");
            assert_eq!(p.code, EditProblemCode::EditConflict);
        }
    }

    #[test]
    fn refuses_an_image_inside_a_form() {
        let mut d = DocBuilder::new();
        let img = d.b.add_stream(IMG_DICT, PIXELS);
        let form = d.b.add_stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
                 /Resources << /XObject << /Im0 {img} 0 R >> >>"
            ),
            b"q 40 0 0 40 72 400 cm /Im0 Do Q",
        );
        d.page(PageSpec::new(
            b"/Fm0 Do",
            &format!("/XObject << /Fm0 {form} 0 R >>"),
        ));
        let c = ctx(d.build());
        let m = model(&c, 0);
        // The Classify walk descends the Form and finds the image under a nested locator.
        let classified = classify_source_page(&c, &m, None);
        let nested = classified
            .occurrences
            .iter()
            .find(|o| o.reason == Some(TextReason::NestedForm))
            .unwrap_or_else(|| {
                panic!("no nested image; occurrences: {:?}", classified.occurrences)
            });
        assert!(
            nested.locator.contains("i:x"),
            "the fixture uses a Form path: {}",
            nested.locator
        );
        let out = plan_page_with_images(
            &c,
            &m,
            &[],
            &[ImageEditIn {
                locator: nested.locator.clone(),
                move_by: MoveIn { x: 10.0, y: 0.0 },
            }],
        )
        .expect("plan");
        let p = out.verdicts[0].problem.as_ref().expect("refused");
        assert_eq!(p.reason, Some(TextReason::NestedForm));
        assert!(out.plan.is_none());
    }

    #[test]
    fn refuses_a_transformed_image() {
        let c = ctx(page_with_image(b"q 40 0 10 40 72 400 cm /Im0 Do Q", false));
        let m = model(&c, 0);
        let out = plan(&c, &m, span_of(&m, false), 10.0, 0.0);
        let p = out.verdicts[0].problem.as_ref().expect("refused");
        assert_eq!(p.reason, Some(TextReason::TransformedImage));
        assert!(out.plan.is_none());
    }
}

impl ImagePlan {
    /// The two zero-length insertions: `q 1 0 0 1 dx dy cm` before the `Do`, `Q` after it.
    pub(crate) fn splices(&self) -> Vec<Splice> {
        // The `cm` operands, not the resulting page-space translation: `cm` scales them by the
        // image's own linear part, and `translate` already compensates for that.
        let [dx, dy] = self.translate;
        let (dx, dy) = match (fmt_precise(dx), fmt_precise(dy)) {
            (Ok(x), Ok(y)) => (x, y),
            _ => (format!("{dx}"), format!("{dy}")),
        };
        vec![
            Splice {
                part: self.part,
                local: self.local_prefix.clone(),
                joined: self.span.start..self.span.start,
                bytes: format!("\nq\n1 0 0 1 {dx} {dy} cm\n").into_bytes(),
            },
            Splice {
                part: self.part,
                local: self.local_suffix.clone(),
                joined: self.span.end..self.span.end,
                bytes: b"\nQ\n".to_vec(),
            },
        ]
    }
}
