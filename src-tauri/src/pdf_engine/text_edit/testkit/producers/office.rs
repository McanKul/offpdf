//! Office, browser, TeX and print producers (§E.2, §A.9): FX-WORD, FX-WORD-TR, FX-LIBRE,
//! FX-LIBRE-CFF, FX-SKIA, FX-QUARTZ, FX-PERGLYPH, FX-PDFTEX, FX-XETEX, FX-INDD, FX-STD14,
//! FX-NONEMB, FX-OCR and FX-SHARED, each with the font setup and operator shapes that producer
//! writes.

use super::{refs, tagged_bookmarked, DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::testkit::cff::CffBuilder;
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_simple, add_type0, glyph_name, latin_truetype, tounicode_bfchar, Program, SimpleFont,
    Type0Font,
};
use crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder;
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
use crate::pdf_engine::text_edit::testkit::type1::{T1Encoding, T1Glyph, Type1Builder};

/// Distinct characters of `text`, first-seen order.
pub fn unique(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if !out.contains(ch) {
            out.push(ch);
        }
    }
    out
}

fn bfchar(entries: &[(u32, usize, String)]) -> Vec<u8> {
    let refs: Vec<(u32, usize, &str)> = entries
        .iter()
        .map(|(c, l, t)| (*c, *l, t.as_str()))
        .collect();
    tounicode_bfchar(&refs)
}

/// Word's simple TrueType subset: WinAnsi, `/Widths` for codes 32–255 (500 for the characters of
/// `text`, 0 for every other code), outlined glyphs for the subset, an empty glyph for the space
/// and a ToUnicode for the codes used. ASCII text only.
pub fn word_font(b: &mut PdfBuilder, base: &str, text: &str) -> u32 {
    let chars = unique(&format!("{text} "));
    let drawn: String = chars.chars().filter(|c| *c != ' ').collect();
    let (program, _) = latin_truetype(&drawn, " ");
    let mut widths = vec![0.0; 224];
    let mut map = Vec::new();
    for ch in chars.chars() {
        let code = u32::from(ch);
        if let Some(slot) = code
            .checked_sub(32)
            .and_then(|i| widths.get_mut(i as usize))
        {
            *slot = 500.0;
            map.push((code, 1, ch.to_string()));
        }
    }
    let mut f = SimpleFont::new("TrueType", base);
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(widths);
    f.flags = Some(32);
    f.program = Program::TrueType(program);
    f.tounicode = Some(bfchar(&map));
    add_simple(b, &f)
}

/// A Type0 Identity-H CIDFontType2 subset whose CIDs 1, 2, … are the characters of `chars`
/// (GID = CID), width 500, with a ToUnicode — Word's companion for non-WinAnsi letters, and the
/// only font kind Chrome writes.
pub fn cid_font(b: &mut PdfBuilder, base: &str, chars: &str) -> u32 {
    cid_font_with(b, base, chars, "/Identity-H")
}

/// `cid_font` with another `/Encoding` (e.g. `/Identity-V`).
pub fn cid_font_with(b: &mut PdfBuilder, base: &str, chars: &str, encoding: &str) -> u32 {
    let mut t = TtfBuilder::new();
    let mut map = Vec::new();
    for (i, ch) in chars.chars().enumerate() {
        t.unicode_glyph(ch, &glyph_name(ch), ch != ' ');
        map.push((i as u32 + 1, 2, ch.to_string()));
    }
    let mut f = Type0Font::new("CIDFontType2", base);
    f.encoding = encoding.to_string();
    f.program = Program::TrueType(t.build());
    f.cid_to_gid = Some(None);
    f.flags = Some(32);
    f.w = Some(format!("[1 [{}]]", vec!["500"; map.len()].join(" ")));
    f.tounicode = Some(bfchar(&map));
    add_type0(b, &f)
}

/// The 2-byte hex codes of `text` in a `cid_font` built from `chars`.
pub fn cid_hex(chars: &str, text: &str) -> String {
    text.chars()
        .map(|ch| {
            let cid = chars.chars().position(|c| c == ch).map_or(0, |i| i + 1);
            format!("{cid:04X}")
        })
        .collect()
}

