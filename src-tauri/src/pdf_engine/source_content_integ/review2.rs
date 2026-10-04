//! PR #97 review regressions R10–R14 (generated in temp; `fixtures/source-edit/` does not grow).

use super::review::{box_obj, helvetica_resources, write_helvetica_page};
use super::*;

// --- PR 97 review fold r3 (R10–R12) ----------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

/// 2×2 DeviceRGB, same bytes as the #32 unique/mask fixtures.
const R3_TINY_RGB: &[u8] = &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16];

fn write_type1_indirect_widths(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    // FirstChar 'H' (72) … LastChar 'i' (105): 34 glyph slots, all 1000.
    const FIRST_CHAR: i64 = 72;
    const LAST_CHAR: i64 = 105;
    let widths: Vec<Object> = (FIRST_CHAR..=LAST_CHAR)
        .map(|_| Object::Integer(1000))
        .collect();
    let widths_id = doc.add_object(Object::Array(widths));

    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    font.set("FirstChar", FIRST_CHAR);
    font.set("LastChar", LAST_CHAR);
    font.set("Widths", Object::Reference(widths_id));

    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));
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
        .expect("write Type1 indirect-Widths classifier fixture");
}

fn write_pattern_cs_page(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Dictionary(font));

    let mut cs = Dictionary::new();
    cs.set("Cs1", Object::Name(b"Pattern".to_vec()));

    let mut pat = Dictionary::new();
    pat.set("Type", "Pattern");
    pat.set("PatternType", 1);
    pat.set("PaintType", 1);
    pat.set("TilingType", 1);
    pat.set("BBox", box_obj([0, 0, 10, 10]));
    pat.set("XStep", 10);
    pat.set("YStep", 10);
    pat.set("Resources", Object::Dictionary(Dictionary::new()));
    let pat_id = doc.add_object(Object::Stream(Stream::new(
        pat,
        b"0 0 10 10 re f\n".to_vec(),
    )));
    let mut patterns = Dictionary::new();
    patterns.set("P1", Object::Reference(pat_id));

    let mut res = Dictionary::new();
    res.set("Font", Object::Dictionary(fonts));
    res.set("ColorSpace", Object::Dictionary(cs));
    res.set("Pattern", Object::Dictionary(patterns));

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
        .expect("write Pattern ColorSpace classifier fixture");
}

fn write_extgstate_smask_image(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut img = Dictionary::new();
    img.set("Type", "XObject");
    img.set("Subtype", "Image");
    img.set("Width", 2);
    img.set("Height", 2);
    img.set("ColorSpace", "DeviceRGB");
    img.set("BitsPerComponent", 8);
    let img_id = doc.add_object(Object::Stream(Stream::new(img, R3_TINY_RGB.to_vec())));

    let mut sm = Dictionary::new();
    sm.set("Type", "XObject");
    sm.set("Subtype", "Image");
    sm.set("Width", 2);
    sm.set("Height", 2);
    sm.set("ColorSpace", "DeviceGray");
    sm.set("BitsPerComponent", 8);
    let smask_id = doc.add_object(Object::Stream(Stream::new(sm, vec![255, 200, 180, 255])));

    let mut gs = Dictionary::new();
    gs.set("Type", "ExtGState");
    gs.set("SMask", Object::Reference(smask_id));
    let gs_id = doc.add_object(Object::Dictionary(gs));

    let mut xobjects = Dictionary::new();
    xobjects.set("Im0", Object::Reference(img_id));
    let mut extg = Dictionary::new();
    extg.set("Gs1", Object::Reference(gs_id));
    let mut res = Dictionary::new();
    res.set("XObject", Object::Dictionary(xobjects));
    res.set("ExtGState", Object::Dictionary(extg));

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
        .expect("write ExtGState SMask + unique Image classifier fixture");
}

// --- R10 -------------------------------------------------------------------

#[test]
fn classify_indirect_widths_not_helvetica_fallback() {
    let scratch = Scratch::new("r10-widths");
    let path = scratch.file("indirect-widths.pdf");
    write_type1_indirect_widths(&path, b"BT /F1 12 Tf 72 720 Td (Hi) Tj ET\n");
    let hits = classify(&path, "R10");
    let occ = first_of_kind(&hits, "text", "R10");
    assert!(
        (occ.rect.w - 24.0).abs() <= 1.0,
        "R10: Type1 indirect /Widths 1000,1000 at Tf=12 must report w≈24, not Helvetica fallback ≈11.34; got w={}",
        occ.rect.w
    );
    assert!(
        (occ.rect.w - 11.34).abs() > 1.0,
        "R10: rect.w must not stay on the Helvetica table ≈11.34; got w={}",
        occ.rect.w
    );
}

// --- R11 -------------------------------------------------------------------

