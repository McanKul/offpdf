//! DTOs of the text-edit Tauri commands (SPEC §B.20) and their builders. JSON shapes mirror the
//! TypeScript types in `src/lib/types.ts` (`TextSourceInfo`, `PageText`, `TextRun`, `TextFont`,
//! `TextPreview`, …); `src/lib/editor/__fixtures__/text-edit-dto-contract.json` holds one sample
//! of each, checked byte for byte by `tests_dto` (DTO-01) and key by key by the frontend.
//!
//! Font keys are opaque per response: `f{obj}-{gen}` for an indirect font, `d{hash}` for a
//! direct one. A run's `editable`/`reason` come from the #33 classifier's run capability
//! (`source_content::classify_source_page`), geometry, metrics and style from the same model.

use crate::pdf_engine::source_content::{
    SourceCapability, SourceFormLine, SourcePageResult, SourceRunCapability,
};
use crate::pdf_engine::text_edit::fit::word_space;
use crate::pdf_engine::text_edit::fonts::{face_surface, FamilyHint, FontKey, FontModel};
use crate::pdf_engine::text_edit::gate::TextWarning;
use crate::pdf_engine::text_edit::preview::PreviewResult;
use crate::pdf_engine::text_edit::reasons::{
    EditProblemCode, Face, StyleField, TextReason, TextWarningCode,
};
use crate::pdf_engine::text_edit::rewrite::{letter_spacing_pt, EditVerdict};
use crate::pdf_engine::text_edit::runs::{PageModel, SpaceMode, TextRun};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub use super::rewrite::TextEditIn; // Deserialize type, defined in rewrite.rs (B.12)

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSourceDto {
    pub fingerprint: String,
    pub page_count: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RectDto {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VecDto {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTextDto {
    pub fingerprint: String,
    pub page_index: u32,
    pub page_reason: Option<TextReason>,
    pub runs: Vec<TextRunDto>,
    pub fonts: Vec<TextFontDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRunDto {
    pub id: String,
    pub order: u32,
    pub line: u32,
    pub text: String,
    pub rect: RectDto,
    pub origin: VecDto,
    pub dir: VecDto,
    pub ascent: f64,
    pub descent: f64,
    pub caret_offsets: Vec<f64>,
    pub editable: bool,
    pub reason: Option<TextReason>,
    /// `Some` iff editable.
    pub metrics: Option<RunMetricsDto>,
    pub style: Option<RunStyleDto>,
    pub substituted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMetricsDto {
    /// Font keys of the typing surface, primary first.
    pub surface: Vec<String>,
    pub tf_size: f64,
    pub effective_size: f64,
    pub char_spacing: f64,
    pub word_spacing: f64,
    pub h_scale: f64,
    pub text_to_user: f64,
    pub letter_spacing_pt: f64,
    pub space_mode: SpaceMode,
    pub kern_space: f64,
    pub original_width: f64,
    pub visible_extent: f64,
    pub next_obstacle: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceOptionDto {
    pub available: bool,
    pub surface: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacesDto {
    pub regular: FaceOptionDto,
    pub bold: FaceOptionDto,
    pub italic: FaceOptionDto,
    pub bold_italic: FaceOptionDto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStyleDto {
    pub fill: Option<String>,
    pub size_changeable: bool,
    pub colour_changeable: bool,
    pub face: Face,
    pub faces: FacesDto,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFontDto {
    pub key: String,
    pub display_name: String,
    pub family_hint: FamilyHint,
    pub embedded: bool,
    pub subset: bool,
    /// Every typeable character.
    pub alphabet: String,
    /// `widths[i]`: width of the i-th character of `alphabet` in thousandths of text space
    /// (the `/Widths` scale).
    pub widths: Vec<f64>,
    /// The typeable space is the single byte 0x20, so `Tw` applies to it.
    pub word_space: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditVerdictDto {
    pub run_id: String,
    pub ok: bool,
    pub code: Option<EditProblemCode>,
    pub chars: Vec<String>,
    /// TEXT_EDIT_REFUSED: the run's reason.
    pub reason: Option<TextReason>,
    /// FACE_UNAVAILABLE: the requested face.
    pub face: Option<Face>,
    /// STYLE_UNAVAILABLE: which control.
    pub field: Option<StyleField>,
    pub detail: Option<String>,
    pub delta_pt: f64,
    pub new_rect: Option<RectDto>,
    /// Caret offsets of the new text (planned advances) when ok.
    pub caret_offsets: Option<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditProblemDto {
    pub code: EditProblemCode,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextWarningDto {
    pub run_id: String,
    pub code: TextWarningCode,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreviewDto {
    /// Base64 of the patched one-page PDF.
    pub page_pdf: Option<String>,
    pub verdicts: Vec<EditVerdictDto>,
    pub page_problem: Option<EditProblemDto>,
    pub warnings: Vec<TextWarningDto>,
}

// ---- Builders ----------------------------------------------------------------------------

pub fn rect_dto(r: [f64; 4]) -> RectDto {
    RectDto {
        x: r[0],
        y: r[1],
        w: r[2],
        h: r[3],
    }
}

fn vec_dto(v: (f64, f64)) -> VecDto {
    VecDto { x: v.0, y: v.1 }
}

/// The opaque key of a font model in DTOs.
pub fn font_key(m: &FontModel) -> String {
    match m.key {
        FontKey::Indirect((n, g)) => format!("f{n}-{g}"),
        FontKey::Direct { name_hash, .. } => format!("d{name_hash:x}"),
    }
}

/// The fonts a response refers to, by key (first use order is irrelevant: keys are sorted).
#[derive(Default)]
struct FontSet {
    by_key: BTreeMap<String, Arc<FontModel>>,
}

impl FontSet {
    fn keys(&mut self, models: &[Arc<FontModel>]) -> Vec<String> {
        models
            .iter()
            .map(|m| {
                let key = font_key(m);
                self.by_key
                    .entry(key.clone())
                    .or_insert_with(|| Arc::clone(m));
                key
            })
            .collect()
    }

    fn into_dtos(self) -> Vec<TextFontDto> {
        self.by_key
            .into_iter()
            .map(|(key, m)| font_dto(key, &m))
            .collect()
    }
}

fn font_dto(key: String, m: &FontModel) -> TextFontDto {
    let alphabet = m.alphabet();
    TextFontDto {
        key,
        display_name: m.display_name.clone(),
        family_hint: m.family_hint,
        embedded: m.embedded,
        subset: m.subset,
        alphabet: alphabet.iter().map(|(c, _)| *c).collect(),
        widths: alphabet.iter().map(|(_, w)| *w).collect(),
        word_space: word_space(m),
    }
}

fn face_option(
    model: &PageModel,
    run: &TextRun,
    own: &[Arc<FontModel>],
    face: Face,
    fonts: &mut FontSet,
) -> FaceOptionDto {
    if face == run.face {
        return FaceOptionDto {
            available: !own.is_empty(),
            surface: fonts.keys(own),
        };
    }
    let other = match own.first() {
        Some(primary) if !run.font_from_extgstate => {
            face_surface(&model.walk.page_fonts, primary, face)
        }
        _ => None,
    };
    match other {
        Some(s) => {
            let models: Vec<Arc<FontModel>> = s.fonts.into_iter().map(|(_, m)| m).collect();
            FaceOptionDto {
                available: !models.is_empty(),
                surface: fonts.keys(&models),
            }
        }
        None => FaceOptionDto {
            available: false,
            surface: Vec::new(),
        },
    }
}

/// Metrics and style of an editable run (the fonts it can type with are added to `fonts`).
fn editable_parts(
    model: &PageModel,
    run: &TextRun,
    fonts: &mut FontSet,
) -> (RunMetricsDto, RunStyleDto) {
    let own: Vec<Arc<FontModel>> = model
        .surface(run)
        .fonts
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    let metrics = RunMetricsDto {
        surface: fonts.keys(&own),
        tf_size: run.tfs,
        effective_size: run.effective_size,
        char_spacing: run.tc,
        word_spacing: run.tw,
        h_scale: run.th,
        text_to_user: run.text_to_user_x,
        // The planner's own reading, so a value sent back unchanged is a no-op (review-T5 L4).
        letter_spacing_pt: letter_spacing_pt(run),
        space_mode: run.space_mode,
        kern_space: run.kern_space,
        original_width: run.original_extent,
        visible_extent: run.visible_extent,
        next_obstacle: run.next_obstacle,
    };
    let mut face = |f: Face| face_option(model, run, &own, f, fonts);
    let faces = FacesDto {
        regular: face(Face::Regular),
        bold: face(Face::Bold),
        italic: face(Face::Italic),
        bold_italic: face(Face::BoldItalic),
    };
    let style = RunStyleDto {
        fill: run.fill_hex.clone(),
        size_changeable: !run.font_from_extgstate,
        colour_changeable: run.tr == 0,
        face: run.face,
        faces,
    };
    (metrics, style)
}

/// `PageText` of one page: geometry, metrics and style from the model, `editable`/`reason` from
/// the classifier's run capabilities (a run the classifier did not list is not editable).
/// A line of text drawn through a Form XObject: shown, never editable (`NESTED_FORM`, §A.6;
/// review-T5 live B1), after the page's own runs.
fn form_line_dto(l: &SourceFormLine) -> TextRunDto {
    TextRunDto {
        id: l.id.clone(),
        order: l.order,
        line: l.line,
        text: l.text.clone(),
        rect: rect_dto(l.rect),
        origin: vec_dto(l.origin),
        dir: vec_dto(l.dir),
        ascent: l.ascent,
        descent: l.descent,
        caret_offsets: l.caret_offsets.clone(),
        editable: false,
        reason: Some(l.reason),
        metrics: None,
        style: None,
        substituted: l.substituted,
    }
}

pub fn page_text(model: &PageModel, classified: &SourcePageResult) -> PageTextDto {
    let caps: HashMap<&str, &SourceRunCapability> = classified
        .runs
        .iter()
        .map(|c| (c.run_id.as_str(), c))
        .collect();
    let mut fonts = FontSet::default();
    let runs = model
        .runs
        .iter()
        .map(|run| {
            let cap = caps.get(run.id.as_str());
            let editable = classified.page_reason.is_none()
                && cap.is_some_and(|c| c.capability == SourceCapability::Supported);
            let reason = cap.map_or(run.reason, |c| c.reason);
            let (metrics, style) = if editable {
                let (m, s) = editable_parts(model, run, &mut fonts);
                (Some(m), Some(s))
            } else {
                (None, None)
            };
            TextRunDto {
                id: run.id.clone(),
                order: run.order,
                line: run.line,
                text: run.text.clone(),
                rect: rect_dto(run.rect),
                origin: vec_dto(run.origin),
                dir: vec_dto(run.dir),
                ascent: run.ascent,
                descent: run.descent,
                caret_offsets: run.caret_offsets.clone(),
                editable,
                reason,
                metrics,
                style,
                substituted: run.substituted,
            }
        })
        .chain(classified.form_lines.iter().map(form_line_dto))
        .collect();
    PageTextDto {
        fingerprint: model.fingerprint.to_string(),
        page_index: model.page_index,
        page_reason: classified.page_reason,
        runs,
        fonts: fonts.into_dtos(),
    }
}

pub fn verdict_dto(v: &EditVerdict) -> EditVerdictDto {
    let p = v.problem.as_ref();
    let ok = p.is_none();
    EditVerdictDto {
        run_id: v.run_id.clone(),
        ok,
        code: p.map(|p| p.code),
        chars: p.map_or_else(Vec::new, |p| p.chars.iter().map(char::to_string).collect()),
        reason: p.and_then(|p| p.reason),
        face: p.and_then(|p| p.face),
        field: p.and_then(|p| p.field),
        detail: p.and_then(|p| p.detail.clone()),
        delta_pt: v.delta_pt,
        new_rect: v.new_rect.map(rect_dto),
        caret_offsets: if ok { v.caret_offsets.clone() } else { None },
    }
}

pub fn warning_dto(w: &TextWarning) -> TextWarningDto {
    TextWarningDto {
        run_id: w.run_id.clone(),
        code: w.code,
        detail: w.detail.clone(),
    }
}

pub fn preview_dto(r: &PreviewResult) -> TextPreviewDto {
    TextPreviewDto {
        page_pdf: r.pdf.as_deref().map(crate::pdf_engine::render::base64),
        verdicts: r.verdicts.iter().map(verdict_dto).collect(),
        page_problem: r.page_problem.as_ref().map(|p| EditProblemDto {
            code: p.code,
            detail: p.detail.clone(),
        }),
        warnings: r.warnings.iter().map(warning_dto).collect(),
    }
}
