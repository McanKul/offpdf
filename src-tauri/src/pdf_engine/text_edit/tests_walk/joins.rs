//! Engine fix pass (2026-10-03): content-part boundaries viewers read differently (review T5 H1,
//! probe R7) are refused `MALFORMED_CONTENT` in every walk, so a line commented out in Poppler
//! and pdf.js is never offered as editable; and ASCII inline images lex in linear time (review
//! T3-budget MEDIUM-4).

use super::fuzz::thread_cpu;
use super::{ctx, model, texts, walk};
use crate::pdf_engine::source_content::classify_source_page;
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::walker::WalkMode;
use std::time::Duration;

/// A one-page document whose `/Contents` is `parts`, with Helvetica as `/F1`.
fn parts_page(parts: &[&[u8]]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(parts, &format!("/Font << /F1 {f} 0 R >>")));
    d.build()
}

const VISIBLE: &[u8] = b"BT /F1 12 Tf 72 700 Td (Visible) Tj ET";
const SECRET: &[u8] = b"BT /F1 12 Tf 72 680 Td (Secret) Tj ET";

/// Every walk of page 0 (Edit, Classify) and the model refuse the page `MALFORMED_CONTENT` (by
/// the boundary check when `joins`, else possibly by the lexer first: a name split in two often
/// leaves an op with one operand too many); the Classify pass lists no occurrence.
fn assert_refused(tag: &str, parts: &[&[u8]], joins: bool) {
    let c = ctx(parts_page(parts));
    let m = model(&c, 0);
    assert_eq!(
        m.page_reason,
        Some(R::MalformedContent),
        "{tag}: {:?}",
        texts(&m)
    );
    let by_joins = m
        .page_detail
        .as_deref()
        .is_some_and(|d| d.starts_with("content parts joined inside"));
    assert!(by_joins || !joins, "{tag}: {:?}", m.page_detail);
    assert!(m.runs.is_empty(), "{tag}");
    for mode in [WalkMode::Edit, WalkMode::Classify] {
        let w = walk(&c, 0, mode.clone());
        assert_eq!(w.page_reason, Some(R::MalformedContent), "{tag} {mode:?}");
        assert!(w.records.is_empty(), "{tag} {mode:?}");
    }
    let cl = classify_source_page(&c, &m, None);
    assert!(
        cl.occurrences.is_empty(),
        "{tag}: {} occurrences",
        cl.occurrences.len()
    );
}

/// The page models with `want` as its runs.
fn assert_modelled(tag: &str, parts: &[&[u8]], want: &[&str]) {
    let m = model(&ctx(parts_page(parts)), 0);
    assert_eq!(m.page_reason, None, "{tag}: {:?}", m.page_detail);
    assert_eq!(texts(&m), want, "{tag}");
}

#[test]
fn a_part_ending_in_a_comment_is_refused_unless_the_next_starts_a_line() {
    // Probe R7: Poppler and pdf.js run the comment on into the next part ("Secret" is never
    // drawn); the joined buffer ended it at the separator and offered "Secret" as editable.
    let commented = [VISIBLE, b"\n% note" as &[u8]].concat();
    assert_refused("comment runs on", &[&commented, SECRET], true);
    let percent_in_a_string = b"BT /F1 12 Tf 72 700 Td (50%) Tj ET" as &[u8];
    assert_modelled(
        "a % inside a string is no comment",
        &[percent_in_a_string, SECRET],
        &["50%", "Secret"],
    );
    let closed = [VISIBLE, b"\n% note\n" as &[u8]].concat();
    assert_modelled(
        "comment closed in its part",
        &[&closed, SECRET],
        &["Visible", "Secret"],
    );
    let next_line = [b"\n" as &[u8], SECRET].concat();
    assert_modelled(
        "the next part starts a line",
        &[&commented, &next_line],
        &["Visible", "Secret"],
    );
    // An empty part between them separates nothing.
    assert_refused(
        "comment over an empty part",
        &[&commented, b"", SECRET],
        true,
    );
}

