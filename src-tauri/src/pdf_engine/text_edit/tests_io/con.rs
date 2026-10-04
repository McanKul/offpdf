//! CON-01…12: page content parts, the joined buffer, qpdf's join rule and ownership.

use crate::pdf_engine::text_edit::content::{
    page_content, part_exclusive, qpdf_join, KidsCounts, PageContent, RefCounts,
};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::engines::{run_tool, RunOpts};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::snapshot::{read_snapshot, snapshot_from_bytes};
use crate::pdf_engine::text_edit::testkit::pdf::{Doc, PdfBuilder};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::validate_output::content_digest;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::ffi::OsString;
use std::path::Path;

fn doc(bytes: &[u8]) -> Document {
    snapshot_from_bytes(Path::new("c.pdf"), bytes.to_vec(), None)
        .map(|s| s.doc)
        .unwrap_or_else(|e| panic!("{e}"))
}

fn content(d: &Document, page: u32) -> Result<PageContent, TextReason> {
    page_content(d, (page, 0), &mut DecodeBudget::new(usize::MAX))
}

fn exclusive(d: &Document, page: u32, part: usize) -> bool {
    let c = content(d, page).unwrap();
    part_exclusive(d, &RefCounts::of(d), &KidsCounts::of(d).unwrap(), &c, part)
}

#[test]
fn con01_parts_joined_with_newline_and_located() {
    let d = Doc::new(&[&[b"q 1 0 0 1 0 0 cm", b"Q"]]);
    let pdf = doc(&d.build());
    let c = content(&pdf, d.page_ids[0]).unwrap();
    assert_eq!(c.joined, b"q 1 0 0 1 0 0 cm\nQ", "CON-01");
    assert_eq!((c.parts[1].start, c.parts[1].len), (17, 1));
    assert_eq!(c.part_bytes(1), b"Q");
    assert_eq!(c.part_bytes(9), b"", "unknown part is empty, never a panic");
    assert_eq!(
        c.locate(&(2..16)),
        Some((0, 2..16)),
        "CON-01 locate in part 0"
    );
    assert_eq!(
        c.locate(&(17..18)),
        Some((1, 0..1)),
        "CON-01 locate in part 1"
    );
    assert_eq!(
        c.concat_digest(),
        content_digest(b"q 1 0 0 1 0 0 cmQ"),
        "CON-01 #34 semantics (no separator)"
    );
    assert_eq!(c.parts[0].digest, content_digest(b"q 1 0 0 1 0 0 cm"));
    assert_eq!(c.parts[0].stream_id, (d.content_ids[0][0], 0));
    assert_eq!(c.contents_array, None, "direct array");
}

#[test]
fn con02_span_over_a_separator_is_not_located() {
    let d = Doc::new(&[&[b"BT (a) Tj", b"ET"]]);
    let c = content(&doc(&d.build()), d.page_ids[0]).unwrap();
    assert_eq!(c.locate(&(3..12)), None, "CON-02 straddle");
    assert_eq!(c.locate(&(9..10)), None, "CON-02 the separator itself");
    assert_eq!(c.locate(&(0..100)), None);
}

