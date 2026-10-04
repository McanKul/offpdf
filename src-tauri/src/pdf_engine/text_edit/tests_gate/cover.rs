//! GATE-18…22 and 24: cover-and-overlay (a new content stream, or a real `qpdf --overlay`), a
//! raster of the page, an annotation over the old text and the old stream re-attached — each
//! fails Phase A at every listed check and, where listed, Phase B — and GATE-21: why #34's
//! `validate_staged_pdf` alone is not enough.

use super::{ed, fails_at, skipping, Honest};
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::helvetica_page;
use crate::pdf_engine::validate_output::{
    catalog_flags_from_doc, validate_staged_pdf, ContentDigest, OutputSnapshot, PageSnapshot,
};
use std::path::Path;

const MEDIA: [f64; 4] = [0.0, 0.0, 612.0, 792.0];

fn page() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Invoice 2026) Tj ET BT /F1 12 Tf 72 650 Td (Unchanged line) Tj ET",
    )
}

fn honest(test: &str) -> Option<Honest> {
    Honest::new(test, page(), 0, &[ed("Invoice 2026", "Invoice 2027")])
}

/// The cover box over the edited line (`[x, y, w, h]`).
fn cover_rect(h: &Honest) -> [f64; 4] {
    let r = h
        .model
        .runs
        .iter()
        .find(|r| r.text == "Invoice 2026")
        .expect("run")
        .rect;
    [r[0] - 1.0, r[1] - 1.0, r[2] + 2.0, r[3] + 2.0]
}

/// A fails at A2, A3, A4 and A5 in turn (each with the earlier ones skipped).
fn fails_a2_to_a5(h: &Honest, file: &Path, a3: &str, id: &str) {
    fails_at(
        h.phase_a(file),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[0]",
        id,
    );
    let r = skipping(&["A2"], || h.phase_a(file));
    fails_at(r, "EDIT_VERIFY_FAILED", "A3", a3, &format!("{id} A3"));
    let r = skipping(&["A2", "A3"], || h.phase_a(file));
    fails_at(r, "EDIT_VERIFY_FAILED", "A4", "", &format!("{id} A4"));
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(file));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "", &format!("{id} A5"));
}

#[test]
fn gate_18_cover_and_overlay_f1_fails_phase_a_and_phase_b() {
    let Some(h) = honest("gate_18") else { return };
    let fake = h.path("fake.pdf");
    fakes::cover_and_overlay_f1(
        h.qpdf(),
        &h.source,
        &fake,
        0,
        cover_rect(&h),
        "F1",
        "Invoice 2027",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_a2_to_a5(
        &h,
        &fake,
        "original content is still attached; part count 2",
        "GATE-18",
    );
    // Phase B on a final file that carries the fake.
    fails_at(
        h.phase_b(&fake, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "not on the page",
        "GATE-18 B1",
    );
    let r = skipping(&["B1"], || h.phase_b(&fake, 0));
    fails_at(
        r,
        "SOURCE_EDIT_GATE_FAILED",
        "B2",
        "original content",
        "GATE-18 B2",
    );
    let r = skipping(&["B1", "B2"], || h.phase_b(&fake, 0));
    fails_at(
        r,
        "SOURCE_EDIT_GATE_FAILED",
        "B3",
        "show records",
        "GATE-18 B3",
    );
}

#[test]
fn gate_19_cover_and_overlay_f2_real_qpdf_overlay() {
    let Some(h) = honest("gate_19") else { return };
    let fake = h.path("fake.pdf");
    fakes::overlay_f2(
        h.qpdf(),
        &h.source,
        &fake,
        1,
        MEDIA,
        cover_rect(&h),
        "Invoice 2027",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&fake),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[0]",
        "GATE-19",
    );
    let r = skipping(&["A2"], || h.phase_a(&fake));
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A3",
        "original content is still attached",
        "GATE-19 A3",
    );
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&fake));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "", "GATE-19 A5");
    // The wrapper holds the old parts: neither form of B1 finds the expected content.
    fails_at(
        h.phase_b(&fake, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "",
        "GATE-19 B1",
    );
    let r = skipping(&["B1"], || h.phase_b(&fake, 0));
    fails_at(
        r,
        "SOURCE_EDIT_GATE_FAILED",
        "B2",
        "original content",
        "GATE-19 B2",
    );
}

