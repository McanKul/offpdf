//! GEO-01…15: orientation as displayed (§A.5 table, composite × page `/Rotate`) and the strict
//! page geometry parser, with `crop.rs` only in the parity test GEO-11.

use super::{close, ctx, model0, reason_of};
use crate::pdf_engine::crop;
use crate::pdf_engine::text_edit::geometry::{
    classify_orientation, display_rotation, mul, page_geometry, ray_extent, Orientation,
};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::snapshot::read_snapshot;
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page, BadBox};
use std::path::Path;

fn one(content: &str, text: &str) -> Option<R> {
    reason_of(&model0(helvetica_page(content.as_bytes())), text)
}

#[test]
fn geo_01_to_09_orientation_cases() {
    let cases: [(&str, &str, &str, Option<R>); 9] = [
        (
            "GEO-01 upright",
            "BT /F1 12 Tf 72 700 Td (Upright) Tj ET",
            "Upright",
            None,
        ),
        (
            "GEO-02 180°",
            "BT /F1 12 Tf -1 0 0 -1 300 400 Tm (Turned) Tj ET",
            "Turned",
            Some(R::RotatedText),
        ),
        (
            "GEO-03 mirror",
            "BT /F1 12 Tf -1 0 0 1 300 400 Tm (Mirror) Tj ET",
            "Mirror",
            Some(R::MirroredText),
        ),
        (
            "GEO-04 -100 Tz",
            "BT /F1 12 Tf -100 Tz 300 400 Td (Mirror) Tj ET",
            "Mirror",
            Some(R::MirroredText),
        ),
        (
            "GEO-05 -12 Tf",
            "BT /F1 -12 Tf 300 400 Td (Turned) Tj ET",
            "Turned",
            Some(R::RotatedText),
        ),
        (
            "GEO-06 45°",
            "BT /F1 12 Tf 0.7071 0.7071 -0.7071 0.7071 300 400 Tm (Angle) Tj ET",
            "Angle",
            Some(R::RotatedText),
        ),
        (
            "GEO-07 skew",
            "BT /F1 12 Tf 1 0.3 0 1 72 400 Tm (Skew) Tj ET",
            "Skew",
            Some(R::SkewedText),
        ),
        (
            "GEO-08 oblique c=0.2",
            "BT /F1 12 Tf 1 0 0.2 1 72 400 Tm (Oblique) Tj ET",
            "Oblique",
            None,
        ),
        (
            "GEO-09 Skia flip",
            "1 0 0 -1 0 792 cm BT /F1 12 Tf 1 0 0 -1 72 100 Tm (Skia) Tj ET",
            "Skia",
            None,
        ),
    ];
    for (id, content, text, want) in cases {
        assert_eq!(one(content, text), want, "{id}");
    }
    assert_eq!(
        one("BT /F1 0 Tf 72 700 Td (Zero) Tj ET", "Zero"),
        Some(R::ZeroSize),
        "Tf 0"
    );
    assert_eq!(
        one("BT /F1 12 Tf 0 Tz 72 700 Td (Zero) Tj ET", "Zero"),
        Some(R::ZeroSize),
        "Tz 0"
    );
    // The table itself, on bare matrices.
    for (m, want) in [
        ([12.0, 0.0, 0.0, 12.0, 0.0, 0.0], Orientation::Upright),
        ([12.0, 0.0, 6.0, 12.0, 0.0, 0.0], Orientation::Upright),
        ([12.0, 0.0, 6.1, 12.0, 0.0, 0.0], Orientation::Skewed),
        ([0.0, 12.0, -12.0, 0.0, 0.0, 0.0], Orientation::Rotated),
        ([0.0, 12.0, 12.0, 0.0, 0.0, 0.0], Orientation::Skewed),
        ([0.0, 0.0, 0.0, 12.0, 0.0, 0.0], Orientation::ZeroSize),
        ([-12.0, 0.0, 0.0, -12.0, 0.0, 0.0], Orientation::Rotated),
        ([12.0, 0.0, 0.0, -12.0, 0.0, 0.0], Orientation::Mirrored),
    ] {
        assert_eq!(classify_orientation(&m), want, "{m:?}");
    }
    assert!(close(
        ray_extent((72.0, 700.0), (1.0, 0.0), [0.0, 0.0, 612.0, 792.0]),
        540.0
    ));
    assert_eq!(
        ray_extent((700.0, 700.0), (1.0, 0.0), [0.0, 0.0, 612.0, 792.0]),
        0.0
    );
}

