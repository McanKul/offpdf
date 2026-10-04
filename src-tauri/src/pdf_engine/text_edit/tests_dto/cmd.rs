//! CMD-01…08: the command layer (`service`, as the Tauri commands call it) with real qpdf and
//! Poppler: open refusals, stale fingerprints, verdicts that are data (not errors), page
//! indices, release and eviction of the temporary folders, the classifier wiring, and the
//! background `qpdf --check`.

use super::{contract, key_paths, kind_paths};
use crate::pdf_engine::source_content::{classify_source_page, SourceCapability};
use crate::pdf_engine::text_edit::cache::seams as cache_seams;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::SourceCheck;
use crate::pdf_engine::text_edit::preview::cache_dir_for;
use crate::pdf_engine::text_edit::reasons::{EditProblemCode, Face, StyleField, TextReason};
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::service;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::pdf::zlib_zero_bomb;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::tests_e2e::preview::edit_in;
use crate::pdf_engine::text_edit::tests_e2e::refusals::needs_repair_doc;
use crate::pdf_engine::text_edit::tests_e2e::save::pages_doc;
use crate::pdf_engine::text_edit::tests_e2e::E2e;
use std::collections::BTreeSet;
use std::path::Path;

fn code_of<T>(r: Result<T, crate::error::AppError>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(e) => e.code,
    }
}

#[test]
fn cmd_01_open_refusals_and_the_synchronous_page_map_check() {
    let Some(t) = E2e::new("cmd_01") else {
        return;
    };
    let ok = t.file("ok.pdf", &pages_doc(&["One", "Two"]));
    let info = t.open(&ok);
    assert_eq!(info.page_count, 2);
    assert!(info.warnings.is_empty());
    assert_eq!(
        info.fingerprint.len(),
        16 + 1 + format!("{:x}", std::fs::metadata(&ok).expect("meta").len()).len()
    );
    let cases = [
        ("enc.pdf", fx::encrypted(&t.engines), "ENCRYPTED"),
        ("signed.pdf", fx::signed(), "SIGNED"),
        ("xfa.pdf", fx::xfa(), "UNSUPPORTED_XFA"),
        // lopdf and qpdf disagree about the pages: refused before anything is inspected.
        ("kid.pdf", fx::kid_without_type(), "PDF_NEEDS_REPAIR"),
        ("hybrid.pdf", fx::hybrid_xref(false), "PDF_NEEDS_REPAIR"),
    ];
    for (name, pdf, code) in cases {
        let p = t.file(name, &pdf);
        assert_eq!(code_of(t.try_open(&p)), code, "CMD-01 {name}");
    }
}

#[test]
fn cmd_02_a_wrong_or_outdated_fingerprint_is_stale() {
    let Some(t) = E2e::new("cmd_02") else {
        return;
    };
    let p = t.file("stale.pdf", &pages_doc(&["Before"]));
    let fp = t.open(&p).fingerprint;
    assert_eq!(code_of(t.try_inspect(&p, "0000000000000000-1", 0)), "STALE");
    std::fs::write(&p, pages_doc(&["After!"])).expect("rewrite");
    let e = t.try_inspect(&p, &fp, 0).err().expect("stale");
    assert_eq!(e.code, "STALE");
    assert_eq!(
        e.message,
        "\u{201c}stale.pdf\u{201d} was changed after you started editing it, so your text changes no longer match it."
    );
    // The reloaded snapshot serves the new fingerprint.
    let fresh = t.open(&p).fingerprint;
    assert_ne!(fresh, fp);
    assert!(t.try_inspect(&p, &fresh, 0).is_ok());
}

