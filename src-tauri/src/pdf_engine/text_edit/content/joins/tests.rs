//! The shared boundary rule (review-final LOW-1/2/4, MEDIUM-2) over every boundary shape the
//! reviews probed (`v04/rfinal/zz_final_probe.rs` `zz_join_shapes` and `zz_h1_stamp_regressions`,
//! review-T5 R1/R7): the scan, the save gate (`gate::join::join_is_neutral` on qpdf's join) and
//! the walker give one answer for each.

use super::first_unsafe_boundary;
use crate::pdf_engine::text_edit::content::qpdf_join;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::gate::join::join_is_neutral;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use std::path::Path;

const HELLO: &[u8] = b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET";
/// An unfiltered one-pixel inline image (its end proven by its size) ending at `EI`.
const IMAGE: &[u8] = b"q 10 0 0 10 300 300 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \x80 EI";

fn page_model_detail(parts: &[&[u8]]) -> (Option<TextReason>, String) {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(parts, &format!("/Font << /F1 {f} 0 R >>")));
    let pdf = d.build();
    let snap = snapshot_from_bytes(Path::new("joins.pdf"), pdf, None).expect("snapshot");
    let model = build_page_model(&SnapshotContext::new(snap), 0, None).expect("model");
    (model.page_reason, model.page_detail.unwrap_or_default())
}

/// (shape, parts, neutral). The pdf.js readings quoted are pdfjs-dist 4.10's (`v04/lastfix-pdfjs`).
fn shapes() -> Vec<(&'static str, Vec<Vec<u8>>, bool)> {
    let p = |parts: &[&[u8]]| parts.iter().map(|x| x.to_vec()).collect::<Vec<_>>();
    let mut dense = b"q ".to_vec();
    dense.extend_from_slice(&b"0 0 m 1 1 l S ".repeat(90_000));
    dense.extend_from_slice(b"Q");
    vec![
        (
            "ET|BT",
            p(&[b"BT /F1 12 Tf 72 720 Td (Visible) Tj ET", HELLO]),
            true,
        ),
        (
            "Q|number",
            p(&[
                b"q BT /F1 12 Tf 72 720 Td (Visible) Tj ET Q",
                b"1 0 0 1 0 0 cm",
            ]),
            true,
        ),
        ("tab|form feed", p(&[b"BT ET\t", b"\x0cBT ET"]), true),
        (
            "string|operator",
            p(&[b"BT /F1 12 Tf 72 700 Td (Hello)", b"Tj ET"]),
            true,
        ),
        (
            "array across",
            p(&[b"BT /F1 12 Tf 72 700 Td [(Spl) -10", b"(it)] TJ ET"]),
            true,
        ),
        // LOW-1: every reader ends the comment at the next part's first byte.
        ("comment|LF", p(&[b"BT ET\n% note", b"\nBT ET"]), true),
        ("comment|CR", p(&[b"BT ET\r% note", b"\rBT ET"]), true),
        (
            "comment|empty|LF",
            p(&[b"BT ET % note", b"", b"\nBT ET"]),
            true,
        ),
        ("comment closed", p(&[b"q\n% note\n", b"Q"]), true),
        ("comment|regular", p(&[b"BT ET\n% note", HELLO]), false),
        (
            "comment NUL|regular",
            p(&[b"BT ET\n% note\x00", HELLO]),
            false,
        ),
        // LOW-4: pdf.js stops a number at a letter; Poppler at the part end.
        ("number|cm", p(&[b"q 1 0 0 1 0 0", b"cm BT ET Q"]), true),
        ("number|digit", p(&[b"1 0 0 1 0 1", b"0 cm"]), false),
        (
            "number|minus",
            p(&[b"q 1 0 0 1 0 0", b"-5 0 0 1 0 0 cm Q"]),
            false,
        ),
        (
            "number|dot",
            p(&[b"q 1 0 0 1 0 0", b".5 0 0 1 0 0 cm Q"]),
            false,
        ),
        // pdf.js finds an unfiltered image's end only at `EI` + space/CR/LF: it reads `EI`|`Q`
        // as image data and loses the text after it (Poppler shows "Hello").
        (
            "EI|Q",
            p(&[
                b"q 10 0 0 10 300 300 cm BI /W 1 /H 1 /CS /G /BPC 8 ID \x80 EI",
                b"Q BT ET",
            ]),
            false,
        ),
        // review-verify MEDIUM-A: pdf.js lexes the 15 bytes after an `EI` to accept it, across
        // the boundary and shifted by qpdf's `\n`; an image ending within the look-ahead window
        // of its part's end refuses the boundary, whatever proved its end (it was neutral when
        // `EI` was followed by a space, CR or LF).
        ("EI|LF", p(&[IMAGE, b"\nQ"]), false),
        (
            "EI|space: pdf.js swallows `rg` and \"Shown\" after a stamp (ei_end_hide)",
            p(&[
                IMAGE,
                b" 0.11 0.2 0.3 rg\nQ BT /F1 12 Tf 72 700 Td (Shown) Tj ET\n% EI Q\n\
                  BT /F1 12 Tf 72 660 Td (Hello) Tj ET",
            ]),
            false,
        ),
        (
            "EI|space: pdf.js shows \"Hidden\" after a stamp (ei_end_reveal)",
            p(&[
                IMAGE,
                b" Q\n%xxxxxxxxxxx\xe9\nBT /F1 12 Tf 72 700 Td (Hidden) Tj ET\n% EI Q\n\
                  BT /F1 12 Tf 72 660 Td (Hello) Tj ET",
            ]),
            false,
        ),
        (
            "EI 2 bytes before the end (ei_inside_window_crosses)",
            p(&[
                &[IMAGE, b"\nQ"].concat(),
                b"%xxxxxxxxxxxx\xe9\nBT /F1 12 Tf 72 700 Td (Hidden) Tj ET\n% EI Q\n\
                  BT /F1 12 Tf 72 660 Td (Hello) Tj ET",
            ]),
            false,
        ),
        (
            "EI then more than the window before the end",
            p(&[&[IMAGE, &b" Q q Q".repeat(12)[..]].concat(), HELLO]),
            true,
        ),
        (
            "inline image split",
            p(&[b"q BI /W 2 /H 1 /BPC 8 /CS /G ID \x01", b"\x02 EI Q"]),
            false,
        ),
        (
            "BI|dict",
            p(&[b"q BI", b" /W 1 /H 1 /CS /G /BPC 8 ID \x80 EI Q"]),
            false,
        ),
        // LOW-2: pdf.js reads `B`|`M…` as its partial `BM`; any boundary inside BX … EX is refused.
        (
            "BX: B|M",
            p(&[b"BX 0 0 m 600 0 l h 1 g B", b"Mfoo EX BT ET"]),
            false,
        ),
        ("BX: whitespace", p(&[b"BX\n", b"foo EX\n"]), false),
        ("BX closed", p(&[b"BX foo EX\n", b"BT ET"]), true),
        ("B|M (partial)", p(&[b"0 0 m 10 10 l B", b"M2 Q"]), false),
        ("n|ull (partial)", p(&[b"0 0 m n", b"ull"]), false),
        ("s|h", p(&[b"0 0 m 10 10 l s", b"h"]), false),
        ("d|0", p(&[b"[] 0 d", b"0 0 m"]), false),
        (
            "mid-string",
            p(&[b"BT /F1 12 Tf 72 700 Td (Hel", b"lo) Tj ET"]),
            false,
        ),
        (
            "escape|paren",
            p(&[b"BT /F1 12 Tf 72 700 Td (Hel\\", b")lo) Tj ET"]),
            false,
        ),
        (
            "mid-hex",
            p(&[b"BT /F1 12 Tf 72 700 Td <48", b"65> Tj ET"]),
            false,
        ),
        (
            "<|<",
            p(&[b"/Span <", b"< /ActualText (Secret) >> BDC EMC"]),
            false,
        ),
        (
            ">|>",
            p(&[b"/Span << /ActualText (Secret) >", b"> BDC EMC"]),
            false,
        ),
        ("tr|ue", p(&[b"/Span << /A tr", b"ue >> BDC EMC"]), false),
        ("/F|1", p(&[b"BT /F", b"1 12 Tf ET"]), false),
        ("/GS0|gs", p(&[b"/GS0", b"gs"]), false),
        // MEDIUM-2: what the strict lexer refuses is judged at its boundaries only.
        ("270k ops|text", vec![dense, HELLO.to_vec()], true),
        ("text|trailing operand", p(&[HELLO, b"\nq Q 0\n"]), true),
        ("PS operator\\n|text", p(&[b"q (x) PS Q\n", HELLO]), true),
        ("unknown operator\\n|text", p(&[b"q Q foo\n", HELLO]), true),
    ]
}

