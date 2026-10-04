//! PR #97 review regressions R1–R9 (generated in temp; `fixtures/source-edit/` does not grow).

use super::*;

// --- PR 97 review fold (R1–R5) ---------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

pub(super) fn box_obj(b: [i64; 4]) -> Object {
    Object::Array(b.into_iter().map(Object::Integer).collect())
}

#[test]
fn classify_malformed_inline_tail_returns_error_without_panicking() {
    let scratch = Scratch::new("review-malformed-inline");
    let path = scratch.file("tail.pdf");
    let mut content = b"BI /W 1 /H 1 /BPC 8 /CS /G ID x EI\n(".to_vec();
    content.push(b'\\');
    write_helvetica_page(&path, &content);
    let result = std::panic::catch_unwind(|| classify_source_content(&path));
    assert!(
        result.is_ok(),
        "Malformed content must return an AppError, not panic"
    );
    assert!(result.unwrap().is_err());
}

#[test]
fn classify_missing_font_is_not_supported() {
    let scratch = Scratch::new("review-missing-font");
    let path = scratch.file("missing-font.pdf");
    write_helvetica_page(&path, b"BT /Missing 12 Tf 72 400 Td (Hi) Tj ET");
    let hits = classify(&path, "REVIEW-MISSING-FONT");
    assert_eq!(capability_token(&hits[0]), "unsupported");
    assert_eq!(reason_code(&hits[0]).as_deref(), Some("MISSING_FONT"));
}

#[test]
fn classify_repeated_form_occurrences_have_unique_locators() {
    let scratch = Scratch::new("review-repeated-form");
    let path = scratch.file("repeated-form.pdf");
    let mut doc = Document::load(fixture("image-in-form.pdf")).unwrap();
    let page = *doc.get_pages().values().next().unwrap();
    let ids = doc.get_page_contents(page);
    let bytes = doc.get_page_content(page).unwrap();
    let mut repeated = bytes.clone();
    repeated.extend_from_slice(b"\n1 0 0 1 100 0 cm\n");
    repeated.extend_from_slice(&bytes);
    doc.objects.insert(
        ids[0],
        Object::Stream(Stream::new(Dictionary::new(), repeated)),
    );
    doc.save(&path).unwrap();
    let hits = classify(&path, "REVIEW-REPEATED-FORM");
    assert!(hits.len() >= 2);
    let locators: std::collections::HashSet<_> = hits.iter().map(|h| &h.locator).collect();
    assert_eq!(
        locators.len(),
        hits.len(),
        "Every occurrence needs its own locator"
    );
    assert_ne!(hits[0].rect, hits[1].rect);
    assert_eq!(
        resolve_source_locator(&path, &hits[1].locator).unwrap(),
        hits[1]
    );
    assert_eq!(classify(&path, "REVIEW-REPEATED-FORM"), hits);
}

pub(super) fn helvetica_resources() -> Dictionary {
    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    res
}

pub(super) fn write_helvetica_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path).expect("write generated classifier fixture");
}

fn write_text_with_empty_sig_widget(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n".to_vec(),
    )));
    let widget_id = doc.new_object_id();
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    page.set("Annots", vec![Object::Reference(widget_id)]);
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut widget = Dictionary::new();
    widget.set("Type", "Annot");
    widget.set("Subtype", "Widget");
    widget.set("FT", "Sig");
    widget.set("T", Object::string_literal("Sig1"));
    widget.set("Rect", box_obj([72, 72, 172, 92]));
    widget.set("P", page_id);
    doc.objects.insert(widget_id, Object::Dictionary(widget));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut acro = Dictionary::new();
    acro.set("Fields", vec![Object::Reference(widget_id)]);
    let acro_id = doc.add_object(Object::Dictionary(acro));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("AcroForm", acro_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write empty-sig-widget classifier fixture");
}

fn write_applied_signature(path: &Path) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n".to_vec(),
    )));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(helvetica_resources()));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut sig = Dictionary::new();
    sig.set("Type", "Sig");
    sig.set(
        "ByteRange",
        vec![
            Object::Integer(0),
            Object::Integer(10),
            Object::Integer(20),
            Object::Integer(30),
        ],
    );
    let _sig_id = doc.add_object(Object::Dictionary(sig));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write applied-signature classifier fixture");
}

// --- R1 --------------------------------------------------------------------

#[test]
fn classify_180_degree_text_is_rotated() {
    let scratch = Scratch::new("r1-180");
    let path = scratch.file("text-180.pdf");
    write_helvetica_page(&path, b"BT /F1 12 Tf -1 0 0 -1 200 400 Tm (Hi) Tj ET\n");
    let hits = classify(&path, "R1");
    let occ = first_of_kind(&hits, "text", "R1");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R1: 180° Tm [-1 0 0 -1 200 400] must not be supported"
    );
    assert_unsupported(occ, "text", "ROTATED_TEXT", "R1");
}