#[test]
fn classify_named_pattern_cs_is_unsupported() {
    let scratch = Scratch::new("r11-pattern");
    let path = scratch.file("pattern-cs.pdf");
    write_pattern_cs_page(
        &path,
        b"BT /F1 12 Tf /Cs1 cs /P1 scn 72 720 Td (Hi) Tj ET\n",
    );
    let hits = classify(&path, "R11");
    let occ = first_of_kind(&hits, "text", "R11");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R11: /Cs1 cs Pattern resource + (Hi) Tj must not be supported; got {} reason={:?}",
        capability_token(occ),
        reason_code(occ)
    );
    assert_unsupported(occ, "text", "PATTERN", "R11");
}

// --- R12 -------------------------------------------------------------------

#[test]
fn classify_extgstate_smask_image_is_masked() {
    let scratch = Scratch::new("r12-gs-smask");
    let path = scratch.file("gs-smask.pdf");
    write_extgstate_smask_image(&path, b"q 40 0 0 40 72 400 cm /Gs1 gs /Im0 Do Q\n");
    let hits = classify(&path, "R12");
    let occ = first_of_kind(&hits, "image", "R12");
    assert_ne!(
        capability_token(occ),
        "supported",
        "R12: unique Image after ExtGState /Gs1 /SMask must not be supported; got {} reason={:?}",
        capability_token(occ),
        reason_code(occ)
    );
    assert_unsupported(occ, "image", "MASKED_IMAGE", "R12");
}

// --- PR 97 review fold r4 (R13) --------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.

fn write_unique_rgb_image(path: &Path, content: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        content.to_vec(),
    )));

    let mut img = Dictionary::new();
    img.set("Type", "XObject");
    img.set("Subtype", "Image");
    img.set("Width", 2);
    img.set("Height", 2);
    img.set("ColorSpace", "DeviceRGB");
    img.set("BitsPerComponent", 8);
    let img_id = doc.add_object(Object::Stream(Stream::new(img, R3_TINY_RGB.to_vec())));

    let mut xobjects = Dictionary::new();
    xobjects.set("Im0", Object::Reference(img_id));
    let mut res = Dictionary::new();
    res.set("XObject", Object::Dictionary(xobjects));

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
        .expect("write unique 2×2 DeviceRGB Image classifier fixture");
}

// --- R13a ------------------------------------------------------------------

#[test]
fn classify_stacked_cm_image_origin() {
    let scratch = Scratch::new("r13a-stacked-cm");
    let path = scratch.file("stacked-cm.pdf");
    write_unique_rgb_image(&path, b"q 2 0 0 2 0 0 cm 20 0 0 20 36 200 cm /Im0 Do Q\n");
    let hits = classify(&path, "R13a");
    let occ = first_of_kind(&hits, "image", "R13a");
    assert_supported_text_or_image(occ, "image", "R13a");
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0
            && (occ.rect.y - 400.0).abs() <= 1.0
            && (occ.rect.w - 40.0).abs() <= 1.0
            && (occ.rect.h - 40.0).abs() <= 1.0,
        "R13a: stacked cm image rect must be ~{{x:72, y:400, w:40, h:40}}, not origin ~(36, 200); got {{x:{}, y:{}, w:{}, h:{}}}",
        occ.rect.x,
        occ.rect.y,
        occ.rect.w,
        occ.rect.h
    );
    assert!(
        (occ.rect.x - 36.0).abs() > 1.0 || (occ.rect.y - 200.0).abs() > 1.0,
        "R13a: stacked cm must not leave the image at the second-cm translation (36, 200); got {{x:{}, y:{}, w:{}, h:{}}}",
        occ.rect.x,
        occ.rect.y,
        occ.rect.w,
        occ.rect.h
    );
}

// --- R13b ------------------------------------------------------------------

#[test]
fn classify_scaled_tm_second_show_x() {
    let scratch = Scratch::new("r13b-scaled-tm");
    let path = scratch.file("scaled-tm-two-tj.pdf");
    write_helvetica_page(
        &path,
        b"BT /F1 1 Tf 12 0 0 12 72 720 Tm (Hel) Tj (lo) Tj ET\n",
    );
    let hits = classify(&path, "R13b");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R13b: (Hel) Tj (lo) Tj must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    let first = texts[0];
    let second = texts[1];
    // Helvetica H=667 e=556 l=278 → 1.501 at Tf=1. Scaled Tm 12× must
    // advance ~18.012 user units → second.x ≈ 90, not text-space 1.501
    // added in user space (≈73.5).
    assert!(
        second.rect.x > first.rect.x + 15.0 || (second.rect.x - 90.0).abs() <= 2.0,
        "R13b: second rect.x after 12 0 0 12 72 720 Tm (Hel) Tj must be ≈90 (±2), not ≈73.5; first.x={} second.x={}",
        first.rect.x,
        second.rect.x
    );
    assert!(
        (second.rect.x - 73.5).abs() > 1.0,
        "R13b: second rect.x must not stay at origin+text-space width ≈73.5; first.x={} second.x={}",
        first.rect.x,
        second.rect.x
    );
}

