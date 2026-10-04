//! GATE-01…08 and 11…13: planner bugs. The tampered replacement is written consistently (the expected
//! parts, the qpdf update and the "before" digest all carry it), so A0–A3 pass and the re-walk
//! (A4) must catch it — with the code the editor shows.

use super::{ed, fails_at, skipping, Ed, Honest};
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::verify::test_seams::skip_grammar;

/// "Hello" followed on its line by " tail" at 14 pt (a pen-chained follower that is not joined).
fn hello_tail() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj /F1 14 Tf ( tail) Tj ET BT /F1 12 Tf 72 650 Td (Other) Tj ET",
    )
}

fn honest(test: &str, pdf: Vec<u8>, edits: &[Ed<'_>]) -> Option<Honest> {
    Honest::new(test, pdf, 0, edits)
}

#[test]
fn gate_01_compensation_dropped_is_pen_drift() {
    let Some(h) = honest("gate_01", hello_tail(), &[ed("Hello", "Help")]) else {
        return;
    };
    // "Help" (2056) is 222 thousandths shorter than "Hello" (2278): `[<48656C70> -222] TJ`.
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace(" -222] TJ", "] TJ"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "PEN_DRIFT",
        "A4",
        "drift",
        "GATE-01",
    );
}

#[test]
fn gate_02_compensation_sign_flipped_or_off_by_one_unit() {
    let Some(h) = honest("gate_02", hello_tail(), &[ed("Hello", "Help")]) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("flip.pdf", |s| s.replace(" -222] TJ", " 222] TJ"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "PEN_DRIFT",
        "A4",
        "drift",
        "GATE-02 sign",
    );
    // One unit = 0.012 pt at 12 pt: above the 0.01 pt tolerance.
    let (plan, digest, out) = h.bad_plan("unit.pdf", |s| s.replace(" -222] TJ", " -221] TJ"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "PEN_DRIFT",
        "A4",
        "drift",
        "GATE-02 unit",
    );
}

#[test]
fn gate_03_tc_not_restored() {
    let style = SourceTextStyleIn {
        letter_spacing_pt: Some(1.0),
        ..Default::default()
    };
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style,
    }];
    let Some(h) = honest("gate_03", hello_tail(), &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace("] TJ 0 Tc", "] TJ"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "STATE_CHANGED",
        "A4",
        "field=tc",
        "GATE-03",
    );
}

#[test]
fn gate_04_b15_fill_leaks_onto_a_later_path() {
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 72 600 50 50 re f");
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style: SourceTextStyleIn {
            fill: Some("#ff0000".into()),
            ..Default::default()
        },
    }];
    let Some(h) = honest("gate_04", pdf, &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace("] TJ 0 g", "] TJ"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "STATE_CHANGED",
        "A4",
        "paint_changed",
        "GATE-04",
    );
    // The independent render sees the red square too.
    let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "check=render", "GATE-04 A5");
}

#[test]
fn gate_05_default_black_restore_and_05b_cross_family() {
    // Never set: `0 0 0 rg` is the initial black by effect — passes (mobile edit.test.ts:825).
    let red = || SourceTextStyleIn {
        fill: Some("#ff0000".into()),
        ..Default::default()
    };
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 72 600 50 50 re f");
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style: red(),
    }];
    let Some(h) = honest("gate_05", pdf, &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("ok.pdf", |s| s.replace("] TJ 0 g", "] TJ 0 0 0 rg"));
    let r = h.phase_a_of(&plan, &digest, &out);
    assert!(
        r.is_ok(),
        "GATE-05 default black by effect: {:?}",
        r.err().map(|e| e.details)
    );
    // Set explicitly as `0 g`, restored as `0 0 0 1 k`: another colour family (D35).
    let pdf = helvetica_page(b"0 g BT /F1 12 Tf 72 700 Td (Hello) Tj ET 72 600 50 50 re f");
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style: red(),
    }];
    let Some(h) = honest("gate_05b", pdf, &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace("] TJ 0 g", "] TJ 0 0 0 1 k"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "STATE_CHANGED",
        "A4",
        "field=fill",
        "GATE-05b",
    );
}

