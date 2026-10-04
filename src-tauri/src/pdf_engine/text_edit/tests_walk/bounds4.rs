//! Review regressions (T3 fix pass 3, review T3 r2): what a page model keeps per element is
//! budgeted per page — TJ kerns (HIGH-1), lexed operand nodes alive at once (HIGH-1, page content
//! and descended Forms), characters of glyph text (HIGH-2) — and `approx_bytes` counts every
//! shared allocation a digest holds, while a state re-set to the value in force keeps sharing its
//! digest (MEDIUM-2: the review's P13, P14 and P15 pages, smaller).

use super::fuzz::thread_cpu;
use super::{ctx, model, model0, reason_of, walk};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::runs::PageModel;
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_type0, tounicode_bfchar, Program, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::pdf::zlib;
use crate::pdf_engine::text_edit::testkit::producers::{
    helvetica_doc, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::thread_peak;
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
use crate::pdf_engine::text_edit::walker::{
    WalkMode, KERNS_PER_PAGE_MAX, OPERAND_NODES_MAX, TEXT_CHARS_PER_PAGE_MAX,
};
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1 << 20;

/// Bytes `f`'s result still holds when it returns: the thread's peak above an untouched 1 GiB
/// reservation made after `f` (larger than every build peak of these tests, which assert their
/// peaks far below it, so the peak is the held bytes plus the reservation). The snapshot context
/// must be made outside `f`: its parse allocates on other threads, and freeing that here would
/// count as negative bytes.
fn held_by<T>(f: impl FnOnce() -> T) -> (T, usize) {
    const PAD: usize = 1 << 30;
    let ((value, pad), peak) = thread_peak(|| {
        let value = f();
        let pad: Vec<u8> = Vec::with_capacity(PAD);
        (value, pad)
    });
    drop(pad);
    assert!(peak >= PAD, "peak {peak} below the reservation");
    (value, peak - PAD)
}

/// `approx_bytes` lies in [held / 1.25, held], give or take an eighth of the font models it
/// counts: since the fix pass of 2026-10-03 a model counts the fonts it keeps alive, by
/// `FontModel::approx_bytes`, an estimate (Helvetica's is ~2 KB above what it allocates).
fn assert_size(tag: &str, m: &PageModel, held: usize) {
    let approx = m.approx_bytes();
    let mut seen = std::collections::HashSet::new();
    let fonts: usize = m
        .walk
        .page_fonts
        .iter()
        .map(|(_, f)| f)
        .chain(m.walk.records.iter().filter_map(|r| r.font.as_ref()))
        .filter(|f| seen.insert(std::sync::Arc::as_ptr(f) as usize))
        .map(|f| f.approx_bytes())
        .sum();
    println!(
        "{tag}: held {} KiB, approx_bytes {} KiB (fonts {} KiB)",
        held >> 10,
        approx >> 10,
        fonts >> 10
    );
    assert!(
        approx <= held + fonts / 8 && approx >= held / 5 * 4,
        "{tag}: approx_bytes {approx} B against {held} B held"
    );
}

/// `ops` TJ ops of one glyph followed by `kerns` kerns of 1/1000 em each (to the right).
fn kern_page(ops: usize, kerns: usize) -> Vec<u8> {
    let arr = format!("[(a){}] TJ\n", " -1".repeat(kerns));
    helvetica_page(format!("BT /F1 1 Tf 72 700 Td {}ET", arr.repeat(ops)).as_bytes())
}

#[test]
fn tj_kerns_are_charged_to_a_page_budget() {
    assert_eq!(KERNS_PER_PAGE_MAX, 400_000);
    // Review P17, smaller: 7 TJ arrays of 65,535 kerns (458,745) from ~4 KB of deflated
    // content. Uncounted, P17's 374 arrays held 4.3 GiB (~184 B per kern).
    let pdf = kern_page(7, 65_535);
    let c = ctx(pdf);
    let (m, peak) = thread_peak(|| model(&c, 0));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("TJ kerns per page"));
    assert!(m.walk.ops.is_empty() && m.walk.records.is_empty());
    assert!(
        peak < 48 * MIB,
        "a refused kern page peaked at {} MiB",
        peak / MIB
    );
    // The inspect path's Classify walk is refused the same way.
    let w = walk(&c, 0, WalkMode::Classify);
    assert_eq!(w.page_reason, Some(R::PageTooComplex));
    assert_eq!(w.page_detail.as_deref(), Some("TJ kerns per page"));
    // Under the budget (6 × 65,535 = 393,210) the page is modelled and its size reported.
    let c = ctx(kern_page(6, 65_535));
    let ((m, cpu), held) = held_by(|| {
        let started = thread_cpu();
        let m = model(&c, 0);
        (m, thread_cpu().saturating_sub(started))
    });
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    // One line: six glyphs, 65.5 em apart (each gap reads as one synthetic space).
    assert_eq!(reason_of(&m, "a a a a a a"), None);
    assert!(held < 96 * MIB, "393,210 kerns hold {} MiB", held / MIB);
    assert!(cpu < Duration::from_secs(4), "393,210 kerns took {cpu:?}");
    assert_size("393,210 kerns", &m, held);
}

