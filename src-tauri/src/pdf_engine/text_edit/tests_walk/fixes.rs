//! Engine fix pass (2026-10-03) regressions for review T3-budget: font models are charged to the
//! page model (HIGH-1); a page refused after its walk keeps no walk and is not walked again
//! (MEDIUM-1); the Classify pass runs on the budget its model left and reuses a walk with no Form
//! (MEDIUM-2); a model keeps no lexed ops, so dense pages with a matrix per glyph model and
//! classify (MEDIUM-3); a growing vector's old buffer counts (LOW-1).
//!
//! "Held" is what the calling thread still has allocated when a build returns (`thread_peak_held`);
//! an inspect costs the model it holds plus its Classify pass's peak.

use super::{ctx, model, walk};
use crate::pdf_engine::source_content::classify_source_page;
use crate::pdf_engine::text_edit::fonts::FontModel;
use crate::pdf_engine::text_edit::fonts::TypingSurface;
use crate::pdf_engine::text_edit::limits::{set_model_bytes_override, PAGE_MODEL_BYTES_MAX};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::runs::{surface_of, PageModel};
use crate::pdf_engine::text_edit::testkit::fonts::{add_type0, cmap, Program, Type0Font};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::{thread_peak, thread_peak_held};
use crate::pdf_engine::text_edit::walker::budget::{ModelBudget, MODEL_SIZE};
use crate::pdf_engine::text_edit::walker::{PaintKind, WalkMode};
use std::collections::HashSet;
use std::sync::Arc;

const MIB: usize = 1 << 20;
/// §H R20's per-file budget.
const PEAK_MAX: usize = 256 * MIB;
/// `fonts::FONT_CACHE_BYTES_MAX`: the snapshot's own font cache, bounded apart from the models.
const FONT_CACHE_MAX: usize = 128 * MIB;

/// Distinct font models `m` keeps alive (page fonts and records), once each by allocation.
fn kept_fonts(m: &PageModel) -> Vec<Arc<FontModel>> {
    let mut seen = HashSet::new();
    let page = m.walk.page_fonts.iter().map(|(_, f)| f);
    page.chain(m.walk.records.iter().filter_map(|r| r.font.as_ref()))
        .filter(|f| seen.insert(Arc::as_ptr(f) as usize))
        .cloned()
        .collect()
}

/// `pages` pages, each drawing `n` Type0 fonts once and listing `n` more it does not use; every
/// font has its own one-line ToUnicode mapping 65,536 codes (a few bytes deflated, MiBs of model).
fn font_pages(n: usize, pages: usize) -> Vec<u8> {
    let tu = cmap(
        "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
         1 beginbfrange\n<0000> <FFFF> <0000>\nendbfrange",
    );
    let mut d = DocBuilder::new();
    for _ in 0..pages {
        let mut fonts = String::new();
        let mut c = String::from("BT ");
        for k in 0..2 * n {
            let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Big");
            f.program = Program::None;
            f.w = Some("[1 [500]]".to_string());
            f.tounicode = Some(tu.clone());
            let id = add_type0(&mut d.b, &f);
            fonts.push_str(&format!("/T{k} {id} 0 R "));
            if k < n {
                c.push_str(&format!("/T{k} 10 Tf 72 {} Td <0001> Tj ", 700 - k));
            }
        }
        c.push_str("ET");
        d.page(PageSpec::new(c.as_bytes(), &format!("/Font << {fonts} >>")));
    }
    d.build()
}

