//! CLS-H01…H14 (SPEC §C rows 1–15): the #33 maintainer items as regressions — bounded read,
//! object loading and decoding, no raw fallback, per-page budgets, the `q` stack, one snapshot,
//! custom-encoded fonts, rise and render modes, spacing in bounds.

use super::*;
use crate::pdf_engine::source_content::{classify_source_page, SourcePageResult};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::{fnv1a_u64, read_snapshot, snapshot_from_bytes};
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_simple, latin_truetype, Program, SimpleFont,
};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, cid_font, cid_hex, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use std::io::Write;

fn write(scratch: &Scratch, name: &str, bytes: &[u8]) -> PathBuf {
    let p = scratch.file(name);
    fs::write(&p, bytes).unwrap();
    p
}

fn page_result(bytes: Vec<u8>, page: u32) -> SourcePageResult {
    let ctx = SnapshotContext::new(
        snapshot_from_bytes(Path::new("h.pdf"), bytes, None).expect("fixture opens"),
    );
    classify_source_page(
        &ctx,
        &build_page_model(&ctx, page, None).expect("model"),
        None,
    )
}

fn text_reasons(r: &SourcePageResult) -> Vec<Option<TextReason>> {
    r.occurrences
        .iter()
        .filter(|o| kind_token(o) == "text")
        .map(|o| o.reason)
        .collect()
}

fn classify_bytes(
    scratch: &Scratch,
    name: &str,
    bytes: &[u8],
) -> Result<Vec<SourceOccurrence>, AppError> {
    classify_source_content(&write(scratch, name, bytes))
}

#[test]
fn cls_h01_hardening_file_read_once_and_capped() {
    let scratch = Scratch::new("h01");
    let huge = scratch.file("huge.pdf");
    File::create(&huge)
        .unwrap()
        .set_len(FILE_CAP_BYTES + 1)
        .unwrap();
    expect_err_code(
        classify_source_content(&huge),
        "FILE_TOO_LARGE",
        "CLS-H01 sparse 400 MiB + 1",
    );
    // The per-page API works on the bytes of the one read: the path is never opened again.
    let bytes = fx::word();
    let ghost = scratch.file("never-written.pdf");
    let ctx = SnapshotContext::new(snapshot_from_bytes(&ghost, bytes.clone(), None).unwrap());
    let result = classify_source_page(&ctx, &build_page_model(&ctx, 0, None).unwrap(), None);
    assert!(
        !ghost.exists(),
        "CLS-H01 nothing read from or written to the path"
    );
    let on_disk = write(&scratch, "word.pdf", &bytes);
    assert_eq!(
        classify_source_content(&on_disk).unwrap(),
        result.occurrences,
        "CLS-H01 the same occurrences from the one read"
    );
}

#[test]
fn cls_h02_hardening_objstm_bomb_file_too_complex() {
    let s = Scratch::new("h02");
    expect_err_code(
        classify_bytes(&s, "b.pdf", &fx::objstm_bomb()),
        "FILE_TOO_COMPLEX",
        "CLS-H02",
    );
}

#[test]
fn cls_h03_hardening_xref_stream_bomb_file_too_complex() {
    let s = Scratch::new("h03");
    expect_err_code(
        classify_bytes(&s, "b.pdf", &fx::xref_bomb()),
        "FILE_TOO_COMPLEX",
        "CLS-H03",
    );
}

#[test]
fn cls_h04_hardening_deep_nesting_file_too_complex() {
    let s = Scratch::new("h04");
    expect_err_code(
        classify_bytes(&s, "n.pdf", &fx::deep_nesting(101)),
        "FILE_TOO_COMPLEX",
        "CLS-H04",
    );
    assert!(
        classify_bytes(&s, "ok.pdf", &fx::deep_nesting(100)).is_ok(),
        "CLS-H04 100 levels"
    );
}

#[test]
fn cls_h05_hardening_flate_bomb_page_too_complex() {
    let s = Scratch::new("h05");
    expect_err_code(
        classify_bytes(&s, "b.pdf", &fx::flate_bomb()),
        "PAGE_TOO_COMPLEX",
        "CLS-H05",
    );
    let r = page_result(fx::flate_bomb(), 0);
    assert_eq!(
        (r.page_reason, r.occurrences.len()),
        (Some(TextReason::PageTooComplex), 0)
    );
    let other = page_result(fx::flate_bomb(), 1);
    assert_eq!(
        other.page_reason, None,
        "CLS-H05 the next page is unaffected"
    );
}

