//! Smoke tests of every producer-shaped fixture (§E.2): each builder opens through the
//! production snapshot reader and shows the behaviour §A.9 expects of it.

use super::{ctx, model, model0, reason_of, walk};
use crate::error::AppError;
use crate::pdf_engine::text_edit::engines::{qpdf_page_map, RunOpts};
use crate::pdf_engine::text_edit::fonts::face_surface;
use crate::pdf_engine::text_edit::reasons::{Face, TextReason as R};
use crate::pdf_engine::text_edit::snapshot::{
    check_page_map, read_snapshot, snapshot_from_bytes, SourceSnapshot,
};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, BadBox, ClipKind, InlineProofKind, LegacyFilter,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::walker::{PaintKind, WalkMode};
use std::path::Path;

type Case = (&'static str, Vec<u8>, &'static str, Option<R>);

fn check(cases: Vec<Case>) {
    for (id, pdf, text, want) in cases {
        let m = model0(pdf);
        assert_eq!(m.page_reason, None, "{id}: page {:?}", m.page_detail);
        assert_eq!(reason_of(&m, text), want, "{id}: {text}");
    }
}

#[test]
fn producers_office_smoke() {
    check(vec![
        ("FX-WORD", fx::word(), "Invoice 2026", None),
        ("FX-WORD", fx::word(), "Due", None),
        ("FX-WORD-TR", fx::word_tr(), "Sağlık Bakanlığı Raporu", None),
        ("FX-LIBRE", fx::libre(), "Libre text", None),
        ("FX-LIBRE", fx::libre(), "text", None),
        ("FX-LIBRE-CFF", fx::libre_cff(), "Hello Hello", None),
        ("FX-SKIA", fx::skia(), "Chrome", None),
        ("FX-SKIA", fx::skia(), "Bold", None),
        ("FX-SKIA", fx::skia(), "Italic", None),
        ("FX-QUARTZ", fx::quartz(), "Quartz", None),
        ("FX-PERGLYPH", fx::per_glyph(None), "Per glyph line", None),
        ("FX-PDFTEX", fx::pdftex(), "Hello World", None),
        ("FX-XETEX", fx::xetex(), "XeTeX", None),
        ("FX-INDD", fx::indd(), "visible layer", None),
        (
            "FX-INDD",
            fx::indd(),
            "hidden layer",
            Some(R::OptionalContent),
        ),
        ("FX-STD14", fx::std14(), "Helvetica line", None),
        ("FX-STD14", fx::std14(), "Times line", None),
        ("FX-STD14", fx::std14(), "Courier line", None),
        ("FX-NONEMB", fx::nonemb(), "Not embedded", None),
        ("FX-OCR", fx::ocr(), "scanned words", Some(R::InvisibleText)),
    ]);
    let skia = model0(fx::skia());
    let bold = skia.runs.iter().find(|r| r.text == "Bold").expect("Bold");
    assert_eq!(bold.tr, 2, "FX-SKIA synthetic bold stays Tr 2");
    let nonemb = model0(fx::nonemb());
    assert!(nonemb.runs[0].substituted, "FX-NONEMB is substituted");
    let indd = ctx(fx::indd());
    let classify = walk(&indd, 0, WalkMode::Classify);
    assert!(
        classify.records.iter().any(|r| r.depth == 1),
        "FX-INDD text in a Form"
    );
    let r = model(&indd, 0);
    assert!(
        r.runs
            .iter()
            .any(|r| r.text == "visible layer" && r.tc == 0.12),
        "FX-INDD Tc"
    );
}

#[test]
fn producers_shared_smoke() {
    let c = ctx(fx::shared());
    assert_eq!(c.snap.pages.len(), 4);
    assert_eq!(
        reason_of(&model(&c, 0), "Shared body"),
        Some(R::SharedContent),
        "FX-SHARED p1"
    );
    assert_eq!(
        reason_of(&model(&c, 1), "Shared body"),
        Some(R::SharedContent),
        "FX-SHARED p2"
    );
    let p3 = model(&c, 2);
    assert_eq!(
        reason_of(&p3, "Letterhead"),
        Some(R::SharedContent),
        "FX-SHARED letterhead"
    );
    assert_eq!(reason_of(&p3, "Body three"), None, "FX-SHARED own part");
}

