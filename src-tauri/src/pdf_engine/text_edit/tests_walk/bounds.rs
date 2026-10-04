//! Review regressions (T3 fix pass): the walk's work and memory stay bounded whatever a page
//! repeats — state digests share their bytes (HIGH-1), deep hashes are memoised and charged per
//! walk (HIGH-2), and the page-wide run checks hold one flag per run and a capped number of
//! comparisons (MEDIUM-1).

use super::fuzz::thread_cpu;
use super::{model0, run_with, texts};
use crate::pdf_engine::text_edit::limits::{PAGE_DECODE_BUDGET, STREAM_MAX_DECODED};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::testkit::pdf::zlib_zero_bomb;
use crate::pdf_engine::text_edit::testkit::producers::{
    helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::thread_peak;
use crate::pdf_engine::text_edit::walker::hash::tests::{set_steps_override, take_decode_attempts};
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1 << 20;

/// `records` × `(a)Tj` after `prefix`, in one text object.
fn many_shows(prefix: &[u8], records: usize) -> Vec<u8> {
    let mut c = prefix.to_vec();
    c.extend_from_slice(b" BT /F1 1 Tf 72 700 Td ");
    for _ in 0..records {
        c.extend_from_slice(b"(a)Tj ");
    }
    c.extend_from_slice(b"ET");
    c
}

#[test]
fn state_digests_share_verbatim_bytes_and_line_state() {
    // 1 MiB between `0` and `g`: the op span is 1 MiB, and 200 records each hold two digests.
    let mut padded = b"0".to_vec();
    padded.extend(std::iter::repeat(b' ').take(MIB));
    padded.extend_from_slice(b" g");
    let (m, peak) = thread_peak(|| model0(helvetica_page(&many_shows(&padded, 200))));
    assert_eq!(m.page_reason, None);
    assert_eq!(m.walk.records.len(), 200);
    assert!(
        peak < 24 * MIB,
        "1 MiB colour op × 200 records peaked at {peak} B"
    );
    let (first, last) = (&m.walk.records[0], &m.walk.records[199]);
    let (a, b) = (
        first.before.fill.color_op.as_ref().expect("colour op"),
        last.after.fill.color_op.as_ref().expect("colour op"),
    );
    assert!(
        Arc::ptr_eq(a, b),
        "the verbatim bytes are shared, not copied"
    );
    assert_eq!(a.len(), MIB + 3);

    // A 60,000-entry dash array in force for 200 records.
    let mut dash = b"[".to_vec();
    for _ in 0..60_000 {
        dash.extend_from_slice(b"1 ");
    }
    dash.extend_from_slice(b"] 0 d");
    let (m, peak) = thread_peak(|| model0(helvetica_page(&many_shows(&dash, 200))));
    assert_eq!(m.page_reason, None);
    assert_eq!(m.walk.records[199].before.gs.dash.0.len(), 60_000);
    assert!(peak < 24 * MIB, "60k dash × 200 records peaked at {peak} B");

    // A marked-content tag of 64 KiB under 200 records.
    let mut tagged = b"/".to_vec();
    tagged.extend(std::iter::repeat(b'T').take(64 << 10));
    tagged.extend_from_slice(b" BMC");
    let mut c = many_shows(&tagged, 200);
    c.extend_from_slice(b" EMC");
    let (m, peak) = thread_peak(|| model0(helvetica_page(&c)));
    assert_eq!(m.page_reason, None);
    assert!(
        peak < 16 * MIB,
        "64 KiB tag × 200 records peaked at {peak} B"
    );
}

/// One page drawing "Hello" after `ops`, with `/GS<i>` ExtGStates from `ext_gstates`.
fn gs_page(ops: &str, ext_gstates: &[String]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let names: Vec<String> = ext_gstates
        .iter()
        .enumerate()
        .map(|(i, body)| format!("/GS{i} {} 0 R", d.add(body)))
        .collect();
    let content = format!("{ops} BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!(
            "/Font << /F1 {f} 0 R >> /ExtGState << {} >>",
            names.join(" ")
        ),
    ));
    d.build()
}

