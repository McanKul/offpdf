//! Review regressions (T3 fix pass): model fidelity and the Phase B wrapper — a direct ExtGState
//! font (MEDIUM-2), text drawn after an advance the model cannot know (MEDIUM-3), a TJ opening
//! with a column-wide kern (LOW-1), the wrapper Form's dictionary and size (LOW-2, LOW-6),
//! pattern identity (LOW-3), unused fonts under their own budget (LOW-5), and a cancellable
//! classifier walk.

use super::{content, ctx, model0, reason_of, texts, walk};
use crate::pdf_engine::source_content::classify_source_page;
use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::{run_tool, RunOpts};
use crate::pdf_engine::text_edit::lexer::{lex_content, LexLimits, Operand, Operator};
use crate::pdf_engine::text_edit::reasons::TextReason as R;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::read_snapshot;
use crate::pdf_engine::text_edit::state::same_paint;
use crate::pdf_engine::text_edit::testkit::pdf::zlib_zero_bomb;
use crate::pdf_engine::text_edit::testkit::producers::{
    cid_font, cid_hex, helvetica_doc, helvetica_page, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::walker::{walk_page, PageWalk, WalkMode};
use std::ffi::OsString;
use std::io::Write;
use std::sync::atomic::AtomicBool;

#[test]
fn a_direct_extgstate_font_is_malformed() {
    let gs_font = |base: &str| {
        format!(
            "<< /Font [<< /Type /Font /Subtype /Type1 /BaseFont /{base} \
             /Encoding /WinAnsiEncoding >> 12] >>"
        )
    };
    let mut d = DocBuilder::new();
    d.page(PageSpec::new(
        b"/GA gs BT 72 700 Td (iiii) Tj ET /GB gs BT 72 600 Td (iiii) Tj ET",
        &format!(
            "/ExtGState << /GA {} /GB {} >>",
            gs_font("Helvetica"),
            gs_font("Courier")
        ),
    ));
    let m = model0(d.build());
    assert_eq!(m.page_reason, Some(R::MalformedContent));
    assert!(m
        .page_detail
        .as_deref()
        .is_some_and(|d| d.contains("indirect")));
    // Indirect fonts keep their own models: Courier is wider than Helvetica.
    let mut d = DocBuilder::new();
    let h = d.add(HELVETICA);
    let c =
        d.add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>");
    d.page(PageSpec::new(
        b"/GA gs BT 72 700 Td (iiii) Tj ET /GB gs BT 72 600 Td (iiii) Tj ET",
        &format!("/ExtGState << /GA << /Font [{h} 0 R 12] >> /GB << /Font [{c} 0 R 12] >> >>"),
    ));
    let m = model0(d.build());
    let widths: Vec<f64> = m.runs.iter().map(|r| r.original_extent).collect();
    assert_eq!(widths.len(), 2);
    assert!(widths[1] > widths[0] * 2.0, "{widths:?}");
}

/// `/F2` a 2-byte font drawing "A", `/F1` Helvetica, in one text object.
fn odd_then(between: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f2 = cid_font(&mut d.b, "ABCDEF+Arimo", "AB");
    let f1 = d.add(HELVETICA);
    let content = format!(
        "BT /F2 10 Tf 72 700 Td <{}00> Tj {between} /F1 10 Tf (Hello) Tj ET",
        cid_hex("AB", "A")
    );
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R /F2 {f2} 0 R >>"),
    ));
    d.build()
}

#[test]
fn text_after_an_unknown_advance_is_refused_until_the_line_moves() {
    // An odd-length string in a 2-byte font: viewers advance by CID 0, the model by nothing.
    let m = model0(odd_then(""));
    assert_eq!(reason_of(&m, "A"), Some(R::AmbiguousUnicode));
    assert_eq!(reason_of(&m, "Hello"), Some(R::MissingWidths));
    let m = model0(odd_then("0 -20 Td"));
    assert_eq!(
        reason_of(&m, "Hello"),
        None,
        "a line move makes the pen known again"
    );
    // A missing font advances by an unknown amount too.
    let missing = |between: &str| {
        helvetica_page(
            format!("BT /F9 10 Tf 72 700 Td (abc) Tj {between} /F1 10 Tf (Hello) Tj ET").as_bytes(),
        )
    };
    assert_eq!(
        reason_of(&model0(missing("")), "Hello"),
        Some(R::MissingWidths)
    );
    assert_eq!(
        reason_of(&model0(missing("ET BT 72 650 Td")), "Hello"),
        None
    );
    // A code with a glyph name but no width (outside /FirstChar…/LastChar, no /MissingWidth):
    // poppler uses Helvetica's own metrics for it, the model has none.
    let partial = |shown: &str| {
        let mut d = DocBuilder::new();
        let f = d.add(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding \
             /FirstChar 101 /LastChar 111 \
             /Widths [556 278 556 556 222 222 500 222 833 556 556] >>",
        );
        d.page(PageSpec::new(
            format!("BT /F1 12 Tf 72 700 Td {shown} ET").as_bytes(),
            &format!("/Font << /F1 {f} 0 R >>"),
        ));
        d.build()
    };
    let m = model0(partial("(Hel) Tj (lo) Tj"));
    assert!(!m.runs.is_empty());
    assert!(
        m.runs.iter().all(|r| r.reason == Some(R::MissingWidths)),
        "{:?}",
        texts(&m)
    );
    let m = model0(partial("(el) Tj (lo) Tj"));
    assert_eq!(texts(&m), ["ello"]);
    assert_eq!(reason_of(&m, "ello"), None);
}