#[test]
fn font_models_count_in_the_page_model_that_keeps_them() {
    // Review HIGH-1: 6 such pages held 530 MiB of font models, outside every budget, with
    // `model_bytes` and `approx_bytes` at 0 (T5's cache counted them as nothing).
    let pages = 3;
    let c = ctx(font_pages(8, pages));
    let (models, peak, held) =
        thread_peak_held(|| (0..pages as u32).map(|p| model(&c, p)).collect::<Vec<_>>());
    let mut charged = 0usize;
    for (p, m) in models.iter().enumerate() {
        assert_eq!(m.page_reason, None, "page {p}: {:?}", m.page_detail);
        let fonts: usize = kept_fonts(m).iter().map(|f| f.approx_bytes()).sum();
        println!(
            "fonts 8+8 page {p}: charged {} MiB, approx {} MiB, fonts kept {} ({} MiB), \
             page fonts {}",
            m.walk.model_bytes / MIB,
            m.approx_bytes() / MIB,
            kept_fonts(m).len(),
            fonts / MIB,
            m.walk.page_fonts.len()
        );
        assert!(fonts > 8 * 4 * MIB, "page {p}: the drawn fonts are kept");
        assert!(
            m.approx_bytes() >= fonts,
            "page {p}: approx counts its fonts"
        );
        assert!(m.approx_bytes() <= m.walk.model_bytes, "page {p}");
        assert!(m.walk.model_bytes <= PAGE_MODEL_BYTES_MAX, "page {p}");
        assert!(
            m.walk.page_fonts.len() < 16,
            "page {p}: unused fonts left out"
        );
        charged += m.walk.model_bytes;
    }
    println!(
        "fonts 8+8 x {pages}: peak {} MiB, held {} MiB, charged {} MiB",
        peak / MIB,
        held / MIB,
        charged / MIB
    );
    assert!(
        held <= charged + FONT_CACHE_MAX + 16 * MIB,
        "everything the models keep is in their model_bytes (held {held}, charged {charged})"
    );
}

#[test]
fn drawn_fonts_past_the_budget_refuse_the_page_and_unused_ones_are_left_out() {
    let c = ctx(font_pages(8, 1));
    set_model_bytes_override(Some(40 * MIB));
    let refused = model(&c, 0);
    set_model_bytes_override(None);
    assert_eq!(refused.page_reason, Some(R::PageTooComplex));
    assert_eq!(refused.page_detail.as_deref(), Some(MODEL_SIZE));
    assert!(
        kept_fonts(&refused).is_empty(),
        "a refused page keeps no font"
    );
    // One drawn font and eight unused ones: the page models, with the siblings that fit.
    let c = ctx(font_pages(1, 1));
    let full = model(&c, 0);
    set_model_bytes_override(Some(24 * MIB));
    let small = model(&c, 0);
    set_model_bytes_override(None);
    assert_eq!(small.page_reason, None, "{:?}", small.page_detail);
    assert_eq!(
        full.walk.page_fonts.len(),
        2,
        "1 drawn + the unused one that fits"
    );
    assert_eq!(
        small.walk.page_fonts.len(),
        1,
        "no room for an unused sibling"
    );
    assert!(small.walk.model_bytes <= 24 * MIB);
}

/// "Hello" plus `n` `(a)Tj` ops under one state on a line far below (review probe shape).
fn near_budget(n: usize) -> Vec<u8> {
    let mut c = String::from("BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 1 Tf 0 20 Td ");
    c.push_str(&"(a)Tj ".repeat(n));
    c.push_str("ET");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(
        PageSpec::new(c.as_bytes(), &format!("/Font << /F1 {f} 0 R >>"))
            .media(Some("[0 0 2400 3400]")),
    );
    d.build()
}

#[test]
fn a_page_refused_after_its_walk_keeps_no_walk_and_is_not_walked_again() {
    // Long strings: the run stage costs about what the walk does, so a budget between the two
    // refuses the page at the run stage.
    let mut content = String::from("BT /F1 1 Tf 0 20 Td ");
    for _ in 0..2_000 {
        content.push_str(&format!("({}) Tj 0 -1.2 Td ", "abcdefgh ".repeat(12)));
    }
    content.push_str("ET");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(
        PageSpec::new(content.as_bytes(), &format!("/Font << /F1 {f} 0 R >>"))
            .media(Some("[0 0 2400 3400]")),
    );
    let c = ctx(d.build());
    let full = model(&c, 0);
    let w = walk(&c, 0, WalkMode::Edit);
    assert_eq!(full.page_reason, None);
    let (walked, total) = (w.model_bytes, full.walk.model_bytes);
    assert!(walked < total, "walk {walked} < model {total}");
    set_model_bytes_override(Some((walked + total) / 2 + MIB));
    let (m, _, held) = thread_peak_held(|| model(&c, 0));
    let (cl, classify_peak) = thread_peak(|| classify_source_page(&c, &m, None));
    set_model_bytes_override(None);
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(
        m.page_detail.as_deref(),
        Some(MODEL_SIZE),
        "at the run stage"
    );
    assert!(
        m.walk.records.is_empty() && m.walk.paints.is_empty() && m.walk.page_fonts.is_empty(),
        "the refused model released its walk ({} records)",
        m.walk.records.len()
    );
    assert!(m.record_reason.is_empty());
    assert!(m.approx_bytes() <= m.walk.model_bytes);
    assert!(held < 2 * content.len() + MIB, "held {held} B");
    assert_eq!(cl.occurrence_reason, Some(R::PageTooComplex));
    assert!(classify_peak < MIB, "not walked again: {classify_peak} B");
}

