//! PB-01…08: Phase B on final files made by real qpdf passes after an honest edit — the parts
//! form, qpdf's overlay wrapper (also on rotated and cropped pages with the boxes remapped as the
//! Save pipeline does), stamps, flattened parts and an appended signed file pass; fakes inside a
//! wrapper and a wrapper that clips the page fail.

use super::{ed, fails_at, skipping, Honest};
use crate::pdf_engine::text_edit::content::{page_content, qpdf_join};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits, Operator};
use crate::pdf_engine::text_edit::limits::{
    set_file_cap_override, PAGE_DECODE_BUDGET, VERIFY_CAP_MARGIN_BYTES,
};
use crate::pdf_engine::text_edit::snapshot::{read_snapshot, read_verification_snapshot};
use crate::pdf_engine::text_edit::testkit::fakes::{self, json_dict};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, helvetica_page, DocBuilder, PageSpec,
};
use crate::pdf_engine::validate_output::content_digest;
use lopdf::Object;
use serde_json::json;
use std::path::{Path, PathBuf};

fn page() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello world) Tj ET BT /F1 12 Tf 72 650 Td (Second line) Tj ET",
    )
}

fn honest(test: &str) -> Option<Honest> {
    Honest::new(test, page(), 0, &[ed("Hello world", "Hello there")])
}

/// One empty page (`blank.pdf`).
fn blank(h: &Honest) -> PathBuf {
    let mut d = DocBuilder::new();
    d.page(PageSpec::new(b"", ""));
    let p = h.path("blank.pdf");
    std::fs::write(&p, d.build()).expect("blank");
    p
}

/// A stamp page: a filled square far from the edited line.
fn stamp(h: &Honest) -> PathBuf {
    let p = h.path("stamp.pdf");
    std::fs::write(
        &p,
        helvetica_page(b"0 0 1 rg 400 100 60 60 re f BT /F1 10 Tf 405 120 Td (Stamp) Tj ET"),
    )
    .expect("stamp");
    p
}

/// Whether page `page` of `pdf` is qpdf's overlay wrapper (`q cm Do Q` only).
fn is_wrapper(pdf: &Path, page: u32) -> bool {
    let snap = read_verification_snapshot(pdf, 1 << 30).unwrap_or_else(|e| panic!("{e}"));
    let ctx = SnapshotContext::new(snap);
    let id = ctx.page_id(page).expect("page");
    let content =
        page_content(ctx.doc(), id, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("content");
    let ops = lex_content(&content.joined, &LexLimits::page(), None).expect("lex");
    !ops.is_empty()
        && ops.iter().all(|o| {
            matches!(
                o.operator,
                Operator::q | Operator::Q | Operator::cm | Operator::Do
            )
        })
}

#[test]
fn pb_01_parts_form_after_assembly() {
    let Some(h) = honest("pb_01") else { return };
    let other = h.path("other.pdf");
    std::fs::write(
        &other,
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Other file) Tj ET"),
    )
    .expect("other");
    let out = h.path("final.pdf");
    fakes::assemble(h.qpdf(), &[&h.staged, &other], &out).unwrap_or_else(|e| panic!("{e}"));
    assert!(!is_wrapper(&out, 0), "PB-01 parts form");
    let r = h.phase_b(&out, 0);
    assert!(r.is_ok(), "PB-01: {:?}", r.err().map(|e| e.details));
    // The proof Phase A handed over: the source page, every expected part with its digest and
    // the page's show records after the edit.
    let proof = h.proof();
    assert_eq!(proof.source_page_index, 0, "PB-01 proof page");
    let digests: Vec<_> = proof
        .expected_parts
        .iter()
        .map(|p| content_digest(p))
        .collect();
    assert_eq!(proof.part_digests, digests, "PB-01 part digests");
    assert_eq!(proof.records.len(), 2, "PB-01 two show records");
}

#[test]
fn pb_02_wrapper_form_after_an_empty_overlay() {
    let Some(h) = honest("pb_02") else { return };
    let out = h.path("final.pdf");
    fakes::overlay(h.qpdf(), &h.staged, &out, &blank(&h), None).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        is_wrapper(&out, 0),
        "PB-02 every page becomes the wrapper (P-OV)"
    );
    let r = h.phase_b(&out, 0);
    assert!(r.is_ok(), "PB-02: {:?}", r.err().map(|e| e.details));
}