/// `lines` × `[1 1 … 1] 0 d 0 0 1 1 re f` (65,542 operand nodes each), then "Hello".
fn dash_lines(lines: usize) -> String {
    let line = format!("[{}] 0 d 0 0 1 1 re f\n", vec!["1"; 65_536].join(" "));
    line.repeat(lines)
}

#[test]
fn lexed_operand_nodes_are_charged_to_a_page_budget() {
    assert_eq!(OPERAND_NODES_MAX, 1_500_000);
    // Review P16, smaller: 23 dash arrays of 65,536 numbers (1,507,466 nodes). Kept, P16's 748
    // arrays held 1.26 GiB in `PageWalk::ops`.
    let content = format!("{}BT /F1 12 Tf 72 700 Td (Hello) Tj ET", dash_lines(23));
    assert!(zlib(content.as_bytes()).len() < 16 << 10);
    let c = ctx(helvetica_page(content.as_bytes()));
    let (m, held) = held_by(|| model(&c, 0));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("content operands"));
    assert!(
        held < content.len() + MIB,
        "a refused page holds {} KiB",
        held >> 10
    );
    // 22 arrays (1,441,929 nodes) fit, and `approx_bytes` counts them.
    let content = format!("{}BT /F1 12 Tf 72 700 Td (Hello) Tj ET", dash_lines(22));
    let c = ctx(helvetica_page(content.as_bytes()));
    let (m, held) = held_by(|| model(&c, 0));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "Hello"), None);
    assert_size("1,441,929 operand nodes", &m, held);
}

/// A page painting Form `/Fa` (`a` dash lines; it paints `/Fb` when `nested`) `paints` times,
/// and Form `/Fb` (`b` dash lines) `paints` more times unless nested.
fn form_page(a: usize, b: usize, nested: bool, paints: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let fb = d.b.add_flate(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792]",
        dash_lines(b).as_bytes(),
    );
    let inner = if nested { "/Fb Do" } else { "" };
    let fa = d.b.add_flate(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
             /Resources << /XObject << /Fb {fb} 0 R >> >>"
        ),
        format!("{}{inner}", dash_lines(a)).as_bytes(),
    );
    let mut content = "/Fa Do ".repeat(paints);
    if !nested {
        content.push_str(&"/Fb Do ".repeat(paints));
    }
    content.push_str("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Fa {fa} 0 R /Fb {fb} 0 R >>"),
    ));
    d.build()
}

#[test]
fn form_operands_count_only_while_the_form_runs() {
    // Classify descends Forms: a Form's lexed ops live while it runs, nested Forms on top of
    // their parents. Two nested Forms of 12 dash lines each (786,504 nodes each) are refused.
    let c = ctx(form_page(12, 12, true, 1));
    let w = walk(&c, 0, WalkMode::Classify);
    assert_eq!(w.page_reason, Some(R::PageTooComplex));
    assert_eq!(w.page_detail.as_deref(), Some("content operands"));
    // Edit mode paints Forms without descending.
    assert_eq!(walk(&c, 0, WalkMode::Edit).page_reason, None);
    // The same two Forms side by side, each painted twice: never more than one alive.
    let c = ctx(form_page(12, 12, false, 2));
    let w = walk(&c, 0, WalkMode::Classify);
    assert_eq!(w.page_reason, None, "{:?}", w.page_detail);
    assert_eq!(w.paints.len(), 4 * (1 + 12));
}

/// An embedded Identity-H TrueType font whose ToUnicode maps CID 1 to 256 × "A" (the longest
/// destination T2 reads), drawn `glyphs` times in Tj ops of at most 1,000 glyphs.
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
    let mut left = glyphs;
    while left > 0 {
        let n = left.min(1_000);
        c.push(b'(');
        for _ in 0..n {
            c.extend_from_slice(&[0, 1]);
        }
        c.extend_from_slice(b") Tj ");
        left -= n;
    }
    c.extend_from_slice(b"ET");
    d.page(PageSpec::new(&c, &format!("/Font << /F0 {f0} 0 R >>")));
    d.build()
}