#[test]
fn cmd_03_user_problems_are_verdicts_not_errors() {
    let Some(t) = E2e::new("cmd_03") else {
        return;
    };
    let word = t.file("word.pdf", &fx::word());
    let p = t.preview(
        &word,
        0,
        &[(
            "Invoice 2026",
            "Invoice 2026Y",
            SourceTextStyleIn::default(),
        )],
    );
    let v = &p.verdicts[0];
    assert_eq!((v.ok, v.code), (false, Some(EditProblemCode::GlyphMissing)));
    assert_eq!(v.chars, vec!["Y".to_string()]);
    let italic = SourceTextStyleIn {
        face: Some(Face::Italic),
        ..SourceTextStyleIn::default()
    };
    let p = t.preview(&word, 0, &[("Due", "Due", italic)]);
    let v = &p.verdicts[0];
    assert_eq!(
        (v.code, v.face),
        (Some(EditProblemCode::FaceUnavailable), Some(Face::Italic))
    );
    let gs = t.file("gs.pdf", &fx::extgstate_font());
    let bigger = SourceTextStyleIn {
        size_pt: Some(20.0),
        ..SourceTextStyleIn::default()
    };
    let p = t.preview(&gs, 0, &[("Hi there", "Hi there", bigger)]);
    let v = &p.verdicts[0];
    assert_eq!(
        (v.code, v.field),
        (
            Some(EditProblemCode::StyleUnavailable),
            Some(StyleField::Size)
        )
    );
    // A refused run: the verdict names its reason.
    let indd = t.file("indd.pdf", &fx::indd());
    let (fp, run) = t.run(&indd, 0, "hidden layer");
    let p = service::preview_edits(
        &t.cache,
        &t.engines,
        &t.temp_root(),
        &indd.to_string_lossy(),
        &fp,
        0,
        &[edit_in(
            &run.id,
            "hidden layer",
            "hidden layers",
            SourceTextStyleIn::default(),
        )],
    )
    .expect("CMD-03 refused preview is not an error");
    let v = &p.verdicts[0];
    assert_eq!(
        (v.code, v.reason),
        (
            Some(EditProblemCode::TextEditRefused),
            Some(TextReason::OptionalContent)
        )
    );
    assert!(p.page_pdf.is_none());
}

#[test]
fn cmd_04_a_page_the_file_does_not_have_is_invalid_pages() {
    let Some(t) = E2e::new("cmd_04") else {
        return;
    };
    let p = t.file("one.pdf", &pages_doc(&["Only page"]));
    let fp = t.open(&p).fingerprint;
    let e = t.try_inspect(&p, &fp, 5).err().expect("no page 6");
    assert_eq!(e.code, "INVALID_PAGES");
}

/// Waits for the background check of `path` (so its folder is not busy).
pub(super) fn settle(t: &E2e, path: &Path) {
    let src = t
        .cache
        .open(path, &t.temp_root(), &t.engines)
        .expect("open");
    let check = t.cache.source_check(&src, &t.engines).expect("check");
    assert!(matches!(check.wait(None), Ok(SourceCheck::Clean)));
}

