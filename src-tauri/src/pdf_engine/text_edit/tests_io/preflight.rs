//! Preflight hardening (review-T1 fix passes): SNAP-08b/08c xref-stream fields, SNAP-10b/10c
//! nesting wherever lopdf starts parsing, SNAP-10d `/Length` chains, SNAP-10e scan budget,
//! SNAP-10f/10g linear header probes and budgeted `stream` probes, SNAP-10h one object-stream
//! scan budget per load, GUARD-02b preflight fuzz, GUARD-02c header finder vs a naive probe.
//! Fixtures with hand-placed xref offsets are written by `Raw`; the xref-chain cases are in
//! `tests_io/xref.rs`.

use crate::error::AppError;
use crate::pdf_engine::text_edit::engines::{run_tool, RunOpts};
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::snapshot::headers::Headers;
use crate::pdf_engine::text_edit::snapshot::preflight::preflight;
use crate::pdf_engine::text_edit::snapshot::{read_snapshot, snapshot_from_bytes, SourceSnapshot};
use crate::pdf_engine::text_edit::testkit::pdf::{simple_pdf, zlib, Doc, XrefStyle};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::Instant;

const HELLO: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET";

pub(super) fn from_bytes(bytes: &[u8]) -> Result<SourceSnapshot, AppError> {
    snapshot_from_bytes(Path::new("fixture.pdf"), bytes.to_vec(), None)
}

/// (code, details) of opening `bytes`; "OK" when it opens.
pub(super) fn outcome(bytes: &[u8]) -> (String, String) {
    match from_bytes(bytes) {
        Ok(_) => ("OK".to_string(), String::new()),
        Err(e) => (e.code.clone(), e.details.unwrap_or_default()),
    }
}

pub(super) fn opens(bytes: &[u8], id: &str) -> SourceSnapshot {
    from_bytes(bytes).unwrap_or_else(|e| panic!("{id}: {e} ({:?})", e.details))
}

pub(super) fn refused(bytes: &[u8], detail: &str, id: &str) {
    let (code, details) = outcome(bytes);
    assert_eq!(code, "FILE_TOO_COMPLEX", "{id}: {details}");
    assert!(details.contains(detail), "{id}: {details}");
}

pub(super) fn deep(n: usize) -> String {
    format!("{}{}", "[".repeat(n), "]".repeat(n))
}

/// A file written byte by byte with a classic xref table whose offsets may point anywhere.
/// Objects 1–3 are a catalog, a page tree and one page.
pub(super) struct Raw {
    out: Vec<u8>,
    offs: BTreeMap<u32, usize>,
}

impl Raw {
    pub(super) fn new() -> Raw {
        let mut r = Raw {
            out: b"%PDF-1.7\n".to_vec(),
            offs: BTreeMap::new(),
        };
        r.obj(1, b"<< /Type /Catalog /Pages 2 0 R >>");
        r.obj(2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        r.obj(
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        );
        r
    }

    pub(super) fn obj(&mut self, id: u32, body: &[u8]) {
        self.obj_at(&format!("{id} 0 obj\n"), id, 0, body);
    }

    /// Writes `head`, `body`, `endobj`; the xref entry of `id` points `skip` bytes into `head`.
    fn obj_at(&mut self, head: &str, id: u32, skip: usize, body: &[u8]) {
        self.offs.insert(id, self.out.len() + skip);
        self.out.extend_from_slice(head.as_bytes());
        self.out.extend_from_slice(body);
        self.out.extend_from_slice(b"\nendobj\n");
    }

    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
    }

    /// Points the xref entry of `id` at the first occurrence of `needle` written so far.
    fn point(&mut self, id: u32, needle: &[u8]) {
        let at = self
            .out
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("needle written");
        self.offs.insert(id, at);
    }