/// The model of page 0 and its Classify pass: the bytes held after the build plus the pass's
/// peak (what an inspect costs at once).
fn inspect(tag: &str, pdf: Vec<u8>) -> (PageModel, usize, usize) {
    let c = ctx(pdf);
    let (m, model_peak, held) = thread_peak_held(|| model(&c, 0));
    let (cl, classify_peak) = thread_peak(|| classify_source_page(&c, &m, None));
    let records = m.walk.records.len();
    println!(
        "{tag}: model peak {} MiB, held {} MiB, charged {} MiB, {:?} {:?}, {records} records, \
         {} runs; classify peak {} MiB, {:?}, {} occurrences; inspect {} MiB",
        model_peak / MIB,
        held / MIB,
        m.walk.model_bytes / MIB,
        m.page_reason,
        m.page_detail,
        m.runs.len(),
        classify_peak / MIB,
        cl.occurrence_reason,
        cl.occurrences.len(),
        (held + classify_peak) / MIB
    );
    assert!(model_peak < PEAK_MAX, "{tag}: model peak {model_peak}");
    assert!(
        m.walk.ops.is_empty() && m.walk.ops_bytes == 0,
        "{tag}: a model keeps no ops"
    );
    assert!(m.approx_bytes() <= m.walk.model_bytes, "{tag}");
    assert!(m.walk.model_bytes <= PAGE_MODEL_BYTES_MAX, "{tag}");
    let occurrences = match cl.occurrence_reason {
        None => cl.occurrences.len(),
        Some(_) => 0,
    };
    (m, held + classify_peak, occurrences)
}

#[test]
fn the_classify_pass_shares_its_model_budget() {
    // Review MEDIUM-2: 100,000 shows under one state peaked at 210 MiB (model 106 + Classify
    // walk 104 on a budget of its own); 135,000 at 303 MiB.
    for n in [100_000usize, 135_000] {
        let (m, at_once, occurrences) = inspect(&format!("near budget {n}"), near_budget(n));
        // One page budget for both (each on its own budget peaked at 186 MiB here).
        assert!(at_once < PAGE_MODEL_BYTES_MAX, "{n}: inspect {at_once} B");
        if m.page_reason.is_none() {
            assert_eq!(occurrences, n + 1, "{n}: Classify complete");
        }
    }
    // A page that paints a Form is walked again in Classify mode, on what its model left.
    let (m, at_once, occurrences) = inspect("nested form", fx::nested_form());
    assert_eq!(m.page_reason, None);
    assert!(
        occurrences > m.walk.records.len(),
        "the Form's text is listed too"
    );
    assert!(at_once < 16 * MIB);
}

