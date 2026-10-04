//! File-level hazards and the revision-2 fixtures of §E.2: bombs and deep nesting, signed,
//! encrypted and XFA files, hybrid xref and page-tree shapes lopdf misreads, shared inherited
//! resources, legacy filters, two-column pages, stroked text, swapped images, bad page boxes and
//! an indirect `/Extensions`.

use super::{helvetica_page, structure, DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::engines::{run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::testkit::pdf::{zlib_zero_bomb, XrefStyle};
use crate::pdf_engine::text_edit::testkit::Scratch;
use std::ffi::OsString;

const HELLO: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET";

/// A one-page Helvetica document with `extra(d)` returning catalog entries.
fn with_catalog(extra: impl FnOnce(&mut DocBuilder) -> String) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::new(HELLO, &format!("/Font << /F1 {f} 0 R >>")));
    let entries = extra(&mut d);
    d.catalog_extra.push_str(&entries);
    d.build()
}

/// An unreferenced object nested `levels` deep (`FILE_TOO_COMPLEX` past 100).
pub fn deep_nesting(levels: usize) -> Vec<u8> {
    with_catalog(|d| {
        d.add(format!("{}{}", "[".repeat(levels), "]".repeat(levels)));
        String::new()
    })
}

/// An object stream that inflates to 1 GiB.
pub fn objstm_bomb() -> Vec<u8> {
    with_catalog(|d| {
        d.b.add_stream(
            "/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode",
            &zlib_zero_bomb(1024),
        );
        String::new()
    })
}

/// An xref stream that inflates to 1 GiB.
pub fn xref_bomb() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(PageSpec::new(HELLO, &format!("/Font << /F1 {f} 0 R >>")));
    d.build_with(&XrefStyle::Stream {
        in_objstm: vec![],
        bomb_mib: Some(1024),
    })
}

/// A page whose content stream inflates past the 32 MiB stream cap (page `PAGE_TOO_COMPLEX`),
/// followed by a normal page.
pub fn flate_bomb() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    let bomb = d.b.add_stream("/Filter /FlateDecode", &zlib_zero_bomb(64));
    d.page_raw(&format!("{bomb} 0 R"), &PageSpec::new(b"", &res));
    d.page(PageSpec::new(HELLO, &res));
    d.build()
}

/// An applied signature (`/Type /Sig` with `/ByteRange`).
pub fn signed() -> Vec<u8> {
    with_catalog(|d| {
        let sig =
            d.add("<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 10 20 30] /Contents <00> >>");
        let field = d.add(format!(
            "<< /FT /Sig /T (Signature1) /V {sig} 0 R /Subtype /Widget /Rect [0 0 0 0] >>"
        ));
        format!("/AcroForm << /Fields [{field} 0 R] /SigFlags 3 >>")
    })
}

/// A dynamic XFA form.
pub fn xfa() -> Vec<u8> {
    with_catalog(|d| {
        let x = d.b.add_stream("", b"<xdp:xdp/>");
        format!("/AcroForm << /Fields [] /XFA {x} 0 R >>")
    })
}

/// A Helvetica page encrypted by `qpdf --encrypt` (AES-256).
pub fn encrypted(engines: &Engines) -> Vec<u8> {
    let s = Scratch::new("producers-enc");
    let plain = s.write("plain.pdf", &helvetica_page(HELLO));
    let out = s.path("enc.pdf");
    let args: Vec<OsString> = vec![
        "--encrypt".into(),
        "user".into(),
        "owner".into(),
        "256".into(),
        "--".into(),
        plain.into(),
        out.clone().into(),
    ];
    let r = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).expect("qpdf --encrypt");
    assert_eq!(r.code, 0, "qpdf --encrypt: {}", r.stderr);
    std::fs::read(&out).expect("encrypted output")
}

/// The page sits in an object stream listed by a hybrid file's `/XRefStm`; Word style
/// (`listed_in_classic`) also lists the object stream in the classic table, which lopdf reads.
pub fn hybrid_xref(listed_in_classic: bool) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.page(PageSpec::new(HELLO, &format!("/Font << /F1 {f} 0 R >>")));
    d.build_with(&XrefStyle::Hybrid {
        in_objstm: vec![page],
        objstm_in_classic: listed_in_classic,
    })
}