    pub(super) fn finish(mut self) -> Vec<u8> {
        let max = self.offs.keys().max().copied().unwrap_or(0);
        let x = self.out.len();
        self.raw(format!("xref\n0 {}\n", max + 1).as_bytes());
        for id in 0..=max {
            let row = match self.offs.get(&id) {
                Some(off) if id != 0 => format!("{off:010} 00000 n \n"),
                _ => "0000000000 65535 f \n".to_string(),
            };
            self.raw(row.as_bytes());
        }
        let tail = format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            max + 1
        );
        self.raw(tail.as_bytes());
        self.out
    }
}

#[test]
fn snap08b_xref_index_counts_are_summed_without_overflow() {
    let d = Doc::new(&[&[HELLO]]);
    let stream = XrefStyle::Stream {
        in_objstm: vec![],
        bomb_mib: None,
    };
    // 10,000 counts of 999,999,999,999,999 overflow an i64 sum (wrapped negative before the fix,
    // a debug-build panic in the test binary).
    let index = "0 999999999999999 ".repeat(10_000);
    let bytes =
        d.b.build_with(&format!("{} /Index [{index}]", d.trailer()), &stream);
    refused(&bytes, "too many objects", "SNAP-08b overflowing /Index");
    let one = format!("{} /Index [0 {}]", d.trailer(), limits::MAX_OBJECTS + 1);
    refused(
        &d.b.build_with(&one, &stream),
        "too many objects",
        "SNAP-08b one count",
    );
    let fine = d.b.build_with(&d.trailer(), &stream);
    assert_eq!(opens(&fine, "SNAP-08b plain").pages.len(), 1);
}

#[test]
fn snap08c_xref_predictor_row_is_bounded() {
    let d = Doc::new(&[&[HELLO]]);
    let stream = XrefStyle::Stream {
        in_objstm: vec![],
        bomb_mib: None,
    };
    let with = |parms: &str| {
        let trailer = format!("{} /DecodeParms << {parms} >>", d.trailer());
        d.b.build_with(&trailer, &stream)
    };
    // lopdf allocates max(1, Columns) × max(1, Colors) × max(8, BPC) / 8 bytes per row: 2^62
    // aborted the process ("memory allocation failed") in the probe.
    for parms in [
        "/Predictor 12 /Columns 4611686018427387904",
        "/Predictor 10 /Columns 2000 /Colors 2000",
        "/Predictor 15 /Columns 3 /BitsPerComponent 1000000000",
    ] {
        refused(&with(parms), "predictor row", &format!("SNAP-08c {parms}"));
    }
    // TIFF predictor 2 is ignored by lopdf's xref decoding, so its row is never allocated.
    let tiff = with("/Predictor 2 /Columns 4611686018427387904");
    assert_eq!(outcome(&tiff).0, "OK", "SNAP-08c predictor 2");
    let Some(engines) = engines_or_skip("snap08c_xref_predictor_row_is_bounded") else {
        return;
    };
    let s = Scratch::new("snap08c");
    let src = s.write("in.pdf", &simple_pdf(HELLO));
    let out = s.path("xref-stream.pdf");
    let args = [
        OsString::from("--object-streams=generate"),
        src.into(),
        out.clone().into(),
    ];
    let run = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).unwrap();
    assert_eq!(run.code, 0, "SNAP-08c qpdf");
    let generated = std::fs::read(&out).unwrap();
    assert!(
        generated.windows(13).any(|w| w == b"/Predictor 12"),
        "SNAP-08c qpdf writes a PNG-predicted xref stream"
    );
    assert_eq!(
        read_snapshot(&out).unwrap().pages.len(),
        1,
        "SNAP-08c a real predicted xref stream opens"
    );
}

