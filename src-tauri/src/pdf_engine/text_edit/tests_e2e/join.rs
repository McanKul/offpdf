//! review-T5 H1: qpdf's overlay join must never change what a page shows. A page whose content
//! parts meet inside a comment or a string reads differently once qpdf joins them with a `\n`
//! (Poppler reads the parts as one stream: the comment ran on and hid the next part). Every Save
//! that wraps such a page — a stamp alone, or a stamp with text changes — must be refused, while
//! split pages whose parts meet between tokens (E2E-10c) still save.

use super::E2e;
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use serde_json::{json, Value};

const COMMENT: [&[u8]; 2] = [
    b"BT /F1 12 Tf 72 720 Td (Visible) Tj ET\n% note",
    b"BT /F1 12 Tf 72 700 Td (Secret) Tj ET",
];
const MID_STRING: [&[u8]; 2] = [
    b"BT /F1 12 Tf 72 720 Td (Visible) Tj ET BT /F1 12 Tf 72 700 Td (Hel",
    b"lo) Tj ET",
];

/// The reviewer's two pages: (name, parts, Poppler's words of the source page).
const UNSAFE: [(&str, [&[u8]; 2], &str); 2] = [
    ("comment", COMMENT, "Visible"),
    ("mid-string", MID_STRING, "VisibleHello"),
];

fn stamp(page: u32) -> Value {
    json!({
        "kind": "text", "pageIndex": page,
        "rect": { "x": 300.0, "y": 120.0, "w": 200.0, "h": 30.0 },
        "content": "Stamp", "fontSize": 14.0, "color": "#c71c1c", "align": null, "opacity": 1.0,
    })
}

/// A file whose pages are `pages` (each a list of content parts), all with Helvetica as /F1.
fn doc(pages: &[&[&[u8]]]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    for parts in pages {
        d.page(PageSpec::parts(parts, &res));
    }
    d.build()
}

#[test]
fn h1_stamp_only_saves_of_unsafe_split_pages_are_refused() {
    let Some(t) = E2e::new("h1_stamp_only") else {
        return;
    };
    for (name, parts, words) in UNSAFE {
        let src = t.file(&format!("{name}.pdf"), &doc(&[&parts]));
        assert_eq!(t.words(&src, 0), words, "{name}: what the source shows");
        match t.save(&[(&src, "1-z")], vec![stamp(0)]) {
            Err(e) => assert_eq!(e.code, "INVALID_OUTPUT", "{name}: {e} {:?}", e.details),
            Ok(saved) => panic!(
                "{name}: a stamp-only save was published; Poppler now reads {:?}",
                t.words(&saved.path, 0)
            ),
        }
    }
    // The legitimate split (E2E-10c's shape, parts meeting between operators) still saves.
    let ok = doc(&[&[
        b"BT /F1 12 Tf 72 720 Td (Split zero) Tj ET",
        b"BT /F1 12 Tf 72 700 Td (Split one) Tj ET",
    ]]);
    let src = t.file("ok.pdf", &ok);
    let saved = t
        .save(&[(&src, "1-z")], vec![stamp(0)])
        .unwrap_or_else(|e| panic!("token-boundary split: {e} {:?}", e.details));
    assert!(t.words(&saved.path, 0).contains("SplitzeroSplitone"));
}

#[test]
fn h1_text_edit_saves_with_unsafe_split_pages_are_refused() {
    let Some(t) = E2e::new("h1_text_edit") else {
        return;
    };
    let line: &[&[u8]] = &[b"BT /F1 12 Tf 72 720 Td (Edited line) Tj ET"];
    for (name, parts, _) in UNSAFE {
        // A text change on page 1 and a stamp; page 2 is the unsafe split page, unedited.
        let src = t.file(&format!("{name}-2.pdf"), &doc(&[line, &parts]));
        let objects = vec![t.edit(&src, 0, 0, "Edited line", "Changed line"), stamp(0)];
        match t.save(&[(&src, "1-z")], objects) {
            Err(e) => assert_eq!(e.code, "INVALID_OUTPUT", "{name}: {e} {:?}", e.details),
            Ok(saved) => panic!(
                "{name}: published; Poppler reads page 2 as {:?}",
                t.words(&saved.path, 1)
            ),
        }
        // A text change on the split page itself and a stamp: refused at inspect (the line is
        // not offered), by Phase B or by #34 — never published.
        let src = t.file(&format!("{name}-1.pdf"), &doc(&[&parts]));
        let Some(edit) = t.try_edit(&src, 0, 0, "Visible", "Visibly") else {
            continue;
        };
        if let Ok(saved) = t.save(&[(&src, "1-z")], vec![edit, stamp(0)]) {
            panic!(
                "{name}: published; Poppler reads {:?}",
                t.words(&saved.path, 0)
            );
        }
    }
}

/// review-final MEDIUM-2: the join rule needs no lex of the page. A stamp-only save of a
/// two-part page with a neutral boundary publishes even when the strict lexer refuses the page
/// (more than 250,000 ops; a trailing operand), as it did before review-T5 H1.
#[test]
fn medium2_stamp_only_saves_of_pages_the_strict_lexer_refuses_still_publish() {
    let Some(t) = E2e::new("medium2_stamp_only") else {
        return;
    };
    let mut dense = b"q ".to_vec();
    dense.extend_from_slice(&b"0 0 m 1 1 l S ".repeat(90_000));
    dense.extend_from_slice(b"Q");
    let text: &[u8] = b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET";
    let pages: [(&str, [&[u8]; 2]); 2] = [
        ("270k-ops", [&dense, text]),
        ("trailing-operand", [text, b"\nq Q 0\n"]),
    ];
    for (name, parts) in pages {
        let src = t.file(&format!("{name}.pdf"), &doc(&[&parts]));
        let saved = t
            .save(&[(&src, "1-z")], vec![stamp(0)])
            .unwrap_or_else(|e| panic!("{name}: {e} {:?}", e.details));
        assert_eq!(t.words(&saved.path, 0), "HelloStamp", "{name}");
    }
}

/// review-final LOW-1: a part ending in a comment, the next starting a line, is one page for the
/// walker and the gate alike: its lines are editable, and stamp, text and text + stamp saves all
/// publish (before: every overlay save failed `INVALID_OUTPUT` or `SOURCE_EDIT_GATE_FAILED`).
#[test]
fn low1_a_comment_ending_a_part_before_a_new_line_saves_with_stamps() {
    let Some(t) = E2e::new("low1_comment_line") else {
        return;
    };
    let parts: [&[u8]; 2] = [
        b"BT /F1 12 Tf 72 720 Td (Visible) Tj ET\n% note",
        b"\nBT /F1 12 Tf 72 700 Td (Hello) Tj ET",
    ];
    let src = t.file("comment-line.pdf", &doc(&[&parts]));
    let stamped = t
        .save(&[(&src, "1-z")], vec![stamp(0)])
        .unwrap_or_else(|e| panic!("stamp only: {e} {:?}", e.details));
    assert_eq!(t.words(&stamped.path, 0), "VisibleHelloStamp");
    let edit = t.edit(&src, 0, 0, "Hello", "Help");
    let both = t
        .save(&[(&src, "1-z")], vec![edit, stamp(0)])
        .unwrap_or_else(|e| panic!("text + stamp: {e} {:?}", e.details));
    assert_eq!(t.words(&both.path, 0), "VisibleHelpStamp");
}