#[test]
fn a_tj_opening_with_a_column_kern_starts_a_new_run() {
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj [-20000 (Right)] TJ ET",
    ));
    assert_eq!(texts(&m), ["Left", "Right"]);
    let m = model0(helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Left) Tj [-100 (Right)] TJ ET",
    ));
    assert_eq!(texts(&m), ["LeftRight"], "a small opening kern still joins");
}

/// A page painted only through `/Fx0` (qpdf's overlay wrapper shape) holding `data`.
fn wrapper_page(data: &[u8], filter: &str, form_extra: &str, page_extra: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let form = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] \
             /Resources << /Font << /F1 {f} 0 R >> >> {filter} {form_extra}"
        ),
        data,
    );
    d.page(
        PageSpec::new(
            b"q 1 0 0 1 0 0 cm /Fx0 Do Q",
            &format!("/XObject << /Fx0 {form} 0 R >>"),
        )
        .with(page_extra),
    );
    d.build()
}

fn wrapped(pdf: Vec<u8>) -> PageWalk {
    let c = ctx(pdf);
    let pc = content(&c, 0);
    walk_page(
        &c,
        0,
        &pc,
        WalkMode::Wrapped {
            name: b"Fx0".to_vec(),
        },
        None,
    )
}

#[test]
fn the_wrapper_form_dictionary_holds_only_qpdf_keys() {
    let hi = b"BT /F1 12 Tf 72 700 Td (Hi) Tj ET";
    let group = "/Group << /S /Transparency /CS /DeviceRGB >>";
    let ok = wrapped(wrapper_page(hi, "", "", ""));
    assert_eq!(ok.page_reason, None, "{:?}", ok.page_detail);
    assert_eq!(ok.records.len(), 1);
    let same_group = wrapped(wrapper_page(hi, "", group, group));
    assert_eq!(same_group.page_reason, None, "{:?}", same_group.page_detail);
    for (form_extra, page_extra, what) in [
        ("/OC << /Type /OCG /Name (L) >>", "", "/OC"),
        ("/Ref << /F (x.pdf) /Page 0 >>", "", "/Ref"),
        (group, "", "a /Group the page lacks"),
        (
            group,
            "/Group << /S /Transparency /CS /DeviceGray >>",
            "a different /Group",
        ),
    ] {
        let w = wrapped(wrapper_page(hi, "", form_extra, page_extra));
        assert_eq!(w.page_reason, Some(R::MalformedContent), "{what}");
        assert!(
            w.page_detail
                .as_deref()
                .is_some_and(|d| d.starts_with("wrapper")),
            "{what}: {:?}",
            w.page_detail
        );
    }
}

#[test]
fn qpdf_copies_the_page_group_onto_its_wrapper() {
    let Some(engines) = engines_or_skip("qpdf_copies_the_page_group_onto_its_wrapper") else {
        return;
    };
    let s = Scratch::new("wrapgroup");
    let group = "/Group << /S /Transparency /CS /DeviceRGB /I true >>";
    let src = s.write(
        "src.pdf",
        &helvetica_doc(b"BT /F1 12 Tf 72 700 Td (Grouped) Tj ET", "", group),
    );
    let blank = s.write("blank.pdf", &helvetica_doc(b"", "", ""));
    let out = s.path("out.pdf");
    let args: Vec<OsString> = vec![
        src.into(),
        "--overlay".into(),
        blank.into(),
        "--".into(),
        out.clone().into(),
    ];
    let r = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).expect("qpdf --overlay");
    assert!(r.code == 0 || r.code == 3, "qpdf: {}", r.stderr);
    let c = SnapshotContext::new(read_snapshot(&out).expect("overlay output opens"));
    let pc = content(&c, 0);
    let ops = lex_content(&pc.joined, &LexLimits::page(), None).expect("wrapper lexes");
    let name = ops
        .iter()
        .find(|o| o.operator == Operator::Do)
        .and_then(|o| o.operands.first().and_then(Operand::as_name))
        .expect("the page paints a wrapper Form")
        .to_vec();
    let w = walk_page(&c, 0, &pc, WalkMode::Wrapped { name }, None);
    assert_eq!(w.page_reason, None, "{:?}", w.page_detail);
    assert_eq!(w.records.len(), 1);
}