// --- R2 --------------------------------------------------------------------

#[test]
fn classify_second_tj_advances_tm() {
    let scratch = Scratch::new("r2-advance");
    let path = scratch.file("two-tj.pdf");
    write_helvetica_page(&path, b"BT /F1 12 Tf 72 720 Td (Hel) Tj (lo) Tj ET\n");
    let hits = classify(&path, "R2");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R2: (Hel) Tj (lo) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    assert!(
        (texts[0].rect.x - 72.0).abs() <= 1.0,
        "R2: first origin x must be ~72; got {}",
        texts[0].rect.x
    );
    assert!(
        texts[1].rect.x > texts[0].rect.x,
        "R2: second rect.x must be > first (must not share origin 72); first x={} second x={}",
        texts[0].rect.x,
        texts[1].rect.x
    );
    assert!(
        (texts[1].rect.x - 72.0).abs() > 1.0,
        "R2: second show must not reuse origin 72; first x={} second x={}",
        texts[0].rect.x,
        texts[1].rect.x
    );
}

// --- R3 --------------------------------------------------------------------

#[test]
fn classify_empty_sig_widget_does_not_refuse_file() {
    let scratch = Scratch::new("r3-empty-sig");
    let path = scratch.file("empty-sig.pdf");
    write_text_with_empty_sig_widget(&path);
    let hits = match classify_source_content(&path) {
        Ok(hits) => hits,
        Err(err) => panic!(
            "R3: empty /FT /Sig widget (Type Annot, no ByteRange) + Helvetica (Hi) Tj must be Ok with a text occurrence, not AppError SIGNED; got {} ({})",
            err.code, err.message
        ),
    };
    let occ = first_of_kind(&hits, "text", "R3");
    assert_ne!(
        reason_code(occ).as_deref(),
        Some("SIGNED"),
        "R3: Helvetica text on a file with an empty Sig widget must not be SIGNED"
    );
}

#[test]
fn classify_applied_signature_is_signed() {
    let scratch = Scratch::new("r3-applied-sig");
    let path = scratch.file("applied-sig.pdf");
    write_applied_signature(&path);
    expect_err_code(
        classify_source_content(&path),
        "SIGNED",
        "R3: /Type /Sig + ByteRange still refuses",
    );
}

// --- R4 --------------------------------------------------------------------

#[test]
fn classify_text_after_inline_image_is_kept() {
    let scratch = Scratch::new("r4-inline-rest");
    let path = scratch.file("inline-then-text.pdf");
    write_helvetica_page(
        &path,
        b"q 24 0 0 12 72 400 cm\n\
BI\n\
/W 2 /H 1 /CS /DeviceRGB /BPC 8 /F /AHx\n\
ID\n\
C8101010C810>\n\
EI\n\
Q\n\
BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n",
    );
    let hits = classify(&path, "R4");
    let _text = first_of_kind(&hits, "text", "R4");
}

// --- R5 --------------------------------------------------------------------

#[test]
fn classify_text_bounds_use_tm_scale() {
    let scratch = Scratch::new("r5-tm-scale");
    let path = scratch.file("tf1-tm12.pdf");
    write_helvetica_page(&path, b"BT /F1 1 Tf 12 0 0 12 72 720 Tm (Hi) Tj ET\n");
    let hits = classify(&path, "R5");
    let occ = first_of_kind(&hits, "text", "R5");
    assert!(
        (occ.rect.h - 12.0).abs() <= 1.0,
        "R5: /F1 1 Tf + 12 0 0 12 Tm must report height ~12, not ~1; got h={}",
        occ.rect.h
    );
    assert!(
        (occ.rect.h - 1.0).abs() > 1.0,
        "R5: rect.h must not stay at Tf size ~1; got h={}",
        occ.rect.h
    );
}

// --- PR 97 review fold r2 (R6–R9) ------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

