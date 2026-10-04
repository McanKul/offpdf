//! Review regressions (T3 fix pass 2): work and memory per walk stay bounded whatever a page
//! repeats — a long unpainted path (HIGH-1), many optional-content groups (MEDIUM-2), a page
//! model at the op cap (MEDIUM-3), a line alternating between many sibling fonts (LOW-1) — and
//! the Phase B wrapper's `/Group` is hashed apart from the content (LOW-3).

use super::fuzz::thread_cpu;
use super::{content, ctx, model0, reason_of};
use crate::pdf_engine::text_edit::limits::PAGE_MODEL_BYTES_MAX;
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::testkit::pdf::zlib;
use crate::pdf_engine::text_edit::testkit::producers::{
    helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::thread_peak;
use crate::pdf_engine::text_edit::walker::hash::tests::set_steps_override;
use crate::pdf_engine::text_edit::walker::{walk_page, WalkMode};
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1 << 20;

#[test]
fn a_long_unpainted_path_costs_constant_work_per_op() {
    // 200,000 rectangles (800,000 points) before one `n`: each op checks only its own points.
    let mut c = "0 0 1 1 re ".repeat(200_000);
    c.push_str("n BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let started = thread_cpu();
    let m = model0(helvetica_page(c.as_bytes()));
    let cpu = thread_cpu().saturating_sub(started);
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "Hello"), None);
    assert!(cpu < Duration::from_secs(2), "200,000 re took {cpu:?}");
    // Curves too (3 points per op).
    let mut c = String::from("0 0 m ");
    c.push_str(&"1 1 2 2 3 3 c ".repeat(100_000));
    c.push_str("S BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let started = thread_cpu();
    let m = model0(helvetica_page(c.as_bytes()));
    let cpu = thread_cpu().saturating_sub(started);
    assert_eq!(reason_of(&m, "Hello"), None);
    assert!(cpu < Duration::from_secs(2), "100,000 c took {cpu:?}");
    // A point that overflows is still refused, at the op that adds it (a CTM of scale 1e306,
    // then x = 1e9).
    let scale = "1000000000 0 0 1000000000 0 0 cm ".repeat(34);
    let c = format!("{scale} 0 0 m 1 1 l 1000000000 1 l S");
    let m = model0(helvetica_page(c.as_bytes()));
    assert_eq!(m.page_reason, Some(R::MalformedContent));
    assert!(m
        .page_detail
        .as_deref()
        .is_some_and(|d| d.starts_with("non-finite path")));
}

/// `k` optional-content groups, all listed in `/OCGs` and `/D /ON` except the last, which is in
/// `/OFF`; each is opened once, and "Hello" is drawn inside group `inside`.
fn ocg_page(k: usize, inside: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let ids: Vec<u32> = (0..k)
        .map(|i| d.add(format!("<< /Type /OCG /Name (L{i}) >>")))
        .collect();
    let refs: Vec<String> = ids.iter().map(|i| format!("{i} 0 R")).collect();
    let props: Vec<String> = ids
        .iter()
        .enumerate()
        .map(|(n, i)| format!("/P{n} {i} 0 R"))
        .collect();
    let mut c = String::new();
    for n in 0..k {
        if n == inside {
            c.push_str(&format!(
                "/OC /P{n} BDC BT /F1 12 Tf 72 700 Td (Hello) Tj ET EMC "
            ));
        } else {
            c.push_str(&format!("/OC /P{n} BDC EMC "));
        }
    }
    let (on, off) = refs.split_at(k.saturating_sub(1));
    d.catalog_extra = format!(
        "/OCProperties << /OCGs [{}] /D << /ON [{}] /OFF [{}] >> >>",
        refs.join(" "),
        on.join(" "),
        off.join(" ")
    );
    d.page(PageSpec::new(
        c.as_bytes(),
        &format!(
            "/Font << /F1 {f} 0 R >> /Properties << {} >>",
            props.join(" ")
        ),
    ));
    d.build()
}

#[test]
fn optional_content_states_cost_constant_work_per_group() {
    // 20,000 distinct groups: the catalog's lists are read once, each lookup is a binary search
    // (scanning /OCGs, /ON and /OFF per group took ~4.6 s in a debug build).
    let pdf = ocg_page(20_000, 10_000);
    let started = thread_cpu();
    let m = model0(pdf);
    let cpu = thread_cpu().saturating_sub(started);
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "Hello"), None, "a group on in /D is visible");
    assert!(cpu < Duration::from_secs(2), "20,000 groups took {cpu:?}");
    // The same lists still decide: the last group is in /OFF.
    let m = model0(ocg_page(50, 49));
    assert_eq!(reason_of(&m, "Hello"), Some(R::OptionalContent));
}