/// Object 99 (`[`×n `]`×n) where a linear scan never counts it: inside fake stream framing,
/// a literal string, a comment, or a header split by comments, or behind a header whose first
/// number only ends in 99. The xref entry of 99 points at it.
fn hidden_nesting(kind: &str, n: usize) -> Vec<u8> {
    let value = format!("99 0 obj {} endobj", deep(n));
    let mut r = Raw::new();
    match kind {
        "fake stream" => {
            r.obj(4, b"<< /Length 1 >>\nstream\nx\nendstream");
            r.raw(format!("<<>>stream\n{value}\nendstream\n").as_bytes());
        }
        "string" => r.obj(4, format!("({value})").as_bytes()),
        "comment" => r.raw(format!("% {value}\n").as_bytes()),
        "split header" => r.raw(format!("99 %a\n0 %b\nobj {}\nendobj\n", deep(n)).as_bytes()),
        _ => r.raw(format!("1{value}\n").as_bytes()), // "199 0 obj …", xref points at "99"
    }
    r.point(99, b"99 ");
    r.finish()
}

#[test]
fn snap10b_nesting_anywhere_lopdf_can_parse_is_bounded() {
    for kind in ["fake stream", "string", "comment", "split header", "suffix"] {
        let shallow = opens(&hidden_nesting(kind, 3), "SNAP-10b");
        assert!(
            shallow.doc.objects.contains_key(&(99, 0)),
            "SNAP-10b {kind}: lopdf parses object 99 where its xref entry points"
        );
        assert_eq!(
            outcome(&hidden_nesting(kind, 100)).0,
            "OK",
            "SNAP-10b {kind} 100"
        );
        refused(
            &hidden_nesting(kind, 101),
            "nested more than 100 levels",
            &format!("SNAP-10b {kind} 101"),
        );
    }
}

#[test]
fn snap10c_object_stream_members_are_nesting_checked() {
    let member = |n: usize| {
        let mut d = Doc::new(&[&[HELLO]]);
        let id = d.b.add(deep(n));
        let bytes = d.build_with(&XrefStyle::Stream {
            in_objstm: vec![id],
            bomb_mib: None,
        });
        (bytes, id)
    };
    let (ok, id) = member(100);
    assert!(
        opens(&ok, "SNAP-10c 100")
            .doc
            .objects
            .contains_key(&(id, 0)),
        "SNAP-10c member loaded"
    );
    // Compressed: invisible to any raw scan; lopdf parses it after the guard decodes it.
    let (bad, _) = member(101);
    refused(&bad, "too deeply nested", "SNAP-10c 101");
}

/// `n` streams from id 100, each with `/Length` → the next id; the last → an integer object.
/// Each header is written as `{prefix}{id} 0 obj` with the xref entry `prefix.len()` bytes in.
fn length_chain(n: u32, prefix: &str) -> Vec<u8> {
    let mut r = Raw::new();
    for id in 100..100 + n {
        let body = format!("<< /Length {} 0 R >>\nstream\nx\nendstream", id + 1);
        r.obj_at(
            &format!("{prefix}{id} 0 obj\n"),
            id,
            prefix.len(),
            body.as_bytes(),
        );
    }
    r.obj(100 + n, b"1");
    r.finish()
}

#[test]
fn snap10d_length_reference_chains_are_bounded() {
    let max = limits::LENGTH_REF_CHAIN_MAX as u32;
    // n streams → lopdf parses n + 1 objects for the first one.
    let ok = opens(&length_chain(max - 1, ""), "SNAP-10d at the limit");
    assert!(ok.doc.objects.contains_key(&(100, 0)));
    refused(
        &length_chain(max, ""),
        "/Length references",
        "SNAP-10d over",
    );
    // Headers "7100 0 obj" with the xref pointing at "100": lopdf reads id 100 (suffix).
    let suffix = opens(&length_chain(3, "7"), "SNAP-10d suffix");
    assert!(
        suffix.doc.objects.contains_key(&(100, 0)),
        "SNAP-10d lopdf parses the suffix id"
    );
    refused(
        &length_chain(max, "7"),
        "/Length references",
        "SNAP-10d suffix over",
    );
    refused(
        &length_chain(5_000, ""),
        "/Length references",
        "SNAP-10d 5,000",
    );
}