#[test]
fn cls_h06_hardening_tounicode_bomb_refuses_font() {
    let mut d = DocBuilder::new();
    let font = cid_font(&mut d.b, "ABCDEF+Bomb", "AB");
    let bomb = d.b.add_stream(
        "/Filter /FlateDecode",
        &crate::pdf_engine::text_edit::testkit::pdf::zlib_zero_bomb(8),
    );
    let body = String::from_utf8(d.b.body(font).unwrap().to_vec()).unwrap();
    let patched = body
        .replacen("/ToUnicode", "/Bomb", 1)
        .trim_end_matches(">>")
        .to_string()
        + &format!(" /ToUnicode {bomb} 0 R >>");
    d.b.set(font, patched);
    d.page(PageSpec::new(
        format!("BT /F1 12 Tf 72 700 Td <{}> Tj ET", cid_hex("AB", "AB")).as_bytes(),
        &format!("/Font << /F1 {font} 0 R >>"),
    ));
    let started = std::time::Instant::now();
    let r = page_result(d.build(), 0);
    assert_eq!(r.page_reason, None, "CLS-H06 the page itself is fine");
    assert_eq!(
        text_reasons(&r),
        [Some(TextReason::AmbiguousUnicode)],
        "CLS-H06 the font is refused"
    );
    assert!(
        started.elapsed().as_secs() < 10,
        "CLS-H06 capped while inflating"
    );
}

/// One Helvetica page whose content stream is `data` with stream dictionary `dict`.
fn filtered_page(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let id = d.b.add_stream(dict, data);
    d.page_raw(
        &format!("{id} 0 R"),
        &PageSpec::new(b"", &format!("/Font << /F1 {f} 0 R >>")),
    );
    d.build()
}

#[test]
fn cls_h07_hardening_no_raw_fallback_on_filter_errors() {
    let plain: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET BT /F1 12 Tf 72 700 Td (Lo) Tj ET";
    let s = Scratch::new("h07");
    // (1) /FlateDecode over plain bytes.
    let r = classify_bytes(
        &s,
        "plain.pdf",
        &filtered_page("/Filter /FlateDecode", plain),
    );
    expect_err_code(r, "MALFORMED_CONTENT", "CLS-H07 Flate over plain bytes");
    // (2) /ASCIIHexDecode is decoded, not passed through raw.
    let hex: String = plain.iter().map(|b| format!("{b:02X}")).collect::<String>() + ">";
    let hits = classify(
        &write(
            &s,
            "ahx.pdf",
            &filtered_page("/Filter /ASCIIHexDecode", hex.as_bytes()),
        ),
        "CLS-H07",
    );
    assert_eq!(hits.len(), 2, "CLS-H07 ASCIIHex decoded: {hits:?}");
    assert!(hits.iter().all(|o| capability_token(o) == "supported"));
    assert!((hits[0].rect.x - 72.0).abs() < 1e-6);
    // (3) 55 % of a Flate stream: never Ok with fewer rows.
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    enc.write_all(plain).unwrap();
    let full = enc.finish().unwrap();
    let cut = full[..full.len() * 55 / 100].to_vec();
    let truncated = filtered_page("/Filter /FlateDecode", &cut);
    expect_err_code(
        classify_bytes(&s, "cut.pdf", &truncated),
        "MALFORMED_CONTENT",
        "CLS-H07 truncated",
    );
    let r = page_result(truncated, 0);
    assert_eq!(r.page_reason, Some(TextReason::MalformedContent));
    assert!(
        r.occurrences.is_empty() && r.runs.is_empty(),
        "CLS-H07 zero rows, not a partial list"
    );
}