#[test]
fn con03_contents_shapes() {
    let mut d = Document::with_version("1.7");
    let direct = d.add_object(dictionary! { "Type" => "Page", "Contents" => Object::Stream(Stream::new(dictionary! {}, b"q Q".to_vec())) });
    assert_eq!(
        content(&d, direct.0).unwrap_err(),
        TextReason::MalformedContent,
        "CON-03 direct stream"
    );
    let number = d.add_object(dictionary! { "Type" => "Page", "Contents" => 5 });
    assert_eq!(
        content(&d, number.0).unwrap_err(),
        TextReason::MalformedContent
    );
    let dict_id = d.add_object(dictionary! { "A" => 1 });
    let to_dict = d.add_object(dictionary! { "Type" => "Page", "Contents" => dict_id });
    assert_eq!(
        content(&d, to_dict.0).unwrap_err(),
        TextReason::MalformedContent,
        "reference to a non-stream"
    );
    let missing = d.add_object(dictionary! { "Type" => "Page" });
    assert!(
        content(&d, missing.0).unwrap().parts.is_empty(),
        "missing /Contents = empty page"
    );
    let null = d.add_object(dictionary! { "Type" => "Page", "Contents" => Object::Null });
    assert!(
        content(&d, null.0).unwrap().joined.is_empty(),
        "null /Contents = empty page"
    );
    assert_eq!(
        content(&d, 999).unwrap_err(),
        TextReason::MalformedContent,
        "no such page"
    );
    let s = d.add_object(Stream::new(dictionary! {}, b"q Q".to_vec()));
    let arr = d.add_object(Object::Array(vec![
        Object::Reference(s),
        Object::Reference(s),
    ]));
    let via_array = d.add_object(dictionary! { "Type" => "Page", "Contents" => arr });
    let c = content(&d, via_array.0).unwrap();
    assert_eq!(
        (c.joined.as_slice(), c.contents_array),
        (&b"q Q\nq Q"[..], Some(arr)),
        "reference to an array object"
    );
}

#[test]
fn con04_with_replaced_parts() {
    let d = Doc::new(&[&[b"q", b"BT (a) Tj ET", b"Q"]]);
    let c = content(&doc(&d.build()), d.page_ids[0]).unwrap();
    let r = c.with_replaced_parts(&[(1, b"BT (abc) Tj ET".to_vec()), (7, b"ignored".to_vec())]);
    assert_eq!(r.joined, b"q\nBT (abc) Tj ET\nQ", "CON-04");
    assert_eq!((r.parts[2].start, r.parts[1].len), (17, 14));
    assert_eq!(r.parts[1].digest, content_digest(b"BT (abc) Tj ET"));
    assert_eq!(r.parts[0].digest, c.parts[0].digest);
    assert_eq!(r.locate(&(5..9)), Some((1, 3..7)));
    assert_eq!(r.page_id, c.page_id);
}

/// Two pages; page 1 is `[shared own]`; page 2's `/Contents` is built from (shared, own2).
fn two_pages(page2_contents: impl Fn(u32, u32) -> String) -> (Document, u32, u32) {
    let mut b = PdfBuilder::new();
    let shared = b.add_stream("", b"0 0 m 10 10 l S");
    let own = b.add_stream("", b"BT (own) Tj ET");
    let own2 = b.add_stream("", b"BT (own 2) Tj ET");
    let (cat, pages) = (b.alloc(), b.alloc());
    let p1 = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /Contents [{shared} 0 R {own} 0 R] >>"
    ));
    let p2 = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /Contents {} >>",
        page2_contents(shared, own2)
    ));
    b.set(
        pages,
        format!("<< /Type /Pages /Kids [{p1} 0 R {p2} 0 R] /Count 2 /MediaBox [0 0 612 792] >>"),
    );
    b.set(cat, format!("<< /Type /Catalog /Pages {pages} 0 R >>"));
    (doc(&b.build(&format!("/Root {cat} 0 R"))), p1, p2)
}

#[test]
fn con05_shared_stream_and_shared_array() {
    let (d, p1, p2) = two_pages(|shared, _| format!("{shared} 0 R"));
    assert!(!exclusive(&d, p1, 0), "CON-05 shared part");
    assert!(exclusive(&d, p1, 1), "CON-05 own part");
    assert!(!exclusive(&d, p2, 0));
    let mut b = PdfBuilder::new();
    let s = b.add_stream("", b"q Q");
    let arr = b.add(format!("[{s} 0 R]"));
    let (cat, pages) = (b.alloc(), b.alloc());
    let p1 = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /Contents {arr} 0 R >>"
    ));
    let p2 = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /Contents {arr} 0 R >>"
    ));
    b.set(
        pages,
        format!("<< /Type /Pages /Kids [{p1} 0 R {p2} 0 R] /Count 2 /MediaBox [0 0 612 792] >>"),
    );
    b.set(cat, format!("<< /Type /Catalog /Pages {pages} 0 R >>"));
    let d = doc(&b.build(&format!("/Root {cat} 0 R")));
    assert!(
        !exclusive(&d, p1, 0),
        "CON-05 shared /Contents array object"
    );
    assert_eq!(RefCounts::of(&d).count((arr, 0)), 2);
}

