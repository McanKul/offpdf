//! review-verify HIGH-A: IND-09c / IND-04c. A narrow follower right after the edited run — a 7 pt
//! superscript footnote marker, a ":" or a "." in another colour — lies within the 2 pt pad of the
//! edited run's old box and within G-RENDER's mask slack. With A4 — our own walk — skipped (an
//! error both walks would share, B13), Poppler alone must refuse every visible change to it:
//! G-TEXT finds it as a neighbour by our model's glyphs (a word Poppler joined across the edit,
//! "Hello:", is split at its outer edge), and G-RENDER checks its pixels outside the edited
//! glyphs' own boxes. The variants are the verifier's (`v04/verify-logs`,
//! `probe-zv_b2_narrow_followers.log`).

use super::liberation;
use super::overlap::bump_last_kern;
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::{helvetica_page, DocBuilder, PageSpec};
use crate::pdf_engine::text_edit::tests_gate::{ed, fails_at, skipping, Honest};

/// (label, page content, the follower's show op as the honest content writes it, whether it is
/// pen-chained to "Hello").
const FOLLOWERS: [(&str, &str, &str, bool); 4] = [
    (
        "superscript",
        "BT /F1 12 Tf 72 700 Td (Hello) Tj /F1 7 Tf 5 Ts (1) Tj 0 Ts ET",
        "(1) Tj",
        true,
    ),
    (
        "red colon",
        "BT /F1 12 Tf 72 700 Td (Hello) Tj 1 0 0 rg (:) Tj 0 g ET",
        "(:) Tj",
        true,
    ),
    (
        "separate colon",
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT 1 0 0 rg /F1 12 Tf 99.34 700 Td (:) Tj ET 0 g",
        "(:) Tj",
        false,
    ),
    (
        "separate period",
        "BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT 1 0 0 rg /F1 12 Tf 100.3 700 Td (.) Tj ET 0 g",
        "(.) Tj",
        false,
    ),
];

/// A shorter line, and one whose new glyphs run over the follower (an overlap warning).
const EDITS: [&str; 2] = ["Help", "Hello world"];

/// Every A5 failure of these tests names the follower.
const NEXT: &str = "next to the edited line";

/// The honest edit of `content`, verified (an overlapping edit next to the follower used to be
/// refused: Poppler orders the follower's word between the new ones).
fn honest(test: &str, content: &str, new: &str) -> Option<Honest> {
    let id = format!("{test}_{}", new.len());
    Honest::new(
        &id,
        helvetica_page(content.as_bytes()),
        0,
        &[ed("Hello", new)],
    )
}

/// IND-09c (writer): the follower deleted, whitened, made invisible, moved or relabelled in the
/// written content, with A2–A4 skipped, fails A5.
#[test]
fn ind_09c_a_narrow_follower_the_writer_changed_fails_a5() {
    for (label, content, show, _) in FOLLOWERS {
        for new in EDITS {
            let Some(h) = honest("ind_09c_writer", content, new) else {
                return;
            };
            let written = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
            assert!(written.contains(show), "IND-09c {label}: {written}");
            let glyph = show.trim_end_matches(" Tj");
            let cases = [
                ("deleted", String::new()),
                ("white", format!("1 g {show} 0 g")),
                ("invisible", format!("3 Tr {show} 0 Tr")),
                ("moved", format!("[-250 {glyph}] TJ")),
                ("relabelled", "(7) Tj".to_string()),
            ];
            for (name, replacement) in cases {
                let id = format!("IND-09c {label} {new:?} {name}");
                let out = h.path(&format!("{name}.pdf"));
                let bad = written.replacen(show, &replacement, 1).into_bytes();
                fakes::replace_streams(h.qpdf(), &h.source, &out, &[(h.part_id(0), bad)])
                    .unwrap_or_else(|e| panic!("{id}: {e}"));
                let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
                fails_at(r, "EDIT_VERIFY_FAILED", "A5", NEXT, &id);
            }
        }
    }
}

/// IND-09c (planner): a compensation 40, 250 or 600 thousandths of an em too small moves a
/// pen-chained follower 0.48 to 7.2 pt; A4 sees it (`PEN_DRIFT`), and so does A5 alone.
#[test]
fn ind_09c_a_narrow_follower_moved_by_a_shared_width_error_fails_a5() {
    for (label, content, _, chained) in FOLLOWERS {
        if !chained {
            continue; // its own text object: the edited run's kerns cannot move it
        }
        for new in EDITS {
            let Some(h) = honest("ind_09c_planner", content, new) else {
                return;
            };
            for delta in [40i64, 250, 600] {
                let id = format!("IND-09c {label} {new:?} +{delta}");
                let name = format!("kern-{delta}.pdf");
                let (plan, digest, out) = h.bad_plan(&name, |s| bump_last_kern(s, delta));
                let full = h.phase_a_of(&plan, &digest, &out);
                fails_at(full, "PEN_DRIFT", "A4", "drift", &format!("{id} A4"));
                let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
                fails_at(r, "EDIT_VERIFY_FAILED", "A5", NEXT, &id);
            }
        }
    }
}

