//! GATE-09, 10 and 14…17: writer bugs — qpdf's input is updated with other bytes than the plan
//! expects (or in the wrong place). A2 (whole graph) fails first; skipping it, A3 (exact parts)
//! fails; skipping both, the re-walk (A4) fails where the spec lists it.

use super::{ed, fails_at, opts, plan_of, skipping, Honest};
use crate::pdf_engine::text_edit::apply::{apply_update, write_update_json, PartUpdate};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::gate::{
    verify_edited_copy, EditedPageInput, PhaseAInput, PopplerRef,
};
use crate::pdf_engine::text_edit::graph::{graph_digest, DataKey};
use crate::pdf_engine::text_edit::limits::VERIFY_CAP_MARGIN_BYTES;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::{
    helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use std::collections::HashMap;

/// Two lines in one part: "Hello" (edited) and its neighbour "Other".
fn two_lines() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Other) Tj ET")
}

impl Honest {
    /// qpdf's input with part `part` of the edited page replaced by `bytes` (a writer bug).
    fn writer(&self, name: &str, part: usize, bytes: &[u8]) -> std::path::PathBuf {
        let out = self.path(name);
        fakes::replace_streams(
            self.qpdf(),
            &self.source,
            &out,
            &[(self.part_id(part), bytes.to_vec())],
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        out
    }

    /// The honestly edited part `i`, as text.
    fn expected(&self, i: usize) -> String {
        String::from_utf8_lossy(&self.plan.expected_parts[i]).into_owned()
    }

    /// A2 fails on `file`; with A2 skipped A3 fails; with both skipped A4 fails with `a4`.
    fn a2_a3_a4(&self, file: &std::path::Path, a4: Option<(&str, &str)>, id: &str) {
        fails_at(
            self.phase_a(file),
            "EDIT_VERIFY_FAILED",
            "A2",
            "what=data",
            id,
        );
        let r = skipping(&["A2"], || self.phase_a(file));
        fails_at(r, "EDIT_VERIFY_FAILED", "A3", "differ", &format!("{id} A3"));
        if let Some((code, what)) = a4 {
            let r = skipping(&["A2", "A3"], || self.phase_a(file));
            fails_at(r, code, "A4", what, &format!("{id} A4"));
        }
    }
}

#[test]
fn gate_09_an_unedited_tj_deleted() {
    let Some(h) = Honest::new("gate_09", two_lines(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let bytes = h.expected(0).replace("(Other) Tj", "");
    let out = h.writer("bad.pdf", 0, bytes.as_bytes());
    h.a2_a3_a4(
        &out,
        Some(("EDIT_VERIFY_FAILED", "record_unpaired")),
        "GATE-09",
    );
}

#[test]
fn gate_10_wrong_code_written() {
    let Some(h) = Honest::new("gate_10", two_lines(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let honest = h.expected(0);
    assert!(honest.contains("<48656C70>"), "GATE-10 {honest}");
    let out = h.writer(
        "bad.pdf",
        0,
        honest.replace("<48656C70>", "<48656C71>").as_bytes(),
    );
    h.a2_a3_a4(&out, Some(("EDIT_VERIFY_FAILED", "what=text")), "GATE-10");
}

#[test]
fn gate_14_shared_stream_edited_in_place() {
    // Page 1 has a stream of its own; pages 2 and 3 share one. A planner whose SHARED_CONTENT
    // refusal was bypassed targets the shared stream: the digest keeps that stream's original
    // data (it is reached twice), so A2 fails at the page that shares it.
    let Some(engines) = crate::pdf_engine::text_edit::testkit::engines_or_skip("gate_14") else {
        return;
    };
    let body = b"BT /F1 12 Tf 72 700 Td (Shared body) Tj ET";
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Own page) Tj ET",
        &res,
    ));
    let shared = d.b.add_stream("", body);
    d.page_raw(&format!("{shared} 0 R"), &PageSpec::new(b"", &res));
    d.page_raw(&format!("{shared} 0 R"), &PageSpec::new(b"", &res));
    let pdf = d.build();
    // The plan is made on a twin page with the same bytes that is not shared.
    let twin = SnapshotContext::new(
        snapshot_from_bytes(std::path::Path::new("twin.pdf"), helvetica_page(body), None)
            .unwrap_or_else(|e| panic!("{e}")),
    );
    let (_, plan) = plan_of(&twin, 0, &[ed("Shared body", "Shared bodies")]);
    let dir = crate::pdf_engine::text_edit::testkit::Scratch::new("gate_14");
    let source = dir.write("source.pdf", &pdf);
    let ctx = SnapshotContext::new(
        snapshot_from_bytes(&source, pdf, None).unwrap_or_else(|e| panic!("{e}")),
    );
    let model = build_page_model(&ctx, 1, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        model.runs[0].reason,
        Some(crate::pdf_engine::text_edit::reasons::TextReason::SharedContent),
        "GATE-14 the refusal that was bypassed"
    );
    let id = (shared, 0);
    let updates = vec![PartUpdate {
        object_id: id,
        decoded: plan.expected_parts[0].clone(),
    }];
    let update = dir.path("update.json");
    write_update_json(&updates, ctx.doc().max_id, &update).unwrap_or_else(|e| panic!("{e}"));
    let out = dir.path("bad.pdf");
    apply_update(&engines, &source, &update, &out, &[], &opts()).unwrap_or_else(|e| panic!("{e}"));
    let replaced: HashMap<_, _> = updates
        .into_iter()
        .map(|u| (u.object_id, u.decoded))
        .collect();
    let digest = graph_digest(ctx.doc(), &replaced, None).unwrap_or_else(|m| panic!("{m:?}"));
    assert!(
        !digest
            .entries
            .iter()
            .any(|e| matches!(e.data, DataKey::Replaced { .. })),
        "GATE-14 a shared stream is never Replaced"
    );
    let input = PhaseAInput {
        before: &digest,
        before_page_count: 3,
        staged: &out,
        staged_cap: 2 * std::fs::metadata(&source).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES,
        pages: vec![EditedPageInput {
            model: (&model).into(),
            plan: &plan,
            input_page_index: 1,
            input_render: PopplerRef {
                pdf: source.clone(),
                page_1: 2,
            },
        }],
        source_benign: &[],
    };
    let r = verify_edited_copy(&input, &engines, dir.dir(), &opts());
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A2",
        "Contents what=data",
        "GATE-14",
    );
}

#[test]
fn gate_15_untouched_operators_reformatted() {
    // A writer that re-serialises the part (lopdf `Content::encode`, never used in production).
    let pdf = helvetica_page(
        b"BT\n/F1   12 Tf\n72 700 Td (Hello) Tj ET\nBT /F1 12 Tf 72.000 650 Td (Other)Tj ET",
    );
    let Some(h) = Honest::new("gate_15", pdf, 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let decoded = lopdf::content::Content::decode(&h.plan.expected_parts[0]).expect("decode");
    let encoded = decoded.encode().expect("encode");
    assert_ne!(
        encoded, h.plan.expected_parts[0],
        "GATE-15 the round trip reformats"
    );
    let out = h.writer("bad.pdf", 0, &encoded);
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "what=data",
        "GATE-15",
    );
    let r = skipping(&["A2"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A3", "differ", "GATE-15 A3");
}

#[test]
fn gate_16_splice_off_by_one_byte_or_on_the_wrong_page() {
    let Some(h) = Honest::new("gate_16", two_lines(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    // One byte late: the planned replacement lands one byte to the right.
    let original = h.model.content.part_bytes(0).to_vec();
    let s = &h.plan.splices[0];
    let mut late = original[..s.local.start + 1].to_vec();
    late.extend_from_slice(&s.bytes);
    late.extend_from_slice(&original[(s.local.end + 1).min(original.len())..]);
    let out = h.writer("late.pdf", 0, &late);
    // The shifted splice breaks the content syntax, which `qpdf --check` (A0) already sees.
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A0",
        "qpdf --check",
        "GATE-16 off by one",
    );
    let r = skipping(&["A0", "A2"], || h.phase_a(&out));
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A3",
        "differ",
        "GATE-16 off by one A3",
    );
    // The right bytes on the wrong page.
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::new(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET", &res));
    d.page(PageSpec::new(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET", &res));
    let Some(h) = Honest::new("gate_16b", d.build(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let page2 = crate::pdf_engine::text_edit::runs::build_page_model(&h.ctx, 1, None)
        .unwrap_or_else(|e| panic!("{e}"));
    let out = h.path("wrong-page.pdf");
    fakes::replace_streams(
        h.qpdf(),
        &h.source,
        &out,
        &[(
            page2.content.parts[0].stream_id,
            h.plan.expected_parts[0].clone(),
        )],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "what=data",
        "GATE-16 wrong page",
    );
}

#[test]
fn gate_17_neighbour_run_changed() {
    let Some(h) = Honest::new("gate_17", two_lines(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let out = h.writer(
        "bad.pdf",
        0,
        h.expected(0).replace("(Other)", "(Otter)").as_bytes(),
    );
    h.a2_a3_a4(
        &out,
        Some(("EDIT_VERIFY_FAILED", "record_unpaired")),
        "GATE-17",
    );
}
