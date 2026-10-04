//! Review regressions (T3 budget pass): every amplification probe of review rounds 1–3 — a page of
//! a few KB deflated that built a page model of hundreds of MiB to GiBs — is refused
//! `PAGE_TOO_COMPLEX` or holds far less, in `build_page_model` and in `classify_source_page`, and
//! so are amplifications no review listed (a long font name copied per glyph unit, interned
//! ExtGState lists with no text, a Form only the Classify walk descends): the page-model budget
//! (`PAGE_MODEL_BYTES_MAX`) is charged before every allocation that grows with the page. Normal
//! pages hold a few MiB and `approx_bytes` never exceeds what their build charged.
//!
//! Peaks are the calling thread's allocation peak (`thread_peak`); the snapshot is parsed
//! outside it (lopdf parses on other threads).

use super::fuzz::thread_cpu;
use super::{ctx, model, reason_of};
use crate::pdf_engine::source_content::{classify_source_page, SourcePageResult};
use crate::pdf_engine::text_edit::lexer::{
    lex_content, scan_tokens, LexError, LexLimits, ScanMode, OPERAND_NODES,
};
use crate::pdf_engine::text_edit::limits::{set_model_bytes_override, PAGE_MODEL_BYTES_MAX};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::runs::PageModel;
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_type0, tounicode_bfchar, Program, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
use crate::pdf_engine::text_edit::testkit::{thread_peak, thread_peak_held};
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1 << 20;
/// What a page's build or Classify pass may peak at (§H R20's per-file budget).
const PEAK_MAX: usize = 256 * MIB;
/// The budget plus what is allowed outside it (one stream decoded for a hash, a lexed stream's
/// string copies): every page refused by the budget peaks below this.
const REFUSED_PEAK_MAX: usize = PAGE_MODEL_BYTES_MAX + 32 * MIB;

struct Probe {
    model: PageModel,
    model_peak: usize,
    classify: SourcePageResult,
    classify_peak: usize,
    cpu: Duration,
}

/// Builds page 0's model and classifies it, measuring each; both stay below `PEAK_MAX`, and so
/// does the inspect as a whole — the model held while its Classify pass runs (review T3-budget
/// MEDIUM-2: the two were measured apart, each from zero) — and a model never holds more than the
/// budget or than its charge.
fn probe(tag: &str, pdf: Vec<u8>) -> Probe {
    let c = ctx(pdf);
    let started = thread_cpu();
    let (m, model_peak, held) = thread_peak_held(|| model(&c, 0));
    let (classify, classify_peak) = thread_peak(|| classify_source_page(&c, &m, None));
    let cpu = thread_cpu().saturating_sub(started);
    println!(
        "{tag}: model peak {} MiB, held {} MiB, charged {} MiB, approx_bytes {} MiB, {:?} {:?}; \
         classify peak {} MiB, {:?}, {} occurrences; cpu {cpu:?}",
        model_peak / MIB,
        held / MIB,
        m.walk.model_bytes / MIB,
        m.approx_bytes() / MIB,
        m.page_reason,
        m.page_detail,
        classify_peak / MIB,
        classify.occurrence_reason,
        classify.occurrences.len()
    );
    assert!(model_peak < PEAK_MAX, "{tag}: model peak {model_peak} B");
    assert!(
        held.saturating_add(classify_peak) < PEAK_MAX,
        "{tag}: inspect peak {held} + {classify_peak} B"
    );
    assert!(
        m.approx_bytes() <= m.walk.model_bytes,
        "{tag}: approx above the charge"
    );
    assert!(
        m.walk.model_bytes <= PAGE_MODEL_BYTES_MAX,
        "{tag}: charge above the budget"
    );
    Probe {
        model: m,
        model_peak,
        classify,
        classify_peak,
        cpu,
    }
}