/// IND-04c: the edited show leaks white fill, `3 Tr` or red fill onto the follower. A4 refuses
/// each (`STATE_CHANGED`, or the splice grammar's `forbidden_operator`); with A4 skipped, A5 does
/// too — except a red superscript under the overlapping edit, which keeps its ink under the new
/// space's mask: a visible re-colour there is A4's alone (DEVIATIONS `[fix-last]`). A follower
/// that sets its own fill is not affected by a fill leak, so only `3 Tr` reaches it.
#[test]
fn ind_04c_a_state_leak_onto_a_narrow_follower_fails_a5() {
    for (label, content, _, _) in FOLLOWERS {
        let leaks: &[(&str, &str, &str)] = if label == "superscript" {
            &[
                ("white", " 1 g", "fill"),
                ("invisible", " 3 Tr", "op=Tr"),
                ("red", " 1 0 0 rg", "fill"),
            ]
        } else {
            &[("invisible", " 3 Tr", "op=Tr")]
        };
        for new in EDITS {
            let Some(h) = honest("ind_04c", content, new) else {
                return;
            };
            for (name, leak, what) in leaks {
                let id = format!("IND-04c {label} {new:?} {name}");
                let (plan, digest, out) =
                    h.bad_plan(&format!("{name}.pdf"), |s| format!("{s}{leak}"));
                let full = h.phase_a_of(&plan, &digest, &out);
                let code = if *name == "invisible" {
                    "EDIT_VERIFY_FAILED"
                } else {
                    "STATE_CHANGED"
                };
                fails_at(full, code, "A4", what, &format!("{id} A4"));
                if *name == "red" && new != "Help" {
                    continue;
                }
                let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
                fails_at(r, "EDIT_VERIFY_FAILED", "A5", NEXT, &id);
            }
        }
    }
}

/// IND-09c (honest, program glyph boxes): new glyphs of an embedded font that run over the next
/// cells change no pixel of those cells outside their own boxes grown by `RENDER_OWN_PAD_PX`
/// (Poppler's anti-aliasing reaches one pixel past a glyph's box: the overlap sweeps on
/// LibreOffice files were refused without the pad).
#[test]
fn ind_09c_an_overlap_in_an_embedded_font_passes() {
    let mut d = DocBuilder::new();
    let chars = "Desk lampwithongcb4120.QyPr";
    let (font, gids) = liberation(&mut d, "LiberationSans-Regular", chars);
    let hex = |t: &str| -> String { t.chars().map(|c| format!("{:04X}", gids[&c])).collect() };
    let cell = |x: u32, y: u32, t: &str| format!("BT /F1 11 Tf {x} {y} Td <{}> Tj ET ", hex(t));
    let content = [
        cell(58, 700, "Desk lamp"),
        cell(132, 700, "4"),
        cell(156, 700, "120.00"),
        cell(58, 660, "Qty"),
        cell(80, 660, "Price"),
    ]
    .concat();
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {font} 0 R >>"),
    ));
    let pdf = d.build();
    for (i, (old, new)) in [
        ("Desk lamp", "Desk lamp with a long cable"),
        ("Qty", "QtyQtyQty"),
    ]
    .into_iter()
    .enumerate()
    {
        let test = format!("ind_09c_embedded_{i}");
        if Honest::new(&test, pdf.clone(), 0, &[ed(old, new)]).is_none() {
            return;
        }
    }
}

/// No new false refusal (review-verify HIGH-A): a new letter that covers a "." of another run
/// (Poppler merges them: "phraser."), and a title under a smaller stamp whose glyphs' centres fall
/// inside the title's words, both verify (both were refused by the first version of this fix on
/// the LibreOffice and MuPDF-stamped corpus files).
#[test]
fn ind_09c_a_covered_follower_and_a_stamp_over_the_line_pass() {
    let cases = [
        (
            "BT /F1 12 Tf 72 700 Td (red phrase) Tj 1 0 0 rg (.) Tj 0 g ET",
            "red phrase",
            "red phraser",
        ),
        (
            "BT /F1 26 Tf 34 700 Td (Quarterly Report) Tj ET BT /F1 9 Tf 72 715 Td (Approved) Tj ET",
            "Quarterly Report",
            "Quarterly Reports",
        ),
    ];
    for (i, (content, old, new)) in cases.into_iter().enumerate() {
        let pdf = helvetica_page(content.as_bytes());
        if Honest::new(&format!("ind_09c_honest_{i}"), pdf, 0, &[ed(old, new)]).is_none() {
            return;
        }
    }
}