#[test]
fn gate_20_raster_fake() {
    let Some(h) = honest("gate_20") else { return };
    let fake = h.path("fake.pdf");
    fakes::raster_fake(
        h.qpdf(),
        &h.engines.pdftoppm,
        &h.staged,
        &h.source,
        &fake,
        0,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(h.phase_a(&fake), "EDIT_VERIFY_FAILED", "A2", "", "GATE-20");
    let r = skipping(&["A2"], || h.phase_a(&fake));
    fails_at(r, "EDIT_VERIFY_FAILED", "A3", "bytes differ", "GATE-20 A3");
    let r = skipping(&["A2", "A3"], || h.phase_a(&fake));
    fails_at(r, "EDIT_VERIFY_FAILED", "A4", "", "GATE-20 A4");
}

/// `qpdf --check` for `validate_staged_pdf`.
fn check_runner(
    qpdf: &Path,
) -> impl FnMut(&[String]) -> Result<(i32, String), crate::error::AppError> + '_ {
    move |args| {
        let out = std::process::Command::new(qpdf)
            .args(args)
            .output()
            .expect("qpdf");
        Ok((
            out.status.code().unwrap_or(2),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
    }
}

fn snapshot_with(h: &Honest, digest: ContentDigest) -> OutputSnapshot {
    OutputSnapshot {
        pages: vec![PageSnapshot {
            media_box: MEDIA,
            crop_box: None,
            trim_box: None,
            rotate: 0,
            user_unit: 1.0,
            content_digest: digest,
        }],
        catalog: catalog_flags_from_doc(h.ctx.doc()),
    }
}

#[test]
fn gate_21_phase_b_composes_with_34() {
    let Some(h) = honest("gate_21") else { return };
    let copy = |name: &str, from: &Path| {
        let p = h.path(name);
        std::fs::copy(from, &p).expect("copy");
        p
    };
    // #34 with the source page digest refuses the honest edit (the content changed) …
    let source_digest = h.model.content.concat_digest();
    let staged = copy("honest-34.pdf", &h.staged);
    let r = validate_staged_pdf(
        &staged,
        &snapshot_with(&h, source_digest),
        None,
        check_runner(h.qpdf()),
    );
    assert!(
        r.is_err(),
        "GATE-21 #34 with the source digest refuses an honest edit"
    );
    // … and passes it with the proof's independently computed digest.
    let staged = copy("honest-proof.pdf", &h.staged);
    let r = validate_staged_pdf(
        &staged,
        &snapshot_with(&h, h.proof().expected_page_digest),
        None,
        check_runner(h.qpdf()),
    );
    assert!(
        r.is_ok(),
        "GATE-21 #34 with the proof digest: {:?}",
        r.err().map(|e| e.details)
    );
    assert!(
        h.phase_b(&h.staged, 0).is_ok(),
        "GATE-21 Phase B passes the honest edit"
    );
    // GATE-18's fake keeps the source stream: #34 alone passes it, Phase B does not.
    let fake = h.path("fake.pdf");
    fakes::cover_and_overlay_f1(
        h.qpdf(),
        &h.source,
        &fake,
        0,
        cover_rect(&h),
        "F1",
        "Invoice 2027",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let staged = copy("fake-34.pdf", &fake);
    let r = validate_staged_pdf(
        &staged,
        &snapshot_with(&h, source_digest),
        None,
        check_runner(h.qpdf()),
    );
    assert!(
        r.is_ok(),
        "GATE-21 #34 alone passes the fake: {:?}",
        r.err().map(|e| e.details)
    );
    fails_at(
        h.phase_b(&fake, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B1",
        "",
        "GATE-21 Phase B",
    );
}

#[test]
fn gate_22_freetext_annotation_over_the_old_text() {
    let Some(h) = honest("gate_22") else { return };
    let fake = h.path("fake.pdf");
    let c = cover_rect(&h);
    fakes::freetext(
        h.qpdf(),
        &h.staged,
        &fake,
        0,
        [c[0], c[1], c[0] + c[2], c[1] + c[3]],
        "Invoice 2027",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&fake),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[0]",
        "GATE-22",
    );
}

#[test]
fn gate_24_old_stream_reattached_as_a_form() {
    let Some(h) = honest("gate_24") else { return };
    let fake = h.path("fake.pdf");
    let old = h.model.content.part_bytes(0).to_vec();
    fakes::reattach_old(h.qpdf(), &h.staged, &fake, 0, &old).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&fake),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[0]",
        "GATE-24",
    );
    let r = skipping(&["A2"], || h.phase_a(&fake));
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A3",
        "original content is still attached",
        "GATE-24 A3",
    );
    // Phase B sees it too: B1 finds the expected parts, B2 the old stream in the resources.
    fails_at(
        h.phase_b(&fake, 0),
        "SOURCE_EDIT_GATE_FAILED",
        "B2",
        "original content",
        "GATE-24 B2",
    );
}
