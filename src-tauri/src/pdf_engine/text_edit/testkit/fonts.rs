//! PDF font dictionary helpers for tests (T2, SPEC §E.1): simple and Type0 font objects for
//! `PdfBuilder` documents (programs, descriptors, encodings, widths, ToUnicode CMaps), one-page
//! documents carrying them, and loading their `FontModel`s through the real snapshot reader.

use super::pdf::PdfBuilder;
use super::ttf::TtfBuilder;
use super::type1::Type1File;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::fonts::{agl, FontCache, FontKey, FontModel};
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::snapshot::{snapshot_from_bytes, SourceSnapshot};
use lopdf::Object;
use std::sync::Arc;

/// An embedded program and the descriptor key it goes under.
#[derive(Debug, Clone)]
pub enum Program {
    None,
    /// `FontFile2`.
    TrueType(Vec<u8>),
    /// `FontFile` with `Length1/2/3`.
    Type1(Type1File),
    /// `FontFile3 /Subtype /Type1C`.
    Cff(Vec<u8>),
    /// `FontFile3 /Subtype /CIDFontType0C`.
    CidCff(Vec<u8>),
    /// `FontFile3 /Subtype /OpenType`.
    OpenType(Vec<u8>),
    /// Any key / FontFile3 subtype / data (mismatch and corruption tests).
    Raw {
        key: &'static str,
        subtype: Option<&'static str>,
        data: Vec<u8>,
    },
}

/// A simple font (`/Type1`, `/TrueType`, `/MMType1`, `/Type3`).
#[derive(Debug, Clone)]
pub struct SimpleFont {
    pub subtype: &'static str,
    pub base_font: String,
    /// Raw PDF for `/Encoding`, e.g. `/WinAnsiEncoding` or `<< /Differences [65 /A] >>`.
    pub encoding: Option<String>,
    pub first_char: u32,
    pub widths: Option<Vec<f64>>,
    /// `/Flags`; a descriptor is written whenever flags, a program or extras are given.
    pub flags: Option<i64>,
    /// Extra descriptor entries, e.g. `/MissingWidth 300 /CharSet (/a/b)`.
    pub descriptor_extra: String,
    pub program: Program,
    pub tounicode: Option<Vec<u8>>,
    /// Extra font dictionary entries.
    pub font_extra: String,
    /// Compress program and ToUnicode streams with Flate.
    pub flate: bool,
}

impl SimpleFont {
    pub fn new(subtype: &'static str, base_font: &str) -> Self {
        SimpleFont {
            subtype,
            base_font: base_font.to_string(),
            encoding: None,
            first_char: 0,
            widths: None,
            flags: None,
            descriptor_extra: String::new(),
            program: Program::None,
            tounicode: None,
            font_extra: String::new(),
            flate: true,
        }
    }
}

/// A Type0 font with one descendant CIDFont.
#[derive(Debug, Clone)]
pub struct Type0Font {
    pub base_font: String,
    /// Raw PDF for `/Encoding` (default `/Identity-H`).
    pub encoding: String,
    pub cid_subtype: &'static str,
    /// Raw PDF array for `/W`.
    pub w: Option<String>,
    pub dw: Option<f64>,
    /// `None` = absent; `Some(None)` = `/Identity`; `Some(Some(bytes))` = stream.
    pub cid_to_gid: Option<Option<Vec<u8>>>,
    pub flags: Option<i64>,
    pub descriptor_extra: String,
    pub cid_extra: String,
    pub program: Program,
    pub tounicode: Option<Vec<u8>>,
}

impl Type0Font {
    pub fn new(cid_subtype: &'static str, base_font: &str) -> Self {
        Type0Font {
            base_font: base_font.to_string(),
            encoding: "/Identity-H".to_string(),
            cid_subtype,
            w: None,
            dw: None,
            cid_to_gid: None,
            flags: Some(4),
            descriptor_extra: String::new(),
            cid_extra: String::new(),
            program: Program::None,
            tounicode: None,
        }
    }
}

fn add_data(b: &mut PdfBuilder, dict: &str, data: &[u8], flate: bool) -> u32 {
    if flate {
        b.add_flate(dict, data)
    } else {
        b.add_stream(dict, data)
    }
}

