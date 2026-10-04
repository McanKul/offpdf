//! Geometry, syntax and graphics-state edges (§E.2) — mostly one Helvetica (`/F1`) page each:
//! rotation and crop, split parts, inline images with each payload proof, odd syntax, the text
//! state operators, ExtGState fonts, encodings, ActualText, shadows, clips, patterns, masks,
//! render modes, vertical and Type3 fonts, Forms and the orientation cases of §A.5.

use super::office::{cid_font_with, word_font};
use super::{helvetica_doc, helvetica_page, structure, DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::testkit::pdf::zlib;

fn line(text: &str) -> Vec<u8> {
    format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET").into_bytes()
}

/// A page with `/Rotate angle`; `counter_rotated` draws the text rotated against the page so it
/// reads upright on screen (§A.5 "as displayed"), else it is upright in user space.
pub fn rotated(angle: i64, counter_rotated: bool) -> Vec<u8> {
    let tm = match (counter_rotated, angle.rem_euclid(360)) {
        (true, 90) => "0 1 -1 0",
        (true, 180) => "-1 0 0 -1",
        (true, 270) => "0 -1 1 0",
        _ => "1 0 0 1",
    };
    let content = format!("BT /F1 12 Tf {tm} 300 400 Tm (Rotated page) Tj ET");
    helvetica_doc(content.as_bytes(), "", &format!("/Rotate {angle}"))
}

/// A CropBox offset from the MediaBox.
pub fn cropped_offset() -> Vec<u8> {
    helvetica_doc(&line("Cropped page"), "", "/CropBox [36 48 576 744]")
}

/// `/UserUnit unit` on the page.
pub fn user_unit(unit: f64) -> Vec<u8> {
    helvetica_doc(&line("Custom unit"), "", &format!("/UserUnit {unit}"))
}

/// A text object opened in part 1 and closed in part 2 (`/Contents` array).
pub fn two_parts_mid_bt() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 720 Td (Hi) Tj",
            b"ET\nBT /F1 12 Tf 72 680 Td (Lo) Tj ET",
        ],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// A show op whose operand ends part 1 and whose operator starts part 2.
pub fn straddling_op() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::parts(
        &[
            b"BT /F1 12 Tf 72 720 Td (Hello)",
            b"Tj ET BT /F1 12 Tf 72 680 Td (After) Tj ET",
        ],
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// How an inline image's payload end is known (§A.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineProofKind {
    Length,
    Unfiltered,
    Flate,
    Dct,
}

/// An inline image before a text line; `Dct` (no `/L`) can only end heuristically, so the text
/// after it is refused `INLINE_IMAGE`.
pub fn inline_image(proof: InlineProofKind) -> Vec<u8> {
    let head = b"q 24 0 0 12 72 400 cm BI /W 2 /H 1 /CS /RGB /BPC 8 ";
    let pixels: &[u8] = &[200, 16, 16, 16, 200, 16];
    let (dict, data): (&[u8], Vec<u8>) = match proof {
        InlineProofKind::Length => (b"/L 6 ", pixels.to_vec()),
        InlineProofKind::Unfiltered => (b"", pixels.to_vec()),
        InlineProofKind::Flate => (b"/F /Fl ", zlib(pixels)),
        InlineProofKind::Dct => (b"/F /DCT ", b"\xff\xd8\xff\xe0 fake jpeg \xff\xd9".to_vec()),
    };
    let mut content = head.to_vec();
    content.extend_from_slice(dict);
    content.extend_from_slice(b"ID ");
    content.extend_from_slice(&data);
    content.extend_from_slice(b" EI Q BT /F1 12 Tf 72 720 Td (After the image) Tj ET");
    helvetica_page(&content)
}

/// Comments, CR LF, tabs and form feeds between tokens.
pub fn comments_and_odd_ws() -> Vec<u8> {
    helvetica_page(
        b"% leading comment\r\nBT\t/F1 12 Tf\x0c72 720 Td%inline\n(Odd ws)Tj\r\nET % done",
    )
}

/// `'` and `"` show ops.
pub fn quote_ops() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 12 Tf 14 TL 72 720 Td (Line one) Tj (Line two) ' 1 0.5 (Line three) \" ET",
    )
}

/// B1: `[(AB) -500 (CD)] TJ (tail) Tj`.
pub fn kerned_then_tail() -> Vec<u8> {
    helvetica_page(b"BT /F1 10 Tf 72 700 Td [(AB) -500 (CD)] TJ (tail) Tj ET")
}

/// B2: `/F1 1 Tf 12 0 0 12 Tm`.
pub fn tf1_tm12() -> Vec<u8> {
    helvetica_page(b"BT /F1 1 Tf 12 0 0 12 72 720 Tm (Hi) Tj ET")
}

/// `80 Tz 1 Tc 2 Tw 3 Ts`.
pub fn tz_tc_tw_ts() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf 80 Tz 1 Tc 2 Tw 3 Ts 72 720 Td (a b) Tj ET")
}

/// The font comes from an ExtGState `/Font` entry (no `Tf`).
pub fn extgstate_font() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::new(
        b"/GS1 gs BT 72 720 Td (Hi there) Tj ET",
        &format!("/ExtGState << /GS1 << /Type /ExtGState /Font [{f} 0 R 12] >> >>"),
    ));
    d.build()
}

