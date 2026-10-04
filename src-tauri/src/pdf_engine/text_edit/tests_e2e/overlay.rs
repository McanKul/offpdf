//! E2E-10a…f and E2E-11: text changes saved together with objects that make the pipeline run
//! `qpdf --overlay` (every page becomes qpdf's wrapper Form, D29) and with redaction. Phase B and
//! #34 must pass in every variant; E2E-10c needs #34's `alt_content_digest` (VO-01).

use super::save::pages_doc;
use super::{page_holdings, page_parts, E2e, SaveOpts};
use crate::pdf_engine::edit_forms::FormValue;
use crate::pdf_engine::text_edit::content::qpdf_join;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, DocBuilder, PageSpec, HELVETICA,
};
use serde_json::{json, Value};

fn stamp(page: u32, text: &str) -> Value {
    json!({
        "kind": "text", "pageIndex": page,
        "rect": { "x": 300.0, "y": 120.0, "w": 200.0, "h": 30.0 },
        "content": text, "fontSize": 14.0, "color": "#c71c1c", "align": null, "opacity": 1.0,
    })
}

fn shape(page: u32) -> Value {
    json!({
        "kind": "rect", "pageIndex": page,
        "rect": { "x": 400.0, "y": 300.0, "w": 60.0, "h": 40.0 },
        "fill": "#14733d", "stroke": null, "strokeWidth": null, "opacity": 1.0,
    })
}

#[test]
fn e2e_10a_stamp_on_the_edited_page() {
    let Some(t) = E2e::new("e2e_10a") else {
        return;
    };
    let src = t.file("a.pdf", &fx::word());
    let objects = vec![
        t.edit(&src, 0, 0, "Invoice 2026", "Invoice 2027"),
        stamp(0, "Approved"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-10a save");
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Invoice 2026", "Invoice 2027")]);
    assert!(
        t.words(&saved.path, 0).contains("Approved"),
        "E2E-10a stamp"
    );
}

#[test]
fn e2e_10b_stamp_on_another_page_only() {
    let Some(t) = E2e::new("e2e_10b") else {
        return;
    };
    let src = t.file("b.pdf", &pages_doc(&["Edited line", "Stamped page"]));
    let objects = vec![
        t.edit(&src, 0, 0, "Edited line", "Changed line"),
        stamp(1, "Seen"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-10b save");
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Edited line", "Changed line")]);
    t.assert_unedited(&src, 1, &saved.path, 1);
}

#[test]
fn e2e_10c_split_page_without_trailing_newlines_and_a_stamp() {
    let Some(t) = E2e::new("e2e_10c") else {
        return;
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 720 Td (Split zero) Tj ET",
            b"BT /F1 12 Tf 72 700 Td (Split one) Tj ET",
        ],
        &res,
    ));
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 720 Td (Other zero) Tj ET",
            b"BT /F1 12 Tf 72 700 Td (Other one) Tj ET",
        ],
        &res,
    ));
    let src = t.file("c.pdf", &d.build());
    let objects = vec![
        t.edit(&src, 0, 0, "Split zero", "Split 0"),
        stamp(0, "Stamp"),
    ];
    let saved = t
        .save(&[(&src, "1-z")], objects)
        .expect("E2E-10c save (needs alt_content_digest)");
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Split zero", "Split 0")]);
    // The unedited split page sits in qpdf's wrapper as its parts joined with a "\n" — not
    // lopdf's plain concatenation, which is what #34 expected before the fix.
    let parts = page_parts(&src, 1);
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let joined = qpdf_join(&refs);
    assert_ne!(
        joined,
        parts.concat(),
        "E2E-10c fixture must need the join rule"
    );
    assert!(
        page_holdings(&saved.path, 1).contains(&joined),
        "E2E-10c wrapper data"
    );
}

#[test]
fn e2e_10d_offset_crop_with_trim_inside_and_a_stamp() {
    let Some(t) = E2e::new("e2e_10d") else {
        return;
    };
    let pdf = helvetica_doc(
        b"BT /F1 12 Tf 72 700 Td (Cropped trim line) Tj ET",
        "",
        "/CropBox [36 48 576 744] /TrimBox [72 72 540 720]",
    );
    let src = t.file("d.pdf", &pdf);
    let objects = vec![
        t.edit(&src, 0, 0, "Cropped trim line", "Cropped trim lines"),
        stamp(0, "Boxed"),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-10d save");
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Cropped trim line", "Cropped trim lines")],
    );
}

#[test]
fn e2e_10e_rotated_page_and_a_shape() {
    let Some(t) = E2e::new("e2e_10e") else {
        return;
    };
    let src = t.file("e.pdf", &fx::rotated(90, true));
    let objects = vec![
        t.edit(&src, 0, 0, "Rotated page", "Rotated pages"),
        shape(0),
    ];
    let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-10e save");
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Rotated page", "Rotated pages")],
    );
}