/// FX-WORD: WinAnsi TrueType zeroed subset, `/Widths` 0 for unused codes, ToUnicode, one
/// `BT … Tm [..] TJ ET` per format run (line 1 is "Inv"+12+"oice" and " 2026"), a page-sized
/// `re W n` clip, `/P <</MCID n>> BDC`, tagged and bookmarked.
pub fn word() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = word_font(&mut d.b, "ABCDEF+Calibri", "Invoice 2026 Due 7");
    let page = d.reserve();
    let extra = tagged_bookmarked(&mut d, page, &[0, 1]);
    let content = "/P <</MCID 0>> BDC q 0 0 612 792 re W n \
        BT /F1 11.04 Tf 1 0 0 1 72 700 Tm [(Inv)12(oice)]TJ ET \
        BT /F1 11.04 Tf 1 0 0 1 110.5075 700 Tm [( 2026)]TJ ET Q EMC \
        /P <</MCID 1>> BDC BT /F1 11.04 Tf 1 0 0 1 72 680 Tm [(Due)]TJ ET EMC";
    d.page_at(
        page,
        PageSpec::new(content.as_bytes(), &format!("/Font << /F1 {f1} 0 R >>")).with(&extra),
    );
    d.build()
}

/// The Turkish characters of FX-WORD-TR's Identity-H companion (CIDs 1–4).
pub const WORD_TR_CID_CHARS: &str = "ğışİ";

/// FX-WORD-TR: FX-WORD's font plus a Type0 CIDFontType2 companion with the same BaseFont for
/// the non-WinAnsi letters; "Sağlık Bakanlığı Raporu" as seven per-font `BT … Tm` segments at
/// Word's rounded positions (one lands 0.01 pt late: a converted gap).
pub fn word_tr() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = word_font(&mut d.b, "ABCDEF+Calibri", "Sa lk Bakan Raporu");
    let tr = WORD_TR_CID_CHARS;
    let f2 = cid_font(&mut d.b, "ABCDEF+Calibri", tr);
    let segs: [(&str, String, usize); 7] = [
        ("F1", "(Sa)".into(), 2),
        ("F2", format!("<{}>", cid_hex(tr, "ğ")), 1),
        ("F1", "(l)".into(), 1),
        ("F2", format!("<{}>", cid_hex(tr, "ı")), 1),
        ("F1", "(k Bakanl)".into(), 8),
        ("F2", format!("<{}>", cid_hex(tr, "ığı")), 3),
        ("F1", "( Raporu)".into(), 7),
    ];
    let mut x = 72.0f64;
    let mut body = String::from("/P <</MCID 0>> BDC ");
    for (i, (font, shown, n)) in segs.iter().enumerate() {
        let at = if i == 3 { x + 0.01 } else { x };
        body.push_str(&format!(
            "BT /{font} 11 Tf 1 0 0 1 {at:.2} 700 Tm [{shown}]TJ ET "
        ));
        x += 5.5 * *n as f64;
    }
    body.push_str("EMC");
    let page = d.reserve();
    let extra = tagged_bookmarked(&mut d, page, &[0]);
    d.page_at(
        page,
        PageSpec::new(
            body.as_bytes(),
            &format!("/Font << /F1 {f1} 0 R /F2 {f2} 0 R >>"),
        )
        .with(&extra),
    );
    d.build()
}

/// A symbolic TrueType subset with codes 1..n for the characters of `chars`, glyphs found by
/// the (3,0) cmap at 0xF000 + code (LibreOffice) or the (1,0) cmap at the code (Quartz), no
/// `/Encoding`, and a ToUnicode.
fn symbolic_font(b: &mut PdfBuilder, base: &str, chars: &str, mac_cmap: bool) -> u32 {
    let mut t = TtfBuilder::new();
    let mut map = Vec::new();
    let mut widths = vec![0.0];
    for (i, ch) in chars.chars().enumerate() {
        let code = i as u32 + 1;
        let gid = t.glyph(&glyph_name(ch), ch != ' ', 500);
        if mac_cmap {
            t.cmap10.push((code as u8, gid));
        } else {
            t.cmap30.push((0xF000 + code, gid));
        }
        map.push((code, 1, ch.to_string()));
        widths.push(500.0);
    }
    let mut f = SimpleFont::new("TrueType", base);
    f.flags = Some(4);
    f.first_char = 0;
    f.widths = Some(widths);
    f.program = Program::TrueType(t.build());
    f.tounicode = Some(bfchar(&map));
    add_simple(b, &f)
}