#[test]
fn gate_06_tf_restored_with_another_size() {
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj 0 0 1 rg ( tail) Tj ET 0 g BT /F1 12 Tf 72 650 Td (Other) Tj ET",
    );
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style: SourceTextStyleIn {
            size_pt: Some(14.0),
            ..Default::default()
        },
    }];
    let Some(h) = honest("gate_06", pdf, &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace("TJ /F1 12 Tf", "TJ /F1 13 Tf"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "STATE_CHANGED",
        "A4",
        "field=tfs",
        "GATE-06",
    );
}

#[test]
fn gate_07_tz_tr_ts_in_the_replacement() {
    let Some(h) = honest("gate_07", hello_tail(), &[ed("Hello", "Help")]) else {
        return;
    };
    for (op, field) in [("50 Tz", "th"), ("1 Tr", "tr"), ("3 Ts", "ts")] {
        let name = format!("bad-{}.pdf", &op[op.len() - 2..]);
        let (plan, digest, out) = h.bad_plan(&name, |s| format!("{op} {s}"));
        let r = h.phase_a_of(&plan, &digest, &out);
        fails_at(
            r,
            "EDIT_VERIFY_FAILED",
            "A4",
            "forbidden_operator",
            &format!("GATE-07 {op}"),
        );
        let _g = skip_grammar();
        let r = h.phase_a_of(&plan, &digest, &out);
        fails_at(
            r,
            "STATE_CHANGED",
            "A4",
            &format!("field={field}"),
            &format!("GATE-07 {op} bypassed"),
        );
    }
}

#[test]
fn gate_08_q_or_bt_in_the_replacement() {
    let Some(h) = honest("gate_08", hello_tail(), &[ed("Hello", "Help")]) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("q.pdf", |s| format!("q {s} Q"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "op=q",
        "GATE-08 q",
    );
    let (plan, digest, out) = h.bad_plan("bt.pdf", |s| format!("ET BT {s}"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "forbidden_operator",
        "GATE-08 BT",
    );
    let _g = skip_grammar();
    let (plan, digest, out) = h.bad_plan("bt2.pdf", |s| format!("BT {s} ET"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "page_refused",
        "GATE-08 BT bypassed",
    );
}

#[test]
fn gate_11_non_sibling_font() {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let c =
        d.add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F2 12 Tf 72 650 Td (Mono) Tj ET",
        &format!("/Font << /F1 {f} 0 R /F2 {c} 0 R >>"),
    ));
    let Some(h) = honest("gate_11", d.build(), &[ed("Hello", "Help")]) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| format!("/F2 12 Tf {s} /F1 12 Tf"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "what=font",
        "GATE-11",
    );
}

#[test]
fn gate_12_b2_effective_size_written_as_tf() {
    let edits = [Ed {
        old: "Hi",
        new: "Hi",
        style: SourceTextStyleIn {
            size_pt: Some(13.0),
            ..Default::default()
        },
    }];
    let Some(h) = honest("gate_12", fx::tf1_tm12(), &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("bad.pdf", |s| s.replace("/F1 1.0833 Tf", "/F1 13 Tf"));
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "what=size",
        "GATE-12",
    );
}

#[test]
fn gate_13_b3_code_with_an_empty_glyph() {
    // A Word subset of "Hello": "Hello" → "Hollo" is honest; the bug writes "Y" (no glyph in the
    // subset, width 0) for the new "o", and expects it too.
    let Some(h) = honest("gate_13", fx::subset_without_y(), &[ed("Hello", "Hollo")]) else {
        return;
    };
    let (plan, digest, out) = h.bad_run("bad.pdf", |run| {
        let s = &mut run.splices[0];
        let text = String::from_utf8_lossy(&s.bytes).replacen("<486F", "<4859", 1);
        s.bytes = text.into_bytes();
        run.expected.glyphs[1].2.value = 0x59;
        run.expected.text = "HYllo".into();
    });
    fails_at(
        h.phase_a_of(&plan, &digest, &out),
        "EDIT_VERIFY_FAILED",
        "A4",
        "what=glyph",
        "GATE-13",
    );
}
