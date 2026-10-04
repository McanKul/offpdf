//! The gate's cost bounds and cancellation (T4, beyond §E.5): A5's pixel check, A5's word
//! matching and B1's wrapper search stay linear on inputs that are quadratic for a per-pixel ×
//! per-mask scan, a first-unused-match scan or a window-by-window search, and a check that fails
//! while the job is being cancelled reports `CANCELLED` (Phase A, Phase B and the preview), never
//! a verification code.

use super::{ed, fails_at, opts, skipping, Honest};
use crate::pdf_engine::text_edit::engines::RunOpts;
use crate::pdf_engine::text_edit::gate::originals::find_up_to;
use crate::pdf_engine::text_edit::gate::verify_final_output;
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::limits::{RENDER_OUTSIDE_PIXELS_MAX, VERIFY_CAP_MARGIN_BYTES};
use crate::pdf_engine::text_edit::poppler::near::KeptGlyph;
use crate::pdf_engine::text_edit::poppler::{
    check_independent, check_pixels, NearGlyphs, Raster, Word,
};
use crate::pdf_engine::text_edit::preview::{cache_dir_for, preview_page};
use crate::pdf_engine::text_edit::rewrite::{ExpectedRun, SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::helvetica_page;
use crate::pdf_engine::text_edit::tests_plan::{ok_plan, plan_one, style, thread_cpu};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// A page 612 × 792 pt; rendered at 72 DPI one point is one pixel, y down from the top.
const PAGE_W: u32 = 612;
const PAGE_H: u32 = 792;
const DPI: u32 = 72;

/// An expectation of a real plan (A5 reads only its masks and whether its glyphs changed) and the
/// page geometry it was planned on.
fn template() -> (ExpectedRun, PageGeometry) {
    let (_, m, out) = plan_one(
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Help",
        style(),
    );
    let exp = ok_plan(&out).runs[0].expected.clone();
    (exp, m.walk.geometry.clone())
}

fn edit_with(
    template: &ExpectedRun,
    masks: Vec<[f64; 4]>,
) -> (ExpectedRun, [f64; 4], [f64; 4], String) {
    let mut exp = template.clone();
    exp.mask_boxes = masks;
    exp.glyphs_changed = true;
    (exp, [0.0; 4], [0.0; 4], String::new())
}

/// No glyph of our model around the edits.
fn none() -> NearGlyphs {
    NearGlyphs::default()
}

fn white() -> Raster {
    Raster {
        w: PAGE_W,
        h: PAGE_H,
        rgb: vec![255; (PAGE_W * PAGE_H * 3) as usize],
    }
}

/// Darkens the render pixel under user point (`x`, `y`) at 72 DPI.
fn ink(r: &mut Raster, x: u32, y: u32) {
    let row = PAGE_H - 1 - y;
    let i = ((row * PAGE_W + x) * 3) as usize;
    r.rgb[i..i + 3].copy_from_slice(&[0, 0, 0]);
}

/// review-verify HIGH-A: a glyph no edit changes that lies under an edit's masks (here a 4 pt
/// follower within the slack of a 10 pt glyph's mask) is checked outside the edited glyph's own
/// box grown by `RENDER_OWN_PAD_PX`: more than `RENDER_KEPT_PIXELS_MAX` changed pixels there
/// fail, fewer and those under the edited glyph do not.
#[test]
fn kept_glyphs_under_the_masks_are_checked_outside_the_edited_glyphs_own_boxes() {
    let (t, geom) = template();
    let mut exp = t;
    exp.glyph_origins = vec![(100.0, 700.0)];
    exp.mask_boxes = vec![[100.0, 700.0, 110.0, 710.0]];
    exp.old_ink_boxes = Vec::new();
    exp.glyphs_changed = true;
    let edits = vec![(exp, [0.0; 4], [0.0; 4], String::new())];
    let follower = KeptGlyph {
        rect: [112.0, 700.0, 116.0, 710.0],
        text: Some("1".into()),
    };
    let near = NearGlyphs {
        kept: vec![follower],
        edited: Vec::new(),
    };
    let src = white();
    let changed = |xs: &[u32]| {
        let mut dst = white();
        for x in xs {
            ink(&mut dst, *x, 705);
        }
        dst
    };
    let three = changed(&[113, 114, 115]);
    let e = check_pixels(&src, &three, &geom, &edits, &near, DPI).expect_err("follower changed");
    assert_eq!(e.0, "render");
    assert!(
        e.1.contains("a glyph no edit changes changed, pixels=3"),
        "{}",
        e.1
    );
    assert!(
        check_pixels(&src, &three, &geom, &edits, &none(), DPI).is_ok(),
        "without our model the follower lies under the masks"
    );
    let within = changed(&[113, 114]);
    assert!(check_pixels(&src, &within, &geom, &edits, &near, DPI).is_ok());
    let own = changed(&[108, 109, 110]);
    assert!(
        check_pixels(&src, &own, &geom, &edits, &near, DPI).is_ok(),
        "the edited glyph's own box and its one-pixel pad are the edit's"
    );
}

#[test]
fn pixel_check_counts_each_outside_pixel_once_and_marks_each_edit() {
    let (t, geom) = template();
    // Two overlapping masks (edits A and B) and one far away (C).
    let edits = vec![
        edit_with(&t, vec![[100.0, 700.0, 110.0, 710.0]]),
        edit_with(&t, vec![[105.0, 700.0, 120.0, 710.0]]),
        edit_with(&t, vec![[400.0, 100.0, 410.0, 110.0]]),
    ];
    let src = white();
    let mut dst = white();
    // One changed pixel inside A ∩ B: both are visible, nothing is outside.
    ink(&mut dst, 107, 705);
    let visible = check_pixels(&src, &dst, &geom, &edits, &none(), DPI).expect("inside the masks");
    assert_eq!(visible, vec![true, true, false], "A and B visible, C not");
    // RENDER_OUTSIDE_PIXELS_MAX changed pixels outside every mask still pass …
    for k in 0..RENDER_OUTSIDE_PIXELS_MAX as u32 {
        ink(&mut dst, 300 + k, 400);
    }
    assert!(check_pixels(&src, &dst, &geom, &edits, &none(), DPI).is_ok());
    // … one more fails, counted once although the masks overlap elsewhere.
    ink(&mut dst, 300, 390);
    let e = check_pixels(&src, &dst, &geom, &edits, &none(), DPI).expect_err("one too many");
    assert_eq!(
        e,
        (
            "render",
            format!("pixels={}", RENDER_OUTSIDE_PIXELS_MAX + 1)
        )
    );
    // A different render size is refused before any pixel is compared.
    let small = Raster {
        w: 10,
        h: 10,
        rgb: vec![255; 300],
    };
    assert_eq!(
        check_pixels(&src, &small, &geom, &edits, &none(), DPI).map_err(|e| e.0),
        Err("render")
    );
}

#[test]
fn pixel_check_stays_linear_with_many_edits_and_dense_changes() {
    // 200 edits of 100 mask boxes each (the most a page allows, EDITS_PER_PAGE_MAX), and every
    // pixel inside every box changed: ~300,000 changed pixels against 20,000 masks. A per-pixel
    // scan of every mask would test ~6·10⁹ boxes; the row sweep is one pass over the render.
    let (t, geom) = template();
    let src = white();
    let mut dst = white();
    let mut edits = Vec::new();
    for e in 0..200u32 {
        let (col, row) = (e % 6, e / 6);
        let (x0, y0) = (6 + col * 100, 40 + row * 22);
        let mut masks = Vec::new();
        for k in 0..100u32 {
            let (x, y) = (x0 + (k % 25) * 4, y0 + (k / 25) * 4);
            masks.push([
                f64::from(x),
                f64::from(y),
                f64::from(x + 4),
                f64::from(y + 4),
            ]);
            for dx in 0..4 {
                for dy in 0..4 {
                    ink(&mut dst, x + dx, y + dy);
                }
            }
        }
        edits.push(edit_with(&t, masks));
    }
    let started = thread_cpu();
    let visible =
        check_pixels(&src, &dst, &geom, &edits, &none(), DPI).expect("every change is masked");
    let spent = thread_cpu().saturating_sub(started);
    assert!(visible.iter().all(|v| *v), "every edit changed pixels");
    assert!(
        spent < Duration::from_secs(2),
        "pixel check over 200 × 100 masks took {spent:?}"
    );
}

/// G-TEXT inputs: `n` words "ab" on a grid away from the edited line (frame y ≈ 90), and the
/// edited word itself ("Hello" before, "Help" after).
fn grid_words(n: usize) -> (Vec<Word>, Vec<Word>) {
    let word = |text: &str, x: f64, y: f64| Word {
        text: text.into(),
        x0: x,
        y0: y,
        x1: x + 4.0,
        y1: y + 0.4,
    };
    let mut src = vec![word("Hello", 72.0, 85.0)];
    for i in 0..n {
        src.push(word(
            "ab",
            (i % 100) as f64 * 5.0,
            200.0 + (i / 100) as f64 * 0.5,
        ));
    }
    let mut dst = src.clone();
    dst[0].text = "Help".into();
    (src, dst)
}

#[test]
fn word_matching_stays_linear_in_the_word_count() {
    // 200,000 words (more than a capped pdftotext output holds), in reverse order on the after
    // side: a "first unused match" scan of the after list would compare ~2·10¹⁰ pairs.
    let (t, geom) = template();
    let edits = vec![(
        t,
        [72.0, 695.0, 100.0, 710.0],
        [72.0, 695.0, 100.0, 710.0],
        "Hello".to_string(),
    )];
    let raster = Raster {
        w: 1,
        h: 1,
        rgb: vec![255; 3],
    };
    let (src, mut dst) = grid_words(200_000);
    dst[1..].reverse();
    let started = thread_cpu();
    let r = check_independent(&src, &dst, &raster, &raster, &geom, &edits, &none(), DPI);
    let spent = thread_cpu().saturating_sub(started);
    assert!(r.is_ok(), "the same words in another order match: {r:?}");
    assert!(
        spent < Duration::from_secs(2),
        "word matching took {spent:?}"
    );
    // A moved word is still found, wherever it is in the list.
    let (src, mut dst) = grid_words(1_000);
    dst[500].x0 += 0.06;
    let r = check_independent(&src, &dst, &raster, &raster, &geom, &edits, &none(), DPI);
    assert!(
        matches!(&r, Err(("words", d)) if d.contains("moved or changed")),
        "{r:?}"
    );
    // Words of one text a hair apart, ordered against each other: the search gives up (fails
    // closed) instead of scanning quadratically.
    let (mut src, _) = grid_words(0);
    let n = 40_000usize;
    for i in 0..n {
        let x1 = if i < n / 2 { 10.0 } else { 10.09 };
        src.push(Word {
            text: "ab".into(),
            x0: 0.0,
            y0: 300.0,
            x1,
            y1: 301.0,
        });
    }
    let mut dst = src.clone();
    dst[0].text = "Help".into();
    dst[1..].reverse();
    let started = thread_cpu();
    let r = check_independent(&src, &dst, &raster, &raster, &geom, &edits, &none(), DPI);
    let spent = thread_cpu().saturating_sub(started);
    assert!(
        matches!(&r, Err(("words", d)) if d.contains("too many overlapping words")),
        "{r:?}"
    );
    assert!(
        spent < Duration::from_secs(2),
        "bounded word search took {spent:?}"
    );
}

#[test]
fn wrapper_search_stays_linear_on_repetitive_data() {
    // "aaa…a" against "aaa…ab": a window-by-window comparison would read ~3·10¹² bytes.
    let data = vec![b'a'; 4 << 20];
    let mut needle = vec![b'a'; 1 << 20];
    needle.push(b'b');
    let started = thread_cpu();
    assert_eq!(find_up_to(&data, &needle, 2), Ok(Vec::new()));
    // Every window a hit: the search stops at the second one.
    let short = vec![b'a'; 1 << 16];
    assert_eq!(find_up_to(&data, &short, 2), Ok(vec![0, 1]));
    let spent = thread_cpu().saturating_sub(started);
    assert!(
        spent < Duration::from_secs(2),
        "wrapper search took {spent:?}"
    );
    // Exact positions, the end of the data, and needles that cannot fit.
    assert_eq!(find_up_to(b"x\nABC\nABC", b"ABC", 5), Ok(vec![2, 6]));
    assert_eq!(find_up_to(b"zzABC", b"ABC", 5), Ok(vec![2]));
    assert_eq!(find_up_to(b"AB", b"ABC", 5), Ok(Vec::new()));
    assert_eq!(find_up_to(b"ABC", b"", 5), Ok(Vec::new()));
}

#[test]
fn failures_while_cancelled_report_cancelled() {
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Invoice 2026) Tj ET BT /F1 12 Tf 72 650 Td (Unchanged line) Tj ET",
    );
    let Some(h) = Honest::new(
        "bounds_cancel",
        pdf,
        0,
        &[ed("Invoice 2026", "Invoice 2027")],
    ) else {
        return;
    };
    let fake = h.path("fake.pdf");
    fakes::cover_and_overlay_f1(
        h.qpdf(),
        &h.source,
        &fake,
        0,
        [70.0, 695.0, 90.0, 16.0],
        "F1",
        "Invoice 2027",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let flag = AtomicBool::new(true);
    let cancelled = RunOpts {
        cancel: Some(&flag),
        ..RunOpts::default()
    };
    // With no subprocess before it, the fake fails A2 — reported as the cancel it happened under.
    let r = skipping(&["A0", "A1"], || h.phase_a_opts(&fake, &cancelled));
    assert_eq!(r.err().map(|e| e.code), Some("CANCELLED".to_string()));
    // Not cancelled, the same file fails A2 with its verification code.
    let r = skipping(&["A0", "A1"], || h.phase_a_opts(&fake, &opts()));
    fails_at(r, "EDIT_VERIFY_FAILED", "A2", "path=", "cancel control");
    // Phase B and the preview stop with CANCELLED too.
    let cap = 4 * std::fs::metadata(&fake).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES;
    let r = verify_final_output(&fake, cap, &h.engines, &[(0, h.proof())], &cancelled);
    assert_eq!(r.err().map(|e| e.code), Some("CANCELLED".to_string()));
    let run = h
        .model
        .runs
        .iter()
        .find(|r| r.text == "Invoice 2026")
        .expect("run");
    let edit = TextEditIn {
        run_id: run.id.clone(),
        original_text: run.text.clone(),
        text: "Invoice 2027".into(),
        style: SourceTextStyleIn::default(),
    };
    let cache = cache_dir_for(h.dir.dir(), "cancelled");
    let model = build_page_model(&h.ctx, h.page, None).expect("model");
    let r = preview_page(
        &h.ctx,
        std::sync::Arc::new(model),
        &[edit],
        &cache,
        &h.engines,
        &[],
        &cancelled,
    );
    assert_eq!(r.err().map(|e| e.code), Some("CANCELLED".to_string()));
}
