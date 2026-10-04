//! CLS-H15…H27 (SPEC §C rows 11–19 and F1): transforms, clip extents, shared content, per-face
//! widths, CID widths, v2 locators, the JSON shape, the frozen vocabulary, the reasons #33 never
//! tested (VERTICAL, CLIPPED, occurrence GEOMETRY, ENCRYPTED) and tagged/bookmarked pages.

use super::*;
use crate::pdf_engine::source_content::{classify_source_page, SourcePageResult};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::engines_or_skip;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, ClipKind, DocBuilder, PageSpec, HELVETICA,
};

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

fn first_reason(bytes: Vec<u8>, kind: &str) -> Option<TextReason> {
    page_result(bytes, 0)
        .occurrences
        .into_iter()
        .find(|o| kind_token(o) == kind)
        .unwrap_or_else(|| panic!("a {kind} occurrence"))
        .reason
}

fn image_page(cm: &str, extra_content: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let img = d.b.add_stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[128],
    );
    d.page(PageSpec::new(
        format!("{extra_content} q {cm} cm /Im0 Do Q").as_bytes(),
        &format!("/XObject << /Im0 {img} 0 R >>"),
    ));
    d.build()
}

#[test]
fn cls_h15_hardening_unsupported_transforms() {
    use TextReason as R;
    assert_eq!(
        first_reason(fx::mirrored(), "text"),
        Some(R::MirroredText),
        "CLS-H15 -1 0 0 1 Tm"
    );
    assert_eq!(
        first_reason(fx::negative_tz(), "text"),
        Some(R::MirroredText),
        "CLS-H15 -100 Tz"
    );
    assert_eq!(
        first_reason(fx::negative_tf(), "text"),
        Some(R::RotatedText),
        "CLS-H15 -12 Tf"
    );
    assert_eq!(
        first_reason(image_page("0 40 -40 0 200 400", ""), "image"),
        Some(R::TransformedImage),
        "CLS-H15 rotated image"
    );
    assert_eq!(
        first_reason(image_page("40 0 0 -40 72 400", ""), "image"),
        Some(R::TransformedImage),
        "CLS-H15 flipped image"
    );
    assert_eq!(
        first_reason(image_page("40 0 0 40 72 400", ""), "image"),
        None,
        "CLS-H15 upright image"
    );
}

#[test]
fn cls_h16_hardening_clip_extent() {
    assert_eq!(
        first_reason(fx::clip(ClipKind::Page), "text"),
        None,
        "CLS-H16 page-sized re W n"
    );
    assert_eq!(
        first_reason(fx::clip(ClipKind::Small), "text"),
        Some(TextReason::Clipped),
        "CLS-H16 small"
    );
    assert_eq!(
        first_reason(fx::clip(ClipKind::Curve), "text"),
        Some(TextReason::Clipped),
        "CLS-H16 Bézier"
    );
}

#[test]
fn cls_h17_hardening_shared_content_on_both_pages() {
    let s = Scratch::new("h17");
    let p = s.file("shared.pdf");
    fs::write(&p, fx::shared()).unwrap();
    let hits = classify(&p, "CLS-H17");
    let shared_rows: Vec<&SourceOccurrence> = hits
        .iter()
        .filter(|o| o.text.as_deref() == Some("Shared body"))
        .collect();
    assert_eq!(shared_rows.len(), 2, "CLS-H17 one row per page");
    for o in shared_rows {
        assert_unsupported(o, "text", "SHARED_CONTENT", "CLS-H17");
    }
}

