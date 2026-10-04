//! E2E-01…06: one save per font class and page geometry, through the real export.

use super::{page_parts, E2e};
use crate::pdf_engine::text_edit::engines::RunOpts;
use crate::pdf_engine::text_edit::poppler::pdftotext_words;
use crate::pdf_engine::text_edit::reasons::{TextReason, ORIGINAL_UNCHANGED};
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page};
use serde_json::json;
use std::path::Path;

fn corpus(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/source-edit")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Poppler's box of the word `word` on page `page` (0-based).
pub(crate) fn word_box(t: &E2e, pdf: &Path, page: u32, word: &str) -> [f64; 4] {
    let words = pdftotext_words(&t.engines, pdf, page + 1, &RunOpts::default())
        .unwrap_or_else(|e| panic!("pdftotext: {e}"));
    let w = words.iter().find(|w| w.text == word).unwrap_or_else(|| {
        panic!(
            "no word {word:?} in {:?}",
            words.iter().map(|w| &w.text).collect::<Vec<_>>()
        )
    });
    [w.x0, w.y0, w.x1, w.y1]
}

pub(crate) fn assert_same_box(a: [f64; 4], b: [f64; 4], what: &str) {
    for (x, y) in a.iter().zip(&b) {
        assert!((x - y).abs() <= 0.05, "{what} moved: {a:?} → {b:?}");
    }
}

/// One save of `edits` (`(old, new)`) on page 1 of `pdf`, checked on the published file.
fn save_one(t: &E2e, name: &str, pdf: &[u8], edits: &[(&str, &str)]) -> std::path::PathBuf {
    let src = t.file(&format!("{name}.pdf"), pdf);
    let objects = edits
        .iter()
        .map(|(old, new)| t.edit(&src, 0, 0, old, new))
        .collect();
    let saved = t
        .save(&[(&src, "1-z")], objects)
        .unwrap_or_else(|e| panic!("{name}: save failed: {e} {:?}", e.details));
    let changes: Vec<_> = edits
        .iter()
        .map(|(o, n)| (src.as_path(), 0, 0, *o, *n))
        .collect();
    t.assert_saved(&saved.path, &changes);
    saved.path
}

#[test]
fn e2e_01_corpus_text_tj_hi_becomes_hello() {
    let Some(t) = E2e::new("e2e_01") else {
        return;
    };
    save_one(&t, "text-tj", &corpus("text-tj.pdf"), &[("Hi", "Hello")]);
}

#[test]
fn e2e_02_kerned_line_and_a_follower_on_the_same_line_stay_in_place() {
    let Some(t) = E2e::new("e2e_02") else {
        return;
    };
    save_one(
        &t,
        "text-tj-kerned",
        &corpus("text-tj-kerned.pdf"),
        &[("Hi", "Hit")],
    );
    // B1: the TJ kern counts in the pen, so the tail and a follower run keep their places.
    let pdf = helvetica_page(
        b"BT /F1 10 Tf 72 700 Td [(AB) -500 (CD)] TJ (tail) Tj ET \
          BT /F1 10 Tf 300 700 Td (Follower) Tj ET",
    );
    let src = t.file("kerned-follower.pdf", &pdf);
    let before = word_box(&t, &src, 0, "Follower");
    let out = save_one(
        &t,
        "kerned-follower-2",
        &pdf,
        &[("AB CDtail", "AB CDtails")],
    );
    assert_same_box(before, word_box(&t, &out, 0, "Follower"), "E2E-02 follower");
}

#[test]
fn e2e_03_word_line_keeps_its_kerns_byte_for_byte() {
    let Some(t) = E2e::new("e2e_03") else {
        return;
    };
    let out = save_one(&t, "word", &fx::word(), &[("Invoice 2026", "Invoice 2027")]);
    let content = page_parts(&out, 0).concat();
    let text = String::from_utf8_lossy(&content);
    // "Inv" + the original kern bytes 12 + "oice…" (PLAN-04's kept prefix).
    assert!(
        text.contains("<496E76> 12 <"),
        "E2E-03 kern not kept: {text}"
    );
    assert!(!text.contains("2026"), "E2E-03 old literal left: {text}");
}

#[test]
fn e2e_04_word_turkish_line_across_sibling_fonts_and_a_missing_glyph() {
    let Some(t) = E2e::new("e2e_04") else {
        return;
    };
    save_one(
        &t,
        "word-tr",
        &fx::word_tr(),
        &[("Sağlık Bakanlığı Raporu", "Sağlığı Bakanlığı Raporu")],
    );
    let src = t.file("word-tr-y.pdf", &fx::word_tr());
    let obj = t.edit(
        &src,
        0,
        0,
        "Sağlık Bakanlığı Raporu",
        "Yağlık Bakanlığı Raporu",
    );
    let e = t
        .save(&[(&src, "1-z")], vec![obj])
        .err()
        .expect("Y is not in the subset");
    assert_eq!(e.code, "GLYPH_MISSING", "E2E-04: {e}");
    assert_eq!(e.message, "On page 1, the document's font can't draw: Y");
    assert!(e
        .suggestion
        .as_deref()
        .unwrap_or_default()
        .ends_with(ORIGINAL_UNCHANGED));
}

#[test]
fn e2e_05_every_producer_font_class_saves() {
    let Some(t) = E2e::new("e2e_05") else {
        return;
    };
    save_one(&t, "libre", &fx::libre(), &[("Libre text", "Libre exit")]);
    save_one(&t, "quartz", &fx::quartz(), &[("Quartz", "Quart")]);
    // Skia: the flipped line, the synthetic-bold Tr 2 line and the sheared line in one save.
    save_one(
        &t,
        "skia",
        &fx::skia(),
        &[("Chrome", "Comet"), ("Bold", "Bolder"), ("Italic", "Ital")],
    );
    // pdfTeX: the new space is written as a kern (the subset has no space glyph).
    save_one(
        &t,
        "pdftex",
        &fx::pdftex(),
        &[("Hello World", "Hello Wet World")],
    );
    save_one(&t, "xetex", &fx::xetex(), &[("XeTeX", "TeX")]);
    save_one(
        &t,
        "indd",
        &fx::indd(),
        &[("visible layer", "visible layers")],
    );
    save_one(&t, "std14", &fx::std14(), &[("Times line", "Times lines")]);
    save_one(
        &t,
        "nonemb",
        &fx::nonemb(),
        &[("Not embedded", "Not embed")],
    );
    save_one(
        &t,
        "libre-cff",
        &fx::libre_cff(),
        &[("Hello Hello", "Hello Hole")],
    );
    // InDesign's hidden layer is refused at inspect and at Save.
    let src = t.file("indd-hidden.pdf", &fx::indd());
    let (fp, run) = t.run(&src, 0, "hidden layer");
    assert!(!run.editable);
    assert_eq!(run.reason, Some(TextReason::OptionalContent));
    let obj = json!({
        "id": "hidden", "kind": "sourceText", "pageIndex": 0,
        "rect": { "x": run.rect.x, "y": run.rect.y, "w": run.rect.w, "h": run.rect.h },
        "locked": true, "runId": run.id, "sourceFingerprint": fp, "sourcePageIndex": 0,
        "originalText": run.text, "text": "hidden layers", "style": {},
    });
    let e = t.save(&[(&src, "1-z")], vec![obj]).err().expect("refused");
    assert_eq!(e.code, "TEXT_EDIT_REFUSED", "{e}");
}

#[test]
fn e2e_06_rotated_and_cropped_pages() {
    let Some(t) = E2e::new("e2e_06") else {
        return;
    };
    for angle in [90, 180, 270] {
        save_one(
            &t,
            &format!("rotate-{angle}"),
            &fx::rotated(angle, true),
            &[("Rotated page", "Rotated pages")],
        );
    }
    save_one(
        &t,
        "crop",
        &fx::cropped_offset(),
        &[("Cropped page", "Cropped pages")],
    );
}
