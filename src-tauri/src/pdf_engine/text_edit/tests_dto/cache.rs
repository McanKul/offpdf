//! The editor cache under the review-T5 findings: a background `qpdf --check` that lost its input
//! (M1: "Clear temp files" during a check, and the harness deleting a folder under a running
//! check), a release during a preview (M2), several calls reading a large file at once (M3), and
//! page models sized by `walk.model_bytes` (fonts included).

use super::cmd::settle;
#[cfg(unix)]
use crate::pdf_engine::text_edit::cache::seams;
use crate::pdf_engine::text_edit::cache::{clear_stale_folders, TextEditCache};
use crate::pdf_engine::text_edit::preview::cache_dir_for;
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::service;
use crate::pdf_engine::text_edit::tests_e2e::save::pages_doc;
use crate::pdf_engine::text_edit::tests_e2e::E2e;
use std::path::Path;
use std::sync::{Arc, Barrier};

/// A qpdf for background checks that waits until `gate` exists, then runs the real qpdf.
#[cfg(unix)]
pub(crate) fn held_qpdf(t: &E2e, gate: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = t.scratch.path("held-qpdf.sh");
    let body = format!(
        "#!/bin/sh\nwhile [ ! -e '{}' ]; do sleep 0.02; done\nexec '{}' \"$@\"\n",
        gate.display(),
        t.engines.qpdf.display()
    );
    std::fs::write(&script, body).expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

/// review-T5 M1, the production trigger: "Clear temp files" deletes the folder before the
/// background check reads it. The check ends in an error (never a verdict about these bytes),
/// the preview starts it again on a fresh copy, and a Save of the same bytes is not refused.
#[cfg(unix)]
#[test]
fn m1_clear_temp_files_during_a_check_is_not_a_repair_verdict() {
    let Some(t) = E2e::new("m1_clear_temp") else {
        return;
    };
    let gate = t.scratch.path("go");
    let src = t.file("cleared.pdf", &pages_doc(&["Cleared line"]));
    seams::set_background_qpdf(Some(held_qpdf(&t, &gate)));
    let fp = t.open(&src).fingerprint;
    seams::set_background_qpdf(None);
    let dir = cache_dir_for(&t.temp_root(), &fp);
    std::fs::remove_dir_all(t.temp_root().join("textedit")).expect("clear temp files");
    std::fs::write(&gate, b"").expect("gate");
    t.cache.wait_checks();
    assert!(
        !dir.join("source.pdf").exists(),
        "the check ran on a missing file"
    );
    let p = t.preview(
        &src,
        0,
        &[(
            "Cleared line",
            "Cleared lines",
            SourceTextStyleIn::default(),
        )],
    );
    assert!(p.page_pdf.is_some(), "{:?}", p.page_problem);
    let saved = t
        .save(
            &[(&src, "1-z")],
            vec![t.edit(&src, 0, 0, "Cleared line", "Cleared lines")],
        )
        .unwrap_or_else(|e| panic!("save of the same bytes: {e} {:?}", e.details));
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Cleared line", "Cleared lines")],
    );
}

/// review-T5 M1, the test-harness side: dropping an `E2e` waits for its checks, so no test
/// deletes a folder under a running check.
#[cfg(unix)]
#[test]
fn m1_a_dropped_harness_leaves_no_check_running() {
    let Some(t) = E2e::new("m1_harness_drop") else {
        return;
    };
    let gate = t.scratch.path("go");
    let src = t.file("held.pdf", &pages_doc(&["Held check"]));
    seams::set_background_qpdf(Some(held_qpdf(&t, &gate)));
    t.open(&src);
    seams::set_background_qpdf(None);
    assert_eq!(t.cache.running_checks(), 1, "the check is held");
    let cache = t.cache.clone();
    let opener = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::fs::write(gate, b"").expect("gate");
    });
    drop(t);
    assert_eq!(cache.running_checks(), 0, "a check outlived its harness");
    opener.join().expect("gate thread");
}

/// review-T5 M2: a release during a preview. The preview still works (it writes its copies
/// into the leased folder), and the folder is gone when the preview ends: no copy of the user's
/// file outlives the session.
#[test]
fn m2_release_during_a_preview_leaves_no_copy() {
    let Some(t) = E2e::new("m2_release_preview") else {
        return;
    };
    let src = t.file("leased.pdf", &pages_doc(&["Leased line"]));
    let fp = t.open(&src).fingerprint;
    settle(&t, &src);
    let dir = cache_dir_for(&t.temp_root(), &fp);
    let cache = t.cache.clone();
    let path = src.clone();
    service::seams::set_during_preview(Some(Box::new(move || {
        service::release_source(&cache, &path.to_string_lossy());
    })));
    let p = t.preview(
        &src,
        0,
        &[("Leased line", "Leased lines", SourceTextStyleIn::default())],
    );
    service::seams::set_during_preview(None);
    assert!(p.page_pdf.is_some(), "{:?}", p.page_problem);
    assert_eq!(t.cache.cached_len(), 0, "released");
    assert!(
        !dir.exists(),
        "a copy outlived the release: {:?}",
        std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );
}

