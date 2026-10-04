//! The cross-reference chain the preflight checks is the one lopdf 0.34 reads (review-T1 round
//! 2, HIGH-3): SNAP-08d duplicate keys count with their last value, SNAP-08e a table's trailer
//! is the one lopdf's grammar reaches, SNAP-08f `/XRefStm` of an xref-stream trailer is followed,
//! SNAP-08g xref-stream arrays must be integers. Each refused case is one that round 1 let
//! through to lopdf (its outcome there is noted per case).

use super::preflight::{deep, opens, outcome};
use std::fmt::Write as _;

const OBJECTS: [&str; 3] = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
];

/// lopdf allocates this predictor row (4,000,000 bytes) only when the stream is Flate-encoded;
/// these xref streams are not, so lopdf reads them fine while the preflight refuses the row.
/// That makes "which section did the preflight check" visible without crashing lopdf.
const PREDICTOR_BOMB: &str = "/DecodeParms << /Predictor 12 /Columns 2000 /Colors 2000 >>";

/// Objects 1–3 written by hand, then xref sections whose offsets the test wires together.
struct File {
    out: Vec<u8>,
    offs: Vec<usize>,
}

impl File {
    fn new() -> File {
        let mut f = File {
            out: b"%PDF-1.7\n".to_vec(),
            offs: Vec::new(),
        };
        for (i, body) in OBJECTS.iter().enumerate() {
            f.offs.push(f.out.len());
            f.out
                .extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        f
    }

    /// An xref stream object `id` listing objects 0–3 (`/W [1 4 2]`, unfiltered). `pre` goes
    /// before and `post` after the standard entries, so a key in both occurs twice.
    fn xref_stream(&mut self, id: u32, pre: &str, post: &str) -> usize {
        let mut rows = vec![0u8, 0, 0, 0, 0, 0xFF, 0xFF];
        for off in &self.offs {
            rows.push(1);
            rows.extend_from_slice(&u32::try_from(*off).unwrap().to_be_bytes());
            rows.extend_from_slice(&[0, 0]);
        }
        let at = self.out.len();
        let head = format!(
            "{id} 0 obj\n<< {pre} /Type /XRef /Size 4 /W [1 4 2] {post} /Length {} >>\nstream\n",
            rows.len()
        );
        self.out.extend_from_slice(head.as_bytes());
        self.out.extend_from_slice(&rows);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
        at
    }

    /// A classic table for objects 0–3 followed by `after` (the caller writes the trailer).
    fn table(&mut self, after: &str) -> usize {
        let at = self.out.len();
        let mut text = "xref\n0 4\n0000000000 65535 f \n".to_string();
        for off in &self.offs {
            let _ = write!(text, "{off:010} 00000 n \n");
        }
        text.push_str(after);
        self.out.extend_from_slice(text.as_bytes());
        at
    }

    fn finish(mut self, start: usize) -> Vec<u8> {
        self.out
            .extend_from_slice(format!("startxref\n{start}\n%%EOF\n").as_bytes());
        self.out
    }
}

fn assert_refused(bytes: &[u8], code: &str, detail: &str, id: &str) {
    let (got, details) = outcome(bytes);
    assert_eq!(got, code, "{id}: {details}");
    assert!(details.contains(detail), "{id}: {details}");
}

/// A main xref stream with `pre`/`post` entries around the standard ones.
fn main_stream(pre: &str, post: &str) -> Vec<u8> {
    let mut f = File::new();
    let x = f.xref_stream(9, pre, &format!("/Root 1 0 R {post}"));
    f.finish(x)
}

#[test]
fn snap08d_duplicate_keys_count_with_their_last_value() {
    // lopdf's `Dictionary::set` keeps the last value; round 1 checked the first one.
    // (entry written after the standard one, code, detail); written before it, the file opens.
    // Round 1 passed both to lopdf: `/Size` was then looped over until the rows ran out, and
    // `/W [1 4 9]` misread every row.
    let cases = [
        ("/Size 3000000", "FILE_TOO_COMPLEX", "too many objects"),
        ("/W [1 4 9]", "INVALID_PDF", "xref stream /W"),
    ];
    for (post, code, detail) in cases {
        assert_refused(
            &main_stream("", post),
            code,
            detail,
            &format!("SNAP-08d {post}"),
        );
        let first = main_stream(post, "");
        assert_eq!(
            opens(&first, "SNAP-08d").pages.len(),
            1,
            "SNAP-08d first {post}"
        );
    }
    // Nested: lopdf reads `/Columns` from the `/DecodeParms` dictionary the same way.
    let parms = |a: &str, b: &str| {
        main_stream(
            "",
            &format!("/DecodeParms << /Predictor 12 /Columns {a} /Columns {b} >>"),
        )
    };
    assert_refused(
        &parms("1", "4611686018427387904"),
        "FILE_TOO_COMPLEX",
        "predictor row",
        "SNAP-08d nested",
    );
    assert_eq!(outcome(&parms("4611686018427387904", "1")).0, "OK");
    // `/Prev` twice in a table trailer: lopdf follows the last one (round 1: opened).
    let prev = |first_bad: bool| {
        let mut f = File::new();
        let good = f.xref_stream(10, "", "");
        let bad = f.xref_stream(11, "", PREDICTOR_BOMB);
        let (a, b) = if first_bad { (bad, good) } else { (good, bad) };
        let t = f.table(&format!(
            "trailer\n<< /Size 4 /Root 1 0 R /Prev {a} /Prev {b} >>\n"
        ));
        f.finish(t)
    };
    assert_refused(
        &prev(false),
        "FILE_TOO_COMPLEX",
        "predictor row",
        "SNAP-08d /Prev",
    );
    assert_eq!(outcome(&prev(true)).0, "OK", "SNAP-08d /Prev last good");
}

#[test]
fn snap08e_table_trailer_is_the_one_lopdf_parses() {
    let fake = "%trailer << /Size 4 /Root 1 0 R >>\n";
    let with = |between: &str, trailer: &str| {
        let mut f = File::new();
        let bad = f.xref_stream(10, "", PREDICTOR_BOMB);
        let trailer = trailer.replace("{bad}", &bad.to_string());
        let t = f.table(&format!(
            "{between}trailer\n<< /Size 4 /Root 1 0 R {trailer} >>\n"
        ));
        f.finish(t)
    };
    // A `trailer` inside a comment after the entries hides the real one, whose `/Prev` lopdf
    // follows (round 1: opened).
    assert_refused(
        &with(fake, "/Prev {bad}"),
        "FILE_TOO_COMPLEX",
        "predictor row",
        "SNAP-08e hidden /Prev",
    );
    // The real trailer's nesting is checked, not the comment's (round 1: opened).
    assert_refused(
        &with(fake, &format!("/Deep {}", deep(40))),
        "FILE_TOO_COMPLEX",
        "nesting",
        "SNAP-08e hidden nesting",
    );
    // Comments there are legal (lopdf's `space`): the file opens.
    let fine = with(&format!("% producer note\n{fake}"), "");
    assert_eq!(opens(&fine, "SNAP-08e comments").pages.len(), 1);
    // A `trailer` only inside a comment is no trailer for lopdf.
    let mut f = File::new();
    let t = f.table(fake);
    assert_refused(
        &f.finish(t),
        "INVALID_PDF",
        "no trailer",
        "SNAP-08e comment only",
    );
}

#[test]
fn snap08f_xrefstm_of_an_xref_stream_trailer_is_followed() {
    // lopdf takes `/XRefStm` from its newest trailer even when that is an xref stream (it reads
    // it while following `/Prev`); round 1 followed it only from tables (opened).
    let with = |stm_bad: bool| {
        let mut f = File::new();
        let prev = f.xref_stream(10, "", "");
        let stm = f.xref_stream(11, "", if stm_bad { PREDICTOR_BOMB } else { "" });
        let x = f.xref_stream(12, "", &format!("/Root 1 0 R /Prev {prev} /XRefStm {stm}"));
        f.finish(x)
    };
    assert_refused(&with(true), "FILE_TOO_COMPLEX", "predictor row", "SNAP-08f");
    assert_eq!(opens(&with(false), "SNAP-08f good").pages.len(), 1);
}

#[test]
fn snap08g_xref_stream_arrays_are_integers() {
    // lopdf replaces an `/Index` holding a real by `[0 /Size]`, which round 1 never bounded
    // (it summed the `/Index` counts): here lopdf looped towards `/Size` 3,000,000 until the rows
    // ran out; with a long enough xref stream that is one entry per row byte.
    assert_refused(
        &main_stream("", "/Size 3000000 /Index [0 4.0]"),
        "INVALID_PDF",
        "xref stream /Index",
        "SNAP-08g /Index",
    );
    assert_refused(
        &main_stream("", "/W [1 4.0 2]"),
        "INVALID_PDF",
        "xref stream /W",
        "SNAP-08g /W",
    );
    assert_eq!(
        outcome(&main_stream("", "/Index [0 4]")).0,
        "OK",
        "SNAP-08g ints"
    );
}