#[test]
fn cmd_05_06_release_and_eviction_delete_the_temporary_folder() {
    let Some(t) = E2e::new("cmd_05") else {
        return;
    };
    let temp = t.temp_root();
    let a = t.file("a.pdf", &pages_doc(&["File a"]));
    let fp_a = t.open(&a).fingerprint;
    let dir_a = cache_dir_for(&temp, &fp_a);
    assert!(dir_a.join("source.pdf").is_file(), "CMD-05 snapshot copy");
    settle(&t, &a);
    service::release_source(&t.cache, &a.to_string_lossy());
    assert_eq!(t.cache.cached_len(), 0, "CMD-05 forgotten");
    assert!(
        !dir_a.exists(),
        "CMD-06 release deletes {}",
        dir_a.display()
    );
    // Reopening works; a preview fills the folder again.
    let fp_a = t.open(&a).fingerprint;
    assert!(t.try_inspect(&a, &fp_a, 0).is_ok());
    let _ = t.preview(&a, 0, &[("File a", "File A", SourceTextStyleIn::default())]);
    assert!(
        dir_a.join("p1.pdf").is_file(),
        "CMD-06 preview extraction cached"
    );
    settle(&t, &a);
    // Two more sources evict the first (CACHE_SNAPSHOTS_MAX = 2) and its folder goes.
    let b = t.file("b.pdf", &pages_doc(&["File b"]));
    let c = t.file("c.pdf", &pages_doc(&["File c"]));
    t.open(&b);
    settle(&t, &b);
    t.open(&c);
    assert_eq!(t.cache.cached_len(), 2);
    assert!(
        !dir_a.exists(),
        "CMD-06 eviction deletes {}",
        dir_a.display()
    );
    // A second path with the same bytes keeps the folder until both are released.
    let b2 = t.file("b-copy.pdf", &pages_doc(&["File b"]));
    let fp_b = t.open(&b2).fingerprint;
    let dir_b = cache_dir_for(&temp, &fp_b);
    settle(&t, &b2);
    service::release_source(&t.cache, &b.to_string_lossy());
    assert!(dir_b.is_dir(), "CMD-06 shared folder kept");
    service::release_source(&t.cache, &b2.to_string_lossy());
    assert!(
        !dir_b.exists(),
        "CMD-06 shared folder deleted with the last user"
    );
}

#[test]
fn cmd_07_run_capabilities_come_from_the_classifier() {
    let Some(t) = E2e::new("cmd_07") else {
        return;
    };
    let fixtures = [
        ("word", fx::word()),
        ("indd", fx::indd()),
        ("shared", fx::shared()),
        ("perglyph", fx::per_glyph(Some(9.0))),
        ("type3", fx::type3()),
        ("ocr", fx::ocr()),
        ("skia", fx::skia()),
    ];
    for (name, pdf) in fixtures {
        let path = t.file(&format!("{name}.pdf"), &pdf);
        let pages = t.open(&path).page_count;
        let snap = snapshot_from_bytes(&path, pdf.clone(), None).expect("snapshot");
        let ctx = SnapshotContext::new(snap);
        for page in 0..pages {
            let (_, dto) = t.page(&path, page);
            let model = build_page_model(&ctx, page, None).expect("model");
            let classified = classify_source_page(&ctx, &model, None);
            assert_eq!(
                dto.page_reason, classified.page_reason,
                "CMD-07 {name} p{page}"
            );
            // The page's runs, then its Form lines (refused `NESTED_FORM`, review-T5 live B1).
            assert_eq!(
                dto.runs.len(),
                classified.runs.len() + classified.form_lines.len(),
                "CMD-07 {name} p{page}"
            );
            let (own, forms) = dto.runs.split_at(classified.runs.len());
            for (run, line) in forms.iter().zip(&classified.form_lines) {
                assert_eq!(
                    (&run.id, &run.text),
                    (&line.id, &line.text),
                    "CMD-07 {name}"
                );
                assert!(!run.editable && run.reason == Some(TextReason::NestedForm));
            }
            for (run, cap) in own.iter().zip(&classified.runs) {
                assert_eq!(run.id, cap.run_id, "CMD-07 {name}");
                assert_eq!(
                    run.editable,
                    cap.capability == SourceCapability::Supported,
                    "CMD-07 {name} {:?}",
                    run.text
                );
                assert_eq!(run.reason, cap.reason, "CMD-07 {name} {:?}", run.text);
                assert_eq!(run.metrics.is_some(), run.editable);
                assert_eq!(run.style.is_some(), run.editable);
            }
        }
    }
}

/// A one-page file whose catalog references a stream of `mib` MiB of zeros: our read never
/// decodes it, `qpdf --check` does.
fn slow_check_doc(mib: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let bomb = d.b.add_stream("/Filter /FlateDecode", &zlib_zero_bomb(mib));
    d.catalog_extra
        .push_str(&format!(" /PieceInfo << /OffPDF << /Data {bomb} 0 R >> >>"));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Slow check) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

