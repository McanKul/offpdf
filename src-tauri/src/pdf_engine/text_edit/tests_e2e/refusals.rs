//! E2E-12…16, 19…21 and GATE-25: style changes, refusals that surface at Save, a source changed
//! after inspect, overwrite protection, cancellation, a missing verifier, appended signed files,
//! the verification read cap, and a fake injected into the edited copy.

use super::fonts::{assert_same_box, word_box};
use super::save::pages_doc;
use super::{E2e, SaveOpts};
use crate::error::AppError;
use crate::pdf_engine::qpdf::resolve_qpdf_standalone;
use crate::pdf_engine::text_edit::export::seams;
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::{Face, ORIGINAL_UNCHANGED};
use crate::pdf_engine::text_edit::snapshot::read_snapshot;
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, DocBuilder, PageSpec, HELVETICA,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// A save error carries the unchanged-file sentence exactly once, at the end.
pub(crate) fn assert_save_error(e: &AppError, code: &str) {
    assert_eq!(e.code, code, "{e} {:?}", e.details);
    let s = e.suggestion.as_deref().unwrap_or_default();
    assert!(s.ends_with(ORIGINAL_UNCHANGED), "{code}: suggestion {s:?}");
    assert_eq!(s.matches(ORIGINAL_UNCHANGED).count(), 1, "{code}: {s:?}");
}

/// A `sourceText` object built by hand (for runs the editor would never offer).
fn raw_edit(fp: &str, run_id: &str, old: &str, new: &str) -> Value {
    json!({
        "id": "raw", "kind": "sourceText", "pageIndex": 0,
        "rect": { "x": 72.0, "y": 700.0, "w": 100.0, "h": 12.0 },
        "locked": true, "runId": run_id, "sourceFingerprint": fp, "sourcePageIndex": 0,
        "originalText": old, "text": new, "style": {},
    })
}

/// Helvetica and its bold sibling, a styled line and a follower line.
fn styled_doc() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let r = d.add(HELVETICA);
    let b = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Styled line) Tj ET BT /F1 12 Tf 72 670 Td (Next line) Tj ET \
          BT /F2 12 Tf 72 640 Td (Bold) Tj ET",
        &format!("/Font << /F1 {r} 0 R /F2 {b} 0 R >>"),
    ));
    d.build()
}

#[test]
fn e2e_12_size_bold_red_and_spacing_on_one_line() {
    let Some(t) = E2e::new("e2e_12") else {
        return;
    };
    let src = t.file("styled.pdf", &styled_doc());
    let next_before = word_box(&t, &src, 0, "Next");
    let (_, next_run) = t.run(&src, 0, "Next line");
    let style =
        json!({ "sizePt": 14.0, "face": "bold", "fill": "#c71c1c", "letterSpacingPt": 0.5 });
    let obj = t.edit_styled(&src, 0, 0, "Styled line", "Styled line", style);
    let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("E2E-12 save");
    t.assert_checks_clean(&saved.path);
    assert_same_box(
        next_before,
        word_box(&t, &saved.path, 0, "Next"),
        "E2E-12 next line",
    );
    let (_, run) = t.run(&saved.path, 0, "Styled line");
    let m = run.metrics.as_ref().expect("metrics");
    let s = run.style.as_ref().expect("style");
    assert!(
        (m.effective_size - 14.0).abs() < 1e-6,
        "E2E-12 size {}",
        m.effective_size
    );
    assert!(
        (m.letter_spacing_pt - 0.5).abs() < 1e-4,
        "E2E-12 spacing {}",
        m.letter_spacing_pt
    );
    assert_eq!(s.face, Face::Bold, "E2E-12 face");
    assert_eq!(s.fill.as_deref(), Some("#c71c1c"), "E2E-12 fill");
    let (_, next_after) = t.run(&saved.path, 0, "Next line");
    let style_of =
        |r: &crate::pdf_engine::text_edit::dto::TextRunDto| r.style.clone().map(|s| s.fill);
    assert_eq!(
        style_of(&next_after),
        style_of(&next_run),
        "E2E-12 next line colour"
    );
    assert_eq!(next_after.rect, next_run.rect, "E2E-12 next line box");
}

/// Page 1 is fine; page 2's content has an unterminated string: `qpdf --check` warns about it
/// (not on the benign allow-list), our page-1 walk does not care.
pub(crate) fn needs_repair_doc() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Good line) Tj ET",
        &res,
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (open string Tj ET",
        &res,
    ));
    d.build()
}