/// An ExtGState with `n` unmodelled keys `/<prefix>0 … /<prefix>{n-1}`.
fn private_keys(prefix: &str, n: usize) -> String {
    let keys: Vec<String> = (0..n).map(|i| format!("/{prefix}{i} {i}")).collect();
    format!("<< {} >>", keys.join(" "))
}

#[test]
fn extgstate_work_per_gs_is_capped() {
    // 8,000 keys in one dictionary: refused before any per-key work.
    let started = thread_cpu();
    let m = model0(gs_page("/GS0 gs", &[private_keys("K", 8_000)]));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("ExtGState keys"));
    // 40 private keys applied 20,000 times: one sort and merge per `gs`, digests shared.
    let ops = "/GS0 gs ".repeat(20_000);
    let m = model0(gs_page(&ops, &[private_keys("K", 40)]));
    assert_eq!(m.page_reason, None);
    assert_eq!(m.walk.records[0].before.gs.other.len(), 40);
    assert_eq!(run_with(&m, "Hello").reason, None);
    assert!(
        thread_cpu().saturating_sub(started) < Duration::from_secs(4),
        "ExtGState work"
    );
    // Two dictionaries of 40 different private keys: 80 in force at once is too many.
    let m = model0(gs_page(
        "/GS0 gs /GS1 gs",
        &[private_keys("K", 40), private_keys("L", 40)],
    ));
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    // A later value of the same key replaces the earlier one (sorted, one entry per key).
    let m = model0(gs_page(
        "/GS0 gs /GS1 gs",
        &[
            "<< /TR /Identity /HT /Default >>".into(),
            "<< /TR /Identity >>".into(),
        ],
    ));
    let keys: Vec<&[u8]> = m.walk.records[0]
        .before
        .gs
        .other
        .iter()
        .map(|(k, _)| &k[..])
        .collect();
    assert_eq!(keys, [&b"HT"[..], b"TR"]);
}

/// A page painting the shading `/Sh<k>` `uses` times for each of `distinct` 33 MiB Flate bombs.
fn bomb_shadings(distinct: usize, uses: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let bomb = zlib_zero_bomb(33);
    let names: Vec<String> = (0..distinct)
        .map(|k| {
            let id = d.b.add_stream(
                "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 \
                 /BitsPerFlag 8 /Decode [0 1 0 1 0 1] /Filter /FlateDecode",
                &bomb,
            );
            format!("/Sh{k} {id} 0 R")
        })
        .collect();
    let mut c = String::new();
    for _ in 0..uses {
        for k in 0..distinct {
            c.push_str(&format!("/Sh{k} sh\n"));
        }
    }
    c.push_str("BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    d.page(PageSpec::new(
        c.as_bytes(),
        &format!("/Font << /F1 {f} 0 R >> /Shading << {} >>", names.join(" ")),
    ));
    d.build()
}

#[test]
fn deep_hashes_are_memoised_and_charged_per_walk() {
    // 1,000 `sh` of one shading that inflates past STREAM_MAX_DECODED: one decode attempt.
    let pdf = bomb_shadings(1, 1_000);
    take_decode_attempts();
    let started = thread_cpu();
    let m = model0(pdf);
    let spent = thread_cpu().saturating_sub(started);
    let (attempts, _) = take_decode_attempts();
    assert_eq!(m.page_reason, None);
    assert_eq!(texts(&m), ["Hello"]);
    assert_eq!(attempts, 1, "the shading is hashed once per walk");
    assert!(spent < Duration::from_secs(2), "1,000 sh took {spent:?}");

    // The same bomb as an ExtGState transfer function behind 1,000 `gs`.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let tr = d.b.add_stream(
        "/FunctionType 4 /Domain [0 1] /Range [0 1] /Filter /FlateDecode",
        &zlib_zero_bomb(33),
    );
    d.page(PageSpec::new(
        format!(
            "{}BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
            "/GS1 gs\n".repeat(1_000)
        )
        .as_bytes(),
        &format!("/Font << /F1 {f} 0 R >> /ExtGState << /GS1 << /TR {tr} 0 R >> >>"),
    ));
    let m = model0(d.build());
    assert_eq!(m.page_reason, None);
    assert_eq!(
        take_decode_attempts().0,
        1,
        "the /TR stream is hashed once per walk"
    );

    // 24 different bombs: a failed decode is charged its whole cap, so after the budget is spent
    // the later attempts may inflate nothing (Σ caps ≤ budget + one stream cap).
    let m = model0(bomb_shadings(24, 2));
    let (attempts, caps) = take_decode_attempts();
    assert_eq!(m.page_reason, None);
    assert_eq!(attempts, 24);
    assert!(
        caps <= PAGE_DECODE_BUDGET + STREAM_MAX_DECODED,
        "Σ caps {caps} B: failed decodes were not charged"
    );
}