/// The page is refused `PAGE_TOO_COMPLEX` with `detail`, and so is every occurrence its
/// Classify pass lists (none when that pass is refused too).
fn assert_refused(tag: &str, p: &Probe, detail: &str) {
    assert_eq!(p.model.page_reason, Some(R::PageTooComplex), "{tag}");
    assert_eq!(p.model.page_detail.as_deref(), Some(detail), "{tag}");
    assert!(p.model.runs.is_empty(), "{tag}");
    let c = &p.classify;
    assert!(
        c.occurrence_reason == Some(R::PageTooComplex)
            || (c.occurrence_reason.is_none()
                && c.occurrences
                    .iter()
                    .all(|o| o.reason == Some(R::PageTooComplex))),
        "{tag}: {:?}",
        c.occurrence_reason
    );
    assert!(p.model_peak < REFUSED_PEAK_MAX, "{tag}: {} B", p.model_peak);
    assert!(
        p.classify_peak < REFUSED_PEAK_MAX,
        "{tag}: {} B",
        p.classify_peak
    );
}

/// Review r2 P17 and r3 P5/P6 at full scale: 374 arrays of 65,536 numbers in two content parts
/// (49 MB decoded), shown as TJ kerns or set as dash arrays.
fn number_arrays(tj: bool) -> Vec<u8> {
    let line = if tj {
        format!("[(a){}] TJ\n", " 1".repeat(65_535))
    } else {
        format!("[{}] 0 d\n", vec!["1"; 65_536].join(" "))
    };
    let p1 = format!("BT /F1 1 Tf 72 700 Td {}", line.repeat(187));
    let p2 = format!("{}ET", line.repeat(187));
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[p1.as_bytes(), p2.as_bytes()],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

#[test]
fn number_array_bombs_stop_in_the_lexer() {
    // They peaked at 6.3 GiB (round 2), then 1,169 MiB inside the lexer (round 3 MEDIUM-3): the
    // lexer now stops at the operand nodes the page may hold.
    for (tag, tj) in [("P17 TJ kerns", true), ("P16 dash arrays", false)] {
        let p = probe(tag, number_arrays(tj));
        assert_refused(tag, &p, "content operands");
        assert!(p.cpu < Duration::from_secs(20), "{tag}: {:?}", p.cpu);
    }
}

#[test]
fn the_lexer_counts_operand_nodes_as_it_reads_them() {
    let lim = |n: usize| LexLimits {
        operand_nodes_max: n,
        ..LexLimits::page()
    };
    // `[1 2 3] 0 d` is 5 nodes (the array, three numbers, the phase).
    let src = b"[1 2 3] 0 d";
    assert!(lex_content(src, &lim(5), None).is_ok());
    assert_eq!(
        lex_content(src, &lim(4), None),
        Err(LexError::TooComplex {
            what: OPERAND_NODES
        })
    );
    // Dictionary keys and values count, inline-image dictionaries too.
    let bdc = b"/P <</MCID 0>> BDC EMC";
    assert!(lex_content(bdc, &lim(4), None).is_ok());
    assert!(lex_content(bdc, &lim(3), None).is_err());
    let image = b"BI /W 1 /H 1 /BPC 8 /CS /G ID \x00 EI";
    assert!(lex_content(image, &lim(8), None).is_ok());
    assert!(lex_content(image, &lim(7), None).is_err());
    // The page default is the walker's cap; token scans are bounded by their own token count.
    assert_eq!(LexLimits::page().operand_nodes_max, 1_500_000);
    let many = "1 ".repeat(1_600_000);
    assert!(scan_tokens(many.as_bytes(), ScanMode::Object, 2_000_000).is_ok());
}

/// An embedded Identity-H font whose ToUnicode maps CID 1 to 256 × "A", drawn `glyphs` times.
fn long_text_page(glyphs: usize) -> Vec<u8> {
    let mut t = TtfBuilder::new();
    t.unicode_glyph('A', "A", true);
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Long");
    f.program = Program::TrueType(t.build());
    f.cid_to_gid = Some(None);
    f.flags = Some(32);
    f.w = Some("[1 [500]]".to_string());
    let long = "A".repeat(256);
    f.tounicode = Some(tounicode_bfchar(&[(1, 2, long.as_str())]));
    let mut d = DocBuilder::new();
    let f0 = add_type0(&mut d.b, &f);
    let mut c: Vec<u8> = b"BT /F0 1 Tf 72 700 Td ".to_vec();
    for _ in 0..glyphs / 1_000 {
        c.push(b'(');
        for _ in 0..1_000 {
            c.extend_from_slice(&[0, 1]);
        }
        c.extend_from_slice(b") Tj ");
    }
    c.extend_from_slice(b"ET");
    d.page(PageSpec::new(&c, &format!("/Font << /F0 {f0} 0 R >>")));
    d.build()
}

#[test]
fn the_tounicode_text_bomb_stops_at_its_character_budget() {
    // Review r2 P25 at full scale: 399,000 glyphs of 256 characters (1.45 GiB before fix pass 3).
    let p = probe("P25 text", long_text_page(399_000));
    assert_refused("P25", &p, "text characters per page");
    assert!(p.model_peak < 32 * MIB, "{} B", p.model_peak);
}

/// Review r3 P1: `names` font resources (separate Helvetica objects; `/F1` and names
/// `name_len` bytes long), and `lines` one-glyph lines in `/F1`, one run each.
fn surface_page(names: usize, name_len: usize, lines: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let mut fonts = format!("/F1 {f} 0 R");
    for i in 0..names - 1 {
        let fi = d.add(HELVETICA);
        fonts.push_str(&format!(" /N{:0w$} {fi} 0 R", i, w = name_len - 1));
    }
    let mut c = String::from("BT /F1 1 Tf 72 700 Td (a) Tj ");
    c.push_str(&"1 -1 Td (a) Tj ".repeat(lines - 1));
    c.push_str("ET");
    d.page(PageSpec::new(c.as_bytes(), &format!("/Font << {fonts} >>")));
    d.build()
}

#[test]
fn runs_share_one_surface_per_font_and_long_names_join_no_group() {
    // Review r3 HIGH-1: each run copied its 256 sibling names — 2,019 MiB for 2,000 lines with
    // 4 KiB names, ~20 GiB at 19,999 lines, 798 MiB at the 127-byte name limit.
    for (names, len, lines, held_max) in [
        (256, 4_096, 2_000, 32 * MIB),
        (256, 4_096, 19_999, 64 * MIB),
        (256, 127, 19_999, 64 * MIB),
    ] {
        let tag = format!("P1 {names} names of {len} B, {lines} lines");
        let p = probe(&tag, surface_page(names, len, lines));
        let m = &p.model;
        assert_eq!(m.page_reason, None, "{tag}: {:?}", m.page_detail);
        assert_eq!(m.runs.len(), lines, "{tag}");
        assert!(
            m.walk.model_bytes < held_max,
            "{tag}: {} B",
            m.walk.model_bytes
        );
        assert!(p.cpu < Duration::from_secs(4), "{tag}: {:?}", p.cpu);
        assert_eq!(p.classify.occurrence_reason, None, "{tag}");
        assert_eq!(p.classify.occurrences.len(), lines, "{tag}");
        let first = &m.runs[0].surface;
        assert!(
            m.runs.iter().all(|r| Arc::ptr_eq(&r.surface, first)),
            "{tag}: one shared list"
        );
        // Names past 127 bytes stay out of sibling groups: the run types with `/F1` alone.
        let want = if len > 127 { 1 } else { names };
        assert_eq!(first.len(), want, "{tag}");
        assert_eq!(first[0], b"F1".to_vec(), "{tag}");
        assert_eq!(m.surface(&m.runs[0]).fonts.len(), want, "{tag}");
    }
}

/// One ExtGState with `keys` unmodelled keys `key_len` bytes long (a shared prefix, then three
/// digits), applied `ops` times before "Hello".
fn long_key_gs(keys: usize, key_len: usize, ops: usize) -> Vec<u8> {
    let prefix = "K".repeat(key_len - 3);
    let entries: Vec<String> = (0..keys).map(|k| format!("/{prefix}{k:03} {k}")).collect();
    let gs = format!("/ExtGState << /G << {} >> >>", entries.join(" "));
    let c = format!(
        "BT /F1 12 Tf 72 700 Td {}(Hello) Tj ET",
        "/G gs ".repeat(ops)
    );
    helvetica_doc(c.as_bytes(), &gs, "")
}

#[test]
fn extgstate_keys_are_capped_and_a_repeated_gs_costs_constant_work() {
    // Review r3 MEDIUM-2: 63 keys of 256 KiB re-applied took 2.5 ms per `gs` (≈ 10 min a walk).
    let p = probe("P2 63 keys of 256 KiB", long_key_gs(63, 262_144, 20_000));
    assert_refused("P2", &p, "ExtGState keys");
    assert!(p.cpu < Duration::from_secs(2), "{:?}", p.cpu);
    // At the 127-byte limit (124 bytes shared), 20,000 `gs` stay cheap: the dictionary that put
    // the keys in force is skipped in O(1).
    let p = probe("P2 63 keys of 127 B", long_key_gs(63, 127, 20_000));
    assert_eq!(p.model.page_reason, None, "{:?}", p.model.page_detail);
    assert_eq!(reason_of(&p.model, "Hello"), None);
    assert!(p.cpu < Duration::from_secs(1), "{:?}", p.cpu);
    assert_eq!(p.model.walk.records[0].before.gs.other.len(), 63);
}

/// Review r3 P4, P7 and P8: pages at the op cap whose every show op carries a state of its own.
fn state_per_op_pages() -> Vec<(&'static str, Vec<u8>)> {
    let p4 = helvetica_page(
        format!("BT /F1 1 Tf 72 700 Td {}ET", "0 0 (a) \" ".repeat(249_990)).as_bytes(),
    );
    let keys: Vec<String> = (0..63).map(|k| format!("/Key{k:02} {k}")).collect();
    let mut gs = format!("/ExtGState << /Base << {} >>", keys.join(" "));
    let mut c = String::from("/Base gs BT /F1 1 Tf 72 700 Td ");
    for i in 0..124_990 {
        gs.push_str(&format!(" /G{i} << /Odd {i} >>"));
        c.push_str(&format!("/G{i} gs (a)Tj "));
    }
    gs.push_str(" >>");
    c.push_str("ET");
    let p7 = helvetica_doc(c.as_bytes(), &gs, "");
    let arr = format!("[(a){}] TJ\n", " -1".repeat(65_535));
    let mut c = format!("BT /F1 1 Tf 1 TL 72 700 Td {}", arr.repeat(6));
    c.push_str(&"0 0 (a) \" ".repeat(249_990 - 20));
    c.push_str("ET");
    vec![("P4", p4), ("P7", p7), ("P8", helvetica_page(c.as_bytes()))]
}

#[test]
fn pages_with_a_state_per_op_are_refused_by_the_page_budget() {
    // Review r3 MEDIUM-1: they held 401–432 MiB (and the inspect path ~850 MiB with its Classify
    // walk), while the documented worst case was 266 MiB.
    for (tag, pdf) in state_per_op_pages() {
        let p = probe(tag, pdf);
        assert_refused(tag, &p, "page model size");
        // The Classify walk would charge the same and more: it is not walked again.
        assert!(p.classify_peak < MIB, "{tag}: {} B", p.classify_peak);
    }
}

/// A page whose one Tf names a font resource `name_len` bytes long, then shows `glyphs` glyphs.
fn long_font_name_page(name_len: usize, glyphs: usize) -> Vec<u8> {
    let name = "N".repeat(name_len);
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let c = format!("BT /{name} 1 Tf 72 700 Td ({}) Tj ET", "a".repeat(glyphs));
    d.page(PageSpec::new(
        c.as_bytes(),
        &format!("/Font << /{name} {f} 0 R >>"),
    ));
    d.build()
}

/// `n` distinct ExtGStates, each adding one key to 63 in force, each applied once, and no text.
fn interned_lists_page(n: usize) -> Vec<u8> {
    let keys: Vec<String> = (0..63).map(|k| format!("/Key{k:02} {k}")).collect();
    let mut gs = format!("/ExtGState << /Base << {} >>", keys.join(" "));
    let mut c = String::from("/Base gs ");
    for i in 0..n {
        gs.push_str(&format!(" /G{i} << /Odd {i} >>"));
        c.push_str(&format!("/G{i} gs "));
    }
    gs.push_str(" >>");
    c.push_str("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    helvetica_doc(c.as_bytes(), &gs, "")
}

/// "Hello" on the page, and a Form XObject painted once that holds `n` `"` ops.
fn form_bomb_page(n: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let form = d.b.add_flate(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 {f} 0 R >> >>"),
        format!("BT /F1 1 Tf 72 600 Td {}ET", "0 0 (a) \" ".repeat(n)).as_bytes(),
    );
    d.page(PageSpec::new(
        b"/Fx Do BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Fx {form} 0 R >>"),
    ));
    d.build()
}

#[test]
fn amplifications_no_review_listed_are_refused_by_the_page_budget() {
    // Every glyph unit copies its font's resource name: one 64 KiB name and 4,000 glyphs would
    // hold 250 MiB of copies (and 1 MiB names, the lexer's cap, 4 GiB). Refused before the units
    // are made; at the 127-byte name limit the page is modelled.
    let p = probe(
        "long font name × glyphs",
        long_font_name_page(65_536, 4_000),
    );
    assert_refused("long font name", &p, "page model size");
    assert!(p.model_peak < 32 * MIB, "{} B", p.model_peak);
    let p = probe(
        "127-byte font name × glyphs",
        long_font_name_page(127, 4_000),
    );
    assert_eq!(p.model.page_reason, None, "{:?}", p.model.page_detail);
    assert_eq!(p.model.runs.len(), 1);
    // A walk interns every merged ExtGState key list it puts in force, text or not: 124,990
    // distinct `gs` over 63 keys would hold ~200 MiB of lists and draw nothing.
    let p = probe("interned key lists", interned_lists_page(124_990));
    assert_refused("interned key lists", &p, "page model size");
    // A Form only the Classify walk descends: the model is small, the Classify pass is refused on
    // its own budget and lists no occurrence (never a partial list).
    let p = probe("Form bomb", form_bomb_page(249_000));
    assert_eq!(p.model.page_reason, None, "{:?}", p.model.page_detail);
    assert_eq!(reason_of(&p.model, "Hello"), None);
    assert!(p.model.walk.model_bytes < 4 * MIB);
    assert_eq!(p.classify.occurrence_reason, Some(R::PageTooComplex));
    assert!(p.classify.occurrences.is_empty());
    assert!(p.classify_peak < REFUSED_PEAK_MAX, "{} B", p.classify_peak);
}

#[test]
fn the_structure_parent_array_is_borrowed_per_build() {
    // Review r3 LOW-2: a parent array of 2,000,000 nulls was cloned per build (259 MiB).
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let parents = d.add(format!("[{}]", "null ".repeat(2_000_000)));
    let tree = d.add(format!("<< /Nums [0 {parents} 0 R] >>"));
    let root = d.add(format!(
        "<< /Type /StructTreeRoot /ParentTree {tree} 0 R >>"
    ));
    d.page(
        PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td /P <</MCID 0>> BDC (Hello) Tj EMC ET",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .with("/StructParents 0"),
    );
    d.catalog_extra = format!("/StructTreeRoot {root} 0 R");
    // The snapshot's reference counts (built once, on first use) are made before measuring.
    let c = ctx(d.build());
    assert_eq!(reason_of(&model(&c, 0), "Hello"), None);
    let (m, peak) = thread_peak(|| model(&c, 0));
    let (classify, classify_peak) = thread_peak(|| classify_source_page(&c, &m, None));
    println!(
        "P10 parent tree: model peak {} KiB, classify peak {} KiB",
        peak >> 10,
        classify_peak >> 10
    );
    assert_eq!(reason_of(&m, "Hello"), None);
    assert_eq!(classify.occurrences.len(), 1);
    assert!(peak < MIB, "{peak} B");
    assert!(classify_peak < MIB, "{classify_peak} B");
}

#[test]
fn the_budget_refuses_a_page_the_moment_it_would_hold_more() {
    // 10,000 lines (each 30 pt to the right of the last, so no two are compared as duplicates)
    // with a colour change each: modelled within the default budget, refused under
    // one byte less than it holds, and modelled again under what it holds plus room for scratch.
    let mut c = String::from("BT /F1 12 Tf 72 700 Td ");
    for i in 0..10_000 {
        c.push_str(&format!("{} g 30 -14 Td (Hi) Tj ", i % 2));
    }
    c.push_str("ET");
    let cx = ctx(helvetica_page(c.as_bytes()));
    let m = model(&cx, 0);
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let held = m.walk.model_bytes;
    assert!(m.approx_bytes() <= held && held < 64 * MIB);
    set_model_bytes_override(Some(held - 1));
    let refused = model(&cx, 0);
    let classify = classify_source_page(&cx, &refused, None);
    set_model_bytes_override(Some(held + 32 * MIB));
    let again = model(&cx, 0);
    set_model_bytes_override(None);
    assert_eq!(refused.page_reason, Some(R::PageTooComplex));
    assert_eq!(refused.page_detail.as_deref(), Some("page model size"));
    assert!(refused.runs.is_empty());
    // A model refused for size (here at its run stage) is not walked again by its Classify pass,
    // which would charge the same and more (fix pass 2026-10-03, review T3-budget MEDIUM-1; it
    // used to list every occurrence with the page's refusal).
    assert_eq!(classify.occurrence_reason, Some(R::PageTooComplex));
    assert!(classify.occurrences.is_empty());
    assert_eq!(again.page_reason, None, "{:?}", again.page_detail);
    assert_eq!(again.walk.model_bytes, held, "charges are deterministic");
    assert_eq!(again.runs.len(), m.runs.len());
}

#[test]
fn normal_pages_hold_a_few_mib_and_never_more_than_they_charged() {
    let pages: Vec<(&str, Vec<u8>)> = vec![
        ("FX-WORD", fx::word()),
        ("FX-WORD-TR", fx::word_tr()),
        ("FX-LIBRE", fx::libre()),
        ("FX-SKIA", fx::skia()),
        ("FX-QUARTZ", fx::quartz()),
        ("FX-XETEX", fx::xetex()),
        ("FX-INDD", fx::indd()),
        ("FX-STD14", fx::std14()),
        ("FX-PERGLYPH", fx::per_glyph(None)),
        ("two columns, tagged", fx::two_column(true)),
        ("nested form", fx::nested_form()),
    ];
    for (tag, pdf) in pages {
        let p = probe(tag, pdf);
        assert_eq!(
            p.model.page_reason, None,
            "{tag}: {:?}",
            p.model.page_detail
        );
        assert!(
            p.model.walk.model_bytes < 4 * MIB,
            "{tag}: {} B",
            p.model.walk.model_bytes
        );
        assert_eq!(p.classify.occurrence_reason, None, "{tag}");
        assert!(
            p.model_peak < 16 * MIB && p.classify_peak < 16 * MIB,
            "{tag}"
        );
    }
}