#[test]
fn con06_page_listed_twice_and_letterhead_part() {
    let d = Doc::new(&[&[b"q Q"]]);
    let mut b = d.b.clone();
    let p = d.page_ids[0];
    b.set(
        d.pages,
        format!("<< /Type /Pages /Kids [{p} 0 R {p} 0 R] /Count 2 /MediaBox [0 0 612 792] >>"),
    );
    let pdf = doc(&b.build(&d.trailer()));
    assert_eq!(KidsCounts::of(&pdf).unwrap().count((p, 0)), 2);
    assert!(!exclusive(&pdf, p, 0), "CON-06 page listed twice in /Kids");
    let (d, p1, p2) = two_pages(|shared, own2| format!("[{shared} 0 R {own2} 0 R]"));
    assert!(!exclusive(&d, p1, 0), "CON-06 letterhead part is shared");
    assert!(exclusive(&d, p1, 1), "CON-06 body part is exclusive");
    assert!(!exclusive(&d, p2, 0));
    assert!(exclusive(&d, p2, 1));
}

/// FX-WORD-like tagged/bookmarked page: every legitimate back-reference points at the page.
fn referenced_page() -> (Document, u32) {
    let mut d = Doc::new(&[&[b"/P <</MCID 0>> BDC BT /F1 12 Tf (x) Tj ET EMC"]]);
    let p = d.page_ids[0];
    let (cat, pages) = (d.catalog, d.pages);
    let st_root = d.b.alloc();
    let elems: Vec<u32> = (0..3)
        .map(|i| {
            d.b.add(format!(
                "<< /Type /StructElem /S /P /P {st_root} 0 R /Pg {p} 0 R /K {i} >>"
            ))
        })
        .collect();
    let kids = elems
        .iter()
        .map(|e| format!("{e} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    d.b.set(st_root, format!("<< /Type /StructTreeRoot /K [{kids}] >>"));
    let link = d.b.add(format!(
        "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /P {p} 0 R /Dest [{p} 0 R /Fit] >>"
    ));
    let outlines = d.b.alloc();
    let item = d.b.add(format!(
        "<< /Title (One) /Parent {outlines} 0 R /Dest [{p} 0 R /XYZ 0 792 0] >>"
    ));
    d.b.set(
        outlines,
        format!("<< /Type /Outlines /First {item} 0 R /Last {item} 0 R /Count 1 >>"),
    );
    let body = String::from_utf8(d.b.body(p).unwrap().to_vec()).unwrap();
    d.b.set(
        p,
        body.replace(" >>", &format!(" /Annots [{link} 0 R] /StructParents 0 >>")),
    );
    d.b.set(
        cat,
        format!(
            "<< /Type /Catalog /Pages {pages} 0 R /StructTreeRoot {st_root} 0 R /Outlines {outlines} 0 R \
             /Names << /Dests << /Names [(here) [{p} 0 R /Fit]] >> >> /OpenAction [{p} 0 R /Fit] /MarkInfo << /Marked true >> >>"
        ),
    );
    (doc(&d.build()), p)
}

#[test]
fn con07_tagged_page_is_exclusive() {
    let (d, p) = referenced_page();
    assert!(
        RefCounts::of(&d).count((p, 0)) >= 5,
        "the page is referenced from the struct tree and the annotation"
    );
    assert!(exclusive(&d, p, 0), "CON-07");
}

#[test]
fn con08_destinations_and_open_action_do_not_share() {
    let (d, p) = referenced_page();
    assert_eq!(KidsCounts::of(&d).unwrap().count((p, 0)), 1);
    assert!(
        RefCounts::of(&d).count((p, 0)) >= 9,
        "outline, link, named destination, /OpenAction"
    );
    assert!(exclusive(&d, p, 0), "CON-08");
}

#[test]
fn con09_same_page_under_two_pages_nodes() {
    let mut b = PdfBuilder::new();
    let s = b.add_stream("", b"q Q");
    let (cat, root, n1, n2) = (b.alloc(), b.alloc(), b.alloc(), b.alloc());
    let p = b.add(format!(
        "<< /Type /Page /Parent {n1} 0 R /Contents {s} 0 R >>"
    ));
    b.set(
        root,
        format!("<< /Type /Pages /Kids [{n1} 0 R {n2} 0 R] /Count 2 /MediaBox [0 0 612 792] >>"),
    );
    b.set(
        n1,
        format!("<< /Type /Pages /Parent {root} 0 R /Kids [{p} 0 R] /Count 1 >>"),
    );
    b.set(
        n2,
        format!("<< /Type /Pages /Parent {root} 0 R /Kids [{p} 0 R] /Count 1 >>"),
    );
    b.set(cat, format!("<< /Type /Catalog /Pages {root} 0 R >>"));
    let d = doc(&b.build(&format!("/Root {cat} 0 R")));
    assert_eq!(KidsCounts::of(&d).unwrap().count((p, 0)), 2);
    assert!(!exclusive(&d, p, 0), "CON-09");
    // a node that lists its ancestor is a cycle
    let mut b2 = b.clone();
    b2.set(
        n2,
        format!("<< /Type /Pages /Parent {root} 0 R /Kids [{root} 0 R] /Count 1 >>"),
    );
    let cyclic = lopdf::Document::load_mem(&b2.build(&format!("/Root {cat} 0 R"))).unwrap();
    assert_eq!(
        KidsCounts::of(&cyclic).err(),
        Some(TextReason::MalformedContent),
        "cycle"
    );
}

const JOIN_CASES: &[(&[&[u8]], &[u8])] = &[
    (&[b"0 g", b"1 g"], b"0 g\n1 g"),
    (&[b"0 g\n", b"1 g"], b"0 g\n1 g"),
    (&[b"0 g\r", b"1 g"], b"0 g\r\n1 g"),
    (&[b"0 g", b""], b"0 g\n"),
    (&[b"", b"0 g"], b"\n0 g"),
    (&[b"", b""], b"\n"),
    (&[b"", b"", b""], b"\n"),
    (&[b"", b"", b"0 g"], b"\n0 g"),
    (&[b"", b"", b"", b"A"], b"\n\nA"),
    (&[b"0 g", b"", b"1 g"], b"0 g\n1 g"),
    (&[b"0 g", b"", b"", b"1 g"], b"0 g\n\n1 g"),
    (&[b"0 g", b"", b"", b"", b"1 g"], b"0 g\n\n1 g"),
    (&[b"0 g", b"", b"", b""], b"0 g\n\n"),
    (&[b"0 g", b"", b"1 g\n", b"0.5 g"], b"0 g\n1 g\n0.5 g"),
    (&[b"A", b"", b"B", b"", b"C"], b"A\nB\nC"),
    (&[b"A", b"", b"", b"B", b"", b"", b"C"], b"A\n\nB\n\nC"),
    (&[b"q"], b"q"),
    (&[], b""),
];

#[test]
fn con10_qpdf_join_rule() {
    for (parts, want) in JOIN_CASES {
        assert_eq!(qpdf_join(parts), *want, "CON-10 {parts:?}");
    }
    // Pin the rule on the installed qpdf: its overlay wrapper Form holds the joined parts.
    let Some(engines) = engines_or_skip("con10_qpdf_join_rule") else {
        return;
    };
    let s = Scratch::new("con10");
    let blank = s.write("blank.pdf", &Doc::new(&[&[b""]]).build());
    for (i, (parts, want)) in JOIN_CASES
        .iter()
        .enumerate()
        .filter(|(_, (p, _))| !p.is_empty())
    {
        let src = s.write(&format!("in{i}.pdf"), &Doc::new(&[parts]).build());
        let out = s.path(&format!("out{i}.pdf"));
        let args = [
            src.into_os_string(),
            OsString::from("--overlay"),
            blank.clone().into_os_string(),
            OsString::from("--"),
            out.clone().into_os_string(),
        ];
        let r = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).unwrap();
        assert!(r.code == 0 || r.code == 3, "{}", r.stderr);
        let snap = read_snapshot(&out).unwrap_or_else(|e| panic!("{e}"));
        let page = snap.doc.get_dictionary(snap.pages[0]).unwrap();
        let xobjects = match page.get(b"Resources").unwrap() {
            Object::Reference(id) => snap.doc.get_dictionary(*id).unwrap(),
            Object::Dictionary(d) => d,
            other => panic!("{other:?}"),
        };
        let fx0: ObjectId = xobjects
            .get(b"XObject")
            .and_then(|x| match x {
                Object::Dictionary(d) => d.get(b"Fx0").and_then(Object::as_reference),
                Object::Reference(id) => snap
                    .doc
                    .get_dictionary(*id)
                    .and_then(|d| d.get(b"Fx0"))
                    .and_then(Object::as_reference),
                _ => panic!("xobjects"),
            })
            .unwrap();
        let Object::Stream(form) = snap.doc.objects.get(&fx0).unwrap() else {
            panic!("Fx0 is a stream")
        };
        let data = decode_stream(form, 1 << 20, &mut DecodeBudget::new(usize::MAX)).unwrap();
        assert_eq!(
            data,
            *want,
            "CON-10 installed qpdf joins {parts:?} as {:?}",
            String::from_utf8_lossy(&data)
        );
    }
}

#[test]
fn con11_part_dict_with_extra_key_is_unsupported_filter() {
    let mut d = Doc::new(&[&[b"q Q"]]);
    let id = d.content_ids[0][0];
    d.b.set_stream(id, "/Type /XObject", b"q Q");
    assert_eq!(
        content(&doc(&d.build()), d.page_ids[0]).unwrap_err(),
        TextReason::UnsupportedFilter,
        "CON-11"
    );
    let mut ok = Doc::new(&[&[b"q Q"]]);
    ok.b.set(
        id,
        PdfBuilder::stream_body(
            "/Filter /FlateDecode /DecodeParms << /Predictor 1 >> /DL 3",
            &crate::pdf_engine::text_edit::testkit::pdf::zlib(b"q Q"),
        ),
    );
    assert_eq!(
        content(&doc(&ok.build()), ok.page_ids[0]).unwrap().joined,
        b"q Q",
        "allowed keys"
    );
    let mut lzw = Doc::new(&[&[b"q Q"]]);
    lzw.b.set_stream(
        id,
        "/Filter /LZWDecode",
        b"\x80\x0b\x60\x50\x22\x0c\x0c\x85\x01",
    );
    assert_eq!(
        content(&doc(&lzw.build()), lzw.page_ids[0]).unwrap_err(),
        TextReason::UnsupportedFilter
    );
}

#[test]
fn con12_dangling_contents_element_is_malformed() {
    let mut d = Doc::new(&[&[b"q", b"Q"]]);
    let p = d.page_ids[0];
    let body = String::from_utf8(d.b.body(p).unwrap().to_vec()).unwrap();
    let first = d.content_ids[0][0];
    d.b.set(p, body.replace(&format!("{first} 0 R "), "999 0 R "));
    assert_eq!(
        content(&doc(&d.build()), p).unwrap_err(),
        TextReason::MalformedContent,
        "CON-12"
    );
    let parts: Vec<&[u8]> = vec![b"q"; 257];
    let many = Doc::new(&[&parts]);
    assert_eq!(
        content(&doc(&many.build()), many.page_ids[0]).unwrap_err(),
        TextReason::PageTooComplex,
        "> 256 parts"
    );
    let big = Doc::new(&[&[b"q Q q Q"]]);
    let pdf = doc(&big.build());
    assert_eq!(
        page_content(&pdf, (big.page_ids[0], 0), &mut DecodeBudget::new(3)).unwrap_err(),
        TextReason::PageTooComplex,
        "budget"
    );
}