#[test]
fn cls_h08_hardening_six_pages_of_1000_tj_ok() {
    let mut page = String::from("BT /F1 1 Tf 20 700 Td ");
    for _ in 0..1000 {
        page.push_str("(a) Tj ");
    }
    page.push_str("ET");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    for _ in 0..6 {
        d.page(PageSpec::new(
            page.as_bytes(),
            &format!("/Font << /F1 {f} 0 R >>"),
        ));
    }
    let s = Scratch::new("h08");
    let hits = classify(&write(&s, "six.pdf", &d.build()), "CLS-H08");
    assert_eq!(hits.len(), 6_000, "CLS-H08 budgets are per page");
    assert!(hits.iter().all(|o| capability_token(o) == "supported"));
}

#[test]
fn cls_h09_hardening_page_over_op_budget_isolated() {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::new(b"BT /F1 12 Tf 72 700 Td (Fine) Tj ET", &res));
    let heavy = "q Q ".repeat(125_001);
    d.page(PageSpec::new(heavy.as_bytes(), &res));
    let bytes = d.build();
    assert_eq!(
        page_result(bytes.clone(), 0).page_reason,
        None,
        "CLS-H09 page 1 fine"
    );
    assert_eq!(
        page_result(bytes.clone(), 1).page_reason,
        Some(TextReason::PageTooComplex),
        "CLS-H09 page 2 over PAGE_OPS_MAX"
    );
    let s = Scratch::new("h09");
    expect_err_code(
        classify_bytes(&s, "ops.pdf", &bytes),
        "PAGE_TOO_COMPLEX",
        "CLS-H09 document API",
    );
}

#[test]
fn cls_h10_hardening_q_overflow_refused_never_a_wrong_x() {
    let probe = |depth: usize| {
        format!(
            "{}1 0 0 1 100 0 cm q 1 0 0 1 0 0 cm Q BT /F1 12 Tf 72 720 Td (Hi) Tj ET",
            "q ".repeat(depth)
        )
    };
    let r = page_result(helvetica_page(probe(64).as_bytes()), 0);
    assert_eq!(
        r.page_reason,
        Some(TextReason::MalformedContent),
        "CLS-H10 65th q"
    );
    assert!(r.occurrences.is_empty());
    let r = page_result(helvetica_page(probe(63).as_bytes()), 0);
    assert_eq!(r.page_reason, None);
    assert!(
        (r.occurrences[0].rect.x - 172.0).abs() < 1e-6,
        "CLS-H10 x = 172 within the limit"
    );
}

#[test]
fn cls_h11_hardening_fingerprint_matches_parsed_bytes() {
    let s = Scratch::new("h11");
    let bytes = fx::word();
    let p = write(&s, "w.pdf", &bytes);
    let snap = read_snapshot(&p).unwrap();
    assert_eq!(
        (snap.fingerprint.len, snap.fingerprint.fnv),
        (bytes.len() as u64, fnv1a_u64(&bytes))
    );
    assert_eq!(
        snap.bytes.as_slice(),
        &bytes[..],
        "CLS-H11 parsed from the hashed bytes"
    );
    let hits = classify(&p, "CLS-H11");
    assert!(hits
        .iter()
        .all(|o| o.locator.starts_with(&format!("v2:{}:", snap.fingerprint))));
    assert_eq!(
        resolve_source_locator(&p, &hits[0].locator).unwrap(),
        hits[0]
    );
}

/// The [probe] font: TrueType `ABCDEF+Calibri`, `/Differences [1 /H /i]`, `(\001\002) Tj`.
fn calibri(widths: bool, program: bool) -> Vec<u8> {
    let mut f = SimpleFont::new("TrueType", "ABCDEF+Calibri");
    f.encoding = Some("<< /Type /Encoding /Differences [1 /H /i] >>".into());
    if widths {
        f.first_char = 1;
        f.widths = Some(vec![600.0, 250.0]);
    }
    if program {
        f.flags = Some(32);
        f.program = Program::TrueType(latin_truetype("H", "i").0);
    }
    let mut d = DocBuilder::new();
    let id = add_simple(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 720 Td (\\001\\002) Tj ET",
        &format!("/Font << /F1 {id} 0 R >>"),
    ));
    d.build()
}