/// A zlib stream inflating to `prefix` followed by `mib` MiB of NUL bytes (PDF whitespace).
fn zlib_padded(prefix: &[u8], mib: usize) -> Vec<u8> {
    let zeros = vec![0u8; 1 << 20];
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(prefix).expect("write");
    e.write_all(&zeros).expect("write");
    e.flush().expect("flush");
    let first = e.get_ref().len();
    e.write_all(&zeros).expect("write");
    e.flush().expect("flush");
    let second = e.get_ref().len();
    let full = e.finish().expect("finish");
    let chunk = full[first..second].to_vec();
    let mut out = full[..second].to_vec();
    for _ in 2..mib.max(2) {
        out.extend_from_slice(&chunk);
    }
    out.extend_from_slice(&full[second..]);
    out
}

#[test]
fn the_wrapper_form_gets_the_page_content_cap() {
    // 40 MiB of content: over STREAM_MAX_DECODED (32 MiB), within PAGE_CONTENT_MAX_DECODED.
    let data = zlib_padded(b"BT /F1 12 Tf 72 700 Td (Big) Tj ET\n", 40);
    let w = wrapped(wrapper_page(&data, "/Filter /FlateDecode", "", ""));
    assert_eq!(w.page_reason, None, "{:?}", w.page_detail);
    assert_eq!(w.records.len(), 1);
}

/// "Pat" filled with the axial shading pattern `/P0` from red to `c1`.
fn pattern_page(c1: &str) -> Vec<u8> {
    helvetica_doc(
        b"/Pattern cs /P0 scn BT /F1 12 Tf 72 700 Td (Pat) Tj ET",
        &format!(
            "/Pattern << /P0 << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB \
             /Coords [0 0 1 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] \
             /C1 [{c1}] /N 1 >> >> >> >>"
        ),
        "",
    )
}

#[test]
fn a_pattern_swapped_behind_its_name_changes_the_paint() {
    let fill = |pdf: Vec<u8>| {
        let c = ctx(pdf);
        walk(&c, 0, WalkMode::Edit).records[0].before.fill.clone()
    };
    let (a, same, swapped) = (
        fill(pattern_page("0 0 1")),
        fill(pattern_page("0 0 1")),
        fill(pattern_page("0 1 0")),
    );
    assert!(a.pattern && a.pattern_hash.is_some());
    assert!(same_paint(&a, &same));
    assert!(!same_paint(&a, &swapped), "same name, different pattern");
}

#[test]
fn unused_fonts_never_refuse_the_page() {
    // Seven unused TrueType fonts whose programs inflate to 15 MiB each (105 MiB in all, over
    // PAGE_DECODE_BUDGET): the page's own text stays editable, the fonts that fit are siblings.
    let mut d = DocBuilder::new();
    let f1 = d.add(HELVETICA);
    let bomb = zlib_zero_bomb(15);
    let mut fonts = vec![format!("/F1 {f1} 0 R")];
    for k in 2..=8 {
        let file = d.b.add_stream("/Filter /FlateDecode", &bomb);
        let font = d.add(format!(
            "<< /Type /Font /Subtype /TrueType /BaseFont /Big{k} /FirstChar 32 /LastChar 32 \
             /Widths [250] /FontDescriptor << /Type /FontDescriptor /FontName /Big{k} /Flags 32 \
             /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 \
             /StemV 80 /FontFile2 {file} 0 R >> >>"
        ));
        fonts.push(format!("/F{k} {font} 0 R"));
    }
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET",
        &format!("/Font << {} >>", fonts.join(" ")),
    ));
    let m = model0(d.build());
    assert_eq!(m.page_reason, None, "{:?}", m.page_detail);
    assert_eq!(reason_of(&m, "Hello"), None);
    let n = m.walk.page_fonts.len();
    assert!(
        (2..8).contains(&n),
        "F1 plus the unused fonts that fit: {n}"
    );
}

#[test]
fn the_classify_walk_can_be_cancelled() {
    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let model = build_page_model(&c, 0, None).expect("model");
    let live = classify_source_page(&c, &model, None);
    assert!(!live.occurrences.is_empty());
    let cancel = AtomicBool::new(true);
    let stopped = classify_source_page(&c, &model, Some(&cancel));
    assert_eq!(stopped.occurrence_reason, Some(R::PageTooComplex));
    assert!(stopped.occurrences.is_empty());
}
