//! APP-01…09: the qpdf update (§B.13), qpdf's behaviours the gate relies on (P1, P4, P-RL, P-OV,
//! `/Extensions`), and the canonical graph digest (§B.16.1).

use super::{ctx, edit, model, ok_plan, plan, style};
use crate::pdf_engine::text_edit::apply::{
    apply_update, update_json, updates_for_plan, warning_text, write_update_json, PartUpdate,
};
use crate::pdf_engine::text_edit::content::{page_content, qpdf_join};
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::engines::{Engines, RunOpts};
use crate::pdf_engine::text_edit::graph::{graph_digest, graph_matches, GraphMismatch};
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::snapshot::{fnv1a_u64, read_verification_snapshot};
use crate::pdf_engine::text_edit::testkit::fakes;
use crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_page, DocBuilder, LegacyFilter, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::tests_gate::{ed, fails_at, skipping, Honest};
use lopdf::Object;
use std::collections::HashMap;
use std::path::Path;

fn opts() -> RunOpts<'static> {
    RunOpts::default()
}

/// A verification context over a file qpdf wrote.
fn open(path: &Path) -> SnapshotContext {
    SnapshotContext::new(
        read_verification_snapshot(path, 1 << 30).unwrap_or_else(|e| panic!("{e}")),
    )
}

fn edited(
    engines: &Engines,
    dir: &Scratch,
    pdf: Vec<u8>,
    old: &str,
    new: &str,
) -> (std::path::PathBuf, std::path::PathBuf, Vec<u8>) {
    let source = dir.write("source.pdf", &pdf);
    let c = ctx(pdf);
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, old, new, style())]);
    let p = ok_plan(&out);
    let update = dir.path("update.json");
    write_update_json(
        &updates_for_plan(&m.content, p).expect("updates"),
        c.doc().max_id,
        &update,
    )
    .expect("json");
    let staged = dir.path("staged.pdf");
    apply_update(engines, &source, &update, &staged, &[], &opts())
        .unwrap_or_else(|e| panic!("{e}"));
    (source, staged, p.expected_parts[0].clone())
}

#[test]
fn app_01_update_json_shape() {
    let v = update_json(
        &[PartUpdate {
            object_id: (4, 0),
            decoded: b"abc".to_vec(),
        }],
        9,
    );
    let q = v["qpdf"].as_array().expect("qpdf array");
    assert_eq!(q.len(), 2, "APP-01 header + objects");
    assert_eq!(q[0]["jsonversion"], 2);
    assert_eq!(q[0]["pushedinheritedpageresources"], false);
    assert_eq!(q[0]["calledgetallpages"], false);
    assert_eq!(q[0]["maxobjectid"], 9);
    let obj = &q[1]["obj:4 0 R"]["stream"];
    assert_eq!(
        obj["dict"],
        serde_json::json!({}),
        "APP-01 the empty dict drops /Filter"
    );
    assert_eq!(obj["data"], "YWJj", "APP-01 base64 of the decoded bytes");
    assert_eq!(q[1].as_object().map(|o| o.len()), Some(1));
}