/// Writes the program stream; returns the descriptor entry (`/FontFile2 12 0 R`).
fn add_program(b: &mut PdfBuilder, program: &Program, flate: bool) -> Option<String> {
    let (key, dict, data) = match program {
        Program::None => return None,
        Program::TrueType(d) => ("FontFile2", String::new(), d.clone()),
        Program::Type1(t) => (
            "FontFile",
            format!(
                "/Length1 {} /Length2 {} /Length3 {}",
                t.length1, t.length2, t.length3
            ),
            t.data.clone(),
        ),
        Program::Cff(d) => ("FontFile3", "/Subtype /Type1C".to_string(), d.clone()),
        Program::CidCff(d) => (
            "FontFile3",
            "/Subtype /CIDFontType0C".to_string(),
            d.clone(),
        ),
        Program::OpenType(d) => ("FontFile3", "/Subtype /OpenType".to_string(), d.clone()),
        Program::Raw { key, subtype, data } => (
            *key,
            subtype.map_or(String::new(), |s| format!("/Subtype /{s}")),
            data.clone(),
        ),
    };
    let id = add_data(b, &dict, &data, flate);
    Some(format!("/{key} {id} 0 R"))
}

fn add_descriptor(
    b: &mut PdfBuilder,
    name: &str,
    flags: Option<i64>,
    program: Option<String>,
    extra: &str,
) -> Option<u32> {
    if flags.is_none() && program.is_none() && extra.is_empty() {
        return None;
    }
    Some(b.add(format!(
        "<< /Type /FontDescriptor /FontName /{name} {} /FontBBox [0 -200 1000 800] \
         /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 {} {extra} >>",
        flags.map_or(String::new(), |f| format!("/Flags {f}")),
        program.unwrap_or_default(),
    )))
}

/// Adds a simple font object; returns its id.
pub fn add_simple(b: &mut PdfBuilder, f: &SimpleFont) -> u32 {
    let program = add_program(b, &f.program, f.flate);
    let descriptor = add_descriptor(b, &f.base_font, f.flags, program, &f.descriptor_extra);
    let widths = f.widths.as_ref().map_or(String::new(), |w| {
        format!(
            "/FirstChar {} /LastChar {} /Widths [{}]",
            f.first_char,
            f.first_char + w.len() as u32 - 1,
            w.iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        )
    });
    let tounicode = f.tounicode.as_ref().map_or(String::new(), |t| {
        format!("/ToUnicode {} 0 R", add_data(b, "", t, f.flate))
    });
    b.add(format!(
        "<< /Type /Font /Subtype /{} /BaseFont /{} {} {widths} {} {tounicode} {} >>",
        f.subtype,
        f.base_font,
        f.encoding
            .as_ref()
            .map_or(String::new(), |e| format!("/Encoding {e}")),
        descriptor.map_or(String::new(), |d| format!("/FontDescriptor {d} 0 R")),
        f.font_extra,
    ))
}

/// Adds a Type0 font object (and its descendant); returns the Type0 font's id.
pub fn add_type0(b: &mut PdfBuilder, f: &Type0Font) -> u32 {
    let program = add_program(b, &f.program, true);
    let descriptor = add_descriptor(b, &f.base_font, f.flags, program, &f.descriptor_extra);
    let cid_to_gid = match &f.cid_to_gid {
        None => String::new(),
        Some(None) => "/CIDToGIDMap /Identity".to_string(),
        Some(Some(map)) => format!("/CIDToGIDMap {} 0 R", b.add_flate("", map)),
    };
    let descendant = b.add(format!(
        "<< /Type /Font /Subtype /{} /BaseFont /{} /CIDSystemInfo << /Registry (Adobe) \
         /Ordering (Identity) /Supplement 0 >> {} {} {} {cid_to_gid} {} >>",
        f.cid_subtype,
        f.base_font,
        descriptor.map_or(String::new(), |d| format!("/FontDescriptor {d} 0 R")),
        f.w.as_ref().map_or(String::new(), |w| format!("/W {w}")),
        f.dw.map_or(String::new(), |d| format!("/DW {d}")),
        f.cid_extra,
    ));
    let tounicode = f.tounicode.as_ref().map_or(String::new(), |t| {
        format!("/ToUnicode {} 0 R", b.add_flate("", t))
    });
    b.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{} /Encoding {} /DescendantFonts [{descendant} 0 R] {tounicode} >>",
        f.base_font, f.encoding
    ))
}

/// A ToUnicode CMap whose body (between `begincmap` boilerplate) is `body`.
pub fn cmap(body: &str) -> Vec<u8> {
    format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n{body}\nendcmap\n\
         CMapName currentdict /CMap defineresource pop\nend\nend\n"
    )
    .into_bytes()
}

/// UTF-16BE hex of `text`.
pub fn utf16_hex(text: &str) -> String {
    text.encode_utf16().map(|u| format!("{u:04X}")).collect()
}