#[test]
fn snap10d_indirect_lengths_of_real_producers_open() {
    // pdfTeX/Ghostscript style: stream k, then its length object k + 1.
    let mut after = Raw::new();
    for k in 0..3_000u32 {
        let id = 10 + 2 * k;
        let body = format!("<< /Length {} 0 R >>\nstream\nq Q\nendstream", id + 1);
        after.obj(id, body.as_bytes());
        after.obj(id + 1, b"3");
    }
    assert_eq!(outcome(&after.finish()).0, "OK", "SNAP-10d length after");
    // Lengths written first at low ids, streams later (suffixes of stream ids hit many lengths).
    let mut first = Raw::new();
    for k in 10..2_010u32 {
        first.obj(k, b"3");
    }
    for k in 10..2_010u32 {
        let body = format!("<< /Length {k} 0 R >>\nstream\nq Q\nendstream");
        first.obj(k + 2_000, body.as_bytes());
    }
    assert_eq!(outcome(&first.finish()).0, "OK", "SNAP-10d lengths first");
}

#[test]
fn snap10e_overlapping_scans_hit_the_budget() {
    // Each "9 0 obj" inside the nested strings starts a scan to the end of the object.
    let n = 2_000;
    let body = format!("{}{}", "[(9 0 obj ".repeat(n), ")]".repeat(n));
    let mut r = Raw::new();
    r.obj(4, body.as_bytes());
    let started = Instant::now();
    refused(&r.finish(), "object scan budget", "SNAP-10e");
    assert!(started.elapsed().as_secs() < 2, "SNAP-10e bounded time");
}

#[test]
fn guard02b_preflight_fuzz() {
    const INSERTS: &[&[u8]] = &[
        b"[",
        b"<<",
        b"(",
        b")",
        b"%",
        b"obj",
        b" 1 0 obj ",
        b"/Length 5 0 R",
        b">>stream\n",
        b"endstream",
        b"#",
        b"\\",
        b"999999999999999",
        b"/Index [0 9]",
        b"/Predictor 12",
        b"\n",
        b"<",
        b">",
        b"trailer",
        b"xref\n",
        b"/Prev 9",
        b"/XRefStm 9",
        b"1.5",
        b"%1 ",
    ];
    let mut chain = Raw::new();
    for id in 10..20u32 {
        let body = format!("<< /L#65ngth {} 0 R >>\nstream\nx\nendstream", id + 1);
        chain.obj(id, body.as_bytes());
    }
    let bases = [
        simple_pdf(HELLO),
        Doc::new(&[&[HELLO]]).build_with(&XrefStyle::Stream {
            in_objstm: vec![3],
            bomb_mib: None,
        }),
        chain.finish(),
        hidden_nesting("string", 50),
    ];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below.max(1) as u64) as usize
    };
    let started = Instant::now();
    for case in 0..2_000 {
        let mut bytes = bases[case % bases.len()].clone();
        for _ in 0..1 + next(4) {
            let at = next(bytes.len() + 1);
            match next(4) {
                0 => {
                    let ins = INSERTS[next(INSERTS.len())];
                    let times = 1 + next(150);
                    let block: Vec<u8> = ins
                        .iter()
                        .copied()
                        .cycle()
                        .take(ins.len() * times)
                        .collect();
                    bytes.splice(at..at, block);
                }
                1 => bytes.truncate(at),
                2 if at < bytes.len() => bytes[at] = next(256) as u8,
                _ => {
                    let ins = INSERTS[next(INSERTS.len())];
                    bytes.splice(at..at, ins.iter().copied());
                }
            }
        }
        let _ = preflight(&bytes); // must return (Ok or a refusal), never panic
    }
    assert!(
        started.elapsed().as_secs() < 4,
        "GUARD-02b 2,000 cases took {:?}",
        started.elapsed()
    );
}

