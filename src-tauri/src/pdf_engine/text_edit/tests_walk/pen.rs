//! Review regressions (T3 fix pass 2, MEDIUM-1): text drawn, in the same text object and with no
//! line move, after a font whose advances the model cannot prove to be the viewers' is refused
//! `MISSING_WIDTHS` — a vertical font, a predefined CMap, a model refused before its widths were
//! read, a Type3 font whose `/FontMatrix` is not diagonal — and becomes editable again after a
//! line move. Fonts whose widths are proven (an Identity-H CID font, embedded or not, a diagonal
//! Type3) keep the text after them editable. The pen positions in the comments were measured
//! with poppler 26.04 `pdftotext -bbox` on the same pages.
//!
//! Fix pass 3 (review T3 r2): a Type3 font with a `/FontDescriptor /MissingWidth` (MEDIUM-1), an
//! Identity-H font refused `FONT_UNSUPPORTED` for its `/W` (LOW-1), and `q`/`Q` inside a text
//! object, where poppler and pdf.js disagree on the text position after `Q` (MEDIUM-3; positions
//! from `review-t3-r2/viewers.log`, poppler 26.04 and pdf.js 4.10.38).

use super::{model0, reason_of, run_with};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::testkit::producers::{
    cid_font_with, cid_hex, DocBuilder, PageSpec, HELVETICA,
};

/// One text object: `first` (shown with `/FX`), then `between`, then Helvetica "Hello".
/// `font` builds `/FX` into the document and returns its object number.
fn after(font: impl FnOnce(&mut DocBuilder) -> u32, first: &str, between: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let fx = font(&mut d);
    let f1 = d.add(HELVETICA);
    let content = format!("BT /FX 20 Tf 100 600 Td {first} {between} /F1 20 Tf (Hello) Tj ET");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /FX {fx} 0 R /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// "Hello" right after `first` is `expected`; after `0 -30 Td` it is editable.
fn assert_hello(font: impl Fn(&mut DocBuilder) -> u32, first: &str, expected: Option<R>) {
    let m = model0(after(&font, first, ""));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "Hello"), expected, "{first} then Hello");
    let m = model0(after(&font, first, "0 -30 Td"));
    assert_eq!(
        reason_of(&m, "Hello"),
        None,
        "{first}, a line move, then Hello"
    );
}

/// A non-embedded CIDFontType0 Type0 font with `/Encoding encoding` (Adobe-Japan1 CIDs).
fn kozmin(d: &mut DocBuilder, encoding: &str) -> u32 {
    kozmin_w(d, encoding, "[34 [612] 65 [300]]")
}

/// `kozmin` with the descendant's `/W` array `w`.
fn kozmin_w(d: &mut DocBuilder, encoding: &str, w: &str) -> u32 {
    let desc = d.add(
        "<< /Type /FontDescriptor /FontName /KozMinPr6N-Regular /Flags 6 \
         /FontBBox [-437 -340 1147 1317] /ItalicAngle 0 /Ascent 1317 /Descent -349 \
         /CapHeight 742 /StemV 80 >>",
    );
    let cid = d.add(format!(
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMinPr6N-Regular \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> \
         /FontDescriptor {desc} 0 R /DW 1000 /W {w} >>"
    ));
    d.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /KozMinPr6N-Regular /Encoding {encoding} \
         /DescendantFonts [{cid} 0 R] >>"
    ))
}

/// A Type3 font drawing code 97 ("a") with `/FontMatrix matrix` and `widths`.
fn type3(d: &mut DocBuilder, matrix: &str, widths: &str) -> u32 {
    let glyph =
        d.b.add_stream("", b"1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f");
    d.add(format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix {matrix} \
         /CharProcs << /a {glyph} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> \
         {widths} >>"
    ))
}