#[test]
fn the_edit_walk_of_a_page_without_forms_is_its_classify_walk() {
    // `classify_source_page` lists such a page's occurrences from its model's own walk.
    let pages: Vec<(&str, Vec<u8>)> = vec![
        ("FX-WORD", fx::word()),
        ("FX-LIBRE", fx::libre()),
        ("FX-SKIA", fx::skia()),
        ("FX-QUARTZ", fx::quartz()),
        ("FX-XETEX", fx::xetex()),
        ("FX-INDD", fx::indd()),
        ("FX-PERGLYPH", fx::per_glyph(None)),
        ("two columns, tagged", fx::two_column(true)),
        ("images", fx::swapped_image(false)),
    ];
    let mut compared = 0;
    for (tag, pdf) in pages {
        let c = ctx(pdf);
        let edit = walk(&c, 0, WalkMode::Edit);
        assert_eq!(edit.page_reason, None, "{tag}");
        if edit
            .paints
            .iter()
            .any(|p| matches!(p.kind, PaintKind::FormXObject { .. }))
        {
            continue; // walked again in Classify mode (FX-INDD paints a Form)
        }
        compared += 1;
        let classify = walk(&c, 0, WalkMode::Classify);
        let recs = |w: &crate::pdf_engine::text_edit::walker::PageWalk| -> Vec<String> {
            let r = w.records.iter().map(|r| {
                let text: Vec<_> = r.glyphs.iter().map(|g| g.text.clone()).collect();
                format!(
                    "{} {} {:?} {:?} {text:?} {:?}",
                    r.seq, r.depth, r.span, r.local_span, r.pen_before
                )
            });
            let p = w.paints.iter().map(|p| {
                format!(
                    "{} {} {:?} {:?} {:?} {:?}",
                    p.seq, p.depth, p.kind, p.span, p.bbox, p.xobject
                )
            });
            r.chain(p).collect()
        };
        assert_eq!(recs(&edit), recs(&classify), "{tag}");
        assert_eq!(
            edit.model_bytes, classify.model_bytes,
            "{tag}: the same charge"
        );
    }
    assert!(compared >= 7, "{compared} pages compared");
}

#[test]
fn a_runs_surface_names_its_typing_surface() {
    // `PageModel::surface` now reads `run.surface` (it recomputed the sibling group per call, and
    // the field was read by nothing in production): the two are the same fonts in the same order.
    let pages: Vec<(&str, Vec<u8>)> = vec![
        ("FX-WORD", fx::word()),
        ("FX-WORD-TR", fx::word_tr()),
        ("FX-LIBRE", fx::libre()),
        ("FX-XETEX", fx::xetex()),
        ("FX-INDD", fx::indd()),
        ("FX-STD14", fx::std14()),
    ];
    let mut checked = 0;
    for (tag, pdf) in pages {
        let m = model(&ctx(pdf), 0);
        for run in &m.runs {
            let primary = &m.walk.records[run.members[0]];
            let names = |s: TypingSurface| -> Vec<Vec<u8>> {
                s.fonts.into_iter().map(|(n, _)| n).collect()
            };
            let recomputed = names(surface_of(&m.walk, primary));
            assert_eq!(names(m.surface(run)), recomputed, "{tag}: {:?}", run.text);
            if !run.surface.is_empty() {
                assert_eq!(run.surface.to_vec(), recomputed, "{tag}: {:?}", run.text);
                checked += 1;
            }
        }
    }
    assert!(checked > 10, "{checked} runs");
}

/// 3,000 lines of 32 Courier glyphs (96,000), one `Tm` per glyph (`e2`), and a gray change per
/// word (`g2`, a syntax-highlighted listing): review MEDIUM-3's normal dense pages.
fn dense(shape: &str) -> Vec<u8> {
    let mut c = String::from("BT /F1 1 Tf 1.1 TL\n");
    for i in 0..3_000 {
        let t = format!("Line {i:05} lorem ipsum dolor sit");
        let y = 3300.0 - 1.1 * i as f64;
        let mut k = 0usize;
        for (wi, w) in t.split(' ').enumerate() {
            if shape == "g2" {
                c.push_str(&format!("{} g ", (i + wi) % 2));
            }
            for ch in format!("{w} ").chars() {
                let x = 20.0 + 0.6 * k as f64;
                c.push_str(&format!("1 0 0 1 {x:.1} {y:.2} Tm ({ch}) Tj "));
                k += 1;
            }
        }
        c.push('\n');
    }
    c.push_str("ET");
    let mut d = DocBuilder::new();
    let courier = "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>";
    let f = d.add(courier);
    d.page(
        PageSpec::new(c.as_bytes(), &format!("/Font << /F1 {f} 0 R >>"))
            .media(Some("[0 0 2400 3400]")),
    );
    d.build()
}