// --- PR 97 review fold r5 (R14) --------------------------------------------
// Extra PDFs are generated in temp with lopdf. Do not grow fixtures/source-edit/.
// Page /Contents is an array of two streams; stream 1 has no trailing whitespace
// so a join without a separator fuses `Tj`+`ET` into `TjET`.

const R14_STREAM_1: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hi) Tj";
const R14_STREAM_2: &[u8] = b"ET\nBT /F1 12 Tf 72 680 Td (Lo) Tj ET";

fn stream_content_bytes(doc: &Document, obj: &Object) -> Vec<u8> {
    let id = obj
        .as_reference()
        .expect("Contents array entry must be a stream ref");
    doc.get_object(id)
        .expect("content stream object")
        .as_stream()
        .expect("content must be a stream")
        .content
        .clone()
}

fn write_helvetica_two_content_streams(path: &Path, stream1: &[u8], stream2: &[u8]) {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content1_id = doc.add_object(Object::Stream(
        Stream::new(Dictionary::new(), stream1.to_vec()).with_compression(false),
    ));
    let content2_id = doc.add_object(Object::Stream(
        Stream::new(Dictionary::new(), stream2.to_vec()).with_compression(false),
    ));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("MediaBox", box_obj([0, 0, 612, 792]));
    page.set(
        "Contents",
        vec![
            Object::Reference(content1_id),
            Object::Reference(content2_id),
        ],
    );
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
    doc.save(path)
        .expect("write two-stream Contents classifier fixture");

    // Lock the on-disk page /Contents shape: array of two streams, exact bytes.
    let reloaded =
        Document::load(path).unwrap_or_else(|e| panic!("reload two-stream Contents fixture: {e}"));
    let page_id = *reloaded
        .get_pages()
        .get(&1)
        .expect("two-stream fixture must have page 1");
    let page = reloaded
        .get_object(page_id)
        .expect("page 1 object")
        .as_dict()
        .expect("page 1 dict");
    let contents = page.get(b"Contents").expect("page /Contents");
    let refs = match contents {
        Object::Array(arr) => arr,
        other => panic!(
            "two-stream fixture page /Contents must be an array of two stream refs, got {other:?}"
        ),
    };
    assert_eq!(
        refs.len(),
        2,
        "two-stream fixture page /Contents must have two stream refs; got {}",
        refs.len()
    );
    assert_eq!(
        stream_content_bytes(&reloaded, &refs[0]).as_slice(),
        stream1,
        "two-stream fixture stream 1 bytes must be exact (no trailing newline)"
    );
    assert_eq!(
        stream_content_bytes(&reloaded, &refs[1]).as_slice(),
        stream2,
        "two-stream fixture stream 2 bytes must match"
    );
}

// --- R14 -------------------------------------------------------------------

#[test]
fn classify_contents_array_two_streams_do_not_fuse() {
    let scratch = Scratch::new("r14-two-streams");
    let path = scratch.file("two-contents-streams.pdf");
    write_helvetica_two_content_streams(&path, R14_STREAM_1, R14_STREAM_2);
    let hits = classify(&path, "R14");
    let texts: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "text").collect();
    assert_eq!(
        texts.len(),
        2,
        "R14: two Contents streams (Hi@720 then Lo@680) must emit two text occurrences; got {:?}",
        hits.iter()
            .map(|o| (kind_token(o), o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    assert!(
        texts.iter().any(|o| (o.rect.y - 720.0).abs() <= 1.0),
        "R14: expected a text occurrence at y≈720 (Hi); got {:?}",
        texts
            .iter()
            .map(|o| (o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    assert!(
        texts.iter().any(|o| (o.rect.y - 680.0).abs() <= 1.0),
        "R14: expected a text occurrence at y≈680 (Lo); got {:?}",
        texts
            .iter()
            .map(|o| (o.rect.x, o.rect.y))
            .collect::<Vec<_>>()
    );
    // Deliberate change (§C row 16): the v2 locator of "Lo" names its span in the joined content,
    // which lies in part 1 (after part 0 and its "\n" separator), not in the first stream.
    let lo = texts
        .iter()
        .find(|o| (o.rect.y - 680.0).abs() <= 1.0)
        .expect("R14: Lo");
    let path = lo.locator.split(':').nth(4).expect("R14: locator path");
    let start: usize = path
        .trim_start_matches('p')
        .split('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("R14: depth-0 path p{{start}}-{{end}}: {}", lo.locator));
    assert!(
        start > R14_STREAM_1.len(),
        "R14: {} must point into part 1 (part 0 is {} bytes)",
        lo.locator,
        R14_STREAM_1.len()
    );
}
