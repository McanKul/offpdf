//! GATE-23 and 26…29: collateral damage — marked content dropped around the run, an annotation
//! on another page, a layer switched off, a font's widths changed on an unedited page, and an
//! untouched legacy-filter page altered.

use super::{ed, fails_at, skipping, Honest};
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, word_font, DocBuilder, LegacyFilter, PageSpec, HELVETICA,
};
use lopdf::{Document, Object, ObjectId};
use serde_json::json;

/// The id of `/Resources /<category> /<name>` of page `page` in `doc` (a reference).
fn resource_id(doc: &Document, page: usize, category: &[u8], name: &[u8]) -> ObjectId {
    let page_id = fakes::page_ids(doc)[page];
    let res = doc
        .get_dictionary(page_id)
        .and_then(|d| d.get(b"Resources"))
        .and_then(|r| match r {
            Object::Reference(id) => doc.get_dictionary(*id),
            Object::Dictionary(d) => Ok(d),
            _ => Err(lopdf::Error::DictKey),
        })
        .expect("resources");
    let cat = match res.get(category).expect("category") {
        Object::Reference(id) => doc.get_dictionary(*id).expect("category dict"),
        Object::Dictionary(d) => d,
        _ => panic!("category"),
    };
    cat.get(name)
        .and_then(Object::as_reference)
        .expect("resource reference")
}

#[test]
fn gate_23_marked_content_removed_around_the_run() {
    let pdf = helvetica_page(
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Hello) Tj ET EMC BT /F1 12 Tf 72 650 Td (Other) Tj ET",
    );
    let Some(h) = Honest::new("gate_23", pdf, 0, &[ed("Hello", "Help")]) else {
        return;
    };
    // The writer blanks the BDC and EMC operators (same length: no span moves).
    let honest = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
    let blank = |s: &str, op: &str| s.replacen(op, &" ".repeat(op.len()), 1);
    let bad = blank(&blank(&honest, "/P <</MCID 0>> BDC"), "EMC");
    let out = h.path("bad.pdf");
    fakes::replace_streams(
        h.qpdf(),
        &h.source,
        &out,
        &[(h.part_id(0), bad.into_bytes())],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "what=data",
        "GATE-23",
    );
    let r = skipping(&["A2", "A3"], || h.phase_a(&out));
    fails_at(r, "STATE_CHANGED", "A4", "field=marked", "GATE-23 A4");
}

/// Page 1 (edited) in Helvetica; page 2 in a Word subset with `/Widths` and an `/F1` of its own.
fn two_pages() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let w = word_font(&mut d.b, "ABCDEF+Calibri", "Second page");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Second page) Tj ET",
        &format!("/Font << /F1 {w} 0 R >>"),
    ));
    d.build()
}

#[test]
fn gate_26_annotation_added_on_another_page() {
    let Some(h) = Honest::new("gate_26", two_pages(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let out = h.path("link.pdf");
    fakes::link(h.qpdf(), &h.staged, &out, 1).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[1]",
        "GATE-26 link",
    );
    let out = h.path("freetext.pdf");
    fakes::freetext(
        h.qpdf(),
        &h.staged,
        &out,
        1,
        [72.0, 690.0, 200.0, 712.0],
        "Note",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root/Pages/Kids[1]",
        "GATE-26 FreeText",
    );
}

#[test]
fn gate_27_layer_switched_off_in_the_catalog() {
    let Some(h) = Honest::new(
        "gate_27",
        fx::indd(),
        0,
        &[ed("visible layer", "visible layers")],
    ) else {
        return;
    };
    let staged = fakes::load(&h.staged);
    let cat = staged
        .get_dictionary(fakes::catalog_id(&staged))
        .expect("catalog");
    let ocgs = cat
        .get(b"OCProperties")
        .and_then(Object::as_dict)
        .and_then(|p| p.get(b"OCGs"))
        .and_then(Object::as_array)
        .expect("OCGs");
    let visible = ocgs[0].as_reference().expect("ocg ref");
    let out = h.path("off.pdf");
    fakes::ocg_off(h.qpdf(), &h.staged, &out, visible).unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "path=/Root what=node",
        "GATE-27",
    );
}