#[test]
fn text_after_a_vertical_font_is_refused_until_the_line_moves() {
    // Viewers advance an Identity-V string down by w1 = −1000: poppler puts "Hello" at
    // (100, 560), the model at (120, 600).
    let vertical =
        |d: &mut DocBuilder| cid_font_with(&mut d.b, "ABCDEF+Vertical", "AB", "/Identity-V");
    let shown = format!("<{}> Tj", cid_hex("AB", "AB"));
    assert_hello(vertical, &shown, Some(R::MissingWidths));
}

#[test]
fn text_after_a_predefined_cmap_is_refused_until_the_line_moves() {
    // UniJIS-UCS2-H maps U+0041 to CID 34 (612 wide); the model reads the code as the CID
    // (/W 300): poppler puts "Hello" at x 124.48, the model at 112. The font's own refusal is
    // FONT_NOT_EMBEDDED, which outranks (and hides) UNSUPPORTED_ENCODING.
    let unijis = |d: &mut DocBuilder| kozmin(d, "/UniJIS-UCS2-H");
    assert_hello(unijis, "<00410041> Tj", Some(R::MissingWidths));
}

#[test]
fn text_after_a_font_refused_before_its_widths_is_refused_until_the_line_moves() {
    // `/Widths` one entry short: FONT_UNSUPPORTED and a model with no codes (every advance 0);
    // poppler uses the array: "Hello" at x 148, the model at 100.
    let widths: Vec<&str> = vec!["600"; 94];
    let short = |d: &mut DocBuilder| {
        d.add(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding \
             /FirstChar 32 /LastChar 126 /Widths [{}] >>",
            widths.join(" ")
        ))
    };
    assert_hello(short, "(WIDE) Tj", Some(R::MissingWidths));
    // A non-embedded Symbol font has no codes in the model either (UNSUPPORTED_ENCODING).
    let symbol = |d: &mut DocBuilder| d.add("<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>");
    assert_hello(symbol, "(abc) Tj", Some(R::MissingWidths));
}

#[test]
fn text_after_a_type3_font_is_refused_unless_it_advances_along_the_baseline() {
    let widths = "/FirstChar 97 /LastChar 97 /Widths [1000]";
    // a = 0: poppler advances by w × a = 0 ("Hello" at x 100), the model by w × 0.001 (x 140).
    let rotated = |d: &mut DocBuilder| type3(d, "[0 0.001 -0.001 0 0 0]", widths);
    assert_hello(rotated, "(aa) Tj", Some(R::MissingWidths));
    // b ≠ 0: the displacement leaves the baseline.
    let sheared = |d: &mut DocBuilder| type3(d, "[0.001 0.0005 0 0.001 0 0]", widths);
    assert_hello(sheared, "(aa) Tj", Some(R::MissingWidths));
    // A malformed /Widths (two values for one code) leaves the model without widths.
    let bad = |d: &mut DocBuilder| {
        type3(
            d,
            "[0.001 0 0 0.001 0 0]",
            "/FirstChar 97 /LastChar 97 /Widths [1000 1000]",
        )
    };
    assert_hello(bad, "(aa) Tj", Some(R::MissingWidths));
    // A diagonal matrix: the model advances as viewers do, so the text after it stays editable.
    let diagonal = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", widths);
    assert_hello(diagonal, "(aa) Tj", None);
}

#[test]
fn text_after_a_proven_cid_font_stays_editable() {
    // Identity-H, embedded: the model and poppler agree on x 120.
    let embedded =
        |d: &mut DocBuilder| cid_font_with(&mut d.b, "ABCDEF+Horizontal", "AB", "/Identity-H");
    let shown = format!("<{}> Tj", cid_hex("AB", "AB"));
    assert_hello(embedded, &shown, None);
    // Identity-H, not embedded (FONT_NOT_EMBEDDED): `/W` still gives every advance.
    let not_embedded = |d: &mut DocBuilder| kozmin(d, "/Identity-H");
    assert_hello(not_embedded, "<00220041> Tj", None);
    let m = model0(after(not_embedded, "<00220041> Tj", ""));
    let hello = super::run_with(&m, "Hello");
    // CIDs 34 and 65 are 612 and 300 wide: 100 + (0.612 + 0.3) × 20.
    assert!(
        super::close(hello.origin.0, 118.24),
        "Hello at {:?}",
        hello.origin
    );
}

