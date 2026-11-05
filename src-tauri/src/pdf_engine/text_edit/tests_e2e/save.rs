//! E2E-07…09, 17, 18, 22: multi-page and multi-file saves, split content, no-op changes, the
//! old text gone from the file, and a Word-style hybrid-xref tagged source.

use super::{page_parts, E2e};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::testkit::pdf::XrefStyle;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, tagged_bookmarked, DocBuilder, PageSpec, HELVETICA,
};
use lopdf::{Document, Object};
use serde_json::json;
use std::path::Path;

/// A page with one reused image painted twice and a text line between them.
pub(crate) fn image_doc() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let img = d.b.add_stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 \
         /ColorSpace /DeviceRGB /BitsPerComponent 8",
        &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16],
    );
    d.page(PageSpec::new(
        b"q 40 0 0 40 72 400 cm /Im0 Do Q \
          q 40 0 0 40 300 400 cm /Im0 Do Q \
          BT /F1 12 Tf 72 700 Td (Image page) Tj ET",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Im0 {img} 0 R >>"),
    ));
    d.build()
}

/// Every decoded image XObject stream of `pdf`, sorted (the source pixels must survive a save).
fn image_streams(pdf: &Path) -> Vec<Vec<u8>> {
    let doc = Document::load(pdf).expect("load");
    let mut out: Vec<Vec<u8>> = doc
        .objects
        .values()
        .filter_map(|o| match o {
            Object::Stream(s)
                if s.dict.get(b"Subtype").and_then(Object::as_name).ok()
                    == Some(b"Image".as_slice()) =>
            {
                let mut budget = DecodeBudget::new(1 << 30);
                Some(decode_stream(s, 1 << 30, &mut budget).unwrap_or_else(|_| s.content.clone()))
            }
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// One Helvetica line per page.
pub(crate) fn pages_doc(lines: &[&str]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    for line in lines {
        let content = format!("BT /F1 12 Tf 72 700 Td ({line}) Tj ET");
        d.page(PageSpec::new(
            content.as_bytes(),
            &format!("/Font << /F1 {f} 0 R >>"),
        ));
    }
    d.build()
}

#[test]
fn e2e_07_edits_on_pages_one_and_three_in_one_save() {
    let Some(t) = E2e::new("e2e_07") else {
        return;
    };
    let src = t.file(
        "three.pdf",
        &pages_doc(&["First page", "Second page", "Third page"]),
    );
    let objects = vec![
        t.edit(&src, 0, 0, "First page", "First sheet"),
        t.edit(&src, 2, 2, "Third page", "Third sheet"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-07 save");
    t.assert_saved(
        &saved.path,
        &[
            (&src, 0, 0, "First page", "First sheet"),
            (&src, 2, 2, "Third page", "Third sheet"),
        ],
    );
    t.assert_unedited(&src, 1, &saved.path, 1);
}

#[test]
fn e2e_08_two_files_partial_ranges_and_one_file_in_two_groups() {
    let Some(t) = E2e::new("e2e_08") else {
        return;
    };
    let a = t.file(
        "a.pdf",
        &pages_doc(&["Alpha one", "Alpha two", "Alpha three"]),
    );
    let b = t.file("b.pdf", &pages_doc(&["Beta one", "Beta two"]));
    // Two edited files, the second through a partial range: dest = A1 A2 A3 B2.
    let objects = vec![
        t.edit(&a, 1, 1, "Alpha two", "Alpha 2"),
        t.edit(&b, 3, 1, "Beta two", "Beta 2"),
    ];
    let saved = t
        .save(&[(&a, "1-z"), (&b, "2")], objects)
        .expect("E2E-08 two files");
    t.assert_saved(
        &saved.path,
        &[
            (&a, 1, 1, "Alpha two", "Alpha 2"),
            (&b, 1, 3, "Beta two", "Beta 2"),
        ],
    );
    t.assert_unedited(&a, 0, &saved.path, 0);
    t.assert_unedited(&a, 2, &saved.path, 2);

    // One file in two groups with distinct edited pages: dest = A1 B1 A3.
    let objects = vec![
        t.edit(&a, 0, 0, "Alpha one", "Alpha 1"),
        t.edit(&a, 2, 2, "Alpha three", "Alpha 3"),
    ];
    let saved = t
        .save(&[(&a, "1"), (&b, "1"), (&a, "3")], objects)
        .expect("E2E-08 same file twice");
    t.assert_saved(
        &saved.path,
        &[
            (&a, 0, 0, "Alpha one", "Alpha 1"),
            (&a, 2, 2, "Alpha three", "Alpha 3"),
        ],
    );
    t.assert_unedited(&b, 0, &saved.path, 1);

    // An edited page listed twice (two groups, or a repeated range in one group).
    for groups in [
        vec![(a.as_path(), "1-z"), (a.as_path(), "1")],
        vec![(a.as_path(), "1,2,3,1")],
    ] {
        let obj = t.edit(&a, 0, 0, "Alpha one", "Alpha 1");
        let e = t.save(&groups, vec![obj]).err().expect("duplicate page");
        assert_eq!(e.code, "TEXT_EDIT_DUPLICATE_PAGE", "{e}");
        assert_eq!(
            e.message,
            "Page 1 of \u{201c}a.pdf\u{201d} is in the list more than once and has a text change."
        );
    }
}

#[test]
fn e2e_09_split_contents_and_quote_operators() {
    let Some(t) = E2e::new("e2e_09") else {
        return;
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 720 Td (Part zero) Tj ET",
            b"BT /F1 12 Tf 72 700 Td (Part one) Tj ET",
            b"BT /F1 12 Tf 72 680 Td (Part two) Tj ET",
        ],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let src = t.file("parts.pdf", &d.build());
    let objects = vec![
        t.edit(&src, 0, 0, "Part zero", "Part 0"),
        t.edit(&src, 0, 0, "Part two", "Part 2"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-09 parts");
    t.assert_saved(
        &saved.path,
        &[
            (&src, 0, 0, "Part zero", "Part 0"),
            (&src, 0, 0, "Part two", "Part 2"),
        ],
    );
    let parts = page_parts(&saved.path, 0);
    assert_eq!(parts.len(), 3, "E2E-09 part count");
    assert_eq!(
        parts[1],
        b"BT /F1 12 Tf 72 700 Td (Part one) Tj ET".to_vec()
    );

    let src = t.file("quotes.pdf", &fx::quote_ops());
    let objects = vec![
        t.edit(&src, 0, 0, "Line two", "Line 2"),
        t.edit(&src, 0, 0, "Line three", "Line 3"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-09 quotes");
    t.assert_saved(
        &saved.path,
        &[
            (&src, 0, 0, "Line two", "Line 2"),
            (&src, 0, 0, "Line three", "Line 3"),
        ],
    );
}

#[test]
fn e2e_17_no_op_changes_write_nothing() {
    let Some(t) = E2e::new("e2e_17") else {
        return;
    };
    let src = t.file("noop.pdf", &pages_doc(&["Same text", "Other page"]));
    let (_, run) = t.run(&src, 0, "Same text");
    let size = run
        .metrics
        .as_ref()
        .map(|m| m.effective_size)
        .expect("metrics");
    // Text equal to the original, and a size within STYLE_EPSILON of the current one (B7).
    let objects = vec![
        t.edit(&src, 0, 0, "Same text", "Same text"),
        t.edit_styled(
            &src,
            0,
            0,
            "Same text",
            "Same text",
            json!({ "sizePt": size + 0.0004 }),
        ),
    ];
    for obj in objects {
        let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("E2E-17 save");
        t.assert_checks_clean(&saved.path);
        for page in 0..2 {
            assert_eq!(
                page_parts(&saved.path, page),
                page_parts(&src, page),
                "E2E-17 page {page}"
            );
        }
    }
}

/// Every stream of `pdf`, decoded where our bounded decoder can.
fn decoded_streams(pdf: &Path) -> Vec<Vec<u8>> {
    let doc = Document::load(pdf).expect("load");
    doc.objects
        .values()
        .filter_map(|o| match o {
            Object::Stream(s) => {
                let mut budget = DecodeBudget::new(1 << 30);
                Some(decode_stream(s, 1 << 30, &mut budget).unwrap_or_else(|_| s.content.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn e2e_18_the_old_text_is_not_recoverable_from_the_file() {
    let Some(t) = E2e::new("e2e_18") else {
        return;
    };
    let src = t.file(
        "secret.pdf",
        &pages_doc(&["Secret 4711 code", "Other page"]),
    );
    let obj = t.edit(&src, 0, 0, "Secret 4711 code", "Public code");
    let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("E2E-18 save");
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Secret 4711 code", "Public code")],
    );
    let needle = b"(Secret 4711 code) Tj";
    for data in decoded_streams(&saved.path) {
        assert!(
            !data.windows(needle.len()).any(|w| w == needle),
            "E2E-18: the original operation is still in a stream"
        );
    }
    let raw = std::fs::read(&saved.path).expect("read");
    assert!(
        !raw.windows(4).any(|w| w == b"4711"),
        "E2E-18: old digits in the file"
    );
}

#[test]
fn e2e_22_word_style_hybrid_xref_tagged_bookmarked_source() {
    let Some(t) = E2e::new("e2e_22") else {
        return;
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.reserve();
    let extra = tagged_bookmarked(&mut d, page, &[0]);
    let content = b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Tagged hybrid line) Tj ET EMC";
    d.page_at(
        page,
        PageSpec::new(content, &format!("/Font << /F1 {f} 0 R >>")).with(&extra),
    );
    let pdf = d.build_with(&XrefStyle::Hybrid {
        in_objstm: vec![page],
        objstm_in_classic: true,
    });
    let src = t.file("hybrid.pdf", &pdf);
    let obj = t.edit(&src, 0, 0, "Tagged hybrid line", "Tagged hybrid lines");
    let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("E2E-22 save");
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Tagged hybrid line", "Tagged hybrid lines")],
    );
    let doc = Document::load(&saved.path).expect("load");
    let catalog = doc.catalog().expect("catalog");
    assert!(catalog.has(b"StructTreeRoot"), "E2E-22 structure tree kept");
    assert!(catalog.has(b"Outlines"), "E2E-22 bookmarks kept");
}

#[test]
fn e2e_overlap_warning_reaches_the_job_message() {
    let Some(t) = E2e::new("e2e_overlap") else {
        return;
    };
    let pdf = crate::pdf_engine::text_edit::testkit::producers::helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Name) Tj ET BT /F1 12 Tf 110 700 Td (Value) Tj ET",
    );
    let src = t.file("overlap.pdf", &pdf);
    let obj = t.edit(&src, 0, 0, "Name", "Names and");
    let saved = t
        .save(&[(&src, "1-z")], vec![obj])
        .expect("overlap is a warning");
    t.assert_checks_clean(&saved.path);
    // The new line intentionally overlaps "Value". Poppler can interleave that neighbour
    // when extracting words ("NamesValueand"), so assert the saved PDF's actual text runs
    // rather than requiring the edited words to be contiguous in extraction order.
    let (_, edited_page) = t.page(&saved.path, 0);
    assert!(edited_page.runs.iter().any(|r| r.text == "Names and"));
    assert!(edited_page.runs.iter().any(|r| r.text == "Value"));
    assert!(!edited_page.runs.iter().any(|r| r.text == "Name"));
    assert_eq!(
        saved.warnings,
        vec!["Page 1: the changed line \u{201c}Names and\u{201d} runs into the text that follows it.".to_string()]
    );
}

/// review-T5 live B2: a change that runs into the next table cells (the live check's invoice row)
/// commits with the overlap warning, in the preview and on Save; the cells still read the same
/// at their places in the published file.
#[test]
fn b2_a_line_running_into_the_next_cells_saves_with_the_overlap_warning() {
    let Some(t) = E2e::new("b2_table_overlap") else {
        return;
    };
    let pdf = fx::helvetica_page(
        b"BT /F1 11 Tf 58 700 Td (Desk lamp) Tj ET BT /F1 11 Tf 132 700 Td (4) Tj ET \
          BT /F1 11 Tf 156 700 Td (120.00) Tj ET",
    );
    let src = t.file("invoice-row.pdf", &pdf);
    let new = "Desk lamp with a long cable";
    let p = t.preview(
        &src,
        0,
        &[(
            "Desk lamp",
            new,
            crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn::default(),
        )],
    );
    assert!(p.page_pdf.is_some(), "B2 preview: {:?}", p.page_problem);
    assert_eq!(p.verdicts.len(), 1);
    assert!(p.verdicts[0].ok, "B2 verdict");
    let saved = t
        .save(&[(&src, "1-z")], vec![t.edit(&src, 0, 0, "Desk lamp", new)])
        .unwrap_or_else(|e| panic!("B2 save: {e} {:?}", e.details));
    t.assert_checks_clean(&saved.path);
    assert_eq!(
        saved.warnings,
        vec![format!(
            "Page 1: the changed line \u{201c}{new}\u{201d} runs into the text that follows it."
        )]
    );
    let words = |pdf: &Path| {
        crate::pdf_engine::text_edit::poppler::pdftotext_words(
            &t.engines,
            pdf,
            1,
            &crate::pdf_engine::text_edit::engines::RunOpts::default(),
        )
        .expect("words")
    };
    let after = words(&saved.path);
    for cell in words(&src)
        .iter()
        .filter(|w| w.text == "4" || w.text == "120.00")
    {
        assert!(
            after.iter().any(|w| w.text == cell.text
                && (w.x0 - cell.x0).abs() < 0.05
                && (w.y0 - cell.y0).abs() < 0.05),
            "B2 cell {:?} no longer at its place: {after:?}",
            cell.text
        );
    }
    for word in new.split(' ') {
        assert!(
            after.iter().any(|w| w.text == word),
            "B2 {word:?}: {after:?}"
        );
    }
}

/// Issue #129: Save writes a new PDF whose selected occurrence sits at the requested place, the
/// other occurrence of the same image stays put, the image bytes are unchanged and the text is
/// still extractable.
#[test]
fn e2e_23_a_moved_image_reopens_at_its_new_place() {
    let Some(t) = E2e::new("e2e_23") else {
        return;
    };
    let src = t.file("image.pdf", &image_doc());
    let (fp, dto) = t.page(&src, 0);
    assert_eq!(dto.images.len(), 2, "both occurrences are offered");
    let first = dto
        .images
        .iter()
        .find(|i| i.rect.x < 200.0)
        .expect("the left occurrence");
    assert!(first.movable, "the reused XObject is movable: {:?}", first.reason);
    let (dx, dy) = (100.0, 50.0);
    let moved = json!({
        "id": "si-1",
        "kind": "sourceImage",
        "pageIndex": 0,
        "rect": {
            "x": first.rect.x + dx, "y": first.rect.y + dy,
            "w": first.rect.w, "h": first.rect.h
        },
        "locked": true,
        "locator": first.locator,
        "sourceFingerprint": fp,
        "sourcePageIndex": 0,
        "move": { "x": dx, "y": dy }
    });
    let saved = t
        .save(&[(&src, "1-z")], vec![moved])
        .unwrap_or_else(|e| panic!("E2E-23 save: {e} {:?}", e.details));
    t.assert_checks_clean(&saved.path);
    assert_eq!(
        t.words(&src, 0),
        t.words(&saved.path, 0),
        "the surrounding text is unchanged"
    );
    let (_, after) = t.page(&saved.path, 0);
    assert_eq!(after.images.len(), 2, "the image resource is still reused");
    let near = |r: &crate::pdf_engine::text_edit::dto::RectDto, x: f64, y: f64| {
        (r.x - x).abs() < 0.1 && (r.y - y).abs() < 0.1
    };
    assert!(
        after.images.iter().any(|i| near(&i.rect, 72.0 + dx, 400.0 + dy)),
        "the moved occurrence is at its new place: {:?}",
        after.images.iter().map(|i| i.rect).collect::<Vec<_>>()
    );
    assert!(
        after.images.iter().any(|i| near(&i.rect, 300.0, 400.0)),
        "the other occurrence did not move: {:?}",
        after.images.iter().map(|i| i.rect).collect::<Vec<_>>()
    );
    assert!(
        after.images.iter().all(|i| !near(&i.rect, 72.0, 400.0)),
        "no image was left at the old place"
    );
    assert_eq!(image_streams(&src), image_streams(&saved.path), "image bytes changed");
    let content = String::from_utf8_lossy(&page_parts(&saved.path, 0).concat()).into_owned();
    assert_eq!(content.matches("/Im0 Do").count(), 2, "{content}");
    assert!(
        content.contains("1 0 0 1 2.5 1.25 cm"),
        "the page-space delta is written in CTM units: {content}"
    );
}

/// The `/Link` annotations of page `page` (0-based) of `pdf`.
fn link_count(pdf: &Path, page: u32) -> usize {
    let snap = crate::pdf_engine::text_edit::snapshot::read_verification_snapshot(pdf, 1 << 30)
        .unwrap_or_else(|e| panic!("{e}"));
    let doc = &snap.doc;
    let id = *snap.pages.get(page as usize).expect("page");
    let resolve = |o: &Object| match o {
        Object::Reference(r) => doc.get_object(*r).ok().cloned(),
        other => Some(other.clone()),
    };
    let annots = doc
        .get_dictionary(id)
        .ok()
        .and_then(|d| d.get(b"Annots").ok())
        .and_then(resolve);
    let Some(Object::Array(items)) = annots else {
        return 0;
    };
    items
        .iter()
        .filter_map(resolve)
        .filter(|a| {
            a.as_dict()
                .ok()
                .and_then(|d| d.get(b"Subtype").ok())
                .and_then(|s| s.as_name().ok())
                == Some(b"Link".as_slice())
        })
        .count()
}

/// review-T5 L5: the editor lists a file's links as link objects; a Save made only of text
/// changes therefore means the user deleted every link, and the links must not come back (the
/// L7 rule of an empty edit).
#[test]
fn l5_a_text_only_save_keeps_no_link_the_user_deleted() {
    let Some(t) = E2e::new("l5_links") else {
        return;
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let link = d.add(
        "<< /Type /Annot /Subtype /Link /Rect [72 600 200 620] /Border [0 0 0] \
         /A << /S /URI /URI (https://example.com/) >> >>",
    );
    d.page(
        PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td (Linked page) Tj ET",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .with(&format!("/Annots [{link} 0 R]")),
    );
    let src = t.file("links.pdf", &d.build());
    assert_eq!(link_count(&src, 0), 1, "the source has a link");
    let edit = t.edit(&src, 0, 0, "Linked page", "Linked pages");
    let saved = t.save(&[(&src, "1-z")], vec![edit]).expect("L5 save");
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Linked page", "Linked pages")]);
    assert_eq!(link_count(&saved.path, 0), 0, "the deleted link came back");
}