#[test]
fn dense_pages_with_a_matrix_per_glyph_model_and_classify() {
    // Before: e2 modelled at 148 MiB with its Classify pass refused (0 occurrences); g2 refused
    // "page model size" — both mostly for the six lexed `Tm` operands a model kept per glyph.
    for shape in ["e2", "g2"] {
        let (m, at_once, occurrences) = inspect(shape, dense(shape));
        assert_eq!(m.page_reason, None, "{shape}: {:?}", m.page_detail);
        let lines = if shape == "e2" { 3_000 } else { 18_000 };
        assert_eq!(
            m.runs.len(),
            lines,
            "{shape}: a run per line (per word in g2's colours)"
        );
        assert_eq!(
            occurrences,
            m.walk.records.len(),
            "{shape}: Classify complete"
        );
        assert!(at_once < PEAK_MAX, "{shape}: inspect {at_once} B");
    }
}

#[test]
fn growing_vectors_count_their_old_buffer() {
    // The test allocator counts a realloc as new then old (it used to count the difference only).
    let ((), peak) = thread_peak(|| {
        let mut v: Vec<u8> = vec![0; MIB];
        v.reserve_exact(2 * MIB);
        assert!(v.capacity() >= 3 * MIB);
    });
    assert!(peak >= 4 * MIB, "old and new buffers at once: {peak} B");
    // The budget lets a vector grow only when its old buffer fits beside the new room.
    set_model_bytes_override(Some(1_200));
    let mut mem = ModelBudget::new(0);
    let mut v: Vec<u64> = Vec::new();
    let grown = (0..64).try_for_each(|i| mem.push(&mut v, i));
    let refused = mem.push(&mut v, 64);
    set_model_bytes_override(None);
    assert!(grown.is_ok() && v.capacity() == 64, "512 B held");
    assert!(
        refused.is_err(),
        "512 B more fit the 688 B left, but not with the old 512 B buffer"
    );
}

#[test]
fn text_drawn_through_a_form_is_listed_as_refused_lines() {
    // Live check B1: the repo's text-nested-form.pdf showed "Hi" but listed no run at all, so the
    // editor said the page had no text (§A.6: Form text is "refused, still shown").
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/source-edit/text-nested-form.pdf");
    let c = ctx(std::fs::read(&path).expect("fixture"));
    let m = model(&c, 0);
    let cl = classify_source_page(&c, &m, None);
    let lines: Vec<(&str, R)> = cl
        .form_lines
        .iter()
        .map(|l| (l.text.as_str(), l.reason))
        .collect();
    assert_eq!(lines, [("Hi", R::NestedForm)], "{:?}", m.runs.len());
    // A Form line of two shows on one baseline, after the page's own run, with its own id.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let form = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
             /Resources << /Font << /F1 {f} 0 R >> >>"
        ),
        b"BT /F1 12 Tf 72 600 Td (Hello) Tj ( World) Tj ET BT /F1 12 Tf 72 560 Td (Next) Tj ET",
    );
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Page text) Tj ET /Fm0 Do",
        &format!("/XObject << /Fm0 {form} 0 R >> /Font << /F1 {f} 0 R >>"),
    ));
    let c = ctx(d.build());
    let m = model(&c, 0);
    let cl = classify_source_page(&c, &m, None);
    let texts: Vec<&str> = cl.form_lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["Hello World", "Next"]);
    assert_eq!(m.runs.len(), 1, "the page's own run");
    let first = &cl.form_lines[0];
    assert!(first.order > m.runs[0].order && first.rect[2] > 0.0 && first.rect[3] > 0.0);
    assert!(first.caret_offsets.len() == "Hello World".chars().count() + 1);
    assert_ne!(first.id, cl.form_lines[1].id);
    assert!(cl.form_lines.iter().all(|l| l.reason == R::NestedForm));
    // Pages that paint no Form list none.
    let c = ctx(fx::word());
    assert!(classify_source_page(&c, &model(&c, 0), None)
        .form_lines
        .is_empty());
}