#[test]
fn a_part_split_inside_a_token_is_refused() {
    // Each joins into valid content with a newline, and into other content without one (the
    // joined buffer used to be modelled as if every viewer read the newline).
    let cases: [(&str, &[u8], &[u8]); 6] = [
        ("mid-string", b"BT /F1 12 Tf 72 700 Td (Hel", b"lo) Tj ET"),
        (
            "mid-string at a line end",
            b"BT /F1 12 Tf 72 700 Td (Hel\n",
            b"lo) Tj ET",
        ),
        (
            "mid-hex-string",
            b"BT /F1 12 Tf 72 700 Td <4865",
            b"6C6C6F> Tj ET",
        ),
        (
            "mid-number",
            b"BT /F1 12 Tf 72 700 Td [(He) 1",
            b"2 (llo)] TJ ET",
        ),
        (
            "mid-inline-image",
            b"q BI /W 1 /H 1 /CS /G /BPC 8 ID \x80",
            b" EI Q BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        ),
        (
            "an operator into another (`s` + `h` is pdf.js's `sh`)",
            b"0 0 m 10 10 l s",
            b"h BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        ),
    ];
    for (tag, a, b) in cases {
        assert_refused(tag, &[a, b], true);
    }
    // Names split in two: refused, by the boundary check or by the lexer first.
    for (tag, a, b) in [
        (
            "mid-name",
            b"BT /F" as &[u8],
            b"1 12 Tf 72 700 Td (Hello) Tj ET" as &[u8],
        ),
        (
            "a name and a number",
            b"BT /F1",
            b"12 Tf 72 700 Td (Hello) Tj ET",
        ),
    ] {
        assert_refused(tag, &[a, b], false);
    }
}

#[test]
fn parts_split_between_tokens_still_model() {
    // Operators every viewer ends at a part's end (E2E-10c's `ET`|`BT`, `Q`|`q`), separators on
    // either side, an op whose operands and operator lie in different parts, an array across.
    assert_modelled("ET|BT", &[VISIBLE, SECRET], &["Visible", "Secret"]);
    assert_modelled(
        "Q|q",
        &[
            b"q 1 0 0 1 0 0 cm Q",
            b"q BT /F1 12 Tf 72 700 Td (A) Tj ET Q",
        ],
        &["A"],
    );
    assert_modelled(
        "operands then operator",
        &[b"BT /F1 12 Tf 72 700", b" Td (Split) Tj ET"],
        &["Split"],
    );
    assert_modelled(
        "an array across",
        &[b"BT /F1 12 Tf 72 700 Td [(Spl) -10", b"(it)] TJ ET"],
        &["Split"],
    );
    assert_modelled(
        "a string then an operator",
        &[b"BT /F1 12 Tf 72 700 Td (Hello)", b"Tj ET"],
        &["Hello"],
    );
}

#[test]
fn ascii_inline_images_without_a_terminator_lex_in_linear_time() {
    // Review MEDIUM-4: each image searched the rest of the content for `>` (8,000 images: 19 s);
    // one search per terminator now serves every image that follows.
    for (filter, n) in [("AHx", 8_000usize), ("A85", 8_000)] {
        let content = format!("BI /F /{filter} ID 0 EI n\n").repeat(n);
        let started = thread_cpu();
        let ops = lex_content(content.as_bytes(), &LexLimits::page(), None);
        let spent = thread_cpu().saturating_sub(started);
        assert_eq!(ops.map(|o| o.len()).ok(), Some(2 * n), "{filter}");
        assert!(spent < Duration::from_secs(1), "{filter}: {spent:?}");
    }
    // A terminator beyond the image cap proves nothing: the near `EI` ends the image (the far
    // `> EI` in a comment used to make one image of everything up to it).
    let mut c = b"BI /F /AHx ID 0 EI n ".to_vec();
    c.extend(vec![b' '; (16 << 20) + 8]);
    c.extend_from_slice(b"% > EI\n");
    let ops = lex_content(&c, &LexLimits::page(), None).expect("lexes");
    assert_eq!(ops.len(), 2, "the image and `n`");
    let image = ops[0].inline_image.as_ref().expect("image");
    assert_eq!(&c[image.data.clone()], b"0", "the near EI ends it");
}