/// A bfchar ToUnicode CMap: `(code, code length in bytes, text)`.
pub fn tounicode_bfchar(entries: &[(u32, usize, &str)]) -> Vec<u8> {
    let two = entries.iter().any(|e| e.1 == 2);
    let space = if two { "<0000> <FFFF>" } else { "<00> <FF>" };
    let mut body = format!("1 begincodespacerange\n{space}\nendcodespacerange\n");
    body.push_str(&format!("{} beginbfchar\n", entries.len()));
    for (code, len, text) in entries {
        body.push_str(&format!(
            "<{:0width$X}> <{}>\n",
            code,
            utf16_hex(text),
            width = len * 2
        ));
    }
    body.push_str("endbfchar");
    cmap(&body)
}

/// The shortest AGL name of `ch`, else `uniXXXX`.
pub fn glyph_name(ch: char) -> String {
    agl::names_for(ch)
        .first()
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("uni{:04X}", u32::from(ch)))
}

/// A TrueType program with one glyph per char of `drawn` (outlined) and `blank` (empty glyf
/// entry), each mapped in the (3,1) cmap and named in `post`. Returns the program and the GIDs.
pub fn latin_truetype(drawn: &str, blank: &str) -> (Vec<u8>, Vec<(char, u16)>) {
    let mut t = TtfBuilder::new();
    let mut gids = Vec::new();
    for (ch, outline) in drawn
        .chars()
        .map(|c| (c, true))
        .chain(blank.chars().map(|c| (c, false)))
    {
        let gid = t.unicode_glyph(ch, &glyph_name(ch), outline);
        gids.push((ch, gid));
    }
    (t.build(), gids)
}

/// A one-page document whose page `/Resources /Font` maps each name to its font object.
pub fn page_with_fonts(mut b: PdfBuilder, fonts: &[(&str, u32)]) -> Vec<u8> {
    let catalog = b.alloc();
    let pages = b.alloc();
    let content = b.add_stream("", b"BT ET");
    let entries: String = fonts
        .iter()
        .map(|(name, id)| format!("/{name} {id} 0 R "))
        .collect();
    let page = b.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 612 792] /Contents {content} 0 R \
         /Resources << /Font << {entries}>> >> >>"
    ));
    b.set(
        pages,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    b.set(catalog, format!("<< /Type /Catalog /Pages {pages} 0 R >>"));
    b.build(&format!("/Root {catalog} 0 R"))
}

/// Reads `pdf` with the production snapshot reader.
pub fn snapshot(pdf: Vec<u8>) -> SourceSnapshot {
    snapshot_from_bytes(std::path::Path::new("fonts-fixture.pdf"), pdf, None)
        .unwrap_or_else(|e| panic!("fixture must open: {e}"))
}

/// The page's fonts, loaded through one `FontCache` with a fresh page budget, in resource order.
pub fn load_page_fonts(snap: &SourceSnapshot) -> Vec<(Vec<u8>, Arc<FontModel>)> {
    let cache = FontCache::new();
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let page = snap
        .doc
        .get_object(snap.pages[0])
        .and_then(Object::as_dict)
        .expect("page");
    let resources = page
        .get(b"Resources")
        .and_then(Object::as_dict)
        .expect("resources");
    let fonts = resources
        .get(b"Font")
        .and_then(Object::as_dict)
        .expect("fonts");
    fonts
        .iter()
        .map(|(name, obj)| {
            let id = obj.as_reference().expect("font reference");
            let dict = snap
                .doc
                .get_object(id)
                .and_then(Object::as_dict)
                .expect("font dict");
            let model = cache.get_or_load(&snap.doc, FontKey::Indirect(id), dict, &mut budget);
            (name.clone(), model)
        })
        .collect()
}

/// Builds a one-font page (`/F1`) and returns the loaded model.
pub fn load_one(b: PdfBuilder, font: u32) -> Arc<FontModel> {
    let snap = snapshot(page_with_fonts(b, &[("F1", font)]));
    load_page_fonts(&snap).remove(0).1
}

/// A simple font loaded on its own.
pub fn load_simple(f: &SimpleFont) -> Arc<FontModel> {
    let mut b = PdfBuilder::new();
    let id = add_simple(&mut b, f);
    load_one(b, id)
}

/// A Type0 font loaded on its own.
pub fn load_type0(f: &Type0Font) -> Arc<FontModel> {
    let mut b = PdfBuilder::new();
    let id = add_type0(&mut b, f);
    load_one(b, id)
}
