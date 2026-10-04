//! T5 DTO and command tests: DTO-01…03 (`src/lib/editor/__fixtures__/text-edit-dto-contract.json`
//! is the contract with the TypeScript types; `OFFPDF_UPDATE_GOLDEN=1` rewrites it) and
//! CMD-01…08 (`tests_dto/cmd.rs`, through the service layer the Tauri commands call).

mod cache;
mod cmd;

use crate::pdf_engine::edit_overlay::{EditDocumentIn, EditObjectIn};
use crate::pdf_engine::text_edit::dto::{
    EditProblemDto, EditVerdictDto, FaceOptionDto, FacesDto, PageTextDto, RectDto, RunMetricsDto,
    RunStyleDto, TextEditIn, TextFontDto, TextPreviewDto, TextRunDto, TextSourceDto,
    TextWarningDto, VecDto,
};
use crate::pdf_engine::text_edit::fonts::FamilyHint;
use crate::pdf_engine::text_edit::reasons::{
    EditProblemCode, Face, StyleField, TextReason, TextWarningCode,
};
use crate::pdf_engine::text_edit::runs::SpaceMode;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn contract_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../src/lib/editor/__fixtures__/text-edit-dto-contract.json")
}

fn face_option(available: bool, surface: &[&str]) -> FaceOptionDto {
    FaceOptionDto {
        available,
        surface: surface.iter().map(|s| s.to_string()).collect(),
    }
}

/// One editable and one refused run, with their fonts.
fn page_sample() -> PageTextDto {
    let editable = TextRunDto {
        id: "t1:0123456789abcdef-4d2:0:120-161".into(),
        order: 0,
        line: 0,
        text: "Invoice 2026".into(),
        rect: RectDto {
            x: 72.0,
            y: 697.5,
            w: 61.25,
            h: 11.5,
        },
        origin: VecDto { x: 72.0, y: 700.0 },
        dir: VecDto { x: 1.0, y: 0.0 },
        ascent: 8.75,
        descent: 2.25,
        caret_offsets: vec![
            0.0, 2.5, 8.0, 13.5, 19.0, 22.0, 27.5, 33.0, 35.75, 41.25, 46.75, 52.25, 57.75,
        ],
        editable: true,
        reason: None,
        metrics: Some(RunMetricsDto {
            surface: vec!["f7-0".into(), "f9-0".into()],
            tf_size: 11.04,
            effective_size: 11.04,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            text_to_user: 1.0,
            letter_spacing_pt: 0.0,
            space_mode: SpaceMode::Glyph,
            kern_space: -250.0,
            original_width: 57.75,
            visible_extent: 468.0,
            next_obstacle: Some(120.5),
        }),
        style: Some(RunStyleDto {
            fill: Some("#000000".into()),
            size_changeable: true,
            colour_changeable: true,
            face: Face::Regular,
            faces: FacesDto {
                regular: face_option(true, &["f7-0", "f9-0"]),
                bold: face_option(true, &["f11-0"]),
                italic: face_option(false, &[]),
                bold_italic: face_option(false, &[]),
            },
        }),
        substituted: false,
    };
    let refused = TextRunDto {
        id: "t1:0123456789abcdef-4d2:0:200-230".into(),
        order: 1,
        line: 1,
        text: "hidden layer".into(),
        rect: RectDto {
            x: 72.0,
            y: 677.5,
            w: 60.0,
            h: 11.5,
        },
        origin: VecDto { x: 72.0, y: 680.0 },
        dir: VecDto { x: 1.0, y: 0.0 },
        ascent: 8.75,
        descent: 2.25,
        caret_offsets: vec![
            0.0, 6.0, 8.5, 14.0, 19.5, 25.0, 31.0, 34.0, 36.5, 42.0, 48.0, 53.5, 57.0,
        ],
        editable: false,
        reason: Some(TextReason::OptionalContent),
        metrics: None,
        style: None,
        substituted: false,
    };
    let font = |key: &str, name: &str, alphabet: &str, embedded: bool| TextFontDto {
        key: key.into(),
        display_name: name.into(),
        family_hint: FamilyHint::Sans,
        embedded,
        subset: embedded,
        alphabet: alphabet.into(),
        widths: alphabet.chars().map(|_| 500.0).collect(),
        word_space: alphabet.contains(' '),
    };
    PageTextDto {
        fingerprint: "0123456789abcdef-4d2".into(),
        page_index: 0,
        page_reason: None,
        runs: vec![editable, refused],
        fonts: vec![
            font("f11-0", "Calibri Bold", " 0267I", true),
            font("f7-0", "Calibri", " 0267DIceinouv", true),
            font("f9-0", "Calibri", "ğış", true),
        ],
    }
}