#[test]
fn text_after_a_type3_font_with_a_missing_width_is_refused_until_the_line_moves() {
    let widths = "/FirstChar 97 /LastChar 97 /Widths [1000]";
    let descriptor = |missing: &str| {
        format!(
            "{widths} /FontDescriptor << /Type /FontDescriptor /FontName /T3 /Flags 4 \
             /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 1000 /Descent 0 /CapHeight 1000 \
             /StemV 80 {missing} >>"
        )
    };
    // Review P18: code 98 ("b") is outside /FirstChar../LastChar. The model gives it width 0,
    // poppler and pdf.js the descriptor's /MissingWidth: "Hello" at x 140 for both viewers,
    // x 120 in the model.
    let missing = descriptor("/MissingWidth 1000");
    let with_missing = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", &missing);
    assert_hello(with_missing, "(ab) Tj", Some(R::MissingWidths));
    // An unreadable /MissingWidth is not proven either.
    let odd = descriptor("/MissingWidth /Wide");
    let with_odd = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", &odd);
    assert_hello(with_odd, "(ab) Tj", Some(R::MissingWidths));
    // Review P18b: no descriptor, or a /MissingWidth of 0: all three agree on x 120.
    let none = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", widths);
    assert_hello(none, "(ab) Tj", None);
    let zero = descriptor("/MissingWidth 0");
    let with_zero = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", &zero);
    assert_hello(with_zero, "(ab) Tj", None);
    let absent = descriptor("");
    let without = |d: &mut DocBuilder| type3(d, "[0.001 0 0 0.001 0 0]", &absent);
    assert_hello(without, "(ab) Tj", None);
}

#[test]
fn text_after_an_identity_h_font_with_a_bad_w_is_refused_until_the_line_moves() {
    // A /W entry that is not a number: T2 refuses the font FONT_UNSUPPORTED before it has its
    // widths, so the pen after its glyphs is not known even under Identity-H.
    let bad_w = |d: &mut DocBuilder| kozmin_w(d, "/Identity-H", "[34 [612] 65 [(x)]]");
    assert_hello(bad_w, "<00220041> Tj", Some(R::MissingWidths));
    let m = model0(after(bad_w, "<00220041> Tj", ""));
    let fx = m
        .walk
        .page_fonts
        .iter()
        .find(|(name, _)| name == b"FX")
        .map(|(_, f)| f.refusal);
    assert_eq!(fx, Some(Some(R::FontUnsupported)));
}