/// 1-byte hex codes of `text` in a `symbolic_font` built from `chars`.
fn sym_hex(chars: &str, text: &str) -> String {
    text.chars()
        .map(|ch| {
            let code = chars.chars().position(|c| c == ch).map_or(0, |i| i + 1);
            format!("{code:02X}")
        })
        .collect()
}

/// FX-LIBRE: symbolic TrueType subset, codes 1..n, `(3,0)` cmap, ToUnicode, hex `TJ` with kerns.
pub fn libre() -> Vec<u8> {
    let chars = unique("Libre text");
    let mut d = DocBuilder::new();
    let f1 = symbolic_font(&mut d.b, "BAAAAA+LiberationSerif", &chars, false);
    let content = format!(
        "BT /F1 12 Tf 72 700 Td [<{}>-20<{}>]TJ ET BT /F1 12 Tf 72 680 Td [<{}>]TJ ET",
        sym_hex(&chars, "Libre"),
        sym_hex(&chars, " text"),
        sym_hex(&chars, "text")
    );
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-LIBRE-CFF: a simple `FontFile3 /Type1C` program, WinAnsi, `/Widths`.
pub fn libre_cff() -> Vec<u8> {
    let cff = CffBuilder::new("CAAAAA+SourceSansPro-Regular")
        .glyph("H", true)
        .glyph("e", true)
        .glyph("l", true)
        .glyph("o", true)
        .glyph("space", false)
        .build();
    let mut widths = vec![0.0; 80]; // codes 32..=111
    for ch in [' ', 'H', 'e', 'l', 'o'] {
        if let Some(w) = widths.get_mut(u32::from(ch) as usize - 32) {
            *w = 500.0;
        }
    }
    let mut f = SimpleFont::new("Type1", "CAAAAA+SourceSansPro-Regular");
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(widths);
    f.flags = Some(32);
    f.program = Program::Cff(cff);
    let mut d = DocBuilder::new();
    let f1 = add_simple(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello Hello) Tj ET",
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-SKIA: Chrome/Docs output — a `1 0 0 -1 0 792 cm` flip with flipped `Tm`, a Type0
/// CIDFontType2 subset, one `Tj` per text node ("Chr" + "ome"), a synthetic-bold `Tr 2` line and
/// a synthetic-italic sheared line.
pub fn skia() -> Vec<u8> {
    let chars = unique("ChromeBoldItalic");
    let mut d = DocBuilder::new();
    let f1 = cid_font(&mut d.b, "AAAAAA+Arimo", &chars);
    let h = |t: &str| cid_hex(&chars, t);
    let content = format!(
        "1 0 0 -1 0 792 cm \
         BT /F1 12 Tf 1 0 0 -1 72 100 Tm <{}> Tj ET BT /F1 12 Tf 1 0 0 -1 90 100 Tm <{}> Tj ET \
         2 Tr 0.36 w BT /F1 12 Tf 1 0 0 -1 72 130 Tm <{}> Tj ET 0 Tr \
         BT /F1 12 Tf 1 0 0.2 -1 72 160 Tm <{}> Tj ET",
        h("Chr"),
        h("ome"),
        h("Bold"),
        h("Italic")
    );
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-QUARTZ: symbolic TrueType without `/Encoding` (glyphs by the (1,0) cmap) + ToUnicode, some
/// glyphs in ops of their own.
pub fn quartz() -> Vec<u8> {
    let chars = unique("Quartz");
    let mut d = DocBuilder::new();
    let f1 = symbolic_font(&mut d.b, "CAAAAA+Helvetica-Light", &chars, true);
    let content = format!(
        "BT /TT1 12 Tf 1 0 0 1 72 700 Tm <{}> Tj 1 0 0 1 90 700 Tm <{}> Tj \
         1 0 0 1 96 700 Tm <{}> Tj ET",
        sym_hex(&chars, "Qua"),
        sym_hex(&chars, "r"),
        sym_hex(&chars, "tz")
    );
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /TT1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-PERGLYPH (`gap: None`): "Per glyph line" as 14 one-glyph `Tm`+`Tj` ops in Courier placed
/// edge to edge (they join). With `gap: Some(g)` every glyph starts `g` pt after the previous one
/// (they do not join: a per-glyph page).
pub fn per_glyph(gap: Option<f64>) -> Vec<u8> {
    let step = gap.unwrap_or(7.2); // Courier: 600/1000 × 12
    let mut content = String::from("BT /F1 12 Tf ");
    for (i, ch) in "Per glyph line".chars().enumerate() {
        let x = 72.0 + step * i as f64;
        content.push_str(&format!("1 0 0 1 {x:.4} 700 Tm ({ch}) Tj "));
    }
    content.push_str("ET");
    let mut d = DocBuilder::new();
    let f =
        d.add("<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// FX-PDFTEX: an embedded Type1 `FontFile` (generated: the repo's Foxit programs are CFF, see
/// DEVIATIONS [T2]) with `/Differences` incl. `fi` and `quoteright`, `/CharSet`, no ToUnicode,
/// no space glyph, and word gaps written as `-333` kerns.
pub fn pdftex() -> Vec<u8> {
    let names: [(u8, &str); 11] = [
        (12, "fi"),
        (39, "quoteright"),
        (72, "H"),
        (87, "W"),
        (100, "d"),
        (101, "e"),
        (108, "l"),
        (110, "n"),
        (111, "o"),
        (114, "r"),
        (116, "t"),
    ];
    let mut t1 = Type1Builder::new("ABCDEF+CMR10");
    t1.encoding = T1Encoding::Custom(names.iter().map(|(c, n)| (*c, n.to_string())).collect());
    for (_, name) in names {
        t1 = t1.glyph(name, T1Glyph::Box { width: 500 });
    }
    let mut widths = vec![0.0; 105]; // codes 12..=116
    for (code, _) in names {
        if let Some(w) = widths.get_mut(usize::from(code) - 12) {
            *w = 500.0;
        }
    }
    let mut f = SimpleFont::new("Type1", "ABCDEF+CMR10");
    f.encoding = Some(
        "<< /Type /Encoding /Differences [12 /fi 39 /quoteright 72 /H 87 /W 100 /d /e 108 /l \
         110 /n /o 114 /r 116 /t] >>"
            .into(),
    );
    f.first_char = 12;
    f.widths = Some(widths);
    f.flags = Some(4);
    f.descriptor_extra = "/CharSet (/H/W/d/e/fi/l/n/o/quoteright/r/t)".into();
    f.program = Program::Type1(t1.build());
    let mut d = DocBuilder::new();
    let f1 = add_simple(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 9.9626 Tf 72 700 Td [(Hello)-333(W)80(orld)]TJ 0 -12 Td \
          [(\\014nd)-333(don\\047t)]TJ ET",
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-XETEX: Type0 Identity-H with a CID-keyed `FontFile3 /CIDFontType0C` program + ToUnicode.
pub fn xetex() -> Vec<u8> {
    let chars = "XeT";
    let cff = CffBuilder::new("ABCDEF+LMRoman10-Regular")
        .cid_glyph(1, true)
        .cid_glyph(2, true)
        .cid_glyph(3, true)
        .build();
    let mut f = Type0Font::new("CIDFontType0", "ABCDEF+LMRoman10-Regular");
    f.program = Program::CidCff(cff);
    f.w = Some("[1 [500 500 500]]".into());
    let map: Vec<(u32, usize, String)> = chars
        .chars()
        .enumerate()
        .map(|(i, c)| (i as u32 + 1, 2, c.to_string()))
        .collect();
    f.tounicode = Some(bfchar(&map));
    let mut d = DocBuilder::new();
    let f1 = add_type0(&mut d.b, &f);
    let content = format!("BT /F1 12 Tf 72 700 Td <{}> Tj ET", cid_hex(chars, "XeTeX"));
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-INDD: a `/Type1C` font with `Tc` tracking, a visible and a hidden `/OC` layer, and one line
/// inside a Form XObject.
pub fn indd() -> Vec<u8> {
    let lines = ["visible layer", "hidden layer", "in a frame"];
    let chars = unique(&lines.concat());
    let mut cff = CffBuilder::new("DAAAAA+MinionPro-Regular");
    for ch in chars.chars() {
        cff = cff.glyph(&glyph_name(ch), ch != ' ');
    }
    let mut widths = vec![0.0; 95]; // codes 32..=126
    for ch in chars.chars() {
        if let Some(w) = widths.get_mut(u32::from(ch) as usize - 32) {
            *w = 500.0;
        }
    }
    let mut f = SimpleFont::new("Type1", "DAAAAA+MinionPro-Regular");
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(widths);
    f.flags = Some(34);
    f.program = Program::Cff(cff.build());
    let mut d = DocBuilder::new();
    let f1 = add_simple(&mut d.b, &f);
    let visible = d.add("<< /Type /OCG /Name (Visible) >>");
    let hidden = d.add("<< /Type /OCG /Name (Hidden) >>");
    let form = d.b.add_stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 50] /Resources << /Font << /F1 {f1} 0 R >> >>"
        ),
        b"BT /F1 12 Tf 10 10 Td (in a frame) Tj ET",
    );
    d.catalog_extra = format!(
        "/OCProperties << /OCGs [{visible} 0 R {hidden} 0 R] /D << /Order [{visible} 0 R {hidden} 0 R] /OFF [{hidden} 0 R] >> >>"
    );
    d.page(PageSpec::new(
        b"/OC /L1 BDC BT /F1 12 Tf 0.12 Tc 72 700 Td (visible layer) Tj ET EMC \
          /OC /L2 BDC BT /F1 12 Tf 72 680 Td (hidden layer) Tj ET EMC \
          q 1 0 0 1 72 600 cm /Fm0 Do Q",
        &format!(
            "/Font << /F1 {f1} 0 R >> /Properties << /L1 {visible} 0 R /L2 {hidden} 0 R >> \
             /XObject << /Fm0 {form} 0 R >>"
        ),
    ));
    d.build()
}

/// FX-STD14: Helvetica, Times-Roman and Courier, not embedded, no `/Widths`.
pub fn std14() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let mut fonts = String::new();
    for (i, base) in ["Helvetica", "Times-Roman", "Courier"].iter().enumerate() {
        let id = d.add(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>"
        ));
        fonts.push_str(&format!("/F{} {id} 0 R ", i + 1));
    }
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Helvetica line) Tj ET BT /F2 12 Tf 72 680 Td (Times line) Tj ET \
          BT /F3 12 Tf 72 660 Td (Courier line) Tj ET",
        &format!("/Font << {fonts}>>"),
    ));
    d.build()
}

/// FX-NONEMB: a TrueType font with `/Widths` but no program (the reader substitutes it).
pub fn nonemb() -> Vec<u8> {
    let mut f = SimpleFont::new("TrueType", "Garamond");
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 95]);
    f.flags = Some(32);
    let mut d = DocBuilder::new();
    let f1 = add_simple(&mut d.b, &f);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Not embedded) Tj ET",
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// FX-OCR: a page image under an invisible (`3 Tr`) text layer in a GlyphLessFont.
pub fn ocr() -> Vec<u8> {
    let mut f = SimpleFont::new("TrueType", "GlyphLessFont");
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 95]);
    f.flags = Some(32);
    let mut d = DocBuilder::new();
    let f1 = add_simple(&mut d.b, &f);
    let img = d.b.add_stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[255, 0, 0, 255],
    );
    d.page(PageSpec::new(
        b"q 612 0 0 792 0 0 cm /Im0 Do Q BT 3 Tr /F1 10 Tf 72 700 Td (scanned words) Tj ET",
        &format!("/Font << /F1 {f1} 0 R >> /XObject << /Im0 {img} 0 R >>"),
    ));
    d.build()
}

/// FX-SHARED: pages 1 and 2 share one `/Contents` stream; pages 3 and 4 share a letterhead part
/// (each also has a body part of its own).
pub fn shared() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let res = format!("/Font << /F1 {f} 0 R >>");
    let body =
        d.b.add_stream("", b"BT /F1 12 Tf 72 700 Td (Shared body) Tj ET");
    let head =
        d.b.add_stream("", b"BT /F1 12 Tf 72 750 Td (Letterhead) Tj ET");
    let spec = PageSpec::new(b"", &res);
    d.page_raw(&refs(&[body]), &spec);
    d.page_raw(&refs(&[body]), &spec);
    for text in ["Body three", "Body four"] {
        let own = d.b.add_stream(
            "",
            format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET").as_bytes(),
        );
        d.page_raw(&refs(&[head, own]), &spec);
    }
    d.build()
}