#[test]
fn cls_h12_hardening_subset_and_custom_encoded_fonts() {
    let no_widths = page_result(calibri(false, false), 0);
    assert_eq!(
        text_reasons(&no_widths),
        [Some(TextReason::MissingWidths)],
        "CLS-H12 no /Widths"
    );
    for (program, alphabet) in [(false, vec!['H', 'i']), (true, vec!['H'])] {
        let ctx = SnapshotContext::new(
            snapshot_from_bytes(Path::new("c.pdf"), calibri(true, program), None).unwrap(),
        );
        let model = build_page_model(&ctx, 0, None).unwrap();
        let run = model.runs.first().expect("CLS-H12 run");
        assert_eq!(
            (run.text.as_str(), run.reason),
            ("Hi", None),
            "CLS-H12 program={program}"
        );
        assert_eq!(run.substituted, !program, "CLS-H12 substituted");
        assert_eq!(
            model.surface(run).alphabet(),
            alphabet,
            "CLS-H12 alphabet, program={program}"
        );
        assert!(
            model.surface(run).writer_for('i').is_none() == program,
            "CLS-H12 typing i"
        );
    }
    let libre = page_result(fx::libre(), 0);
    assert!(
        text_reasons(&libre).iter().all(Option::is_none),
        "CLS-H12 LibreOffice symbolic TrueType"
    );
}

#[test]
fn cls_h12b_no_tounicode_on_an_embedded_type0_font() {
    use crate::pdf_engine::text_edit::testkit::fonts::{add_type0, Type0Font};
    use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
    let mut t = TtfBuilder::new();
    t.glyph("A", true, 500);
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+NoMap");
    f.program = Program::TrueType(t.build());
    let mut d = DocBuilder::new();
    let id = add_type0(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td <0001> Tj ET",
        &format!("/Font << /F1 {id} 0 R >>"),
    ));
    assert_eq!(
        text_reasons(&page_result(d.build(), 0)),
        [Some(TextReason::NoTounicode)]
    );
}

#[test]
fn cls_h13_hardening_rise_and_render_mode() {
    let reasons = |content: &[u8]| text_reasons(&page_result(helvetica_page(content), 0));
    assert_eq!(
        reasons(b"BT /F1 12 Tf 3 Tr 72 720 Td (Hi) Tj ET"),
        [Some(TextReason::InvisibleText)],
        "CLS-H13 3 Tr"
    );
    assert_eq!(
        reasons(b"BT /F1 12 Tf 7 Tr 72 720 Td (Hi) Tj ET"),
        [Some(TextReason::TextClipMode)],
        "CLS-H13 7 Tr"
    );
    let risen = page_result(
        helvetica_page(b"BT /F1 12 Tf 30 Ts 72 720 Td (Hi) Tj ET"),
        0,
    );
    assert!(
        (risen.occurrences[0].rect.y - 750.0).abs() < 1e-6,
        "CLS-H13 rect y includes the rise"
    );
}

#[test]
fn cls_h14_hardening_spacing_in_bounds() {
    let w = |content: &[u8]| {
        page_result(helvetica_page(content), 0).occurrences[0]
            .rect
            .w
    };
    let plain = w(b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET");
    let tracked = w(b"BT /F1 12 Tf 10 Tc 72 720 Td (Hi) Tj ET");
    assert!(
        (tracked - plain - 20.0).abs() < 1e-6,
        "CLS-H14 Tc per glyph: {plain} → {tracked}"
    );
    let spaced = w(b"BT /F1 12 Tf 20 Tw 72 720 Td (a b) Tj ET");
    let unspaced = w(b"BT /F1 12 Tf 72 720 Td (a b) Tj ET");
    assert!(
        (spaced - unspaced - 20.0).abs() < 1e-6,
        "CLS-H14 Tw in the width"
    );
    let mut d = DocBuilder::new();
    let f = cid_font(&mut d.b, "ABCDEF+Arimo", "AB");
    d.page(PageSpec::new(
        format!(
            "BT /F1 10 Tf 5 Tc 72 700 Td <{}> Tj ET",
            cid_hex("AB", "AB")
        )
        .as_bytes(),
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let cid = page_result(d.build(), 0).occurrences[0].rect.w;
    assert!(
        (cid - (10.0 + 10.0)).abs() < 1e-6,
        "CLS-H14 CID Tc once per 2-byte code: {cid}"
    );
}