#[test]
fn cmd_08_open_does_not_wait_for_qpdf_check_and_the_preview_does() {
    let Some(t) = E2e::new("cmd_08") else {
        return;
    };
    let slow = t.file("slow.pdf", &slow_check_doc(1024));
    // On Unix the background check is also held until the test lets it go, so "open returned
    // before the check finished" does not depend on how long qpdf takes (review-T5 L6).
    let gate = t.scratch.path("go");
    #[cfg(unix)]
    cache_seams::set_background_qpdf(Some(super::cache::held_qpdf(&t, &gate)));
    let info = t.open(&slow);
    cache_seams::set_background_qpdf(None);
    let src = t
        .cache
        .open(&slow, &t.temp_root(), &t.engines)
        .expect("cached");
    let check = t.cache.source_check(&src, &t.engines).expect("check");
    assert!(check.peek().is_none(), "CMD-08 open waited for the check");
    assert!(
        t.try_inspect(&slow, &info.fingerprint, 0).is_ok(),
        "CMD-08 inspect meanwhile"
    );
    assert!(
        check.peek().is_none(),
        "CMD-08 inspect waited for the check"
    );
    std::fs::write(&gate, b"").expect("gate");
    let waited = check.wait(None).expect("check result");
    assert!(
        matches!(waited, SourceCheck::Clean | SourceCheck::Benign(_)),
        "{waited:?}"
    );
    // A non-benign warning elsewhere in the file: inspect works, the first preview refuses.
    let repair = t.file("repair.pdf", &needs_repair_doc());
    let (fp, dto) = t.page(&repair, 0);
    let run = dto
        .runs
        .iter()
        .find(|r| r.text == "Good line")
        .expect("run");
    let e = service::preview_edits(
        &t.cache,
        &t.engines,
        &t.temp_root(),
        &repair.to_string_lossy(),
        &fp,
        0,
        &[edit_in(
            &run.id,
            "Good line",
            "Good lines",
            SourceTextStyleIn::default(),
        )],
    )
    .err()
    .expect("PDF_NEEDS_REPAIR");
    assert_eq!(e.code, "PDF_NEEDS_REPAIR", "{e}");
    assert!(
        e.details
            .as_deref()
            .unwrap_or_default()
            .contains("EOF while reading token"),
        "{:?}",
        e.details
    );
}

/// A real page's DTO has exactly the contract sample's key paths (DTO-01's link to real data).
#[test]
fn dto_01b_a_real_page_has_the_contract_keys() {
    let Some(t) = E2e::new("dto_01b") else {
        return;
    };
    let p = t.file("keys.pdf", &fx::indd());
    let (_, dto) = t.page(&p, 0);
    let real = serde_json::to_value(&dto).expect("json");
    let sample = &contract()["PageText"];
    let (mut a, mut b) = (BTreeSet::new(), BTreeSet::new());
    key_paths(&real, "", &mut a);
    key_paths(sample, "", &mut b);
    assert_eq!(a, b, "DTO-01b key paths");
    let (mut a, mut b) = (BTreeSet::new(), BTreeSet::new());
    kind_paths(&real, "", &mut a);
    kind_paths(sample, "", &mut b);
    let drift: Vec<_> = a.difference(&b).collect();
    assert!(
        drift.is_empty(),
        "DTO-01b value kinds not in the contract: {drift:?}"
    );
    let preview = t.preview(
        &p,
        0,
        &[(
            "visible layer",
            "visible layers",
            SourceTextStyleIn::default(),
        )],
    );
    let real = serde_json::to_value(&preview).expect("json");
    let (mut a, mut b) = (BTreeSet::new(), BTreeSet::new());
    key_paths(&real, "", &mut a);
    key_paths(&contract()["TextPreview"], "", &mut b);
    assert!(a.is_subset(&b), "DTO-01b preview keys {a:?} ⊄ {b:?}");
}