#[test]
fn cls_h18_hardening_standard14_widths_per_face() {
    let mut d = DocBuilder::new();
    let f = d
        .add("<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let w = page_result(d.build(), 0).occurrences[0].rect.w;
    // Times-Roman AFM: H = 722, i = 278 → 12.0 at 12 pt (Helvetica would give 11.328).
    assert!((w - 12.0).abs() < 1e-6, "CLS-H18 Times widths, got {w}");
    let helv = page_result(helvetica_page(b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET"), 0).occurrences[0]
        .rect
        .w;
    assert!((helv - 11.328).abs() < 1e-6, "CLS-H18 Helvetica {helv}");
}

#[test]
fn cls_h19_hardening_cid_w_and_tc_per_code() {
    use crate::pdf_engine::text_edit::testkit::fonts::{
        add_type0, tounicode_bfchar, Program, Type0Font,
    };
    use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
    let mut t = TtfBuilder::new();
    for ch in ['A', 'B', 'C'] {
        t.unicode_glyph(ch, &ch.to_string(), true);
    }
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Widths");
    f.program = Program::TrueType(t.build());
    f.w = Some("[1 [600] 2 3 800]".into());
    f.tounicode = Some(tounicode_bfchar(&[(1, 2, "A"), (2, 2, "B"), (3, 2, "C")]));
    let mut d = DocBuilder::new();
    let id = add_type0(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 10 Tf 1 Tc 72 700 Td <000100020003> Tj ET",
        &format!("/Font << /F1 {id} 0 R >>"),
    ));
    let w = page_result(d.build(), 0).occurrences[0].rect.w;
    // /W both forms: 600 + 800 + 800 → 22 pt, + 3 × 1 Tc (per code, not per byte).
    assert!((w - 25.0).abs() < 1e-6, "CLS-H19 got {w}");
}

#[test]
fn cls_h20_hardening_locator_second_contents_stream_points_into_part_1() {
    let s = Scratch::new("h20");
    let p = s.file("parts.pdf");
    fs::write(&p, fx::two_parts_mid_bt()).unwrap();
    let hits = classify(&p, "CLS-H20");
    let lo = hits
        .iter()
        .find(|o| o.text.as_deref() == Some("Lo"))
        .expect("Lo");
    let fields: Vec<&str> = lo.locator.split(':').collect();
    assert_eq!(
        (fields[0], fields[2], fields[3]),
        ("v2", "0", "t"),
        "CLS-H20 {}",
        lo.locator
    );
    let start: usize = fields[4]
        .trim_start_matches('p')
        .split('-')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        start > b"BT /F1 12 Tf 72 720 Td (Hi) Tj".len(),
        "CLS-H20 span in part 1: {}",
        lo.locator
    );
    assert_eq!(
        resolve_source_locator(&p, &lo.locator).unwrap(),
        *lo,
        "CLS-H20 resolves"
    );
    // Repeated Forms stay unique through the ordinal.
    let form_twice = {
        let mut d = DocBuilder::new();
        let f = d.add(HELVETICA);
        let form = d.b.add_stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 {f} 0 R >> >>"),
            b"BT /F1 9 Tf 5 5 Td (x) Tj ET",
        );
        d.page(PageSpec::new(
            b"/Fm0 Do 1 0 0 1 100 0 cm /Fm0 Do",
            &format!("/XObject << /Fm0 {form} 0 R >>"),
        ));
        d.build()
    };
    let occ = page_result(form_twice, 0).occurrences;
    assert_eq!(occ.len(), 2);
    assert_ne!(
        occ[0].locator, occ[1].locator,
        "CLS-H20 ordinal keeps Form paints apart"
    );
    assert!(
        occ[0].locator.contains(":x"),
        "CLS-H20 Form chain path: {}",
        occ[0].locator
    );
}

#[test]
fn cls_h21_hardening_json_shape() {
    let r = page_result(fx::word(), 0);
    let json = serde_json::to_value(&r).unwrap();
    for key in [
        "pageIndex",
        "pageReason",
        "runs",
        "occurrences",
        "occurrenceReason",
    ] {
        assert!(json.get(key).is_some(), "CLS-H21 {key}");
    }
    let run = &json["runs"][0];
    assert_eq!(run["capability"], "supported");
    assert!(run["runId"].as_str().unwrap().starts_with("t1:"));
    assert!(run["reason"].is_null());
    let occ = &json["occurrences"][0];
    for key in [
        "pageIndex",
        "kind",
        "rect",
        "locator",
        "capability",
        "reason",
        "text",
    ] {
        assert!(occ.get(key).is_some(), "CLS-H21 occurrence {key}");
    }
    assert_eq!(occ["kind"], "text");
    assert!(occ["rect"].get("w").is_some());
    let refused = serde_json::to_value(page_result(fx::type3(), 0)).unwrap();
    assert_eq!(
        refused["occurrences"][0]["reason"], "TYPE3",
        "CLS-H21 SCREAMING_SNAKE reasons"
    );
    assert_eq!(refused["occurrences"][0]["capability"], "unsupported");
}