fn preview_sample() -> TextPreviewDto {
    TextPreviewDto {
        page_pdf: Some("JVBERi0xLjcK".into()),
        verdicts: vec![
            EditVerdictDto {
                run_id: "t1:0123456789abcdef-4d2:0:120-161".into(),
                ok: true,
                code: None,
                chars: Vec::new(),
                reason: None,
                face: None,
                field: None,
                detail: None,
                delta_pt: 0.0,
                new_rect: Some(RectDto {
                    x: 72.0,
                    y: 697.5,
                    w: 61.25,
                    h: 11.5,
                }),
                caret_offsets: Some(vec![
                    0.0, 2.5, 8.0, 13.5, 19.0, 22.0, 27.5, 33.0, 35.75, 41.25, 46.75, 52.25, 57.75,
                ]),
            },
            EditVerdictDto {
                run_id: "t1:0123456789abcdef-4d2:0:300-320".into(),
                ok: false,
                code: Some(EditProblemCode::GlyphMissing),
                chars: vec!["Y".into()],
                reason: None,
                face: None,
                field: None,
                detail: Some("not in the subset".into()),
                delta_pt: 0.0,
                new_rect: None,
                caret_offsets: None,
            },
            EditVerdictDto {
                run_id: "t1:0123456789abcdef-4d2:0:330-350".into(),
                ok: false,
                code: Some(EditProblemCode::TextEditRefused),
                chars: Vec::new(),
                reason: Some(TextReason::SharedContent),
                face: None,
                field: None,
                detail: None,
                delta_pt: 0.0,
                new_rect: None,
                caret_offsets: None,
            },
            EditVerdictDto {
                run_id: "t1:0123456789abcdef-4d2:0:360-380".into(),
                ok: false,
                code: Some(EditProblemCode::FaceUnavailable),
                chars: vec!["ğ".into()],
                reason: None,
                face: Some(Face::BoldItalic),
                field: None,
                detail: None,
                delta_pt: 0.0,
                new_rect: None,
                caret_offsets: None,
            },
            EditVerdictDto {
                run_id: "t1:0123456789abcdef-4d2:0:390-410".into(),
                ok: false,
                code: Some(EditProblemCode::StyleUnavailable),
                chars: Vec::new(),
                reason: None,
                face: None,
                field: Some(StyleField::Colour),
                detail: None,
                delta_pt: 0.0,
                new_rect: None,
                caret_offsets: None,
            },
        ],
        page_problem: Some(EditProblemDto {
            code: EditProblemCode::PenDrift,
            detail: Some("phase=A check=A4 page=1 drift 0.02 pt".into()),
        }),
        warnings: vec![TextWarningDto {
            run_id: "t1:0123456789abcdef-4d2:0:120-161".into(),
            code: TextWarningCode::NextTextOverlap,
            detail: None,
        }],
    }
}

fn text_edit_in_sample() -> Value {
    json!({
        "runId": "t1:0123456789abcdef-4d2:0:120-161",
        "originalText": "Invoice 2026",
        "text": "Invoice 2027",
        "style": { "sizePt": 12.5, "face": "boldItalic", "fill": "#c71c1c", "letterSpacingPt": 0.5 },
    })
}

fn source_text_export_sample() -> Value {
    json!({
        "id": "obj-7",
        "kind": "sourceText",
        "pageIndex": 2,
        "rect": { "x": 72.0, "y": 697.5, "w": 61.25, "h": 11.5 },
        "locked": true,
        "runId": "t1:0123456789abcdef-4d2:0:120-161",
        "sourceFingerprint": "0123456789abcdef-4d2",
        "sourcePageIndex": 0,
        "originalText": "Invoice 2026",
        "text": "Invoice 2027",
        "style": { "fill": "#c71c1c" },
    })
}

