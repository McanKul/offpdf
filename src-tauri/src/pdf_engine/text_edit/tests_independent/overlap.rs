//! review-final HIGH-1: IND-04/09/11 under an edit that runs into its neighbours. G-RENDER cannot
//! see a neighbour that lies under the new glyphs' masks, so G-TEXT must find it as the same word
//! at its place (or joined with the new glyphs at its outer edge) and G-RENDER's ink check must
//! find its ink still there. A4 — our own walk — is skipped where the tests model an error our
//! walk would share with the planner (B13).

use super::{follower_doc, table_row};
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::helvetica_page;
use crate::pdf_engine::text_edit::tests_gate::{ed, fails_at, skipping, Honest};

/// The last kern of a splice ending `<n>] TJ`, made `delta` thousandths of an em smaller (the
/// follower moves `delta` × size / 1000 pt right).
pub(super) fn bump_last_kern(s: &str, delta: i64) -> String {
    let end = s.rfind("] TJ").expect("a TJ splice");
    let head = &s[..end];
    let start = head.rfind(' ').map_or(0, |i| i + 1);
    let n: i64 = head[start..].parse().expect("a whole kern");
    format!("{}{}{}", &s[..start], n - delta, &s[end..])
}

/// IND-09b: the follower "42" moved by a width error both walks share, with an edit ("Hello" →
/// "Hello world") whose new glyphs cover it — the pixels cannot see it, G-TEXT must.
#[test]
fn ind_09b_follower_moved_under_an_overlapping_edit_fails_the_words() {
    for new in ["Hello world", "Hello wonderful world"] {
        let test = format!("ind_09b_{}", new.len());
        let pdf = follower_doc(Some("-665 -325 2000 1006"));
        let Some(h) = Honest::new(&test, pdf, 0, &[ed("Hello", new)]) else {
            return;
        };
        for delta in [40i64, 600] {
            let name = format!("{}-{delta}.pdf", new.len());
            let (plan, digest, out) = h.bad_plan(&name, |s| bump_last_kern(s, delta));
            let id = format!("IND-09b {new:?} +{delta}");
            fails_at(
                h.phase_a_of(&plan, &digest, &out),
                "PEN_DRIFT",
                "A4",
                "drift",
                &format!("{id} A4"),
            );
            let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
            fails_at(
                r,
                "EDIT_VERIFY_FAILED",
                "A5",
                "next to the edited line",
                &id,
            );
        }
    }
}

/// IND-04b: the edited show leaks its fill onto the next cell, which the new glyphs overlap.
/// White ink is lost under the masks (A5's ink check); a visible re-colour keeps its ink and is
/// A4's alone (`STATE_CHANGED`, DEVIATIONS `[fix-final]`).
#[test]
fn ind_04b_fill_leak_onto_an_overlapped_cell() {
    let pdf = helvetica_page(
        b"BT /F1 11 Tf 58 700 Td (Desk lamp) Tj ET BT /F1 11 Tf 132 700 Td (4) Tj ET",
    );
    let Some(h) = Honest::new(
        "ind_04b",
        pdf,
        0,
        &[ed("Desk lamp", "Desk lamp with a long cable")],
    ) else {
        return;
    };
    for (name, leak) in [("white", " 1 g"), ("red", " 1 0 0 rg")] {
        let (plan, digest, out) = h.bad_plan(&format!("{name}.pdf"), |s| format!("{s}{leak}"));
        let id = format!("IND-04b {name}");
        fails_at(
            h.phase_a_of(&plan, &digest, &out),
            "STATE_CHANGED",
            "A4",
            "fill",
            &format!("{id} A4"),
        );
        if name == "white" {
            let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
            fails_at(r, "EDIT_VERIFY_FAILED", "A5", "lost its ink", &id);
        }
    }
}

/// IND-11b: a writer that moves, deletes or whitens the overlapped cell "4" (A2–A4 skipped, so
/// only Poppler judges) fails A5; the honest overlap passes with its cells at their boxes.
#[test]
fn ind_11b_overlapped_cells_moved_deleted_or_whitened_fail_a5() {
    let Some(h) = Honest::new(
        "ind_11b_overlap",
        table_row(),
        0,
        &[ed("Desk lamp", "Desk lamp with a long cable")],
    ) else {
        return;
    };
    let honest = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
    let cell = "BT /F1 11 Tf 132 700 Td (4) Tj ET";
    assert!(honest.contains(cell), "IND-11b fixture");
    let cases = [
        (
            "moved_1pt",
            "BT /F1 11 Tf 133 700 Td (4) Tj ET",
            "next to the edited line",
        ),
        (
            "moved_4pt",
            "BT /F1 11 Tf 136 700 Td (4) Tj ET",
            "next to the edited line",
        ),
        ("deleted", "", "next to the edited line"),
        (
            "white",
            "BT 1 g /F1 11 Tf 132 700 Td (4) Tj ET 0 g",
            "lost its ink",
        ),
    ];
    for (name, replacement, what) in cases {
        let out = h.path(&format!("{name}.pdf"));
        let bad = honest.replace(cell, replacement).into_bytes();
        fakes::replace_streams(h.qpdf(), &h.source, &out, &[(h.part_id(0), bad)])
            .unwrap_or_else(|e| panic!("{e}"));
        let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
        fails_at(
            r,
            "EDIT_VERIFY_FAILED",
            "A5",
            what,
            &format!("IND-11b {name}"),
        );
    }
}

/// review-final LOW-3: the cell "4" re-labelled "45" through ActualText changes no pixel and
/// Poppler reads "45" at the cell's box. A2 (the object graph differs from the plan's) refuses it
/// in the full Phase A; A5 alone now refuses it too (a longer word at the same box is not a join).
#[test]
fn ind_11c_a_neighbour_relabelled_with_more_characters_fails_a2() {
    let Some(h) = Honest::new("ind_11c", table_row(), 0, &[ed("Desk lamp", "Desk lamps")]) else {
        return;
    };
    let honest = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
    let cell = "BT /F1 11 Tf 132 700 Td (4) Tj ET";
    for label in ["45", "14"] {
        let out = h.path(&format!("relabel-{label}.pdf"));
        let span = format!("/Span <</ActualText ({label})>> BDC {cell} EMC");
        let bad = honest.replace(cell, &span).into_bytes();
        fakes::replace_streams(h.qpdf(), &h.source, &out, &[(h.part_id(0), bad)])
            .unwrap_or_else(|e| panic!("{e}"));
        let id = format!("IND-11c {label}");
        fails_at(h.phase_a(&out), "EDIT_VERIFY_FAILED", "A2", "Contents", &id);
        let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
        fails_at(
            r,
            "EDIT_VERIFY_FAILED",
            "A5",
            "next to the edited line",
            &format!("{id} A5"),
        );
    }
}