/// Two kids, the second without `/Type` (qpdf counts it, lopdf skips it).
pub fn kid_without_type() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    d.page(PageSpec::new(HELLO, &res));
    let content = d.b.add_stream("", b"BT /F1 12 Tf 72 720 Td (Second) Tj ET");
    let id = d.add(format!(
        "<< /Parent {} 0 R /MediaBox [0 0 612 792] /Contents {content} 0 R /Resources << {res} >> >>",
        d.pages
    ));
    d.kids.push(id);
    d.build()
}

/// Two pages sharing one `/Resources` on the `/Pages` node: the bold sibling `/F2` is used only
/// on page 2.
pub fn shared_inherited_resources() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let regular = d.add(HELVETICA);
    let bold = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );
    d.pages_extra = format!("/Resources << /Font << /F1 {regular} 0 R /F2 {bold} 0 R >> >>");
    for content in [
        &b"BT /F1 12 Tf 72 720 Td (Regular words) Tj ET"[..],
        b"BT /F2 12 Tf 72 720 Td (Bold words) Tj ET",
    ] {
        let id = d.b.add_stream("", content);
        let page = d.add(format!(
            "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 612 792] /Contents {id} 0 R >>",
            d.pages
        ));
        d.page_ids.push(page);
        d.kids.push(page);
    }
    d.build()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyFilter {
    RunLength,
    Lzw,
}

fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in data.chunks(128) {
        out.push((chunk.len() - 1) as u8);
        out.extend_from_slice(chunk);
    }
    out.push(128);
    out
}

/// LZW with 9-bit literal codes only: a clear code every 200 literals keeps the table (and so
/// the code width, early change 1) small; ends with EOD.
fn lzw(data: &[u8]) -> Vec<u8> {
    let mut codes = vec![256u16];
    for (i, b) in data.iter().enumerate() {
        if i > 0 && i % 200 == 0 {
            codes.push(256);
        }
        codes.push(u16::from(*b));
    }
    codes.push(257);
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for c in codes {
        acc = (acc << 9) | u32::from(c);
        bits += 9;
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if bits > 0 {
        out.push((acc << (8 - bits)) as u8);
    }
    out
}

/// A page whose content stream uses RunLength or LZW (refused `UNSUPPORTED_FILTER`; qpdf keeps
/// such streams raw), then a Flate-free normal page.
pub fn legacy_filter_page(filter: LegacyFilter) -> Vec<u8> {
    let content = b"BT /F1 12 Tf 72 720 Td (Legacy filter) Tj ET";
    let (name, data) = match filter {
        LegacyFilter::RunLength => ("RunLengthDecode", run_length(content)),
        LegacyFilter::Lzw => ("LZWDecode", lzw(content)),
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    let id = d.b.add_stream(&format!("/Filter /{name}"), &data);
    d.page_raw(&format!("{id} 0 R"), &PageSpec::new(b"", &res));
    d.page(PageSpec::new(HELLO, &res));
    d.build()
}

/// The lines of the two-column page: (MCID, x, y, size, text), in content order (row by row,
/// with the title in the middle).
pub const TWO_COLUMN_LINES: [(i64, f64, f64, f64, &str); 7] = [
    (0, 72.0, 700.0, 12.0, "Left one"),
    (1, 320.0, 700.0, 12.0, "Right one"),
    (2, 72.0, 740.0, 14.0, "A two column page"),
    (3, 72.0, 686.0, 12.0, "Left two"),
    (4, 320.0, 686.0, 12.0, "Right two"),
    (5, 72.0, 672.0, 12.0, "Left three"),
    (6, 320.0, 672.0, 12.0, "Right three"),
];

/// The structure order of the tagged two-column page: right column, title, left column — on
/// purpose neither the content order nor the XY-cut order.
pub const TWO_COLUMN_STRUCT_ORDER: [i64; 7] = [1, 4, 6, 2, 0, 3, 5];

/// A full-width title over two columns, written row by row. `tagged`: every line in its own
/// marked-content sequence, structure order `TWO_COLUMN_STRUCT_ORDER`; `missing_mcid` leaves
/// the last line untagged.
pub fn two_column_with(tagged: bool, missing_mcid: bool) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.reserve();
    let extra = if tagged {
        structure(&mut d, page, &TWO_COLUMN_STRUCT_ORDER, &[])
    } else {
        String::new()
    };
    let mut content = String::new();
    for (i, (mcid, x, y, size, text)) in TWO_COLUMN_LINES.iter().enumerate() {
        let line = format!("BT /F1 {size} Tf {x} {y} Td ({text}) Tj ET");
        let untagged = !tagged || (missing_mcid && i + 1 == TWO_COLUMN_LINES.len());
        if untagged {
            content.push_str(&format!("{line} "));
        } else {
            content.push_str(&format!("/P <</MCID {mcid}>> BDC {line} EMC "));
        }
    }
    d.page_at(
        page,
        PageSpec::new(content.as_bytes(), &format!("/Font << /F1 {f} 0 R >>")).with(&extra),
    );
    d.build()
}

/// §E.2 `two_column(tagged)`.
pub fn two_column(tagged: bool) -> Vec<u8> {
    two_column_with(tagged, false)
}

/// A line drawn as two stroked segments (`tr` 1 or 2) on one baseline: "Bold" with line width
/// `widths[0]` and dash `dashes[0]`, then "face" with `widths[1]`/`dashes[1]`.
pub fn stroke_text(tr: i64, widths: [f64; 2], dashes: [&str; 2]) -> Vec<u8> {
    // Helvetica "Bold" = 667 + 556 + 222 + 556 = 2001 → 24.012 pt at 12 pt.
    let content = format!(
        "{} w {} d BT /F1 12 Tf {tr} Tr 72 700 Td (Bold) Tj ET \
         {} w {} d BT /F1 12 Tf {tr} Tr 96.012 700 Td (face) Tj ET",
        widths[0], dashes[0], widths[1], dashes[1]
    );
    helvetica_page(content.as_bytes())
}

/// The same page with one of two image data variants behind the same name `/Im0`.
pub fn swapped_image(swapped: bool) -> Vec<u8> {
    let pixels: &[u8] = if swapped {
        &[16, 16, 200, 200, 16, 16, 16, 200, 16, 200, 200, 16]
    } else {
        &[200, 16, 16, 16, 200, 16, 16, 16, 200, 200, 200, 16]
    };
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let img = d.b.add_stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8",
        pixels,
    );
    d.page(PageSpec::new(
        b"q 40 0 0 40 72 400 cm /Im0 Do Q BT /F1 12 Tf 72 720 Td (Image page) Tj ET",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Im0 {img} 0 R >>"),
    ));
    d.build()
}

