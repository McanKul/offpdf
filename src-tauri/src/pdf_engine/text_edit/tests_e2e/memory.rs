//! review-T4 M-1: memory of the plan, preview and Save paths on large pages. A Save with several
//! large edited pages must not hold every page's model from planning to Phase A: two extra pages
//! add less than 1.5 page models to the peak of a one-page Save (before the fix, each added its
//! whole model and an unshared copy of every record's state in its proof). review-final MEDIUM-3:
//! a preview of a near-budget page stays below 256 MiB for the whole process.

use super::E2e;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::limits::PREVIEW_SHARED_MODEL_MAX;
use crate::pdf_engine::text_edit::rewrite::{plan_page, SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel};
use crate::pdf_engine::text_edit::service::preview_edits;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::testkit::{
    child_mode, child_value, process_peak, run_child_test, thread_peak,
};

const MIB: usize = 1 << 20;
/// `(a)Tj` ops per page: a page model of a few tens of MiB, well above the kept-model budget of a
/// small file (16 MiB).
const OPS: usize = 30_000;

/// `pages` pages, each "Hello" plus `OPS` one-glyph shows (review-T4 M-1's near-budget shape).
fn dense_doc(pages: usize) -> Vec<u8> {
    let mut content = String::from("BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 1 Tf 0 20 Td ");
    content.push_str(&"(a)Tj ".repeat(OPS));
    content.push_str("ET");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    for _ in 0..pages {
        d.page(PageSpec::new(
            content.as_bytes(),
            &format!("/Font << /F1 {f} 0 R >>"),
        ));
    }
    d.build()
}

/// Peak bytes held on this thread while saving "Hello" → "Help" on every page of `pages` pages.
fn save_peak(t: &E2e, pages: usize) -> usize {
    let src = t.file(&format!("dense-{pages}.pdf"), &dense_doc(pages));
    let edits = (0..pages as u32)
        .map(|p| t.edit(&src, p, p, "Hello", "Help"))
        .collect();
    t.cache.release(&src);
    let (saved, peak) = thread_peak(|| t.save(&[(&src, "1-z")], edits));
    let saved = saved.unwrap_or_else(|e| panic!("{pages}-page save: {e} {:?}", e.details));
    for p in 0..pages as u32 {
        t.assert_changed(&src, p, &saved.path, p, "Hello", "Help");
    }
    peak
}

#[test]
fn save_of_several_large_pages_holds_one_page_model_at_a_time() {
    let Some(t) = E2e::new("m1_save_models") else {
        return;
    };
    let (src, model_bytes) = {
        let src = t.file("dense-model.pdf", &dense_doc(1));
        let fp = t.open(&src).fingerprint;
        let source = t
            .cache
            .get(&src, &t.temp_root(), &fp, &t.engines)
            .expect("source");
        let model = t.cache.page(&source, 0).expect("model");
        (src, model.walk.model_bytes)
    };
    t.cache.release(&src);
    assert!(
        model_bytes > 16 * MIB,
        "the page model ({} MiB) must exceed the kept-model budget",
        model_bytes / MIB
    );
    let one = save_peak(&t, 1);
    let three = save_peak(&t, 3);
    println!(
        "M-1 save peaks: model {} MiB, 1 page {} MiB, 3 pages {} MiB",
        model_bytes / MIB,
        one / MIB,
        three / MIB
    );
    // The two extra pages add their plans and proofs; keeping their models alone would add two
    // whole models.
    assert!(
        three.saturating_sub(one) < model_bytes * 3 / 2,
        "a 3-page save peaks at {} MiB, a 1-page save at {} MiB: models are stacked ({} MiB each)",
        three / MIB,
        one / MIB,
        model_bytes / MIB
    );
}

