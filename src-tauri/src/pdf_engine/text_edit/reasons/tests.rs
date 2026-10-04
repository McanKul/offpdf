//! `reasons_json_matches_enums`, `save_failures_say_original_unchanged` and the copy deck.

use super::*;

const JSON: &str = include_str!("../../../../../src/lib/editor/text-reasons.json");

fn list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn reasons_json_matches_enums() {
    let v: serde_json::Value = serde_json::from_str(JSON).unwrap();
    assert_eq!(v["version"], 1, "reasons_json_matches_enums: version");
    let strs = |xs: &[TextReason]| {
        xs.iter()
            .map(|r| r.as_str().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        list(&v, "run"),
        strs(TextReason::RUN_PRIORITY),
        "reasons_json_matches_enums: run"
    );
    assert_eq!(
        list(&v, "page"),
        strs(TextReason::PAGE),
        "reasons_json_matches_enums: page"
    );
    assert_eq!(
        list(&v, "image"),
        strs(TextReason::IMAGE_PRIORITY),
        "reasons_json_matches_enums: image"
    );
    let problems: Vec<String> = EditProblemCode::ALL
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        list(&v, "problem"),
        problems,
        "reasons_json_matches_enums: problem"
    );
    let warnings: Vec<String> = TextWarningCode::ALL
        .iter()
        .map(|w| w.as_str().to_string())
        .collect();
    assert_eq!(
        list(&v, "warning"),
        warnings,
        "reasons_json_matches_enums: warning"
    );
    // serde spelling equals as_str for every enum value
    let all: Vec<TextReason> = TextReason::RUN_PRIORITY
        .iter()
        .chain(TextReason::PAGE)
        .chain(TextReason::IMAGE_PRIORITY)
        .copied()
        .collect();
    for r in all {
        assert_eq!(serde_json::to_value(r).unwrap(), r.as_str());
        assert_eq!(
            serde_json::from_value::<TextReason>(r.as_str().into()).unwrap(),
            r
        );
    }
    for p in EditProblemCode::ALL {
        assert_eq!(serde_json::to_value(p).unwrap(), p.as_str());
    }
    for w in TextWarningCode::ALL {
        assert_eq!(serde_json::to_value(w).unwrap(), w.as_str());
    }
    // file-level codes built here appear in "file"; Save-only codes in "save"
    let file = list(&v, "file");
    for e in file_level_errors() {
        if e.code != "SOURCE_EDIT_GATE_FAILED" {
            assert!(
                file.contains(&e.code),
                "reasons_json_matches_enums: file code {}",
                e.code
            );
        }
    }
    assert!(list(&v, "save").contains(&"SOURCE_EDIT_GATE_FAILED".to_string()));
    // 38 distinct reason values, none left out of the lists
    let mut distinct: Vec<&str> = TextReason::RUN_PRIORITY
        .iter()
        .chain(TextReason::PAGE)
        .chain(TextReason::IMAGE_PRIORITY)
        .map(|r| r.as_str())
        .collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 38);
    assert_eq!(
        serde_json::to_value(Face::BoldItalic).unwrap(),
        "boldItalic"
    );
    assert_eq!(serde_json::to_value(StyleField::Colour).unwrap(), "colour");
}

fn file_level_errors() -> Vec<AppError> {
    vec![
        file_too_large(),
        file_too_complex("detail"),
        encrypted(),
        signed(),
        unsupported_xfa(),
        malformed_content("detail"),
        pdf_needs_repair(&["w1".to_string(), "w2".to_string()]),
        stale("a.pdf"),
        verifier_missing("pdftotext"),
        qpdf_too_old("10.6.3"),
        source_edit_gate_failed("A2"),
        invalid_xref("detail"),
    ]
}

fn every_save_error() -> Vec<AppError> {
    let mut out = file_level_errors();
    let ctx = ProblemCtx {
        page_number: Some(3),
        file_name: Some("a.pdf"),
        face: Some(Face::Bold),
        reason: None,
    };
    let ctx_none = ProblemCtx {
        page_number: None,
        file_name: None,
        face: None,
        reason: None,
    };
    for code in EditProblemCode::ALL {
        let mut p = EditProblem::new(*code, Some("detail".into()));
        p.chars = vec!['ğ', ' '];
        p.reason = Some(TextReason::Type3);
        p.field = Some(StyleField::Colour);
        out.push(code.to_app_error(&p, &ctx));
        out.push(code.to_app_error(&EditProblem::new(*code, None), &ctx_none));
    }
    for r in TextReason::RUN_PRIORITY
        .iter()
        .chain(TextReason::PAGE)
        .chain(TextReason::IMAGE_PRIORITY)
    {
        out.push(r.to_app_error(Some(2)));
        out.push(r.to_app_error(None));
    }
    out
}