/// Page setups the strict geometry parser refuses (`GEOMETRY`, §A.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadBox {
    /// No `/MediaBox` anywhere in the page tree.
    NoMediaBox,
    /// `/Rotate 90.0` (a real).
    RealRotate,
    /// `/Rotate 45`.
    OddRotate,
    /// Crop ∩ Media thinner than 1 pt.
    ThinCrop,
    /// `/UserUnit (1)` (a string).
    UserUnitString,
    /// `/UserUnit -1`.
    UserUnitNegative,
    /// `/UserUnit 2`.
    UserUnitTwo,
    /// A MediaBox with a name in it.
    NonNumberBox,
}

pub fn bad_boxes(kind: BadBox) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let spec = PageSpec::new(HELLO, &format!("/Font << /F1 {f} 0 R >>"));
    let spec = match kind {
        BadBox::NoMediaBox => spec.media(None),
        BadBox::RealRotate => spec.with("/Rotate 90.0"),
        BadBox::OddRotate => spec.with("/Rotate 45"),
        BadBox::ThinCrop => spec.with("/CropBox [0 0 612 0.5]"),
        BadBox::UserUnitString => spec.with("/UserUnit (1)"),
        BadBox::UserUnitNegative => spec.with("/UserUnit -1"),
        BadBox::UserUnitTwo => spec.with("/UserUnit 2"),
        BadBox::NonNumberBox => spec.media(Some("[0 0 /Wide 792]")),
    };
    d.page(spec);
    d.build()
}

/// The catalog's `/Extensions` as an indirect object (qpdf writes it direct: APP-08).
pub fn extensions_indirect() -> Vec<u8> {
    with_catalog(|d| {
        let ext = d.add("<< /ADBE << /BaseVersion /1.7 /ExtensionLevel 3 >> >>");
        format!("/Extensions {ext} 0 R")
    })
}