/// The whole contract document.
fn contract() -> Value {
    let source = TextSourceDto {
        fingerprint: "0123456789abcdef-4d2".into(),
        page_count: 3,
        warnings: vec![
            "This PDF is larger than 100 MB, so checking and saving text changes takes longer."
                .into(),
        ],
    };
    let ser = |v: Result<Value, serde_json::Error>| v.unwrap_or_else(|e| panic!("serialise: {e}"));
    json!({
        "version": 1,
        "about": "One serialised sample of every Edit text DTO (Rust → TS) and of every input (TS → Rust). Written by src-tauri text_edit::tests_dto (DTO-01, OFFPDF_UPDATE_GOLDEN=1); the TS types in src/lib/types.ts must have exactly these keys.",
        "TextSourceInfo": ser(serde_json::to_value(&source)),
        "PageText": ser(serde_json::to_value(page_sample())),
        "TextPreview": ser(serde_json::to_value(preview_sample())),
        "TextEditIn": text_edit_in_sample(),
        "SourceTextObject": source_text_export_sample(),
    })
}

/// DTO-01: the serialised samples equal the contract file (values compared after parsing).
#[test]
fn dto_01_serialised_dtos_equal_the_contract_file() {
    let expected = contract();
    let path = contract_path();
    if std::env::var("OFFPDF_UPDATE_GOLDEN").as_deref() == Ok("1") {
        let mut text = serde_json::to_string_pretty(&expected).expect("json");
        text.push('\n');
        std::fs::write(&path, text).expect("write contract");
    }
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .expect("contract JSON");
    assert_eq!(
        on_disk, expected,
        "DTO-01: text-edit-dto-contract.json is out of date"
    );
}

/// DTO-02: the contract's inputs deserialize into the Rust input types.
#[test]
fn dto_02_text_edit_in_and_source_text_export_deserialize() {
    let c = contract();
    let edit: TextEditIn = serde_json::from_value(c["TextEditIn"].clone()).expect("TextEditIn");
    assert_eq!(edit.text, "Invoice 2027");
    assert_eq!(edit.style.face, Some(Face::BoldItalic));
    assert_eq!(edit.style.size_pt, Some(12.5));
    assert_eq!(edit.style.letter_spacing_pt, Some(0.5));
    // `style` may be omitted (an unstyled change).
    let bare: TextEditIn = serde_json::from_value(json!({
        "runId": "r", "originalText": "a", "text": "b"
    }))
    .expect("bare TextEditIn");
    assert!(bare.style.is_empty());
    let doc: EditDocumentIn = serde_json::from_value(json!({
        "version": 1,
        "objects": [c["SourceTextObject"].clone()],
    }))
    .expect("export document");
    match doc.objects.first() {
        Some(EditObjectIn::SourceText {
            page_index,
            source_page_index,
            run_id,
            source_fingerprint,
            original_text,
            text,
            style,
            rect,
        }) => {
            assert_eq!((*page_index, *source_page_index), (2, 0));
            assert_eq!(run_id, "t1:0123456789abcdef-4d2:0:120-161");
            assert_eq!(source_fingerprint, "0123456789abcdef-4d2");
            assert_eq!(
                (original_text.as_str(), text.as_str()),
                ("Invoice 2026", "Invoice 2027")
            );
            assert_eq!(style.fill.as_deref(), Some("#c71c1c"));
            assert_eq!((rect.x, rect.w), (72.0, 61.25));
        }
        other => panic!("DTO-02: not a sourceText object: {other:?}"),
    }
}

/// DTO-03: reason, problem and warning codes serialise as the SCREAMING_SNAKE strings of
/// `text-reasons.json`; faces, fields, space modes and family hints in camelCase.
#[test]
fn dto_03_codes_serialise_as_the_shared_strings() {
    let json: Value =
        serde_json::from_str(include_str!("../../../../src/lib/editor/text-reasons.json"))
            .expect("text-reasons.json");
    let list = |key: &str| -> Vec<String> {
        json[key]
            .as_array()
            .expect("list")
            .iter()
            .map(|v| v.as_str().expect("string").to_string())
            .collect()
    };
    let ser = |v: Value| v.as_str().expect("a string").to_string();
    let run: Vec<String> = TextReason::RUN_PRIORITY
        .iter()
        .map(|r| ser(json!(r)))
        .collect();
    let page: Vec<String> = TextReason::PAGE.iter().map(|r| ser(json!(r))).collect();
    let problems: Vec<String> = EditProblemCode::ALL.iter().map(|c| ser(json!(c))).collect();
    let warnings: Vec<String> = TextWarningCode::ALL.iter().map(|c| ser(json!(c))).collect();
    assert_eq!(run, list("run"), "DTO-03 run");
    assert_eq!(page, list("page"), "DTO-03 page");
    assert_eq!(problems, list("problem"), "DTO-03 problem");
    assert_eq!(warnings, list("warning"), "DTO-03 warning");
    for r in TextReason::RUN_PRIORITY.iter().chain(TextReason::PAGE) {
        assert_eq!(ser(json!(r)), r.as_str());
    }
    assert_eq!(json!(Face::BoldItalic), json!("boldItalic"));
    assert_eq!(json!(StyleField::Colour), json!("colour"));
    assert_eq!(json!(SpaceMode::Kern), json!("kern"));
    assert_eq!(json!(FamilyHint::Mono), json!("mono"));
}