#[test]
fn pb_03_wrapper_with_a_stamp_on_the_same_page() {
    let Some(h) = honest("pb_03") else { return };
    let out = h.path("final.pdf");
    fakes::overlay(h.qpdf(), &h.staged, &out, &stamp(&h), None).unwrap_or_else(|e| panic!("{e}"));
    let r = h.phase_b(&out, 0);
    assert!(
        r.is_ok(),
        "PB-03 the stamp is a paint: {:?}",
        r.err().map(|e| e.details)
    );
}

/// `pdf` with every page's boxes set as the Save pipeline does before an overlay
/// (MediaBox = CropBox = TrimBox = the visible box), or restored to `boxes`.
fn set_boxes(h: &Honest, input: &Path, out: &Path, boxes: &[(&str, Option<[f64; 4]>)]) {
    let doc = fakes::load(input);
    let page = fakes::page_ids(&doc)[0];
    let mut d = json_dict(doc.get_dictionary(page).expect("page"));
    if let Some(o) = d.as_object_mut() {
        for (key, value) in boxes {
            match value {
                Some(b) => o.insert(format!("/{key}"), json!(b.to_vec())),
                None => o.remove(&format!("/{key}")),
            };
        }
    }
    fakes::set_value(h.qpdf(), input, out, page, d).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn pb_04_wrapper_on_rotated_and_cropped_pages() {
    for (angle, text) in [(90, "Rotated page"), (270, "Rotated page")] {
        let test = format!("pb_04_{angle}");
        let Some(h) = Honest::new(
            &test,
            fx::rotated(angle, true),
            0,
            &[ed(text, "Rotated pages")],
        ) else {
            return;
        };
        let out = h.path("final.pdf");
        fakes::overlay(h.qpdf(), &h.staged, &out, &blank(&h), None)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(is_wrapper(&out, 0), "PB-04 /Rotate {angle} wrapper");
        let r = h.phase_b(&out, 0);
        assert!(
            r.is_ok(),
            "PB-04 /Rotate {angle}: {:?}",
            r.err().map(|e| e.details)
        );
    }
    // Offset CropBox with Trim ⊂ Crop: qpdf alone would centre the trimmed content in the
    // MediaBox (it moves); the pipeline remaps the boxes to the visible box first, then restores.
    let pdf = helvetica_doc(
        b"BT /F1 12 Tf 72 720 Td (Cropped page) Tj ET",
        "",
        "/CropBox [36 48 576 744] /TrimBox [50 60 560 730]",
    );
    let Some(h) = Honest::new("pb_04_crop", pdf, 0, &[ed("Cropped page", "Cropped pages")]) else {
        return;
    };
    let visible = [36.0, 48.0, 576.0, 744.0];
    let remapped = h.path("remapped.pdf");
    set_boxes(
        &h,
        &h.staged,
        &remapped,
        &[
            ("MediaBox", Some(visible)),
            ("CropBox", Some(visible)),
            ("TrimBox", Some(visible)),
        ],
    );
    let overlaid = h.path("overlaid.pdf");
    fakes::overlay(h.qpdf(), &remapped, &overlaid, &stamp(&h), None)
        .unwrap_or_else(|e| panic!("{e}"));
    let out = h.path("final.pdf");
    set_boxes(
        &h,
        &overlaid,
        &out,
        &[
            ("MediaBox", Some([0.0, 0.0, 612.0, 792.0])),
            ("CropBox", Some(visible)),
            ("TrimBox", Some([50.0, 60.0, 560.0, 730.0])),
        ],
    );
    assert!(is_wrapper(&out, 0), "PB-04 cropped wrapper");
    let r = h.phase_b(&out, 0);
    assert!(
        r.is_ok(),
        "PB-04 offset CropBox: {:?}",
        r.err().map(|e| e.details)
    );
    // Without the remap qpdf moves the content: the wrapper is not the identity — refused.
    let moved = h.path("moved.pdf");
    fakes::overlay(h.qpdf(), &h.staged, &moved, &blank(&h), None).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_b(&moved, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "not the identity",
        "PB-04 unremapped",
    );
}

#[test]
fn pb_05_wrapper_around_a_fake() {
    let Some(h) = honest("pb_05") else { return };
    let c = h
        .model
        .runs
        .iter()
        .find(|r| r.text == "Hello world")
        .expect("run")
        .rect;
    let cover = [c[0] - 1.0, c[1] - 1.0, c[2] + 2.0, c[3] + 2.0];
    // (a) the old parts in /Fx0, the new text in /Fx1 (a real `qpdf --overlay`).
    let fake = h.path("fake-f2.pdf");
    fakes::overlay_f2(
        h.qpdf(),
        &h.source,
        &fake,
        1,
        [0.0, 0.0, 612.0, 792.0],
        cover,
        "Hello there",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_b(&fake, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "",
        "PB-05 B1",
    );
    let r = skipping(&["B1"], || h.phase_b(&fake, 0));
    fails_at(
        r,
        "SOURCE_EDIT_GATE_FAILED",
        "B2",
        "original content",
        "PB-05 B2",
    );
    let r = skipping(&["B1", "B2"], || h.phase_b(&fake, 0));
    fails_at(r, "SOURCE_EDIT_GATE_FAILED", "B3", "", "PB-05 B3");
    // (b) GATE-18's fake (old part + a cover part) wrapped by a later overlay: both in /Fx0.
    let f1 = h.path("fake-f1.pdf");
    fakes::cover_and_overlay_f1(h.qpdf(), &h.source, &f1, 0, cover, "F1", "Hello there")
        .unwrap_or_else(|e| panic!("{e}"));
    let wrapped = h.path("fake-f1-wrapped.pdf");
    fakes::overlay(h.qpdf(), &f1, &wrapped, &blank(&h), None).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_b(&wrapped, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "",
        "PB-05b B1",
    );
    let r = skipping(&["B1"], || h.phase_b(&wrapped, 0));
    fails_at(
        r,
        "SOURCE_EDIT_GATE_FAILED",
        "B2",
        "original content",
        "PB-05b B2",
    );
    let r = skipping(&["B1", "B2"], || h.phase_b(&wrapped, 0));
    fails_at(r, "SOURCE_EDIT_GATE_FAILED", "B3", "", "PB-05b B3");
}

#[test]
fn pb_06_wrapper_bbox_cuts_the_visible_page() {
    let Some(h) = honest("pb_06") else { return };
    let wrapped = h.path("wrapped.pdf");
    fakes::overlay(h.qpdf(), &h.staged, &wrapped, &blank(&h), None)
        .unwrap_or_else(|e| panic!("{e}"));
    let doc = fakes::load(&wrapped);
    let page = fakes::page_ids(&doc)[0];
    let fx0 = doc
        .get_dictionary(page)
        .and_then(|d| d.get(b"Resources"))
        .and_then(Object::as_dict)
        .and_then(|r| r.get(b"XObject"))
        .and_then(Object::as_dict)
        .and_then(|x| x.get(b"Fx0"))
        .and_then(Object::as_reference)
        .expect("/Fx0");
    let stream = doc
        .get_object(fx0)
        .and_then(Object::as_stream)
        .expect("Fx0 stream");
    let mut dict = json_dict(&stream.dict);
    dict.as_object_mut()
        .expect("dict")
        .insert("/BBox".into(), json!([0, 0, 612, 400]));
    let out = h.path("final.pdf");
    fakes::stream_dict(h.qpdf(), &wrapped, &out, fx0, dict).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_b(&out, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "/BBox",
        "PB-06",
    );
}

#[test]
fn pb_07_parts_form_with_flattened_parts_around() {
    let Some(h) = honest("pb_07") else { return };
    let out = h.path("final.pdf");
    fakes::add_parts(
        h.qpdf(),
        &h.staged,
        &out,
        0,
        &[b"q 1 0 0 RG 300 300 80 20 re S Q"],
        &[
            b"q 0 0 1 rg 300 200 80 20 re f Q",
            b"q 0.5 g 300 100 80 20 re f Q",
        ],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let r = h.phase_b(&out, 0);
    assert!(r.is_ok(), "PB-07: {:?}", r.err().map(|e| e.details));
    // The expected parts must be there exactly once, contiguous and in order.
    let joined: Vec<&[u8]> = h.proof().expected_parts.iter().map(Vec::as_slice).collect();
    assert_eq!(
        qpdf_join(&joined),
        h.proof().expected_parts.concat(),
        "PB-07 one part"
    );
}

#[test]
fn pb_08_appended_signed_file_and_a_total_above_the_source_cap() {
    let Some(h) = honest("pb_08") else { return };
    let signed = h.path("signed.pdf");
    std::fs::write(&signed, fx::signed()).expect("signed");
    let out = h.path("final.pdf");
    fakes::assemble(h.qpdf(), &[&h.staged, &signed], &out).unwrap_or_else(|e| panic!("{e}"));
    let size = std::fs::metadata(&out).map_or(0, |m| m.len());
    set_file_cap_override(Some(size / 2));
    let source_policy = read_snapshot(&out);
    let cap = 2 * size + VERIFY_CAP_MARGIN_BYTES;
    let r = crate::pdf_engine::text_edit::gate::verify_final_output(
        &out,
        cap,
        &h.engines,
        &[(0, h.proof())],
        &super::opts(),
    );
    set_file_cap_override(None);
    assert!(
        source_policy.is_err(),
        "PB-08 read_snapshot alone refuses the final file"
    );
    assert!(r.is_ok(), "PB-08 (D31): {:?}", r.err().map(|e| e.details));
}

/// PB-09 (review-T5 H1): the wrapper form is refused when qpdf's join changes what the expected
/// parts mean — a comment at a part end would end (the line it hid would be drawn), or two
/// numbers would no longer be one. The walker refuses such pages outright, so no honest plan
/// has such parts: the final file and the proof are built directly, with a wrapper that holds
/// `qpdf_join(expected_parts)` exactly as qpdf would have written it. B1 must refuse it before
/// B3 compares any record.
#[test]
fn pb_09_wrapper_whose_join_changes_the_edited_content() {
    let Some(engines) = crate::pdf_engine::text_edit::testkit::engines_or_skip("pb_09") else {
        return;
    };
    let dir = crate::pdf_engine::text_edit::testkit::Scratch::new("pb_09");
    let cases: [(&str, [&[u8]; 2]); 2] = [
        (
            "comment",
            [
                b"BT /F1 12 Tf 72 700 Td (Hello there) Tj ET\n% note",
                b"BT /F1 12 Tf 72 650 Td (Second line) Tj ET",
            ],
        ),
        (
            "number",
            [
                b"BT /F1 12 Tf 72 700 Td (Hello there) Tj ET 0 0 1 rg 30",
                b"0 20 20 re f BT /F1 12 Tf 72 650 Td (Second line) Tj ET",
            ],
        ),
    ];
    for (id, parts) in cases {
        let joined = qpdf_join(&parts);
        let mut d = DocBuilder::new();
        let f = d.add(fx::HELVETICA);
        let form = d.b.add_stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
                 /Resources << /Font << /F1 {f} 0 R >> >>"
            ),
            &joined,
        );
        d.page(PageSpec::new(
            b"q 1 0 0 1 0 0 cm /Fx0 Do Q",
            &format!("/XObject << /Fx0 {form} 0 R >> /Font << /F1 {f} 0 R >>"),
        ));
        let out = dir.write(&format!("{id}.pdf"), &d.build());
        let proof = crate::pdf_engine::text_edit::gate::PageProof {
            source_page_index: 0,
            expected_parts: parts.iter().map(|p| p.to_vec()).collect(),
            part_digests: parts.iter().map(|p| content_digest(p)).collect(),
            original_edited_digests: Vec::new(),
            original_edited_rolling: Vec::new(),
            expected_page_digest: content_digest(&parts.concat()),
            records: Vec::new(),
        };
        let cap = 4 * std::fs::metadata(&out).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES;
        let r = crate::pdf_engine::text_edit::gate::verify_final_output(
            &out,
            cap,
            &engines,
            &[(0, &proof)],
            &super::opts(),
        );
        fails_at(r, "SOURCE_EDIT_GATE_FAILED", "B1", "join changes", id);
    }
}