#[test]
fn glyph_text_is_charged_to_a_page_budget() {
    assert_eq!(TEXT_CHARS_PER_PAGE_MAX, 800_000);
    // Review P25, smaller: 40,000 glyphs of 256 characters each (10 M characters). Uncounted,
    // P25's 399,000 glyphs held 1.45 GiB (≈ 3.8 KB per glyph).
    let c = ctx(long_text_page(40_000));
    let (m, peak) = thread_peak(|| model(&c, 0));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("text characters per page"));
    assert!(
        peak < 16 * MIB,
        "a refused page peaked at {} MiB",
        peak / MIB
    );
    // 3,125 glyphs are exactly 800,000 characters; one more is refused.
    let m = model0(long_text_page(3_126));
    assert_eq!(m.page_detail.as_deref(), Some("text characters per page"));
    let c = ctx(long_text_page(3_125));
    let (m, held) = held_by(|| model(&c, 0));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let chars: usize = m.runs.iter().map(|r| r.text.chars().count()).sum();
    assert_eq!(chars, 800_000);
    assert!(
        held < 32 * MIB,
        "800,000 characters hold {} MiB",
        held / MIB
    );
    assert_size("800,000 characters", &m, held);
}

/// `n` repetitions of `each` inside one text object (after `prefix`, before `suffix`).
fn repeated(prefix: &str, each: &str, n: usize, suffix: &str) -> String {
    format!("{prefix}BT /F1 1 Tf 72 700 Td {}ET{suffix}", each.repeat(n))
}

#[test]
fn a_state_reset_to_its_value_keeps_its_digest_and_approx_bytes_counts_shared_parts() {
    const N: usize = 40_000;
    // Review P13: a new colour op before every show op, so a new digest per record.
    let c = ctx(helvetica_page(repeated("", "0 g (a)Tj ", N, "").as_bytes()));
    let (p13, held13) = held_by(|| model(&c, 0));
    assert_eq!(p13.page_reason, None, "{:?}", p13.page_detail);
    assert_size("P13", &p13, held13);
    // Review P14: one ExtGState of 63 unmodelled keys re-applied before every show op. Its key
    // list is kept, so every record shares one digest (it was a new digest and a new 63-entry
    // list per op: 2.5 × P13).
    let keys: Vec<String> = (0..63).map(|k| format!("/Key{k:02} {k}")).collect();
    let gs = format!(
        "/ExtGState << /G << /Type /ExtGState {} >> >>",
        keys.join(" ")
    );
    let pdf = helvetica_doc(repeated("", "/G gs (a)Tj ", N, "").as_bytes(), &gs, "");
    let c = ctx(pdf);
    let (p14, held14) = held_by(|| model(&c, 0));
    let r = &p14.walk.records;
    assert_eq!(r.len(), N);
    assert_eq!(r[0].before.gs.other.len(), 63);
    assert!(Arc::ptr_eq(&r[0].before, &r[N - 1].after));
    assert!(held14 < held13, "P14 holds {held14} B, P13 {held13} B");
    assert_size("P14", &p14, held14);
    // Review P15: a 63-deep marked-content stack, reopened around every show op. The stack is
    // equal to the last digest's, so it is that digest's stack again (it was a copy per op).
    let open = "/X BMC ".repeat(63);
    let close = " EMC".repeat(63);
    let c = ctx(helvetica_page(
        repeated(&open, "EMC /X BMC (a)Tj ", N, &close).as_bytes(),
    ));
    let (p15, held15) = held_by(|| model(&c, 0));
    let r = &p15.walk.records;
    assert_eq!(r[0].before.marked.len(), 63);
    assert!(Arc::ptr_eq(&r[0].before, &r[N - 1].after));
    assert!(held15 < held13, "P15 holds {held15} B, P13 {held13} B");
    assert_size("P15", &p15, held15);
    // The stack is shared even when the rest of the state changes on every op.
    let c = ctx(helvetica_page(
        repeated(&open, "EMC /X BMC 0 g (a)Tj ", N, &close).as_bytes(),
    ));
    let (m, held_mixed) = held_by(|| model(&c, 0));
    let r = &m.walk.records;
    assert!(!Arc::ptr_eq(&r[0].before, &r[1].before));
    assert!(r[0].before.marked.same(&r[N - 1].before.marked));
    assert_size("P15 with a colour per op", &m, held_mixed);
    // `BM`, `RI`, `/D`, `ri` and `d` re-set to the values in force keep sharing too.
    let gs = "/ExtGState << /G << /BM /Multiply /RI /Perceptual /D [[3 1] 0] /CA 0.5 >> >>";
    let each = "/G gs /Perceptual ri [3 1] 0 d (a)Tj ";
    let pdf = helvetica_doc(repeated("", each, 2_000, "").as_bytes(), gs, "");
    let m = model0(pdf);
    let r = &m.walk.records;
    assert_eq!(r[0].before.gs.blend.to_vec(), b"Multiply".to_vec());
    assert_eq!(r[0].before.gs.dash.0.to_vec(), vec![3.0, 1.0]);
    assert!(Arc::ptr_eq(&r[0].before, &r[1_999].after));
}