#[test]
fn producers_edges_smoke() {
    let mut cases: Vec<Case> = vec![
        ("cropped_offset", fx::cropped_offset(), "Cropped page", None),
        ("two_parts_mid_bt", fx::two_parts_mid_bt(), "Hi", None),
        ("two_parts_mid_bt", fx::two_parts_mid_bt(), "Lo", None),
        (
            "straddling_op",
            fx::straddling_op(),
            "Hello",
            Some(R::SplitContent),
        ),
        (
            "comments_and_odd_ws",
            fx::comments_and_odd_ws(),
            "Odd ws",
            None,
        ),
        ("quote_ops", fx::quote_ops(), "Line one", None),
        ("quote_ops", fx::quote_ops(), "Line two", None),
        ("quote_ops", fx::quote_ops(), "Line three", None),
        (
            "kerned_then_tail",
            fx::kerned_then_tail(),
            "AB CDtail",
            None,
        ),
        ("tf1_tm12", fx::tf1_tm12(), "Hi", None),
        ("tz_tc_tw_ts", fx::tz_tc_tw_ts(), "a b", None),
        ("extgstate_font", fx::extgstate_font(), "Hi there", None),
        ("mac_roman", fx::mac_roman(), "café", None),
        ("subset_without_y", fx::subset_without_y(), "Hello", None),
        (
            "actual_text_span",
            fx::actual_text_span(),
            "fi",
            Some(R::ActualText),
        ),
        (
            "actual_text_struct",
            fx::actual_text_struct(),
            "Struct",
            Some(R::ActualText),
        ),
        (
            "duplicate_shadow",
            fx::duplicate_shadow(),
            "Shadow",
            Some(R::DuplicateText),
        ),
        ("clip_page", fx::clip(ClipKind::Page), "Clip me", None),
        (
            "clip_small",
            fx::clip(ClipKind::Small),
            "Clip me",
            Some(R::Clipped),
        ),
        (
            "clip_curve",
            fx::clip(ClipKind::Curve),
            "Clip me",
            Some(R::Clipped),
        ),
        (
            "pattern_fill",
            fx::pattern_fill(),
            "Pattern",
            Some(R::Pattern),
        ),
        ("smask_text", fx::smask_text(), "Masked", Some(R::SoftMask)),
        ("tr7", fx::tr7(), "Clip text", Some(R::TextClipMode)),
        ("identity_v", fx::identity_v(), "Up", Some(R::Vertical)),
        ("type3", fx::type3(), "a", Some(R::Type3)),
        ("mirrored", fx::mirrored(), "Mirror", Some(R::MirroredText)),
        (
            "negative_tz",
            fx::negative_tz(),
            "Mirror",
            Some(R::MirroredText),
        ),
        (
            "negative_tf",
            fx::negative_tf(),
            "Turned",
            Some(R::RotatedText),
        ),
        ("skewed", fx::skewed(), "Skewed", Some(R::SkewedText)),
        ("oblique", fx::oblique(), "Oblique", None),
    ];
    for angle in [90, 180, 270] {
        cases.push((
            "rotated(counter)",
            fx::rotated(angle, true),
            "Rotated page",
            None,
        ));
        cases.push((
            "rotated",
            fx::rotated(angle, false),
            "Rotated page",
            Some(R::RotatedText),
        ));
    }
    for proof in [
        InlineProofKind::Length,
        InlineProofKind::Unfiltered,
        InlineProofKind::Flate,
    ] {
        cases.push((
            "inline_image",
            fx::inline_image(proof),
            "After the image",
            None,
        ));
    }
    cases.push((
        "inline_image(Dct)",
        fx::inline_image(InlineProofKind::Dct),
        "After the image",
        Some(R::InlineImage),
    ));
    check(cases);
    assert_eq!(
        model0(fx::user_unit(2.0)).page_reason,
        Some(R::Geometry),
        "user_unit(2)"
    );
    let y = model0(fx::subset_without_y());
    assert!(
        !y.surface(&y.runs[0]).alphabet().contains(&'Y'),
        "subset_without_y: Y not typeable"
    );
    let nested = ctx(fx::nested_form());
    assert!(
        model(&nested, 0).runs.is_empty(),
        "nested_form: no Edit run"
    );
    assert_eq!(
        walk(&nested, 0, WalkMode::Classify).records.len(),
        1,
        "nested_form"
    );
}

fn snap_code(bytes: Vec<u8>) -> String {
    match snapshot_from_bytes(Path::new("p.pdf"), bytes, None) {
        Ok(_) => "OK".into(),
        Err(e) => e.code,
    }
}