#[test]
fn geo_10_rotate_pairs_judged_as_displayed() {
    for angle in [90, 180, 270] {
        assert_eq!(
            reason_of(&model0(fx::rotated(angle, true)), "Rotated page"),
            None,
            "GEO-10 /Rotate {angle} with counter-rotated text reads upright"
        );
        assert_eq!(
            reason_of(&model0(fx::rotated(angle, false)), "Rotated page"),
            Some(R::RotatedText),
            "GEO-10 /Rotate {angle} with user-upright text is turned on screen"
        );
    }
    let r = display_rotation(90);
    assert_eq!(
        mul(&[0.0, 1.0, -1.0, 0.0, 0.0, 0.0], &r),
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    );
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/source-edit")
        .join(name)
}

#[test]
fn geo_11_parity_with_crop_rs_on_accepted_pages() {
    let mut docs = vec![
        fx::word(),
        fx::word_tr(),
        fx::cropped_offset(),
        fx::two_column(false),
        fx::libre(),
        fx::shared(),
    ];
    for angle in [90, 180, 270] {
        docs.push(fx::rotated(angle, true));
    }
    let mut snaps: Vec<_> = docs.into_iter().map(|d| ctx(d).snap).collect();
    for name in [
        "geom-crop-offset.pdf",
        "geom-rotate-90.pdf",
        "geom-rotate-180.pdf",
        "geom-rotate-270.pdf",
        "text-tj.pdf",
        "image-unique.pdf",
    ] {
        snaps.push(read_snapshot(&corpus(name)).expect("corpus fixture"));
    }
    let mut compared = 0;
    for snap in &snaps {
        for page in &snap.pages {
            let g = page_geometry(&snap.doc, *page).expect("GEO-11 accepted page");
            let visible = crop::visible_box(&snap.doc, *page);
            assert!(
                g.visible
                    .iter()
                    .zip(visible)
                    .all(|(a, b)| (a - b).abs() < 1e-3),
                "GEO-11 visible {:?} vs crop.rs {visible:?}",
                g.visible
            );
            assert_eq!(
                g.rotate,
                crop::page_rotation(&snap.doc, *page),
                "GEO-11 rotate"
            );
            assert_eq!(
                g.user_unit,
                crop::page_user_unit(&snap.doc, *page),
                "GEO-11 UserUnit"
            );
            compared += 1;
        }
    }
    assert!(compared >= 15, "GEO-11 compared {compared} pages");
}

fn geometry_of(
    kind: BadBox,
) -> (
    Result<(), R>,
    crate::pdf_engine::text_edit::context::SnapshotContext,
) {
    let c = ctx(fx::bad_boxes(kind));
    let page = c.snap.pages[0];
    (page_geometry(&c.snap.doc, page).map(|_| ()), c)
}

#[test]
fn geo_12_real_rotate_is_refused() {
    for kind in [BadBox::RealRotate, BadBox::OddRotate] {
        let (g, c) = geometry_of(kind);
        assert_eq!(g, Err(R::Geometry), "GEO-12 {kind:?}");
        assert_eq!(super::model(&c, 0).page_reason, Some(R::Geometry));
    }
    let (_, c) = geometry_of(BadBox::RealRotate);
    assert_eq!(
        crop::page_rotation(&c.snap.doc, c.snap.pages[0]),
        0,
        "crop.rs ignores it silently"
    );
}

#[test]
fn geo_13_missing_media_box_is_refused() {
    let (g, c) = geometry_of(BadBox::NoMediaBox);
    assert_eq!(g, Err(R::Geometry), "GEO-13");
    assert_eq!(
        crop::media_box(&c.snap.doc, c.snap.pages[0]),
        [0.0, 0.0, 612.0, 792.0],
        "GEO-13 crop.rs would say Letter"
    );
    let (g, _) = geometry_of(BadBox::NonNumberBox);
    assert_eq!(g, Err(R::Geometry), "GEO-13 a box that is not four numbers");
}

#[test]
fn geo_14_thin_crop_is_refused() {
    let (g, c) = geometry_of(BadBox::ThinCrop);
    assert_eq!(g, Err(R::Geometry), "GEO-14");
    assert_eq!(
        crop::visible_box(&c.snap.doc, c.snap.pages[0]),
        [0.0, 0.0, 612.0, 792.0],
        "GEO-14 crop.rs would silently use the MediaBox"
    );
}

#[test]
fn geo_15_user_unit_other_than_the_number_one() {
    for kind in [
        BadBox::UserUnitString,
        BadBox::UserUnitNegative,
        BadBox::UserUnitTwo,
    ] {
        let (g, c) = geometry_of(kind);
        assert_eq!(g, Err(R::Geometry), "GEO-15 {kind:?}");
        let m = super::model(&c, 0);
        assert_eq!(
            (m.page_reason, m.runs.len()),
            (Some(R::Geometry), 0),
            "GEO-15 page level"
        );
    }
    assert_eq!(
        model0(fx::user_unit(2.0)).page_reason,
        Some(R::Geometry),
        "GEO-15 UserUnit 2"
    );
    assert_eq!(
        model0(fx::user_unit(1.0)).page_reason,
        None,
        "GEO-15 UserUnit 1"
    );
}