/// A page with an AcroForm text field "name" and a text line.
fn form_doc() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.reserve();
    let widget = d.add(format!(
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /F 4 /Rect [300 500 500 520] \
         /P {page} 0 R /DA (/F1 12 Tf 0 g) >>"
    ));
    d.catalog_extra.push_str(&format!(
        " /AcroForm << /Fields [{widget} 0 R] /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 {f} 0 R >> >> >>"
    ));
    d.page_at(
        page,
        PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td (Form page line) Tj ET",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .with(&format!("/Annots [{widget} 0 R]")),
    );
    d.build()
}

fn highlight(page: u32) -> Value {
    json!({
        "kind": "highlight", "id": "h1", "pageIndex": page,
        "rect": { "x": 70.0, "y": 695.0, "w": 100.0, "h": 16.0 },
        "author": "", "color": "#ffff00", "comment": null, "quads": [],
    })
}

/// The existing pipeline refuses to flatten markup while the file keeps an AcroForm
/// (`FORM_FLATTEN_REQUIRED`, `edit_annots::apply_markup_annots`; flattening the form keeps the
/// catalog entry), so 10f runs as two saves: text + link + flattened form value + markup kept as
/// an annotation, then text + link + flattened markup on a page without a form.
#[test]
fn e2e_10f_text_link_flattened_form_and_flattened_markup() {
    let Some(t) = E2e::new("e2e_10f") else {
        return;
    };
    let link = json!({
        "kind": "link", "pageIndex": 0,
        "rect": { "x": 72.0, "y": 600.0, "w": 120.0, "h": 20.0 },
        "action": { "type": "uri", "uri": "https://example.com/" },
    });
    let src = t.file("f.pdf", &form_doc());
    let objects = vec![
        t.edit(&src, 0, 0, "Form page line", "Form page lines"),
        link.clone(),
        highlight(0),
    ];
    let opts = SaveOpts {
        form_values: vec![FormValue {
            name: "name".into(),
            value: "Ada".into(),
        }],
        flatten_form: true,
        ..SaveOpts::default()
    };
    let saved = t
        .save_with(&[(&src, "1-z")], objects, opts)
        .unwrap_or_else(|e| panic!("E2E-10f form save: {e} {:?}", e.details));
    t.assert_saved(
        &saved.path,
        &[(&src, 0, 0, "Form page line", "Form page lines")],
    );
    assert!(
        t.words(&saved.path, 0).contains("Ada"),
        "E2E-10f flattened field value"
    );

    let src = t.file("f2.pdf", &pages_doc(&["Marked line"]));
    let objects = vec![
        t.edit(&src, 0, 0, "Marked line", "Marked lines"),
        link,
        highlight(0),
    ];
    let opts = SaveOpts {
        flatten_annotations: true,
        ..SaveOpts::default()
    };
    let saved = t
        .save_with(&[(&src, "1-z")], objects, opts)
        .unwrap_or_else(|e| panic!("E2E-10f markup save: {e} {:?}", e.details));
    t.assert_saved(&saved.path, &[(&src, 0, 0, "Marked line", "Marked lines")]);
}

#[test]
fn e2e_11_redaction_on_another_page_and_on_the_same_page() {
    let Some(t) = E2e::new("e2e_11") else {
        return;
    };
    let src = t.file("r.pdf", &pages_doc(&["Keep this line", "Redact this line"]));
    let redact = |page: u32| {
        json!({
            "kind": "redact", "pageIndex": page,
            "rect": { "x": 60.0, "y": 690.0, "w": 300.0, "h": 30.0 },
            "fill": "#000000", "label": null,
        })
    };
    // Redaction on page 2, with and without a stamp (the redaction path's own overlay).
    for with_stamp in [false, true] {
        let mut objects = vec![
            t.edit(&src, 0, 0, "Keep this line", "Kept this line"),
            redact(1),
        ];
        if with_stamp {
            objects.push(stamp(0, "Stamp"));
        }
        let saved = t.save(&[(&src, "1-z")], objects).expect("E2E-11 save");
        t.assert_saved(
            &saved.path,
            &[(&src, 0, 0, "Keep this line", "Kept this line")],
        );
        assert!(
            !t.words(&saved.path, 1).contains("Redact"),
            "E2E-11 redacted text still extractable"
        );
    }
    // Both on page 1: refused before anything is written.
    let objects = vec![
        t.edit(&src, 0, 0, "Keep this line", "Kept this line"),
        redact(0),
    ];
    let e = t.save(&[(&src, "1-z")], objects).err().expect("same page");
    assert_eq!(e.code, "TEXT_EDIT_ON_REDACTED_PAGE", "{e}");
}