#[test]
fn e2e_13_refusals_surface_at_save() {
    let Some(t) = E2e::new("e2e_13") else {
        return;
    };
    // A shared content stream: refused at inspect and at Save.
    let src = t.file("shared.pdf", &fx::shared());
    let (fp, run) = t.run(&src, 0, "Shared body");
    assert!(!run.editable);
    let e = t
        .save(
            &[(&src, "1-z")],
            vec![raw_edit(&fp, &run.id, "Shared body", "Shared text")],
        )
        .err()
        .expect("shared");
    assert_save_error(&e, "TEXT_EDIT_REFUSED");
    assert!(
        e.details
            .as_deref()
            .unwrap_or_default()
            .contains("SHARED_CONTENT"),
        "{e:?}"
    );
    // File-level refusals (the editor refuses these files at open; Save refuses them again).
    let enc = fx::encrypted(&t.engines);
    for (name, pdf, code) in [
        ("encrypted.pdf", enc, "ENCRYPTED"),
        ("signed.pdf", fx::signed(), "SIGNED"),
        ("xfa.pdf", fx::xfa(), "UNSUPPORTED_XFA"),
    ] {
        let src = t.file(name, &pdf);
        let fp = format!("{:016x}-{:x}", 1u64, pdf.len());
        let e = t
            .save(
                &[(&src, "1-z")],
                vec![raw_edit(&fp, "t1:x:0:0-1", "Hello", "Help")],
            )
            .err()
            .unwrap_or_else(|| panic!("{name} saved"));
        assert_save_error(&e, code);
    }
    // qpdf --check finds a real problem elsewhere in the file.
    let src = t.file("repair.pdf", &needs_repair_doc());
    let obj = t.edit(&src, 0, 0, "Good line", "Good lines");
    let e = t.save(&[(&src, "1-z")], vec![obj]).err().expect("repair");
    assert_save_error(&e, "PDF_NEEDS_REPAIR");
}

#[test]
fn e2e_14_source_changed_after_inspect_is_stale() {
    let Some(t) = E2e::new("e2e_14") else {
        return;
    };
    let src = t.file("stale.pdf", &pages_doc(&["Before change"]));
    let obj = t.edit(&src, 0, 0, "Before change", "After change");
    std::fs::write(&src, pages_doc(&["Rewritten line"])).expect("rewrite");
    let e = t.save(&[(&src, "1-z")], vec![obj]).err().expect("stale");
    assert_save_error(&e, "STALE");
    assert_eq!(
        e.message,
        "\u{201c}stale.pdf\u{201d} was changed after you started editing it, so your text changes no longer match it."
    );
}

#[test]
fn e2e_15_destination_is_the_source_or_a_hard_link_to_it() {
    let Some(t) = E2e::new("e2e_15") else {
        return;
    };
    let src = t.file("own.pdf", &pages_doc(&["Own line"]));
    let link = t.scratch.path("link.pdf");
    std::fs::hard_link(&src, &link).expect("hard link");
    for dest in [src.clone(), link] {
        let obj = t.edit(&src, 0, 0, "Own line", "Own lines");
        let opts = SaveOpts {
            dest: Some(dest),
            ..SaveOpts::default()
        };
        let e = t
            .save_with(&[(&src, "1-z")], vec![obj], opts)
            .err()
            .expect("overwrite");
        assert_eq!(e.code, "OVERWRITE", "{e}");
    }
}

static CANCEL_16: AtomicBool = AtomicBool::new(false);

fn cancel_now(_edited: &Path) {
    CANCEL_16.store(true, Ordering::SeqCst);
}

#[test]
fn e2e_16_cancel_during_and_after_the_text_changes() {
    let Some(t) = E2e::new("e2e_16") else {
        return;
    };
    let src = t.file("cancel.pdf", &pages_doc(&["Cancel one", "Cancel two"]));
    // While the edited copy is being proven (Phase A sees the cancel).
    CANCEL_16.store(false, Ordering::SeqCst);
    seams::set_tamper_hook(Some(cancel_now));
    let obj = t.edit(&src, 0, 0, "Cancel one", "Cancel 1");
    let opts = SaveOpts {
        cancel: Some(&CANCEL_16),
        ..SaveOpts::default()
    };
    let r = t.save_with(&[(&src, "1-z")], vec![obj], opts);
    seams::set_tamper_hook(None);
    assert_eq!(
        r.err().map(|e| e.code),
        Some("CANCELLED".to_string()),
        "E2E-16 Phase A"
    );
    // After the text changes, when the pipeline assembles (Phase B / #34 see the cancel).
    let flag = AtomicBool::new(false);
    let cancel_on_run = |_: &[String]| flag.store(true, Ordering::SeqCst);
    let obj = t.edit(&src, 0, 0, "Cancel one", "Cancel 1");
    let opts = SaveOpts {
        cancel: Some(&flag),
        on_run: Some(&cancel_on_run),
        ..SaveOpts::default()
    };
    let r = t.save_with(&[(&src, "1"), (&src, "2")], vec![obj], opts);
    assert_eq!(
        r.err().map(|e| e.code),
        Some("CANCELLED".to_string()),
        "E2E-16 later"
    );
}