fn write_type3_and_helvetica_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let proc_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        b"10 0 0 0 10 10 d1\n0 0 10 10 re f\n".to_vec(),
    )));
    let mut char_procs = Dictionary::new();
    char_procs.set("x", proc_id);

    let mut enc = Dictionary::new();
    enc.set("Type", "Encoding");
    enc.set(
        "Differences",
        vec![Object::Integer(120), Object::Name(b"x".to_vec())],
    );

    let mut t3 = Dictionary::new();
    t3.set("Type", "Font");
    t3.set("Subtype", "Type3");
    t3.set("FontBBox", box_obj([0, 0, 10, 10]));
    t3.set(
        "FontMatrix",
        Object::Array(vec![
            Object::Real(1.0),
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(1.0),
            Object::Real(0.0),
            Object::Real(0.0),
        ]),
    );
    t3.set("CharProcs", Object::Dictionary(char_procs));
    t3.set("Encoding", Object::Dictionary(enc));
    t3.set("FirstChar", 120);
    t3.set("LastChar", 120);
    t3.set("Widths", vec![Object::Integer(10)]);

    let mut f1 = Dictionary::new();
    f1.set("Type", "Font");
    f1.set("Subtype", "Type1");
    f1.set("BaseFont", "Helvetica");

    let mut fonts = Dictionary::new();
    fonts.set("T3", Object::Dictionary(t3));
    fonts.set("F1", Object::Dictionary(f1));
    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));

    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(res));
    let page_id = doc.add_object(Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", "Pages");
    pages.set("Kids", vec![page_id.into()]);
    pages.set("Count", 1);
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(Object::Dictionary(catalog));
    doc.trailer.set("Root", catalog_id);
    doc.save(path)
        .expect("write Type3+Helvetica classifier fixture");
}

// --- R6 --------------------------------------------------------------------

#[test]
fn classify_inline_image_uses_ctm_at_bi() {
    let path = fixture("image-inline.pdf");
    let hits = classify(&path, "R6");
    let occ = first_of_kind(&hits, "image", "R6");
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.x must be ~72 (CTM at BI), not the unit square at origin; got x={}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 400.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.y must be ~400 (CTM at BI), not the unit square at origin; got y={}",
        occ.rect.y
    );
    assert!(
        (occ.rect.w - 24.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.w must be ~24 (CTM at BI), not the unit square; got w={}",
        occ.rect.w
    );
    assert!(
        (occ.rect.h - 12.0).abs() <= 1.0,
        "R6: image-inline.pdf image rect.h must be ~12 (CTM at BI), not the unit square; got h={}",
        occ.rect.h
    );
}

// --- R7 --------------------------------------------------------------------

#[test]
fn classify_q_restores_type3_after_helvetica() {
    let scratch = Scratch::new("r7-q-type3");
    let path = scratch.file("q-type3.pdf");
    write_type3_and_helvetica_page(
        &path,
        // Deliberate change (T3): the text is moved into the page (`72 720 Td`). At the page
        // origin its descenders fall below the MediaBox, and CLIPPED (§A.10 #12) now outranks
        // TYPE3 (#18); this test is about q/Q restoring the font.
        b"BT 72 720 Td /T3 12 Tf (x) Tj q /F1 12 Tf (y) Tj Q (z) Tj ET\n",
    );
    let hits = classify(&path, "R7");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        3,
        "R7: (x) Tj q /F1 (y) Tj Q (z) Tj must emit three text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), capability_token(o), reason_code(o)))
            .collect::<Vec<_>>()
    );
    let last = texts[2];
    assert_ne!(
        capability_token(last),
        "supported",
        "R7: last show (z) after Q must not be Helvetica supported; got {} reason={:?}",
        capability_token(last),
        reason_code(last)
    );
    assert_unsupported(last, "text", "TYPE3", "R7");
}

// --- R8 --------------------------------------------------------------------

#[test]
fn classify_tc_advances_second_tj() {
    let scratch = Scratch::new("r8-tc");
    let path = scratch.file("tc-two-tj.pdf");
    write_helvetica_page(
        &path,
        b"BT /F1 12 Tf 2 Tc 72 720 Td (Hi) Tj (there) Tj ET\n",
    );
    let hits = classify(&path, "R8");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R8: (Hi) Tj (there) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    // Helvetica H=667 i=278 → 11.34 at Tf=12. 2 Tc on two glyphs adds 4
    // user units, so second.x ≈ first.x + 15.34, not first.x + 11.34.
    assert!(
        texts[1].rect.x > texts[0].rect.x + 13.0,
        "R8: 2 Tc must push second.x past first.x + no-Tc Hi width 11.34; first.x={} second.x={} (need second.x > first.x + 13)",
        texts[0].rect.x,
        texts[1].rect.x
    );
}

// --- R9 --------------------------------------------------------------------

#[test]
fn classify_source_drops_rotated_fixture_parenthetical() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pdf_engine/source_content.rs");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "R9: must read source_content.rs via CARGO_MANIFEST_DIR ({}): {e}",
            path.display()
        )
    });
    assert!(
        !src.contains("keeps text-rotated.pdf green"),
        "R9: source_content.rs must not contain the exact substring `keeps text-rotated.pdf green`"
    );
}
