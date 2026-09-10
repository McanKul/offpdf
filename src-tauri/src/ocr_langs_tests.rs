//! Fixture tests for `pdf_engine::ocr_langs`. No live Tesseract.

#![cfg(test)]

use crate::pdf_engine::ocr_langs::{parse_tesseract_list_langs, validate_ocr_lang};

const LIST_LANGS_STDOUT: &str = "\
List of available languages in \"/opt/homebrew/share/tessdata/\" (3):\n\
eng\n\
osd\n\
tur\n";

const LIST_LANGS_EMPTY: &str = "\
List of available languages in \"/tmp/empty-tessdata/\" (0):\n";

const INSTALLED: &[&str] = &["eng", "osd"];

#[test]
fn ocr_parse_list_langs_stdout() {
    let got = parse_tesseract_list_langs(LIST_LANGS_STDOUT);
    assert_eq!(
        got,
        ["eng", "osd", "tur"],
        "ocr-parse-list-langs-stdout: header + eng/osd/tur must become [eng, osd, tur]"
    );
    assert!(
        got.iter()
            .all(|code| !code.contains("List of available") && !code.contains("(3)")),
        "ocr-parse-list-langs-stdout: header line must not appear as a code; got {got:?}"
    );
}

#[test]
fn ocr_parse_list_langs_empty() {
    let got = parse_tesseract_list_langs(LIST_LANGS_EMPTY);
    assert!(
        got.is_empty(),
        "ocr-parse-list-langs-empty: header-only (0): must yield []; got {got:?}"
    );
}

#[test]
fn ocr_reject_empty_lang() {
    for lang in ["", "   "] {
        let err = validate_ocr_lang(lang, INSTALLED).expect_err(&format!(
            "ocr-reject-empty-lang: {lang:?} must be AppError, not Ok"
        ));
        assert_eq!(
            err.code, "OCR_LANG_EMPTY",
            "ocr-reject-empty-lang: {lang:?} must be OCR_LANG_EMPTY; got {} ({})",
            err.code, err.message
        );
    }
}

#[test]
fn ocr_reject_missing_lang() {
    let err = validate_ocr_lang("deu", INSTALLED).expect_err(
        "ocr-reject-missing-lang: deu against [eng, osd] must be AppError, not Ok",
    );
    assert_eq!(
        err.code, "OCR_LANG_MISSING",
        "ocr-reject-missing-lang: deu must be OCR_LANG_MISSING; got {} ({})",
        err.code, err.message
    );

    validate_ocr_lang("eng+tur", &["eng", "osd", "tur"]).unwrap_or_else(|err| {
        panic!(
            "ocr-reject-missing-lang control: eng+tur against [eng, osd, tur] must be Ok; got {} ({})",
            err.code, err.message
        )
    });
}

#[test]
fn ocr_reject_plus_lang_partially_installed() {
    let err = validate_ocr_lang("eng+tur", INSTALLED).expect_err(
        "ocr-reject-plus-lang-partially-installed: eng+tur against [eng, osd] must be AppError, not Ok",
    );
    assert_eq!(
        err.code, "OCR_LANG_MISSING",
        "ocr-reject-plus-lang-partially-installed: eng+tur vs [eng, osd] must be OCR_LANG_MISSING; got {} ({})",
        err.code, err.message
    );
    assert!(
        err.message.contains("tur"),
        "ocr-reject-plus-lang-partially-installed: message must name tur; got {}",
        err.message
    );
}