#[test]
fn app_02_p1_qpdf_applies_the_decoded_bytes() {
    let Some(engines) = engines_or_skip("app_02") else {
        return;
    };
    let dir = Scratch::new("app_02");
    let (_, staged, expected) = edited(
        &engines,
        &dir,
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        "Hello",
        "Help",
    );
    let doc = fakes::load(&staged);
    let page = fakes::page_ids(&doc)[0];
    let id = doc
        .get_dictionary(page)
        .and_then(|d| d.get(b"Contents"))
        .and_then(Object::as_reference)
        .expect("contents");
    let s = doc
        .get_object(id)
        .and_then(Object::as_stream)
        .expect("stream");
    // `--compress-streams=n` (see apply.rs): the edited stream is written without a filter.
    assert!(
        s.dict.get(b"Filter").is_err(),
        "APP-02 no filter: {:?}",
        s.dict
    );
    let data =
        decode_stream(s, 1 << 20, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("decode");
    assert_eq!(data, expected, "APP-02 the planned bytes");
}

#[test]
fn app_03_non_benign_update_warnings_fail() {
    let Some(engines) = engines_or_skip("app_03") else {
        return;
    };
    let dir = Scratch::new("app_03");
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let c = ctx(pdf.clone());
    let m = model(&c, 0);
    let out = plan(&c, &m, &[edit(&m, "Hello", "Help", style())]);
    let update = dir.path("update.json");
    write_update_json(
        &updates_for_plan(&m.content, ok_plan(&out)).expect("updates"),
        c.doc().max_id,
        &update,
    )
    .expect("json");
    // A damaged xref: qpdf reconstructs it and exits 3.
    let i = pdf
        .windows(10)
        .rposition(|w| w == b"startxref\n")
        .expect("startxref");
    let mut damaged = pdf[..i].to_vec();
    damaged.extend_from_slice(b"startxref\n9\n%%EOF\n");
    let input = dir.write("damaged.pdf", &damaged);
    let r = apply_update(
        &engines,
        &input,
        &update,
        &dir.path("out1.pdf"),
        &[],
        &opts(),
    );
    let e = r.err().expect("APP-03 damaged input");
    assert_eq!(e.code, "EDIT_VERIFY_FAILED", "APP-03");
    // A wrong /Size: benign only when the source itself had that warning (`source_benign`).
    let at = pdf.windows(6).rposition(|w| w == b"/Size ").expect("/Size") + 6;
    let digits = pdf[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    let mut wrong = pdf[..at].to_vec();
    wrong.extend_from_slice(b"99");
    wrong.extend_from_slice(&pdf[at + digits..]);
    let input = dir.write("size.pdf", &wrong);
    let r = apply_update(
        &engines,
        &input,
        &update,
        &dir.path("out2.pdf"),
        &[],
        &opts(),
    );
    assert_eq!(
        r.err().map(|e| e.code),
        Some("EDIT_VERIFY_FAILED".to_string()),
        "APP-03 not in source_benign"
    );
    let benign = vec![format!(
        "WARNING: /elsewhere/copy.pdf: reported number of objects (99) is not one plus the highest object number ({})",
        c.doc().max_id
    )];
    let lines = apply_update(
        &engines,
        &input,
        &update,
        &dir.path("out3.pdf"),
        &benign,
        &opts(),
    )
    .unwrap_or_else(|e| panic!("APP-03 benign: {e} {:?}", e.details));
    assert_eq!(lines.len(), 1, "APP-03 the warning is returned");
    assert_eq!(
        warning_text(&lines[0]),
        warning_text(&benign[0]),
        "APP-03 file name ignored"
    );
}

#[test]
fn app_04_p4_mistargeted_update_caught_by_a0_and_a2() {
    let Some(h) = Honest::new(
        "app_04",
        helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
        0,
        &[ed("Hello", "Help")],
    ) else {
        return;
    };
    let out = h.path("mistargeted.pdf");
    fakes::mistargeted(h.qpdf(), &h.source, &out, 0, &h.plan.expected_parts[0])
        .unwrap_or_else(|e| panic!("{e}"));
    fails_at(
        h.phase_a(&out),
        "EDIT_VERIFY_FAILED",
        "A0",
        "qpdf --check",
        "APP-04 A0",
    );
    let r = skipping(&["A0", "A1"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A2", "", "APP-04 A2");
}

#[test]
fn app_05_source_bytes_untouched() {
    let Some(engines) = engines_or_skip("app_05") else {
        return;
    };
    let dir = Scratch::new("app_05");
    let pdf = fx::word();
    let before = fnv1a_u64(&pdf);
    let (source, _, _) = edited(&engines, &dir, pdf, "Due", "Due 7");
    let after = std::fs::read(&source).expect("source");
    assert_eq!(
        fnv1a_u64(&after),
        before,
        "APP-05 qpdf never writes its input"
    );
}

#[test]
fn app_06_p_rl_legacy_filter_pages_kept_raw() {
    for filter in [LegacyFilter::RunLength, LegacyFilter::Lzw] {
        let test = format!("app_06_{filter:?}");
        let pdf = fx::legacy_filter_page(filter);
        let Some(h) = Honest::new(&test, pdf.clone(), 1, &[ed("Hello", "Help")]) else {
            return;
        };
        // Honest passed A2: the legacy page's stream is byte-identical, still filtered.
        let raw = |path: &Path| {
            let doc = fakes::load(path);
            let page = fakes::page_ids(&doc)[0];
            let id = doc
                .get_dictionary(page)
                .and_then(|d| d.get(b"Contents"))
                .and_then(Object::as_reference)
                .expect("contents");
            let s = doc
                .get_object(id)
                .and_then(Object::as_stream)
                .expect("stream");
            (
                s.content.clone(),
                s.dict
                    .get(b"Filter")
                    .and_then(Object::as_name)
                    .map(<[u8]>::to_vec)
                    .ok(),
            )
        };
        assert_eq!(
            raw(&h.staged),
            raw(&h.source),
            "APP-06 {filter:?} raw bytes kept"
        );
    }
}

#[test]
fn app_07_p_ov_overlay_wrapper_holds_the_joined_parts() {
    let Some(engines) = engines_or_skip("app_07") else {
        return;
    };
    let dir = Scratch::new("app_07");
    let parts: [&[u8]; 3] = [
        b"BT /F1 12 Tf 72 700 Td",
        b"(Hi) Tj ET\n",
        b"BT /F1 12 Tf 72 650 Td (Lo) Tj ET",
    ];
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(&parts, &format!("/Font << /F1 {f} 0 R >>")));
    let input = dir.write("parts.pdf", &d.build());
    let mut b = DocBuilder::new();
    b.page(PageSpec::new(b"", ""));
    let blank = dir.write("blank.pdf", &b.build());
    let out = dir.path("overlaid.pdf");
    fakes::overlay(&engines.qpdf, &input, &out, &blank, None).unwrap_or_else(|e| panic!("{e}"));
    let c = open(&out);
    let page = c.page_id(0).expect("page");
    let content =
        page_content(c.doc(), page, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("content");
    let text = String::from_utf8_lossy(&content.joined).into_owned();
    assert!(text.contains("/Fx0 Do"), "APP-07 wrapper: {text}");
    let fx0 = c
        .doc()
        .get_dictionary(page)
        .and_then(|d| d.get(b"Resources"))
        .and_then(Object::as_dict)
        .and_then(|r| r.get(b"XObject"))
        .and_then(Object::as_dict)
        .and_then(|x| x.get(b"Fx0"))
        .and_then(Object::as_reference)
        .expect("/Fx0");
    let s = c
        .doc()
        .get_object(fx0)
        .and_then(Object::as_stream)
        .expect("Fx0");
    let data =
        decode_stream(s, 1 << 20, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("decode");
    assert_eq!(
        data,
        qpdf_join(&parts),
        "APP-07 Form data = qpdf_join(parts) on the installed qpdf"
    );
}

#[test]
fn app_08_indirect_extensions_pass_a2() {
    let Some(h) = Honest::new(
        "app_08",
        fx::extensions_indirect(),
        0,
        &[ed("Hello", "Help")],
    ) else {
        return;
    };
    let doc = fakes::load(&h.staged);
    let cat = doc
        .get_dictionary(fakes::catalog_id(&doc))
        .expect("catalog");
    assert!(
        matches!(cat.get(b"Extensions"), Ok(Object::Dictionary(_))),
        "APP-08 qpdf made /Extensions direct (and A2 passed in Honest::new)"
    );
}

#[test]
fn app_09_graph_digest_and_matches() {
    let Some(engines) = engines_or_skip("app_09") else {
        return;
    };
    let dir = Scratch::new("app_09");
    // A dangling reference in an array and an indirect /Info.
    let mut b = PdfBuilder::new();
    let (cat, pages) = (b.alloc(), b.alloc());
    let font = b.add(HELVETICA);
    let content = b.add_stream("", b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let page = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Contents {content} 0 R \
         /Resources << /Font << /F1 {font} 0 R >> >> >>"
    ));
    b.set(
        pages,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    b.set(
        cat,
        format!("<< /Type /Catalog /Pages {pages} 0 R /Dangling [1 999 0 R 2] >>"),
    );
    let info = b.add("<< /Title (Graph) >>");
    let pdf = b.build(&format!("/Root {cat} 0 R /Info {info} 0 R"));
    let input = dir.write("in.pdf", &pdf);
    let out = dir.path("renumbered.pdf");
    let r = std::process::Command::new(&engines.qpdf)
        .arg(&input)
        .arg(&out)
        .output()
        .expect("qpdf");
    assert!(
        r.status.success() || r.status.code() == Some(3),
        "qpdf rewrite"
    );
    let c = ctx(pdf);
    let digest = graph_digest(c.doc(), &HashMap::new(), None).expect("digest");
    let after = open(&out);
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    assert_eq!(
        graph_matches(after.doc(), &digest, &mut budget, None),
        Ok(()),
        "APP-09 renumbered + dangling"
    );
    // A changed page dictionary: the first mismatch with its canonical path.
    let changed = dir.path("changed.pdf");
    let doc = fakes::load(&out);
    let page = fakes::page_ids(&doc)[0];
    let mut dict = fakes::json_dict(doc.get_dictionary(page).expect("page"));
    dict.as_object_mut()
        .expect("dict")
        .insert("/Rotate".into(), serde_json::json!(90));
    fakes::set_value(&engines.qpdf, &out, &changed, page, dict).unwrap_or_else(|e| panic!("{e}"));
    let m = graph_matches(open(&changed).doc(), &digest, &mut budget, None);
    assert_eq!(
        m,
        Err(GraphMismatch {
            index: m.as_ref().err().map_or(0, |x| x.index),
            path: "/Root/Pages/Kids[0]".into(),
            what: "node"
        }),
        "APP-09 path"
    );
}

#[test]
fn app_09_whole_graph_holds_on_object_streams_inherited_resources_and_tags() {
    // qpdf keeps object streams, inherited page attributes and the structure tree as they were:
    // A2 passes honest edits on such files (each `Honest::new` runs Phase A).
    let cases = [
        ("hybrid", fx::hybrid_xref(true), "Hello", "Help"),
        (
            "inherited",
            fx::shared_inherited_resources(),
            "Regular words",
            "Regular word",
        ),
        (
            "tagged",
            fx::tagged_bookmarked_page(),
            "Tagged line",
            "Tagged lines",
        ),
    ];
    for (name, pdf, old, new) in cases {
        let test = format!("app_09_{name}");
        let Some(h) = Honest::new(&test, pdf, 0, &[ed(old, new)]) else {
            return;
        };
        assert_eq!(h.report.proofs.len(), 1, "APP-09 {name}");
    }
}
