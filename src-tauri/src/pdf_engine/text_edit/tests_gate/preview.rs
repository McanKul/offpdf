//! The preview core (§B.17) under the same Phase A as Save: a verified one-page PDF for an honest
//! edit, no PDF for a failed verdict, files written once in the cache directory and every nonce
//! directory removed. (The PREV-01…07 matrix through the command layer is T5's.)

use super::opts;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::preview::{cache_dir_for, preview_page};
use crate::pdf_engine::text_edit::reasons::{EditProblemCode, TextWarningCode};
use crate::pdf_engine::text_edit::rewrite::{SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use std::sync::Arc;

fn edit_of(m: &crate::pdf_engine::text_edit::runs::PageModel, old: &str, new: &str) -> TextEditIn {
    let run = m.runs.iter().find(|r| r.text == old).expect("run");
    TextEditIn {
        run_id: run.id.clone(),
        original_text: old.into(),
        text: new.into(),
        style: SourceTextStyleIn::default(),
    }
}

#[test]
fn preview_core_returns_a_verified_page_and_cleans_up() {
    let Some(engines) = engines_or_skip("preview_core") else {
        return;
    };
    let dir = Scratch::new("preview_core");
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello world) Tj ET BT /F1 12 Tf 72 650 Td (Second line) Tj ET",
    );
    let path = dir.write("doc.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let m = Arc::new(build_page_model(&ctx, 0, None).expect("model"));
    let cache = cache_dir_for(dir.dir(), &ctx.snap.fingerprint.to_string());
    for round in 0..2 {
        let r = preview_page(
            &ctx,
            Arc::clone(&m),
            &[edit_of(&m, "Hello world", "Hello there")],
            &cache,
            &engines,
            &[],
            &opts(),
        )
        .unwrap_or_else(|e| panic!("preview: {e} {:?}", e.details));
        assert!(
            r.page_problem.is_none() && r.verdicts[0].problem.is_none(),
            "round {round}: {:?}",
            r.page_problem
        );
        assert!(r.warnings.is_empty(), "round {round}: {:?}", r.warnings);
        let bytes = r.pdf.expect("a preview page");
        let out = dir.write("preview.pdf", &bytes);
        let text = std::process::Command::new(&engines.pdftotext)
            .arg(&out)
            .arg("-")
            .output()
            .expect("pdftotext");
        let text = String::from_utf8_lossy(&text.stdout);
        assert!(
            text.contains("Hello there") && !text.contains("Hello world"),
            "{text}"
        );
    }
    // `source.pdf` and `p1.pdf` were written once; no nonce directory is left behind.
    let mut names: Vec<String> = std::fs::read_dir(&cache)
        .expect("cache dir")
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    names.sort();
    assert_eq!(names, vec!["p1.pdf".to_string(), "source.pdf".to_string()]);
}

#[test]
fn preview_core_failed_verdict_returns_no_pdf() {
    let Some(engines) = engines_or_skip("preview_core_fail") else {
        return;
    };
    let dir = Scratch::new("preview_core_fail");
    let pdf = fx::subset_without_y();
    let path = dir.write("doc.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let m = Arc::new(build_page_model(&ctx, 0, None).expect("model"));
    let cache = cache_dir_for(dir.dir(), "fp");
    let r = preview_page(
        &ctx,
        Arc::clone(&m),
        &[edit_of(&m, "Hello", "Yellow")],
        &cache,
        &engines,
        &[],
        &opts(),
    )
    .expect("preview");
    assert!(r.pdf.is_none(), "no bytes for a failed verdict");
    assert_eq!(
        r.verdicts[0].problem.as_ref().map(|p| p.code),
        Some(EditProblemCode::GlyphMissing)
    );
    assert!(!cache.exists(), "nothing written before a plan exists");
}

#[test]
fn preview_core_reports_the_overlap_warning_of_its_verdict() {
    let Some(engines) = engines_or_skip("preview_core_overlap") else {
        return;
    };
    let dir = Scratch::new("preview_core_overlap");
    // "Hi" and "there" on one baseline, "there" 23 pt from the origin: "Hiya" (24 pt) runs 1 pt
    // into it (the §A.8 warning; the line still fits the page).
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET BT /F1 12 Tf 95 700 Td (there) Tj ET \
          BT /F1 12 Tf 72 650 Td (Next line) Tj ET",
    );
    let path = dir.write("doc.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let m = Arc::new(build_page_model(&ctx, 0, None).expect("model"));
    let edit = edit_of(&m, "Hi", "Hiya");
    let cache = cache_dir_for(dir.dir(), "overlap");
    let r = preview_page(
        &ctx,
        Arc::clone(&m),
        &[edit.clone()],
        &cache,
        &engines,
        &[],
        &opts(),
    )
    .unwrap_or_else(|e| panic!("preview: {e} {:?}", e.details));
    assert!(r.pdf.is_some(), "a warning does not block the preview");
    assert_eq!(
        r.verdicts[0].warnings,
        vec![TextWarningCode::NextTextOverlap],
        "the verdict carries the warning"
    );
    let w: Vec<(u32, &str, TextWarningCode)> = r
        .warnings
        .iter()
        .map(|w| (w.page_index, w.run_id.as_str(), w.code))
        .collect();
    assert_eq!(
        w,
        vec![(0, edit.run_id.as_str(), TextWarningCode::NextTextOverlap)],
        "the preview lists it for the page and the run"
    );
}