#[test]
fn gate_28_widths_changed_on_an_unedited_page() {
    let Some(h) = Honest::new("gate_28", two_pages(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let font = resource_id(&fakes::load(&h.staged), 1, b"Font", b"F1");
    let out = h.path("widths.pdf");
    fakes::widths_changed(h.qpdf(), &h.staged, &out, font, 50, 501.0)
        .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "Kids[1]/Resources/Font/F1",
        "GATE-28",
    );
}

#[test]
fn gate_29_untouched_runlength_page_altered() {
    let pdf = fx::legacy_filter_page(LegacyFilter::RunLength);
    let Some(h) = Honest::new("gate_29", pdf, 1, &[ed("Hello", "Help")]) else {
        return;
    };
    let staged = fakes::load(&h.staged);
    let page0 = fakes::page_ids(&staged)[0];
    let rl = staged
        .get_dictionary(page0)
        .and_then(|d| d.get(b"Contents"))
        .and_then(Object::as_reference)
        .expect("RunLength contents");
    // Literal runs of a different text: still RunLength, decodes differently.
    let text = b"BT /F1 12 Tf 72 720 Td (Altered) Tj ET";
    let mut raw = vec![(text.len() - 1) as u8];
    raw.extend_from_slice(text);
    raw.push(128);
    let out = h.path("rl.pdf");
    fakes::raw_stream(
        h.qpdf(),
        &h.staged,
        &out,
        rl,
        json!({ "/Filter": "/RunLengthDecode" }),
        &raw,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A2",
        "Kids[0]/Contents what=data",
        "GATE-29",
    );
}

#[test]
fn phase_a_and_b_with_two_edited_pages_in_one_copy() {
    use super::{opts, plan_of};
    use crate::pdf_engine::text_edit::apply::{apply_update, updates_for_plan, write_update_json};
    use crate::pdf_engine::text_edit::context::SnapshotContext;
    use crate::pdf_engine::text_edit::gate::{
        verify_edited_copy, verify_final_output, BeforeModel, EditedPageInput, PhaseAInput,
        PopplerRef,
    };
    use crate::pdf_engine::text_edit::graph::graph_digest;
    use crate::pdf_engine::text_edit::limits::VERIFY_CAP_MARGIN_BYTES;
    use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
    use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
    use std::collections::HashMap;
    // Pages 1 and 3 edited in one qpdf update (the Save shape for one source file).
    let Some(engines) = engines_or_skip("phase_a_two_pages") else {
        return;
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    for text in ["First page", "Middle page", "Last page"] {
        d.page(PageSpec::new(
            format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET").as_bytes(),
            &res,
        ));
    }
    let pdf = d.build();
    let dir = Scratch::new("phase_a_two_pages");
    let source = dir.write("source.pdf", &pdf);
    let ctx = SnapshotContext::new(snapshot_from_bytes(&source, pdf, None).expect("snapshot"));
    let (m0, p0) = plan_of(&ctx, 0, &[ed("First page", "First pages")]);
    let (m2, p2) = plan_of(&ctx, 2, &[ed("Last page", "Last pages")]);
    let mut updates = updates_for_plan(&m0.content, &p0).expect("updates");
    updates.extend(updates_for_plan(&m2.content, &p2).expect("updates"));
    let update = dir.path("update.json");
    write_update_json(&updates, ctx.doc().max_id, &update).expect("json");
    let staged = dir.path("edited.pdf");
    apply_update(&engines, &source, &update, &staged, &[], &opts()).expect("qpdf");
    let replaced: HashMap<_, _> = updates
        .into_iter()
        .map(|u| (u.object_id, u.decoded))
        .collect();
    let digest = graph_digest(ctx.doc(), &replaced, None).expect("digest");
    let page = |model, plan, i: u32| EditedPageInput {
        model: BeforeModel::Kept(model),
        plan,
        input_page_index: i,
        input_render: PopplerRef {
            pdf: source.clone(),
            page_1: i + 1,
        },
    };
    let input = PhaseAInput {
        before: &digest,
        before_page_count: 3,
        staged: &staged,
        staged_cap: 2 * std::fs::metadata(&source).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES,
        pages: vec![page(&m0, &p0, 0), page(&m2, &p2, 2)],
        source_benign: &[],
    };
    let report = verify_edited_copy(&input, &engines, dir.dir(), &opts())
        .unwrap_or_else(|e| panic!("Phase A: {e} {:?}", e.details));
    assert_eq!(report.proofs.len(), 2);
    let expectations = [(0, &report.proofs[0]), (2, &report.proofs[1])];
    let cap = 4 * std::fs::metadata(&staged).map_or(0, |m| m.len()) + VERIFY_CAP_MARGIN_BYTES;
    let r = verify_final_output(&staged, cap, &engines, &expectations, &opts());
    assert!(r.is_ok(), "Phase B: {:?}", r.err().map(|e| e.details));
    // The proofs swapped between the pages: Phase B refuses.
    let swapped = [(0, &report.proofs[1]), (2, &report.proofs[0])];
    let r = verify_final_output(&staged, cap, &engines, &swapped, &opts());
    fails_at(r, "SOURCE_EDIT_GATE_FAILED", "B1", "", "two pages swapped");
}