#[test]
fn producers_files_smoke() {
    assert_eq!(snap_code(fx::deep_nesting(100)), "OK", "deep_nesting(100)");
    assert_eq!(
        snap_code(fx::deep_nesting(101)),
        "FILE_TOO_COMPLEX",
        "deep_nesting(101)"
    );
    assert_eq!(
        snap_code(fx::objstm_bomb()),
        "FILE_TOO_COMPLEX",
        "objstm_bomb"
    );
    assert_eq!(snap_code(fx::xref_bomb()), "FILE_TOO_COMPLEX", "xref_bomb");
    assert_eq!(snap_code(fx::signed()), "SIGNED", "FX-SIGNED");
    assert_eq!(snap_code(fx::xfa()), "UNSUPPORTED_XFA", "FX-XFA");
    let bomb = ctx(fx::flate_bomb());
    assert_eq!(
        model(&bomb, 0).page_reason,
        Some(R::PageTooComplex),
        "flate_bomb"
    );
    assert_eq!(
        reason_of(&model(&bomb, 1), "Hello"),
        None,
        "flate_bomb: page 2 unaffected"
    );
    let ext = model0(fx::extensions_indirect());
    assert_eq!(reason_of(&ext, "Hello"), None, "extensions_indirect");
    for kind in [
        BadBox::NoMediaBox,
        BadBox::RealRotate,
        BadBox::OddRotate,
        BadBox::ThinCrop,
        BadBox::UserUnitString,
        BadBox::UserUnitNegative,
        BadBox::UserUnitTwo,
        BadBox::NonNumberBox,
    ] {
        assert_eq!(
            model0(fx::bad_boxes(kind)).page_reason,
            Some(R::Geometry),
            "bad_boxes({kind:?})"
        );
    }
    for filter in [LegacyFilter::RunLength, LegacyFilter::Lzw] {
        let c = ctx(fx::legacy_filter_page(filter));
        assert_eq!(
            model(&c, 0).page_reason,
            Some(R::UnsupportedFilter),
            "legacy {filter:?}"
        );
        assert_eq!(reason_of(&model(&c, 1), "Hello"), None);
    }
    let shared = ctx(fx::shared_inherited_resources());
    let p1 = model(&shared, 0);
    assert_eq!(
        reason_of(&p1, "Regular words"),
        None,
        "shared_inherited_resources"
    );
    let primary = &p1.walk.page_fonts[0].1;
    assert!(
        face_surface(&p1.walk.page_fonts, primary, Face::Bold).is_some(),
        "shared_inherited_resources: the bold sibling used only on page 2 is a page font"
    );
    let (a, b) = (ctx(fx::swapped_image(false)), ctx(fx::swapped_image(true)));
    let hash = |c: &crate::pdf_engine::text_edit::context::SnapshotContext| {
        walk(c, 0, WalkMode::Edit)
            .paints
            .into_iter()
            .find_map(|p| match p.kind {
                PaintKind::ImageXObject { hash, .. } => Some(hash),
                _ => None,
            })
    };
    assert_ne!(hash(&a), hash(&b), "swapped_image");
    let two = model0(fx::two_column(false));
    assert_eq!(two.runs.len(), 7, "two_column");
    let stroke = model0(fx::stroke_text(2, [0.5, 1.0], ["[] 0", "[] 0"]));
    assert_eq!(stroke.runs.len(), 2, "stroke_text");
    assert_eq!(
        reason_of(&model0(fx::tagged_bookmarked_page()), "Tagged line"),
        None
    );
}

fn page_map(bytes: &[u8], id: &str) -> Option<(usize, String)> {
    let engines = engines_or_skip(id)?;
    let s = Scratch::new("producers-map");
    let p = s.write("f.pdf", bytes);
    let snap: SourceSnapshot = read_snapshot(&p).unwrap_or_else(|e: AppError| panic!("{id}: {e}"));
    let pages =
        qpdf_page_map(&engines, &p, &RunOpts::default()).unwrap_or_else(|e| panic!("{id}: {e}"));
    let code = match check_page_map(&snap, &pages) {
        Ok(()) => "OK".to_string(),
        Err(e) => e.code,
    };
    Some((snap.pages.len(), code))
}

#[test]
fn producers_engine_backed_smoke() {
    let Some(engines) = engines_or_skip("producers_engine_backed_smoke") else {
        return;
    };
    assert_eq!(snap_code(fx::encrypted(&engines)), "ENCRYPTED", "FX-ENC");
    let word = fx::hybrid_xref(true);
    assert_eq!(
        page_map(&word, "hybrid_xref(true)"),
        Some((1, "OK".to_string()))
    );
    assert_eq!(
        reason_of(&model0(word), "Hello"),
        None,
        "hybrid_xref(true) editable"
    );
    assert_eq!(
        page_map(&fx::hybrid_xref(false), "hybrid_xref(false)"),
        Some((0, "PDF_NEEDS_REPAIR".to_string())),
        "hybrid_xref(false): lopdf misses the page"
    );
    assert_eq!(
        page_map(&fx::kid_without_type(), "kid_without_type"),
        Some((1, "PDF_NEEDS_REPAIR".to_string())),
        "kid_without_type"
    );
}