/// A stack `depth` deep whose innermost tag differs on every one of `n` show ops.
fn new_tag_per_op(depth: usize, n: usize) -> Vec<u8> {
    let mut c = "/X BMC ".repeat(depth);
    c.push_str("BT /F1 1 Tf 72 700 Td ");
    for i in 0..n {
        c.push_str(&format!("EMC /T{i} BMC (a)Tj "));
    }
    c.push_str("ET");
    c.push_str(&" EMC".repeat(depth));
    helvetica_page(c.as_bytes())
}

#[test]
fn a_marked_stack_reopened_with_a_new_tag_costs_one_node_per_op() {
    // A 63-deep stack whose innermost tag differs on every show op. As a copied vector each
    // record held a 63-entry stack of its own (+2.5 KB per record, ~500 MiB at the op cap); as a
    // persistent list it holds one new node and shares the 62 below, so the page holds what a
    // 1-deep stack holds. `approx_bytes` counts each node once (counting only the digests, it
    // fell to ~55 % of the copied-vector page).
    const N: usize = 20_000;
    let cx = ctx(new_tag_per_op(63, N));
    let (m, held) = held_by(|| model(&cx, 0));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let r = &m.walk.records;
    assert_eq!(r.len(), N);
    let (first, last) = (&r[0].before.marked, &r[N - 1].before.marked);
    assert_eq!(first.len(), 63);
    assert!(!first.same(last));
    assert_eq!(
        first.iter().next().map(|e| e.tag.to_vec()),
        Some(b"T0".to_vec())
    );
    let (mut a, mut b) = (first.clone(), last.clone());
    a.pop();
    b.pop();
    assert!(a.same(&b), "the 62 entries below are shared");
    assert_size("a new tag per op, 63 deep", &m, held);
    let shallow = ctx(new_tag_per_op(1, N));
    let (m1, held1) = held_by(|| model(&shallow, 0));
    assert_eq!(m1.page_reason, None, "{:?}", m1.page_detail);
    println!("a new tag per op, 1 deep: held {} KiB", held1 >> 10);
    assert!(
        held < held1 + (64 << 10),
        "63 deep holds {held} B, 1 deep {held1} B"
    );
}

#[test]
fn alternating_extgstates_share_their_key_lists() {
    // 63 unmodelled keys in force, then two ExtGStates that each set one more key to a value of
    // their own, alternating before every show op: each `gs` changes the state, but the merged
    // key list is one of two, shared (it was a new 64-entry list per op, +1.5 KB per record).
    const N: usize = 20_000;
    let keys: Vec<String> = (0..63).map(|k| format!("/Key{k:02} {k}")).collect();
    let gs = format!(
        "/ExtGState << /Base << {} >> /A << /Odd 1 >> /B << /Odd 2 >> >>",
        keys.join(" ")
    );
    let content = repeated("/Base gs ", "/A gs (a)Tj /B gs (a)Tj ", N / 2, "");
    let cx = ctx(helvetica_doc(content.as_bytes(), &gs, ""));
    let (m, held) = held_by(|| model(&cx, 0));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let r = &m.walk.records;
    assert_eq!(r.len(), N);
    assert_eq!(r[0].before.gs.other.len(), 64);
    assert!(!Arc::ptr_eq(&r[0].before.gs.other, &r[1].before.gs.other));
    assert!(Arc::ptr_eq(
        &r[0].before.gs.other,
        &r[N - 2].before.gs.other
    ));
    assert!(Arc::ptr_eq(
        &r[1].before.gs.other,
        &r[N - 1].before.gs.other
    ));
    assert_size("alternating ExtGStates", &m, held);
}

#[test]
fn approx_bytes_counts_the_verbatim_bytes_digests_hold() {
    // Each colour op carries a 2,000-byte comment inside its span, and each record's digest
    // holds those verbatim bytes (for B14 restores): about a third of what the model holds.
    const N: usize = 10_000;
    let each = format!("0 %{}\n g (a)Tj ", "x".repeat(2_000));
    let cx = ctx(helvetica_page(repeated("", &each, N, "").as_bytes()));
    let (m, held) = held_by(|| model(&cx, 0));
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let op = m.walk.records[0]
        .before
        .fill
        .color_op
        .as_ref()
        .map(|b| b.len());
    assert!(op.is_some_and(|n| n > 2_000), "{op:?}");
    assert_size("2,000-byte colour ops", &m, held);
}