#[test]
fn a_page_model_at_the_op_cap_shares_its_digests_and_reports_its_size() {
    // 249,990 one-glyph show ops under one state (≈ 2.2 KB deflated): every record shares one
    // digest and keeps no spare capacity. It peaked at ~700 MiB with two digests per record,
    // ~360 MiB with shared digests alone and ~260 MiB after fix pass 2 (≈ 1 KB per record).
    // [T3-budget] That is more than one page may hold (`PAGE_MODEL_BYTES_MAX`): the page is now
    // refused before it holds more; 100,000 such ops are still modelled the same way.
    let page = |n: usize| {
        let mut c = String::from("BT /F1 1 Tf 72 700 Td ");
        c.push_str(&"(a)Tj ".repeat(n));
        c.push_str("ET");
        c
    };
    let c = page(249_990);
    assert!(zlib(c.as_bytes()).len() < 4096);
    let (m, peak) = thread_peak(|| model0(helvetica_page(c.as_bytes())));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("page model size"));
    println!("op cap: refused, peak {} MiB", peak / MIB);
    assert!(
        peak < PAGE_MODEL_BYTES_MAX + 32 * MIB,
        "the refused model peaked at {} MiB",
        peak / MIB
    );
    let c = page(100_000);
    let started = thread_cpu();
    let (m, peak) = thread_peak(|| model0(helvetica_page(c.as_bytes())));
    let cpu = thread_cpu().saturating_sub(started);
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    let records = &m.walk.records;
    assert_eq!(records.len(), 100_000);
    let (first, last) = (&records[0], &records[99_999]);
    assert!(Arc::ptr_eq(&first.before, &first.after));
    assert!(Arc::ptr_eq(&first.before, &last.after));
    let approx = m.approx_bytes();
    println!(
        "100,000 ops: peak {} MiB, approx_bytes {} MiB, charged {} MiB, cpu {cpu:?}",
        peak / MIB,
        approx / MIB,
        m.walk.model_bytes / MIB
    );
    assert!(peak < 128 * MIB, "the model peaked at {} MiB", peak / MIB);
    assert!(
        approx <= peak && approx >= peak / 2,
        "approx_bytes {approx} B against a peak of {peak} B"
    );
    assert!(approx <= m.walk.model_bytes && m.walk.model_bytes <= PAGE_MODEL_BYTES_MAX);
}

#[test]
fn digests_are_shared_only_while_the_state_is_unchanged() {
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (a) Tj (b) Tj 1 0 0 rg (c) Tj 0 g (d) Tj 0 g (e) Tj ET \
          0 0 10 10 re f 0 0 10 10 re f",
    ));
    let r = &m.walk.records;
    assert_eq!(r.len(), 5);
    assert!(Arc::ptr_eq(&r[0].after, &r[1].before), "unchanged state");
    assert!(!Arc::ptr_eq(&r[1].after, &r[2].before), "a colour change");
    assert_eq!(r[2].before.fill.hex().as_deref(), Some("#ff0000"));
    // `0 g` twice: equal values under new op bytes get a new digest, equal by value.
    assert!(!Arc::ptr_eq(&r[3].before, &r[4].before));
    assert_eq!(*r[3].before, *r[4].before);
    assert_eq!(r[1].before.fill.hex().as_deref(), Some("#000000"));
    let p = &m.walk.paints;
    assert_eq!(p.len(), 2);
    assert!(Arc::ptr_eq(&p[0].state, &p[1].state), "paints share too");
    assert!(m.approx_bytes() > 0);
}