/// One page: "Hello" plus 100,000 `(a)Tj` (review-T4 M-1's near-budget page, an 84 MiB model).
fn near_budget_doc() -> Vec<u8> {
    let mut content = String::from("BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 1 Tf 0 20 Td ");
    content.push_str(&"(a)Tj ".repeat(100_000));
    content.push_str("ET");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

fn hello_to_help(model: &PageModel) -> TextEditIn {
    let run = model
        .runs
        .iter()
        .find(|r| r.text == "Hello")
        .expect("Hello");
    TextEditIn {
        run_id: run.id.clone(),
        original_text: "Hello".into(),
        text: "Help".into(),
        style: SourceTextStyleIn::default(),
    }
}

/// review-T4 M-1 (plan half), the reviewer's probe: the verification walks no longer stack (the
/// after walk is reduced to what its caller keeps before the probe walk), so `plan_page` adds
/// about one walk to the model it plans on (before: two).
#[test]
fn plan_of_a_near_budget_page_does_not_stack_walks() {
    let Some(t) = E2e::new("m1_plan_walks") else {
        return;
    };
    let pdf = near_budget_doc();
    let path = t.file("near-budget.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&path, pdf, None).expect("snapshot"));
    let model = build_page_model(&ctx, 0, None).expect("model");
    let edit = hello_to_help(&model);
    let (out, plan_peak) = thread_peak(|| plan_page(&ctx, &model, std::slice::from_ref(&edit)));
    let out = out.unwrap_or_else(|e| panic!("plan: {e}"));
    assert!(out.plan.is_some(), "a plan");
    let model_bytes = model.walk.model_bytes;
    println!(
        "M-1 near-budget page: model {} MiB, plan_page +{} MiB",
        model_bytes / MIB,
        plan_peak / MIB
    );
    assert!(
        plan_peak < model_bytes * 3 / 2,
        "plan_page holds {} MiB over a {} MiB model: the self-check walks stack",
        plan_peak / MIB,
        model_bytes / MIB
    );
}

/// review-final MEDIUM-3 (§H R20, 256 MiB per file): the whole process — the page's model built
/// and cached as inspect leaves it, then a preview through the service — stays below 256 MiB.
/// The service hands the large model over (`page_to_release`) and the preview frees it once
/// planned, before the extracted page's model and Phase A's walks are built (before: 295 MiB,
/// the source model held throughout). Process-wide, so measured in a child process.
#[test]
fn preview_of_a_near_budget_page_stays_below_256_mib_for_the_process() {
    let Some(_t) = E2e::new("m3_preview_parent") else {
        return;
    };
    let out = run_child_test(
        "pdf_engine::text_edit::tests_e2e::memory::preview_process_peak_child",
        "m3_preview",
    );
    let (peak, model) = (child_value(&out, "PEAK"), child_value(&out, "MODEL"));
    println!(
        "M-3 preview: model {} MiB, process peak {} MiB",
        model / MIB,
        peak / MIB
    );
    assert_eq!(child_value(&out, "PREVIEW_OK"), 1, "{out}");
    assert!(
        model > PREVIEW_SHARED_MODEL_MAX,
        "the page must exercise the hand-over: {out}"
    );
    assert_eq!(
        child_value(&out, "CACHED_AFTER"),
        0,
        "a handed-over model leaves the cache"
    );
    assert!(
        peak < 256 * MIB,
        "the process held {} MiB for a {} MiB model",
        peak / MIB,
        model / MIB
    );
}

#[test]
#[ignore = "child process of preview_of_a_near_budget_page_stays_below_256_mib_for_the_process"]
fn preview_process_peak_child() {
    if child_mode().as_deref() != Some("m3_preview") {
        return;
    }
    let t = E2e::new("m3_preview_child").expect("engines");
    let src = t.file("near-budget.pdf", &near_budget_doc());
    let fp = t.open(&src).fingerprint;
    let ((preview, model_bytes, cached_after), peak) = process_peak(|| {
        let source = t
            .cache
            .get(&src, &t.temp_root(), &fp, &t.engines)
            .expect("source");
        let model = t.cache.page(&source, 0).expect("model");
        let (edit, model_bytes) = (hello_to_help(&model), model.walk.model_bytes);
        drop(model);
        let preview = preview_edits(
            &t.cache,
            &t.engines,
            &t.temp_root(),
            &src.to_string_lossy(),
            &fp,
            0,
            &[edit],
        );
        (preview, model_bytes, t.cache.page_model_bytes(&source))
    });
    let ok = preview.is_ok_and(|p| p.page_pdf.is_some());
    println!(
        "PEAK={peak}\nMODEL={model_bytes}\nCACHED_AFTER={cached_after}\nPREVIEW_OK={}",
        u8::from(ok)
    );
}

/// About `pad_mib` MiB (an incompressible image in the resources) with two pages of "Hello"
/// plus `ops` one-glyph shows each.
fn large_file(pad_mib: usize, ops: usize) -> Vec<u8> {
    let side = 1024usize;
    let len = side * ((pad_mib << 20) / side);
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {side} /Height {} /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Length {len} >>\nstream\n",
        len / side
    )
    .into_bytes();
    image.reserve(len + 16);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        image.push(state as u8);
    }
    image.extend_from_slice(b"\nendstream");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let pad = d.add(image);
    let res = format!("/Font << /F1 {f} 0 R >> /XObject << /Pad {pad} 0 R >>");
    let mut content = String::from("BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 1 Tf 0 20 Td ");
    content.push_str(&"(a)Tj ".repeat(ops));
    content.push_str("ET");
    for _ in 0..2 {
        d.page(PageSpec::new(content.as_bytes(), &res));
    }
    d.build()
}