#[test]
fn cls_h22_hardening_frozen_reasons_come_from_the_json() {
    let frozen = frozen_reasons();
    for legacy in LEGACY_REASONS {
        assert!(
            frozen.iter().any(|f| f == legacy),
            "CLS-H22 {legacy} survives"
        );
    }
    assert!(
        !frozen.iter().any(|f| f == "MALFORMED"),
        "CLS-H22 the old typo is gone"
    );
    for code in [
        "MALFORMED_CONTENT",
        "FILE_TOO_LARGE",
        "INVALID_PDF",
        "TRANSFORMED_IMAGE",
        "STALE",
    ] {
        assert!(frozen.iter().any(|f| f == code), "CLS-H22 {code}");
    }
    for r in TextReason::RUN_PRIORITY
        .iter()
        .chain(TextReason::PAGE)
        .chain(TextReason::IMAGE_PRIORITY)
    {
        assert!(
            frozen.iter().any(|f| f == r.as_str()),
            "CLS-H22 {}",
            r.as_str()
        );
    }
}

#[test]
fn cls_h23_hardening_vertical() {
    assert_eq!(
        first_reason(fx::identity_v(), "text"),
        Some(TextReason::Vertical),
        "CLS-H23"
    );
}

#[test]
fn cls_h24_hardening_clipped() {
    let past = helvetica_page(b"BT /F1 12 Tf 590 720 Td (Overflow) Tj ET");
    assert_eq!(
        first_reason(past, "text"),
        Some(TextReason::Clipped),
        "CLS-H24 past the page edge"
    );
    assert_eq!(
        first_reason(image_page("40 0 0 40 590 400", ""), "image"),
        Some(TextReason::Clipped),
        "CLS-H24 image across the edge"
    );
    assert_eq!(
        first_reason(image_page("40 0 0 40 72 400", "0 0 50 50 re W n"), "image"),
        Some(TextReason::Clipped),
        "CLS-H24 image outside a small clip"
    );
}

#[test]
fn cls_h25_hardening_occurrence_geometry() {
    let s = Scratch::new("h25");
    let p = s.file("unit.pdf");
    fs::write(&p, fx::user_unit(2.0)).unwrap();
    let hits = classify(&p, "CLS-H25 a GEOMETRY page is not an error");
    let text = first_of_kind(&hits, "text", "CLS-H25");
    assert_unsupported(text, "text", "GEOMETRY", "CLS-H25");
    let r = page_result(fx::user_unit(2.0), 0);
    assert_eq!(
        (r.page_reason, r.runs.len()),
        (Some(TextReason::Geometry), 0)
    );
}

#[test]
fn cls_h26_hardening_encrypted() {
    let Some(engines) = engines_or_skip("cls_h26_hardening_encrypted") else {
        return;
    };
    let s = Scratch::new("h26");
    let p = s.file("enc.pdf");
    fs::write(&p, fx::encrypted(&engines)).unwrap();
    expect_err_code(
        classify_source_content(&p),
        "ENCRYPTED",
        "CLS-H26 qpdf --encrypt",
    );
}

#[test]
fn cls_h27_hardening_tagged_bookmarked_page_is_supported() {
    let s = Scratch::new("h27");
    let p = s.file("word.pdf");
    fs::write(&p, fx::word()).unwrap();
    let hits = classify(&p, "CLS-H27");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(texts.len(), 3, "CLS-H27 three show ops");
    for o in texts {
        assert_supported_text_or_image(o, "text", "CLS-H27 tagged, bookmarked Word page (F1)");
    }
}

#[test]
fn classify_walk_refusal_lists_no_occurrences() {
    // A Form that paints itself: Edit text's depth-0 walk never enters it, the Classify walk
    // does and refuses the page — the runs stay listed, the occurrences are not (never partial).
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let form = d.reserve();
    d.b.set_stream(
        form,
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 300 100] \
             /Resources << /XObject << /Fm0 {form} 0 R >> >>"
        ),
        b"/Fm0 Do",
    );
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Body) Tj ET q 1 0 0 1 72 600 cm /Fm0 Do Q",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Fm0 {form} 0 R >>"),
    ));
    let bytes = d.build();
    let r = page_result(bytes.clone(), 0);
    assert_eq!(r.page_reason, None, "Edit text's page is fine");
    assert_eq!(r.runs.len(), 1, "its run is listed: {:?}", r.runs);
    assert_eq!(r.occurrence_reason, Some(TextReason::MalformedContent));
    assert!(r.occurrences.is_empty(), "{:?}", r.occurrences);
    let s = Scratch::new("occ-reason");
    let path = s.file("self-form.pdf");
    fs::write(&path, &bytes).unwrap();
    expect_err_code(
        classify_source_content(&path),
        "MALFORMED_CONTENT",
        "occurrence_reason",
    );
}