/// Times-Roman with `/MacRomanEncoding` ("café", é = 0x8E).
pub fn mac_roman() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /MacRomanEncoding >>",
    );
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 720 Td (caf\\216) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// B3: a Word subset without "Y" (its `/Widths` slot is 0 and no glyph exists).
pub fn subset_without_y() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = word_font(&mut d.b, "ABCDEF+Calibri", "Hello");
    d.page(PageSpec::new(
        &line("Hello"),
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// `/Span <</ActualText (fi)>> BDC` around one line, a plain line after it.
pub fn actual_text_span() -> Vec<u8> {
    helvetica_page(
        b"/Span <</ActualText (fi)>> BDC BT /F1 12 Tf 72 720 Td (fi) Tj ET EMC \
          BT /F1 12 Tf 72 700 Td (Plain) Tj ET",
    )
}

/// A tagged page whose structure element for MCID 0 carries `/ActualText`.
pub fn actual_text_struct() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.reserve();
    let extra = structure(&mut d, page, &[0, 1], &[0]);
    d.page_at(
        page,
        PageSpec::new(
            b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 720 Td (Struct) Tj ET EMC \
              /P <</MCID 1>> BDC BT /F1 12 Tf 72 700 Td (Free) Tj ET EMC",
            &format!("/Font << /F1 {f} 0 R >>"),
        )
        .with(&extra),
    );
    d.build()
}

/// The same text drawn twice, half a point apart (a shadow).
pub fn duplicate_shadow() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 12 Tf 72 720 Td (Shadow) Tj ET BT /F1 12 Tf 72.5 719.5 Td (Shadow) Tj ET",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// Word's page-sized `re W n`.
    Page,
    /// A rectangle that cuts the text.
    Small,
    /// A Bézier path clip.
    Curve,
}

/// Text inside a clip.
pub fn clip(kind: ClipKind) -> Vec<u8> {
    let path = match kind {
        ClipKind::Page => "0 0 612 792 re",
        ClipKind::Small => "72 715 6 6 re",
        ClipKind::Curve => "0 0 m 300 900 600 900 612 0 c h",
    };
    let content = format!("q {path} W n BT /F1 12 Tf 72 720 Td (Clip me) Tj ET Q");
    helvetica_page(content.as_bytes())
}

/// A Helvetica page with a `/Cs1` Pattern colour space and a `/P1` tiling pattern.
pub fn pattern_doc(content: &[u8]) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let pat = d.b.add_stream(
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 \
         /YStep 10 /Resources << >>",
        b"0 0 10 10 re f",
    );
    d.page(PageSpec::new(
        content,
        &format!(
            "/Font << /F1 {f} 0 R >> /ColorSpace << /Cs1 /Pattern >> /Pattern << /P1 {pat} 0 R >>"
        ),
    ));
    d.build()
}

/// Text filled with a pattern.
pub fn pattern_fill() -> Vec<u8> {
    pattern_doc(b"BT /F1 12 Tf /Cs1 cs /P1 scn 72 720 Td (Pattern) Tj ET")
}

/// Text under an ExtGState soft mask.
pub fn smask_text() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let group = d.b.add_stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Group << /S /Transparency /CS /DeviceGray >>",
        b"0.5 g 0 0 612 792 re f",
    );
    d.page(PageSpec::new(
        b"/GS1 gs BT /F1 12 Tf 72 720 Td (Masked) Tj ET",
        &format!(
            "/Font << /F1 {f} 0 R >> /ExtGState << /GS1 << /SMask << /Type /Mask /S /Luminosity /G {group} 0 R >> >> >>"
        ),
    ));
    d.build()
}

/// `7 Tr` (clip only) text, then a normal line drawn inside that text clip.
pub fn tr7() -> Vec<u8> {
    helvetica_page(
        b"BT 7 Tr /F1 12 Tf 72 720 Td (Clip text) Tj ET BT 0 Tr /F1 12 Tf 72 700 Td (After clip) Tj ET",
    )
}

/// A Type0 `/Identity-V` font (vertical writing).
pub fn identity_v() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = cid_font_with(&mut d.b, "ABCDEF+Vertical", "Up", "/Identity-V");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 300 700 Td <00010002> Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// A Type3 font.
pub fn type3() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let proc_id = d.b.add_stream("", b"10 0 0 0 10 10 d1 0 0 10 10 re f");
    let f = d.add(format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] \
         /CharProcs << /a {proc_id} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> \
         /FirstChar 97 /LastChar 97 /Widths [10] >>"
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 720 Td (a) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// Text inside a Form XObject painted by the page.
pub fn nested_form() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let form = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 300 100] /Resources << /Font << /F1 {f} 0 R >> >>"
        ),
        b"BT /F1 12 Tf 10 10 Td (In a form) Tj ET",
    );
    d.page(PageSpec::new(
        b"q 1 0 0 1 72 600 cm /Fm0 Do Q",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Fm0 {form} 0 R >>"),
    ));
    d.build()
}

/// `-1 0 0 1 Tm` (mirrored).
pub fn mirrored() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf -1 0 0 1 300 400 Tm (Mirror) Tj ET")
}

/// `-100 Tz` (mirrored).
pub fn negative_tz() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf -100 Tz 300 400 Td (Mirror) Tj ET")
}

/// `/F1 -12 Tf` (turned 180°).
pub fn negative_tf() -> Vec<u8> {
    helvetica_page(b"BT /F1 -12 Tf 300 400 Td (Turned) Tj ET")
}

/// `1 0.3 0 1 Tm` (skewed baseline).
pub fn skewed() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf 1 0.3 0 1 72 400 Tm (Skewed) Tj ET")
}

/// `1 0 0.2 1 Tm` (synthetic italic: upright).
pub fn oblique() -> Vec<u8> {
    helvetica_page(b"BT /F1 12 Tf 1 0 0.2 1 72 400 Tm (Oblique) Tj ET")
}