/// Helvetica 20 as `/F1`, drawing `content`.
fn helvetica20(content: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = d.add(HELVETICA);
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

#[test]
fn text_after_a_q_inside_a_text_object_is_refused_until_tm() {
    // Review P22 row 1: `Td` between q and Q. The model and poppler put B at (100, 550), pdf.js
    // at (113.34, 600).
    let m = model0(helvetica20(
        "BT /F1 20 Tf 100 600 Td (A) Tj q 0 -50 Td Q (B) Tj ET",
    ));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "A"), None);
    assert_eq!(reason_of(&m, "B"), Some(R::MissingWidths));
    // Row 2: text shown between q and Q. pdf.js draws "Black" over "Red" (x 100), the model and
    // poppler after it (x 136.68).
    let m = model0(helvetica20(
        "BT /F1 20 Tf 100 600 Td q 1 0 0 rg (Red) Tj Q (Black) Tj ET",
    ));
    assert_eq!(reason_of(&m, "Red"), None);
    assert_eq!(reason_of(&m, "Black"), Some(R::MissingWidths));
    // Row 3: `Tm` between q and Q; a `Td` after Q does not settle it (it is relative to the
    // disputed line start: C at (300, 270) in the model, (100, 570) in both viewers), the next
    // `Tm` does.
    let m = model0(helvetica20(
        "BT /F1 20 Tf 1 0 0 1 100 600 Tm q 1 0 0 1 300 300 Tm Q (B) Tj 0 -30 Td (C) Tj \
         1 0 0 1 100 400 Tm (D) Tj ET",
    ));
    assert_eq!(reason_of(&m, "B"), Some(R::MissingWidths));
    assert_eq!(reason_of(&m, "C"), Some(R::MissingWidths));
    assert_eq!(reason_of(&m, "D"), None);
    // A new text object settles it too.
    let m = model0(helvetica20(
        "BT /F1 20 Tf 100 600 Td q 0 -50 Td Q (B) Tj ET BT 100 500 Td (C) Tj ET",
    ));
    assert_eq!(reason_of(&m, "B"), Some(R::MissingWidths));
    assert_eq!(reason_of(&m, "C"), None);
    // A Q inside BT restoring a state saved before BT (under the identity text matrix).
    let m = model0(helvetica20(
        "BT /F1 20 Tf ET q BT 100 600 Td (A) Tj Q (B) Tj ET",
    ));
    assert_eq!(reason_of(&m, "A"), None);
    assert_eq!(reason_of(&m, "B"), Some(R::MissingWidths));
}

#[test]
fn a_q_inside_a_text_object_that_moves_nothing_keeps_the_text_editable() {
    // Only the colour changes between q and Q: every viewer keeps the text position, and the
    // restored state is the saved one, so A and B join into one editable run.
    let m = model0(helvetica20(
        "BT /F1 20 Tf 100 600 Td (A) Tj q 1 0 0 rg Q (B) Tj ET",
    ));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let ab = run_with(&m, "AB");
    assert_eq!(ab.reason, None);
    assert_eq!(ab.members.len(), 2);
    // q/Q outside text objects never touch the text position.
    let m = model0(helvetica20(
        "q BT /F1 20 Tf 100 600 Td (A) Tj ET Q BT /F1 20 Tf 100 500 Td (B) Tj ET",
    ));
    assert_eq!(reason_of(&m, "A"), None);
    assert_eq!(reason_of(&m, "B"), None);
}

#[test]
fn a_q_inside_a_text_object_restoring_another_tm_base_is_refused_until_tm() {
    // Review r3 LOW-1 (`qdecomp1`): the `q` was made after a text object whose `Tm` set the base,
    // and the `Q` comes inside a text object positioned by `Td` from the identity. The composite
    // matrices agree, but poppler's restored text matrix with its kept line start puts C at
    // (200, 1170); the model and pdf.js at (100, 570). B and C are refused until a `Tm`.
    let m = model0(helvetica20(
        "BT /F1 20 Tf 1 0 0 1 100 600 Tm (A) Tj ET q BT 100 600 Td (A) Tj Q (B) Tj \
         0 -30 Td (C) Tj 1 0 0 1 100 400 Tm (D) Tj ET",
    ));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "B"), Some(R::MissingWidths));
    assert_eq!(reason_of(&m, "C"), Some(R::MissingWidths));
    assert_eq!(reason_of(&m, "D"), None);
    // The two "A"s (B no longer joins the second) are drawn on each other: DUPLICATE_TEXT.
    let a: Vec<_> = m.runs.iter().filter(|r| r.text == "A").collect();
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|r| r.reason == Some(R::DuplicateText)));
    // Control: both text objects start from the identity, so every viewer agrees and A and B
    // join into one editable run.
    let m = model0(helvetica20(
        "BT /F1 20 Tf 100 600 Td (A) Tj ET q BT 100 600 Td (A) Tj Q (B) Tj ET",
    ));
    let ab = run_with(&m, "AB");
    assert_eq!(ab.reason, None);
    assert_eq!(ab.members.len(), 2);
}