#[test]
fn a_line_alternating_between_sibling_fonts_builds_each_group_once() {
    // 256 Helvetica resources (all siblings), alternating on one line 64,000 times: each
    // resource's sibling group is built once per page (it was once per join candidate, O(256²)
    // each).
    let mut d = DocBuilder::new();
    let names: Vec<String> = (0..256)
        .map(|k| format!("/F{k} {} 0 R", d.add(HELVETICA)))
        .collect();
    let mut c = String::from("BT 72 700 Td ");
    for i in 0..64_000 {
        c.push_str(&format!("/F{} 1 Tf (a)Tj ", i % 256));
    }
    c.push_str("ET");
    d.page(PageSpec::new(
        c.as_bytes(),
        &format!("/Font << {} >>", names.join(" ")),
    ));
    let pdf = d.build();
    let started = thread_cpu();
    let m = model0(pdf);
    let cpu = thread_cpu().saturating_sub(started);
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert!(
        cpu < Duration::from_secs(4),
        "64,000 font switches took {cpu:?}"
    );
    // Siblings still join (64,000 single-member runs would exceed RUNS_PER_PAGE_MAX).
    assert!(m.runs.len() < 1_000, "{} runs", m.runs.len());
    assert!(m.runs.iter().all(|r| r.members.len() > 1));
}

/// A wrapper page whose `/Group` and the wrapper Form's both name one object holding `pad`
/// numbers, and whose content paints an image whose dictionary holds `pad` numbers too.
fn wrapper_with_group(pad: usize) -> Vec<u8> {
    let numbers = vec!["1"; pad].join(" ");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let group = d.add(format!(
        "<< /S /Transparency /CS /DeviceRGB /Pad [{numbers}] >>"
    ));
    let image = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray \
             /BitsPerComponent 8 /Pad [{numbers}]"
        ),
        &[0],
    );
    let form = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Group {group} 0 R \
             /Resources << /Font << /F1 {f} 0 R >> /XObject << /Im0 {image} 0 R >> >>"
        ),
        b"q 10 0 0 10 300 300 cm /Im0 Do Q BT /F1 12 Tf 72 700 Td (Hi) Tj ET",
    );
    d.page(
        PageSpec::new(
            b"q 1 0 0 1 0 0 cm /Fx0 Do Q",
            &format!("/XObject << /Fx0 {form} 0 R >>"),
        )
        .with(&format!("/Group {group} 0 R")),
    );
    d.build()
}

#[test]
fn the_wrapper_group_is_hashed_apart_from_the_content() {
    // With a 1,000-step hash budget, the /Group (~600 steps) and the image (~600 steps) each
    // fit, but not together: the wrapper check must not spend the content's budget.
    let c = ctx(wrapper_with_group(600));
    let pc = content(&c, 0);
    set_steps_override(Some(1_000));
    let w = walk_page(
        &c,
        0,
        &pc,
        WalkMode::Wrapped {
            name: b"Fx0".to_vec(),
        },
        None,
    );
    set_steps_override(None);
    assert_eq!(w.page_reason, None, "{:?}", w.page_detail);
    assert_eq!(w.records.len(), 1);
    assert_eq!(w.paints.len(), 1);
    // The group alone over the budget still refuses.
    let c = ctx(wrapper_with_group(1_200));
    let pc = content(&c, 0);
    set_steps_override(Some(1_000));
    let w = walk_page(
        &c,
        0,
        &pc,
        WalkMode::Wrapped {
            name: b"Fx0".to_vec(),
        },
        None,
    );
    set_steps_override(None);
    assert_eq!(w.page_reason, Some(R::PageTooComplex));
}