#[test]
fn one_boundary_rule_for_the_scan_the_gate_and_the_walker() {
    for (shape, parts, neutral) in shapes() {
        let parts: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
        assert_eq!(
            first_unsafe_boundary(&parts).is_none(),
            neutral,
            "scan: {shape}"
        );
        let joined = qpdf_join(&parts);
        let inserts = joined.len() != parts.iter().map(|x| x.len()).sum::<usize>();
        assert_eq!(
            join_is_neutral(&parts, &joined),
            neutral || !inserts,
            "gate: {shape}"
        );
        let (reason, detail) = page_model_detail(&parts);
        let by_joins = detail.starts_with("content parts joined inside");
        if neutral {
            assert!(
                !by_joins,
                "walker refused a neutral boundary: {shape}: {detail}"
            );
        } else {
            // Not neutral: the walker refuses the page, by this rule or by its lexer first.
            assert_eq!(
                reason,
                Some(TextReason::MalformedContent),
                "walker: {shape}: {detail}"
            );
        }
    }
}

#[test]
fn the_scan_reports_the_first_unsafe_part_and_skips_empty_ones() {
    let parts: [&[u8]; 5] = [b"q", b"", b"Q ", b"(a", b"b) Tj"];
    assert_eq!(first_unsafe_boundary(&parts), Some(3));
    assert_eq!(first_unsafe_boundary(&[]), None);
    assert_eq!(first_unsafe_boundary(&[b"(unterminated" as &[u8]]), None);
    // A heuristic inline-image end whose look-ahead reaches the part's end is not followed.
    let dct = b"q BI /W 1 /H 1 /CS /G /BPC 8 /F /DCT ID \xff\xd8 EI Q" as &[u8];
    assert_eq!(first_unsafe_boundary(&[dct, HELLO]), Some(0));
    let tail = [dct, &b" q Q".repeat(20)[..]].concat();
    assert_eq!(first_unsafe_boundary(&[&tail, HELLO]), None);
}