/// Every object key path of `v` (`a.b[].c`), arrays merged.
/// (key path, JSON kind) of every non-null value under `v` (review-T5 M5: value types, not only
/// key names; `null` is left out because an `Option` serialises either way).
pub(crate) fn kind_paths(v: &Value, at: &str, out: &mut BTreeSet<(String, &'static str)>) {
    let kind = match v {
        Value::Null => None,
        Value::Bool(_) => Some("boolean"),
        Value::Number(_) => Some("number"),
        Value::String(_) => Some("string"),
        Value::Array(_) => Some("array"),
        Value::Object(_) => Some("object"),
    };
    if let Some(k) = kind {
        out.insert((at.to_string(), k));
    }
    match v {
        Value::Object(m) => m
            .iter()
            .for_each(|(k, x)| kind_paths(x, &format!("{at}.{k}"), out)),
        Value::Array(a) => a
            .iter()
            .for_each(|x| kind_paths(x, &format!("{at}[]"), out)),
        _ => {}
    }
}

pub(crate) fn key_paths(v: &Value, at: &str, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = format!("{at}.{k}");
                out.insert(p.clone());
                key_paths(x, &p, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                key_paths(x, &format!("{at}[]"), out);
            }
        }
        _ => {}
    }
}

/// review-T5 L4: the DTO's current letter spacing is the planner's own reading (one formula), so
/// the editor sending it back unchanged plans nothing — on a run with `Tc`, `Tz` and a scaled
/// text matrix, where a second copy of the formula could drift.
#[test]
fn letter_spacing_sent_back_unchanged_is_a_no_op() {
    use crate::pdf_engine::source_content::classify_source_page;
    use crate::pdf_engine::text_edit::context::SnapshotContext;
    use crate::pdf_engine::text_edit::rewrite::{letter_spacing_pt, plan_page, SourceTextStyleIn};
    use crate::pdf_engine::text_edit::runs::build_page_model;
    use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
    use crate::pdf_engine::text_edit::testkit::producers::helvetica_page;
    use crate::pdf_engine::text_edit::testkit::Scratch;
    let pdf = helvetica_page(b"BT /F1 10 Tf 0.7 Tc 85 Tz 1.5 0 0 1.5 72 700 Tm (Spaced out) Tj ET");
    let dir = Scratch::new("dto_l4");
    let path = dir.write("spaced.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let model = build_page_model(&ctx, 0, None).expect("model");
    let dto = crate::pdf_engine::text_edit::dto::page_text(
        &model,
        &classify_source_page(&ctx, &model, None),
    );
    let run = dto
        .runs
        .iter()
        .find(|r| r.text == "Spaced out")
        .expect("run");
    let metrics = run.metrics.as_ref().expect("editable");
    let planner = model
        .run(&run.id)
        .map(letter_spacing_pt)
        .expect("model run");
    assert_eq!(metrics.letter_spacing_pt, planner, "one formula");
    assert!(
        metrics.letter_spacing_pt > 0.5,
        "{}",
        metrics.letter_spacing_pt
    );
    let edit = TextEditIn {
        run_id: run.id.clone(),
        original_text: run.text.clone(),
        text: run.text.clone(),
        style: SourceTextStyleIn {
            letter_spacing_pt: Some(metrics.letter_spacing_pt),
            ..Default::default()
        },
    };
    let out = plan_page(&ctx, &model, &[edit]).expect("plan");
    assert!(
        out.verdicts[0].problem.is_none(),
        "{:?}",
        out.verdicts[0].problem
    );
    assert!(
        out.plan.is_none(),
        "the unchanged spacing was planned as a change"
    );
}