/// Review round 2 (HIGH-1): `k` digit runs inside one comment line, then `tail`. Every run's
/// header probe walks through the rest of the line; the round-1 cache rescanned it per run.
fn comment_digits(k: usize, tail: &[u8]) -> Vec<u8> {
    let mut body = b"%".to_vec();
    for _ in 0..k {
        body.extend_from_slice(b"1 %");
    }
    body.extend_from_slice(tail);
    body.push(b'\n');
    let mut r = Raw::new();
    r.raw(&body);
    r.finish()
}

#[test]
fn snap10f_header_probes_are_linear() {
    let k = 100_000; // ~0.6 MB of digit runs in one comment (round 1: minutes, quadratic)
    let spaces = " ".repeat(3 * k);
    let digits = "5".repeat(3 * k);
    let comment = format!("%{}", "B".repeat(3 * k));
    let shapes: [(&str, Vec<u8>); 4] = [
        // the review's shape: the shared generation number is followed by a second comment
        ("second comment", format!("\n5 {comment}").into_bytes()),
        // a long run of whitespace after the shared generation number
        ("whitespace", format!("\n5{spaces}x").into_bytes()),
        // a long shared generation number
        ("long generation", format!("\n{digits} x").into_bytes()),
        // many comment lines between the runs and the generation number
        (
            "comment lines",
            format!("{}\n5 x", "\n%1 %1".repeat(k / 4)).into_bytes(),
        ),
    ];
    for (name, tail) in shapes {
        let bytes = comment_digits(k, &tail);
        let started = Instant::now();
        assert_eq!(outcome(&bytes).0, "OK", "SNAP-10f {name}");
        assert!(
            started.elapsed().as_secs() < 2,
            "SNAP-10f {name}: {} bytes took {:?}",
            bytes.len(),
            started.elapsed()
        );
    }
    // Headers reached only through the memoised comment walk are still scanned: every `1` in
    // the line forms `1 … 5 obj` with the value after `obj`.
    let hidden = comment_digits(k, format!("\n5 obj {}", deep(101)).as_bytes());
    let started = Instant::now();
    refused(
        &hidden,
        "nested more than 100 levels",
        "SNAP-10f hidden header",
    );
    assert!(
        started.elapsed().as_secs() < 2,
        "SNAP-10f hidden header time"
    );
    let fine = comment_digits(3, format!("\n5 obj {}", deep(100)).as_bytes());
    assert_eq!(outcome(&fine).0, "OK", "SNAP-10f hidden header at 100");
}

#[test]
fn snap10g_stream_probe_after_a_value_is_budgeted() {
    // Review round 2 (HIGH-2): 20 headers in comment lines all close at one `>>`, followed by
    // 1 MiB of whitespace that each value scan's `stream` probe skips. The values themselves
    // read ~3 KB; the probes read 20 MiB, over the budget of 2 × the file + 1 MiB only when they
    // are debited (round 1 opened this file; with K headers it read K × the whitespace).
    let mut body = b"1 0 obj <<\n".to_vec();
    for k in 2..=20 {
        body.extend_from_slice(format!("%{k} 0 obj <<\n").as_bytes());
    }
    body.extend_from_slice(b">>");
    body.resize(body.len() + (1 << 20), b' ');
    body.extend_from_slice(b"\nendobj\n");
    let mut r = Raw::new();
    r.raw(&body);
    let started = Instant::now();
    refused(&r.finish(), "object scan budget", "SNAP-10g");
    assert!(
        started.elapsed().as_secs() < 2,
        "SNAP-10g took {:?}",
        started.elapsed()
    );
}