#[test]
fn save_failures_say_original_unchanged() {
    for e in every_save_error() {
        let code = e.code.clone();
        let once = save_failure(e);
        let s = once.suggestion.clone().unwrap();
        assert!(
            s.ends_with(ORIGINAL_UNCHANGED),
            "save_failures_say_original_unchanged: {code}: {s}"
        );
        assert_eq!(
            s.matches(ORIGINAL_UNCHANGED).count(),
            1,
            "save_failures_say_original_unchanged: {code}: {s}"
        );
        let twice = save_failure(once.clone());
        assert_eq!(twice.suggestion, once.suggestion, "idempotent: {code}");
        assert!(!once.title.is_empty() && !once.message.is_empty(), "{code}");
    }
    // a suggestion-less error gets the sentence as the whole suggestion
    assert_eq!(
        save_failure(file_too_complex("x")).suggestion.as_deref(),
        Some(ORIGINAL_UNCHANGED)
    );
    // STALE is already terminated and stays unchanged
    assert_eq!(
        save_failure(stale("a.pdf")).suggestion,
        stale("a.pdf").suggestion
    );
}

#[test]
fn problem_copy_matches_the_deck() {
    let ctx = ProblemCtx {
        page_number: Some(4),
        file_name: Some("r.pdf"),
        face: None,
        reason: None,
    };
    let mut p = EditProblem::new(EditProblemCode::GlyphMissing, None);
    p.chars = vec!['ğ', ' ', 'Y'];
    let e = EditProblemCode::GlyphMissing.to_app_error(&p, &ctx);
    assert_eq!(e.code, "GLYPH_MISSING");
    assert_eq!(
        e.message,
        "On page 4, the document's font can't draw: ğ, space, Y"
    );
    let mut f = EditProblem::new(EditProblemCode::FaceUnavailable, None);
    f.face = Some(Face::Bold);
    assert_eq!(
        EditProblemCode::FaceUnavailable
            .to_app_error(&f, &ctx)
            .message,
        "On page 4: This page has no bold version of this font."
    );
    f.face = Some(Face::Italic);
    f.chars = vec!['ş'];
    assert_eq!(
        EditProblemCode::FaceUnavailable
            .to_app_error(&f, &ctx)
            .message,
        "On page 4: The italic version of this font can't draw: ş"
    );
    let mut r = EditProblem::new(EditProblemCode::TextEditRefused, Some("run 7".into()));
    r.reason = Some(TextReason::SharedContent);
    let e = EditProblemCode::TextEditRefused.to_app_error(&r, &ctx);
    assert_eq!(e.message, "On page 4: This part of the page is shared with other pages, so a change here would change them too.");
    assert!(e.details.unwrap().contains("reason: SHARED_CONTENT"));
    let s =
        EditProblemCode::Stale.to_app_error(&EditProblem::new(EditProblemCode::Stale, None), &ctx);
    assert_eq!(s.code, "STALE");
    assert!(s.message.starts_with("\u{201c}r.pdf\u{201d} was changed"));
    for (field, text) in [
        (StyleField::Size, "This line's size is set in a way OffPDF can't change."),
        (StyleField::Face, "This line's font is set in a way OffPDF can't change, so bold and italic aren't available."),
        (StyleField::Colour, "This text is drawn with an outline, so its colour can't be changed here."),
    ] {
        let mut p = EditProblem::new(EditProblemCode::StyleUnavailable, None);
        p.field = Some(field);
        assert_eq!(EditProblemCode::StyleUnavailable.to_app_error(&p, &ctx).message, format!("On page 4: {text}"));
    }
    let page = TextReason::Geometry.to_app_error(Some(9));
    assert_eq!(page.code, "GEOMETRY");
    let run = TextReason::Type3.to_app_error(Some(9));
    assert_eq!(run.code, "TEXT_EDIT_REFUSED");
    assert!(run.details.unwrap().contains("TYPE3"));
}