/// review-final MEDIUM-1 (BENCH-02: 3 × file): a Save holds neither raw bytes once they are
/// parsed nor a source context next to kept models. Process-wide, in a child process, for both
/// sides of the kept-model budget (twice the file):
/// - models kept (40 MiB file, 2 × 12 MiB of models): the peak is Phase A's read of the edited
///   copy (twice the file) next to the models: 104 MiB (with the raw bytes kept: 143 MiB);
/// - context kept (16 MiB file, 2 × 23 MiB): the context, the staged read and the page rebuilt:
///   95 MiB (with the first model kept next to the context: 118 MiB).
///
/// The reviewer's 151 MiB file with 169 MiB of models (`v04/lastfix-logs`): 923 MiB = 6.1 × file
/// before, 475 MiB = 3.15 × file now (models + Phase A's read of the edited copy).
#[test]
fn save_of_a_large_file_holds_its_staged_read_and_models_only() {
    let Some(_t) = E2e::new("m1_save_parent") else {
        return;
    };
    let out = run_child_test(
        "pdf_engine::text_edit::tests_e2e::memory::save_process_peak_child",
        "m1_save",
    );
    for (shape, files) in [("kept", 2), ("context", 3)] {
        let value = |key: &str| child_value(&out, &format!("{shape}_{key}"));
        let (file, models, peak) = (value("FILE"), value("MODELS"), value("PEAK"));
        println!(
            "M-1 save ({shape}): file {} MiB, models {} MiB, process peak {} MiB = {:.2} x file",
            file / MIB,
            models / MIB,
            peak / MIB,
            peak as f64 / file as f64
        );
        assert_eq!(value("OK"), 1, "{shape}: the save must publish: {out}");
        assert!(
            peak < files * file + models + 16 * MIB,
            "{shape}: {} MiB for a {} MiB file with {} MiB of models",
            peak / MIB,
            file / MIB,
            models / MIB
        );
    }
}

#[test]
#[ignore = "child process of save_of_a_large_file_holds_its_staged_read_and_models_only"]
fn save_process_peak_child() {
    if child_mode().as_deref() != Some("m1_save") {
        return;
    }
    for (shape, pad_mib, ops) in [("kept", 40, 15_000), ("context", 16, 30_000)] {
        let t = E2e::new(&format!("m1_save_{shape}")).expect("engines");
        let pdf = large_file(pad_mib, ops);
        let file = pdf.len();
        let src = t.file("large.pdf", &pdf);
        let edits = (0..2u32)
            .map(|p| t.edit(&src, p, p, "Hello", "Help"))
            .collect();
        let fp = t.open(&src).fingerprint;
        let source = t
            .cache
            .get(&src, &t.temp_root(), &fp, &t.engines)
            .expect("source");
        let models: usize = (0..2u32)
            .map(|p| t.cache.page(&source, p).expect("model").walk.model_bytes)
            .sum();
        std::mem::drop((source, pdf));
        t.cache.release(&src);
        t.cache.wait_checks();
        let (saved, peak) = process_peak(|| t.save(&[(&src, "1-z")], edits));
        let ok = saved.is_ok_and(|s| t.words(&s.path, 0).contains("Help"));
        println!(
            "{shape}_FILE={file}\n{shape}_MODELS={models}\n{shape}_PEAK={peak}\n{shape}_OK={}",
            u8::from(ok)
        );
    }
}