/// review-T5 M2: copies left by a crash are deleted at startup.
#[test]
fn m2_startup_deletes_stale_copies() {
    let dir = crate::pdf_engine::text_edit::testkit::Scratch::new("m2_startup");
    let stale = dir.path("temp").join("textedit").join("0123-abcd");
    std::fs::create_dir_all(&stale).expect("stale folder");
    std::fs::write(stale.join("source.pdf"), b"%PDF-1.7").expect("stale copy");
    clear_stale_folders(&dir.path("temp"));
    assert!(!dir.path("temp").join("textedit").exists());
    assert!(dir.path("temp").is_dir(), "only textedit/ is cleared");
}

/// review-T5 M3: calls that overlap on a file too large to cache share one read of it (the
/// test lowers this cache's limit so a small file counts as large).
#[test]
fn m3_overlapping_calls_share_one_read_of_a_large_file() {
    let Some(t) = E2e::new("m3_single_flight") else {
        return;
    };
    let src = t.file("large.pdf", &pages_doc(&["Large file"]));
    t.cache.with_seams(|s| s.snapshot_bytes_max = Some(16));
    const CALLS: usize = 6;
    let start = Arc::new(Barrier::new(CALLS));
    let hold = Arc::new(Barrier::new(CALLS));
    let workers: Vec<_> = (0..CALLS)
        .map(|_| {
            let (cache, engines): (TextEditCache, _) = (t.cache.clone(), t.engines.clone());
            let (start, hold) = (Arc::clone(&start), Arc::clone(&hold));
            let (src, temp) = (src.clone(), t.temp_root());
            std::thread::spawn(move || {
                start.wait();
                let s = cache.open(&src, &temp, &engines).expect("open");
                hold.wait();
                Arc::as_ptr(&s) as usize
            })
        })
        .collect();
    let ptrs: Vec<usize> = workers
        .into_iter()
        .map(|w| w.join().expect("worker"))
        .collect();
    assert_eq!(t.cache.cached_len(), 0, "the file is pinned, not cached");
    assert_eq!(
        t.cache.with_seams(|s| s.loads),
        1,
        "one read for {CALLS} calls"
    );
    assert!(ptrs.iter().all(|p| *p == ptrs[0]), "one shared source");
    // Not kept between calls: the next call reads the file again.
    t.cache
        .open(&src, &t.temp_root(), &t.engines)
        .expect("open");
    assert_eq!(t.cache.with_seams(|s| s.loads), 2);
}

/// Item 7 (review-T3-budget HIGH-1 hand-off): the cache charges each page model its
/// `walk.model_bytes` (everything the model keeps, its fonts included).
#[test]
fn page_models_are_charged_their_model_bytes() {
    let Some(t) = E2e::new("cache_model_bytes") else {
        return;
    };
    let src = t.file("two.pdf", &pages_doc(&["Page one", "Page two"]));
    let fp = t.open(&src).fingerprint;
    let s = t
        .cache
        .get(&src, &t.temp_root(), &fp, &t.engines)
        .expect("source");
    let a = t.cache.page(&s, 0).expect("page 1");
    let b = t.cache.page(&s, 1).expect("page 2");
    let want = a.walk.model_bytes + b.walk.model_bytes;
    assert!(a.walk.model_bytes >= a.approx_bytes() && b.walk.model_bytes >= b.approx_bytes());
    assert_eq!(t.cache.page_model_bytes(&s), want, "LRU charge");
}

/// review-T5 live B1: text drawn through a Form XObject is listed (refused `NESTED_FORM`, never
/// editable), not left out — the page otherwise reads as having no text.
#[test]
fn form_xobject_text_is_listed_as_refused_lines() {
    let Some(t) = E2e::new("b1_form_lines") else {
        return;
    };
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/source-edit/text-nested-form.pdf");
    let src = t.file("nested.pdf", &std::fs::read(fixture).expect("fixture"));
    let (_, dto) = t.page(&src, 0);
    let lines: Vec<_> = dto
        .runs
        .iter()
        .map(|r| (r.text.as_str(), r.editable, r.reason))
        .collect();
    assert_eq!(
        lines,
        vec![(
            "Hi",
            false,
            Some(crate::pdf_engine::text_edit::reasons::TextReason::NestedForm)
        )]
    );
    let run = &dto.runs[0];
    assert!(
        run.metrics.is_none() && run.style.is_none(),
        "no editing data"
    );
    assert!(
        run.rect.w > 0.0 && run.rect.h > 0.0,
        "the line has a box: {:?}",
        run.rect
    );
}