#[test]
fn deep_hash_steps_are_counted_per_walk() {
    let array = |n: usize| format!("[{}]", "0 ".repeat(n));
    let gs = |n: usize| format!("<< /TR {} >>", array(n));
    set_steps_override(Some(1_000));
    // One 600-item direct value used 50 times: hashed once (memoised by address).
    let m = model0(gs_page(&"/GS0 gs ".repeat(50), &[gs(600)]));
    let one = m.page_reason;
    // Two such values in one walk: 1,200 steps > 1,000, though each hash alone fits.
    let m = model0(gs_page("/GS0 gs /GS1 gs", &[gs(600), gs(600)]));
    set_steps_override(None);
    assert_eq!(one, None, "a repeated value is hashed once");
    assert_eq!(m.page_reason, Some(R::PageTooComplex));
    assert_eq!(m.page_detail.as_deref(), Some("object hash budget"));
}

#[test]
fn duplicate_marks_and_obstacles_stay_bounded() {
    // 4,000 identical stacked runs: every one is DUPLICATE_TEXT, with one flag per run.
    let mut c = String::new();
    for _ in 0..4_000 {
        c.push_str("BT /F1 12 Tf 72 700 Td (a) Tj ET\n");
    }
    let started = thread_cpu();
    let (m, peak) = thread_peak(|| model0(helvetica_page(c.as_bytes())));
    assert_eq!(m.runs.len(), 4_000);
    assert!(m.runs.iter().all(|r| r.reason == Some(R::DuplicateText)));
    assert!(peak < 64 * MIB, "4,000 duplicates peaked at {peak} B");
    // A chain: A overlaps B and B overlaps C by 70 %, A and C by 40 %; D stands apart. B is
    // marked by A, then still finds C among the runs not marked yet.
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (a) Tj ET BT /F1 12 Tf 74 700 Td (a) Tj ET \
          BT /F1 12 Tf 76 700 Td (a) Tj ET BT /F1 12 Tf 200 700 Td (a) Tj ET",
    ));
    let reasons: Vec<(f64, Option<R>)> = m.runs.iter().map(|r| (r.origin.0, r.reason)).collect();
    assert_eq!(
        reasons,
        [
            (72.0, Some(R::DuplicateText)),
            (74.0, Some(R::DuplicateText)),
            (76.0, Some(R::DuplicateText)),
            (200.0, None)
        ]
    );
    // 3,000 runs on one baseline, 2 pt apart: the obstacle search is capped per page, and a run
    // the cap cuts short gets the conservative obstacle at its origin.
    let mut c = String::new();
    for i in 0..3_000 {
        let ch = char::from(b'a' + (i % 26) as u8);
        c.push_str(&format!("BT /F1 1 Tf {} 700 Td ({ch}) Tj ET\n", 10 + 2 * i));
    }
    let m = model0(helvetica_page(c.as_bytes()));
    assert_eq!(m.runs.len(), 3_000);
    let first = m
        .runs
        .iter()
        .find(|r| (r.origin.0 - 10.0).abs() < 1e-9)
        .expect("first run");
    assert!(
        first.next_obstacle.is_some_and(|d| (d - 2.0).abs() < 1e-6),
        "the first run measures its neighbour: {:?}",
        first.next_obstacle
    );
    assert!(m.runs.iter().filter(|r| r.next_obstacle.is_none()).count() <= 1);
    assert!(
        m.runs.iter().any(|r| r.next_obstacle == Some(0.0)),
        "9 M pairs reach the cap: the runs left get the conservative obstacle"
    );
    assert!(
        thread_cpu().saturating_sub(started) < Duration::from_secs(20),
        "run checks"
    );
}