#[test]
fn e2e_19_missing_poppler_is_verifier_missing() {
    let Some(t) = E2e::new("e2e_19") else {
        return;
    };
    let src = t.file("nopoppler.pdf", &pages_doc(&["Needs poppler"]));
    let obj = t.edit(&src, 0, 0, "Needs poppler", "Needs Poppler");
    seams::set_poppler_override(Some(PathBuf::from("/nonexistent/offpdf-poppler")));
    let r = t.save(&[(&src, "1-z")], vec![obj]);
    seams::set_poppler_override(None);
    assert_save_error(&r.err().expect("verifier missing"), "VERIFIER_MISSING");
}

#[test]
fn e2e_20_an_unedited_signed_pdf_may_be_appended() {
    let Some(t) = E2e::new("e2e_20") else {
        return;
    };
    let src = t.file("plain.pdf", &pages_doc(&["Plain line"]));
    let signed = t.file("signed.pdf", &fx::signed());
    assert_eq!(
        read_snapshot(&signed).err().map(|e| e.code),
        Some("SIGNED".into())
    );
    let obj = t.edit(&src, 0, 0, "Plain line", "Plain lines");
    let saved = t
        .save(&[(&src, "1-z"), (&signed, "1-z")], vec![obj])
        .unwrap_or_else(|e| panic!("E2E-20 save: {e} {:?}", e.details));
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Plain line", "Plain lines")]);
    t.assert_unedited(&signed, 0, &saved.path, 1);
}

#[test]
fn e2e_21_the_final_file_may_exceed_the_source_cap() {
    let Some(t) = E2e::new("e2e_21") else {
        return;
    };
    let a = pages_doc(&["Capped one"]);
    let b = pages_doc(&["Capped two", "Capped three"]);
    let cap = a.len().max(b.len()) as u64 + 64;
    let src_a = t.file("cap-a.pdf", &a);
    let src_b = t.file("cap-b.pdf", &b);
    let objects = vec![
        t.edit(&src_a, 0, 0, "Capped one", "Capped 1"),
        t.edit(&src_b, 2, 1, "Capped three", "Capped 3"),
    ];
    limits::set_file_cap_override(Some(cap));
    let r = t.save(&[(&src_a, "1-z"), (&src_b, "1-z")], objects);
    let alone = r
        .as_ref()
        .ok()
        .map(|s| read_snapshot(&s.path).err().map(|e| e.code));
    limits::set_file_cap_override(None);
    let saved = r.unwrap_or_else(|e| panic!("E2E-21 save: {e} {:?}", e.details));
    assert_eq!(
        alone,
        Some(Some("FILE_TOO_LARGE".to_string())),
        "E2E-21 read_snapshot alone"
    );
    t.assert_saved(
        &saved.path,
        &[
            (&src_a, 0, 0, "Capped one", "Capped 1"),
            (&src_b, 1, 2, "Capped three", "Capped 3"),
        ],
    );
}

fn tamper_cover(edited: &Path) {
    let qpdf = resolve_qpdf_standalone();
    let fake = edited.with_extension("fake.pdf");
    fakes::cover_and_overlay_f1(
        &qpdf,
        edited,
        &fake,
        0,
        [70.0, 695.0, 200.0, 20.0],
        "F1",
        "Gate line",
    )
    .expect("cover fake");
    std::fs::rename(&fake, edited).expect("swap in the fake");
}

fn tamper_reattach(edited: &Path) {
    let qpdf = resolve_qpdf_standalone();
    let fake = edited.with_extension("fake.pdf");
    fakes::reattach_old(
        &qpdf,
        edited,
        &fake,
        0,
        b"BT /F1 12 Tf 72 700 Td (Gate line) Tj ET",
    )
    .expect("reattach fake");
    std::fs::rename(&fake, edited).expect("swap in the fake");
}

#[test]
fn gate_25_fakes_injected_into_the_edited_copy_never_publish() {
    let Some(t) = E2e::new("gate_25") else {
        return;
    };
    let src = t.file("gate.pdf", &pages_doc(&["Gate line", "Other gate page"]));
    for hook in [tamper_cover as fn(&Path), tamper_reattach] {
        let obj = t.edit(&src, 0, 0, "Gate line", "Gate lines");
        seams::set_tamper_hook(Some(hook));
        let r = t.save(&[(&src, "1-z")], vec![obj]);
        seams::set_tamper_hook(None);
        let e = r.err().expect("GATE-25: a fake was published");
        assert!(
            [
                "EDIT_VERIFY_FAILED",
                "PEN_DRIFT",
                "STATE_CHANGED",
                "SOURCE_EDIT_GATE_FAILED"
            ]
            .contains(&e.code.as_str()),
            "GATE-25 code {e} {:?}",
            e.details
        );
        assert_save_error(&e, &e.code.clone());
        assert!(
            e.details.as_deref().unwrap_or_default().contains("phase=A"),
            "{:?}",
            e.details
        );
    }
}