/// `n` object streams (ids 10…) whose four members all start at offset 0 of one 8 KB array,
/// so each stream's member scans read ~32 KB.
fn overlapping_object_streams(n: u32) -> Vec<u8> {
    let header = "100 0 101 0 102 0 103 0 ";
    let mut data = header.as_bytes().to_vec();
    data.extend_from_slice(format!("[{}]", "0 ".repeat(4_000)).as_bytes());
    let packed = zlib(&data);
    let mut r = Raw::new();
    for id in 10..10 + n {
        let mut body = format!(
            "<< /Type /ObjStm /N 4 /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            header.len(),
            packed.len()
        )
        .into_bytes();
        body.extend_from_slice(&packed);
        body.extend_from_slice(b"\nendstream");
        r.obj(id, &body);
    }
    r.finish()
}

/// Restores the per-thread override when the test ends, also on a failed assertion.
struct ScanOverride;

impl ScanOverride {
    fn set(bytes: usize) -> ScanOverride {
        limits::set_objstm_scan_override(Some(bytes));
        ScanOverride
    }
}

impl Drop for ScanOverride {
    fn drop(&mut self) {
        limits::set_objstm_scan_override(None);
    }
}

#[test]
fn snap10h_object_stream_scans_share_one_budget_per_load() {
    let twenty = overlapping_object_streams(20); // ~640 KB of member scans in total
    let three = overlapping_object_streams(3); // ~96 KB
    let loaded = opens(&twenty, "SNAP-10h default budget");
    assert!(
        loaded.doc.objects.contains_key(&(100, 0)),
        "SNAP-10h members load"
    );
    // Review round 2 (MEDIUM-1): each stream alone is far below its own 2 × data + 1 MiB, so
    // round 1 (a fresh budget per stream) opened this file under any total.
    let _budget = ScanOverride::set(128 << 10);
    refused(&twenty, "too deeply nested", "SNAP-10h shared budget");
    // The budget is reset for every load: a file within it opens twice in a row.
    for round in 1..=2 {
        assert_eq!(outcome(&three).0, "OK", "SNAP-10h reset, load {round}");
    }
}

#[test]
fn guard02c_header_finder_matches_a_naive_probe() {
    // The memoised finder (snapshot/headers.rs) against lopdf's header grammar probed naively
    // from every digit run; tokens chosen so that comments hide runs and headers are common.
    fn space(b: &[u8], mut i: usize) -> usize {
        loop {
            match b.get(i) {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0C' | b'\0') => i += 1,
                Some(b'%') => {
                    while b.get(i).is_some_and(|c| !matches!(c, b'\r' | b'\n')) {
                        i += 1;
                    }
                }
                _ => return i,
            }
        }
    }
    fn digits(b: &[u8], i: usize) -> usize {
        i + b[i..].iter().take_while(|c| c.is_ascii_digit()).count()
    }
    fn naive(b: &[u8]) -> Vec<(std::ops::Range<usize>, usize)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if !b[i].is_ascii_digit() {
                i += 1;
                continue;
            }
            let (start, end) = (i, digits(b, i));
            i = end;
            let gen = space(b, end);
            let gen_end = digits(b, gen);
            let obj = space(b, gen_end);
            if gen_end > gen && b.get(obj..obj + 3) == Some(&b"obj"[..]) {
                out.push((start..end, obj + 3));
            }
        }
        out
    }
    const TOKENS: &[&[u8]] = &[
        b"1", b"12", b" ", b"\n", b"%", b"obj", b"x", b"0", b"\r", b"  ", b"%1 ", b" 0 obj",
    ];
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below as u64) as usize
    };
    let mut headers = 0;
    for case in 0..20_000 {
        let bytes: Vec<u8> = (0..1 + next(120))
            .flat_map(|_| TOKENS[next(TOKENS.len())].iter().copied())
            .collect();
        let expected = naive(&bytes);
        headers += expected.len();
        let found: Vec<_> = Headers::new(&bytes).collect();
        assert_eq!(
            found,
            expected,
            "GUARD-02c case {case}: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
    assert!(headers > 10_000, "GUARD-02c exercised {headers} headers");
}
