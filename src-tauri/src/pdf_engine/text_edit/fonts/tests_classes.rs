//! Font classes and model surfaces: Type0 widths (FONT-20), Standard-14 and other non-embedded
//! fonts (FONT-25…28), faces (FONT-31), sibling surfaces (FONT-32), code preference (FONT-33),
//! plus refusals by class, identity hashing and the cache.

use super::faces::{family_key, is_bold, is_italic, words_of, FaceHints};
use super::std14::{std14_match, Std14Face, Std14Match};
use super::tests::{alphabet, c1, c2, std14, typeable, winansi_truetype};
use super::{face_surface, typing_surface, FamilyHint, FontCache, FontClass, FontKey, FontModel};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::reasons::{Face, TextReason};
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_simple, add_type0, latin_truetype, load_page_fonts, load_simple, load_type0,
    page_with_fonts, snapshot, tounicode_bfchar, Program, SimpleFont, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder;
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
use std::sync::Arc;

fn cid_font_with_widths(w: Option<&str>, dw: Option<f64>) -> Type0Font {
    let mut t = TtfBuilder::new();
    for i in 1..=12 {
        t.glyph(&format!("g{i}"), true, 500);
    }
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Test");
    f.program = Program::TrueType(t.build());
    f.w = w.map(str::to_string);
    f.dw = dw;
    f.tounicode = Some(tounicode_bfchar(&[
        (1, 2, "A"),
        (2, 2, "B"),
        (11, 2, "C"),
        (5, 2, "D"),
    ]));
    f
}

#[test]
fn font_20_type0_w_both_forms_and_dw() {
    let m = load_type0(&cid_font_with_widths(
        Some("[1 [500 600] 10 12 700]"),
        Some(300.0),
    ));
    assert_eq!(m.refusal, None);
    assert_eq!(m.width(c2(1)), 500.0, "FONT-20 c [w1 w2] form");
    assert_eq!(m.width(c2(2)), 600.0);
    assert_eq!(
        m.width(c2(11)),
        700.0,
        "FONT-20 cfirst clast w form (mapped code)"
    );
    assert_eq!(
        m.width(c2(12)),
        700.0,
        "FONT-20 unmapped code inside a range"
    );
    assert_eq!(m.width(c2(5)), 300.0, "FONT-20 /DW");
    assert_eq!(m.width(c2(40)), 300.0);
    assert_eq!(
        m.width(c1(1)),
        0.0,
        "FONT-20 a 1-byte code in a 2-byte font"
    );
    assert_eq!(alphabet(&m), "ABCD");
    let m = load_type0(&cid_font_with_widths(None, None));
    assert_eq!(m.width(c2(1)), 1000.0, "FONT-20 default DW 1000");
    // Later entries win.
    let m = load_type0(&cid_font_with_widths(Some("[1 2 400 2 [650]]"), None));
    assert_eq!((m.width(c2(1)), m.width(c2(2))), (400.0, 650.0));
    for bad in [
        "[1 (x)]",
        "[1 70000 500]",
        "[0 65535 500 0 65535 500]",
        "[1 2]",
        "[(a) [1]]",
        "5",
    ] {
        let m = load_type0(&cid_font_with_widths(Some(bad), None));
        assert_eq!(
            m.refusal,
            Some(TextReason::FontUnsupported),
            "FONT-20 /W {bad}"
        );
    }
}

#[test]
fn font_20b_real_full_truetype_as_cid_font_type2() {
    let noto = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/fonts/NotoSans-Regular.ttf"),
    )
    .expect("NotoSans-Regular.ttf in the repo");
    let face = ttf_parser::Face::parse(&noto, 0).expect("Noto parses");
    let text = "Ağış İ";
    let entries: Vec<(u32, usize, String)> = text
        .chars()
        .map(|ch| {
            (
                u32::from(face.glyph_index(ch).expect("glyph").0),
                2,
                ch.to_string(),
            )
        })
        .collect();
    let refs: Vec<(u32, usize, &str)> = entries
        .iter()
        .map(|(c, l, t)| (*c, *l, t.as_str()))
        .collect();
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+NotoSans-Regular");
    f.program = Program::TrueType(noto.clone());
    f.dw = Some(600.0);
    f.cid_to_gid = Some(None);
    f.tounicode = Some(tounicode_bfchar(&refs));
    let m = load_type0(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(
        alphabet(&m),
        " AğİIş".replace('I', "ı"),
        "FONT-20b every char of {text}"
    );
    assert_eq!(
        m.ascent, 0.8,
        "FONT-20b the descriptor's /Ascent wins over hhea"
    );
}

#[test]
fn font_25_std14_widths_per_face() {
    let times = load_simple(&std14("Times-Roman", "/WinAnsiEncoding"));
    let helv = load_simple(&std14("Helvetica", "/WinAnsiEncoding"));
    let times_bold = load_simple(&std14("Times-Bold", "/WinAnsiEncoding"));
    let courier = load_simple(&std14("Courier", "/WinAnsiEncoding"));
    assert_eq!(times.class, Some(FontClass::Std14(Std14Face::TimesRoman)));
    assert_eq!(times.width(c1(b'a')), 444.0, "FONT-25 Times a");
    assert_eq!(
        helv.width(c1(b'a')),
        556.0,
        "FONT-25 Helvetica a (not Times)"
    );
    assert_eq!(times_bold.width(c1(b'a')), 500.0, "FONT-25 Times-Bold a");
    assert_eq!(courier.width(c1(b'W')), 600.0);
    assert!(!times.substituted && !times.embedded);
    assert_eq!((times.ascent, times.descent), (0.683, -0.217));
    assert_eq!(times.family_hint, FamilyHint::Serif);
    assert_eq!(courier.family_hint, FamilyHint::Mono);
    assert!(times_bold.bold && !times_bold.italic);
    assert!("AZaz09".chars().all(|ch| alphabet(&times).contains(ch)));
    let m = load_simple(&std14(
        "Times-Roman",
        "<< /Differences [128 /gbreve /Gamma] >>",
    ));
    assert_eq!(
        typeable(&m, c1(128)),
        Some('ğ'),
        "FONT-25 gbreve is in the Core-14 AFM"
    );
    assert_eq!(m.text(c1(129)), Some("Γ"));
    assert!(
        !m.drawable(c1(129)),
        "FONT-25 Gamma is not in Times' glyph set"
    );
    // /Widths, when present, win over the AFM.
    let mut f = std14("Times-Roman", "/WinAnsiEncoding");
    f.first_char = 32;
    f.widths = Some(vec![300.0; 224]);
    assert_eq!(load_simple(&f).width(c1(b'a')), 300.0);
    // No /Encoding: StandardEncoding.
    let m = load_simple(&SimpleFont::new("Type1", "Helvetica"));
    assert_eq!(m.text(c1(0x27)), Some("’"));
    assert_eq!(m.text(c1(0xE8)), Some("Ł"));
}

#[test]
fn font_26_std14_aliases() {
    for (name, face) in [
        ("Arial", Std14Face::Helvetica),
        ("ArialMT", Std14Face::Helvetica),
        ("Arial,Bold", Std14Face::HelveticaBold),
        ("Arial-BoldMT", Std14Face::HelveticaBold),
        ("Arial,Italic", Std14Face::HelveticaOblique),
        ("Arial-ItalicMT", Std14Face::HelveticaOblique),
        ("Arial,BoldItalic", Std14Face::HelveticaBoldOblique),
        ("Arial-BoldItalicMT", Std14Face::HelveticaBoldOblique),
        ("TimesNewRoman", Std14Face::TimesRoman),
        ("TimesNewRomanPS", Std14Face::TimesRoman),
        ("TimesNewRomanPSMT", Std14Face::TimesRoman),
        ("TimesNewRomanPS-BoldMT", Std14Face::TimesBold),
        ("TimesNewRomanPS-ItalicMT", Std14Face::TimesItalic),
        ("TimesNewRomanPS-BoldItalicMT", Std14Face::TimesBoldItalic),
        ("TimesNewRoman,Bold", Std14Face::TimesBold),
        ("TimesNewRoman,Italic", Std14Face::TimesItalic),
        ("TimesNewRoman,BoldItalic", Std14Face::TimesBoldItalic),
        ("CourierNew", Std14Face::Courier),
        ("CourierNewPSMT", Std14Face::Courier),
        ("CourierNewPS-BoldMT", Std14Face::CourierBold),
        ("CourierNewPS-ItalicMT", Std14Face::CourierOblique),
        ("CourierNewPS-BoldItalicMT", Std14Face::CourierBoldOblique),
        ("CourierNew,Bold", Std14Face::CourierBold),
        ("CourierNew,Italic", Std14Face::CourierOblique),
        ("CourierNew,BoldItalic", Std14Face::CourierBoldOblique),
        ("Times New Roman", Std14Face::TimesRoman),
    ] {
        assert_eq!(
            std14_match(name),
            Some(Std14Match::Latin(face)),
            "FONT-26 {name}"
        );
    }
    assert_eq!(
        std14_match("ArialNarrow"),
        None,
        "FONT-26 Arial Narrow's metrics differ"
    );
    let m = load_simple(&std14("Arial,Bold", "/WinAnsiEncoding"));
    assert_eq!(m.class, Some(FontClass::Std14(Std14Face::HelveticaBold)));
    assert_eq!(m.width(c1(b'a')), 556.0);
    assert!(m.bold);
    let m = load_simple(&std14("CourierNewPSMT", "/WinAnsiEncoding"));
    assert_eq!(m.width(c1(b'i')), 600.0, "FONT-26 Courier widths");
    let m = load_simple(&std14("ArialNarrow", "/WinAnsiEncoding"));
    assert_eq!(
        m.refusal,
        Some(TextReason::MissingWidths),
        "FONT-26 not Standard-14"
    );
}

#[test]
fn font_27_non_embedded_non_std14() {
    let mut f = SimpleFont::new("TrueType", "Calibri");
    f.encoding =
        Some("<< /BaseEncoding /WinAnsiEncoding /Differences [128 /gbreve /uni4E00 /x] >>".into());
    f.first_char = 32;
    let mut widths = vec![500.0; 224];
    widths[usize::from(b'x' - 32)] = 0.0;
    widths[130 - 32] = 0.0;
    f.widths = Some(widths);
    f.flags = Some(32);
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleNonEmbedded));
    assert!(m.substituted && !m.embedded, "FONT-27 flagged substituted");
    assert_eq!(typeable(&m, c1(b'A')), Some('A'));
    assert_eq!(typeable(&m, c1(128)), Some('ğ'), "FONT-27 Latin Extended-A");
    assert_eq!(m.text(c1(129)), Some("一"), "FONT-27 CJK reads");
    assert_eq!(
        typeable(&m, c1(129)),
        None,
        "FONT-27 outside the Latin/Greek/Cyrillic list"
    );
    assert_eq!(typeable(&m, c1(b'x')), None, "FONT-27 width 0");
    f.widths = None;
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::MissingWidths),
        "FONT-27 no /Widths"
    );
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(4);
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::UnsupportedEncoding),
        "FONT-27 symbolic"
    );
}

#[test]
fn font_28_symbol_and_zapf_dingbats_are_unsupported_encodings() {
    for name in ["Symbol", "ZapfDingbats", "Symbol,Bold"] {
        let m = load_simple(&SimpleFont::new("Type1", name));
        assert_eq!(
            m.refusal,
            Some(TextReason::UnsupportedEncoding),
            "FONT-28 {name}"
        );
        assert_eq!(m.class, None);
    }
}

#[test]
fn font_28b_class_refusals() {
    let mut t3 = SimpleFont::new("Type3", "T3");
    t3.encoding = Some("<< /Differences [65 /A /B] >>".into());
    t3.first_char = 65;
    t3.widths = Some(vec![700.0, 0.6]);
    t3.font_extra = "/FontMatrix [0.001 0 0 0.001 0 0] /FontBBox [0 0 1000 1000]".into();
    let m = load_simple(&t3);
    assert_eq!(m.refusal, Some(TextReason::Type3));
    assert_eq!(
        m.width(c1(b'A')),
        700.0,
        "Type3 widths scale by /FontMatrix"
    );
    assert_eq!(m.text(c1(b'A')), Some("A"), "Type3 codes still read");
    assert_eq!(alphabet(&m), "");
    t3.font_extra = "/FontMatrix [1 0 0 1 0 0]".into();
    assert!((load_simple(&t3).width(c1(b'B')) - 600.0).abs() < 1e-3);
    let m = load_simple(&SimpleFont::new("MMType1", "Minion_MM"));
    assert_eq!(m.refusal, Some(TextReason::FontUnsupported), "MMType1");
    let m = load_simple(&SimpleFont::new("OpenType", "X"));
    assert_eq!(
        m.refusal,
        Some(TextReason::FontUnsupported),
        "unknown subtype"
    );
    let mut f = std14("Helvetica", "/WinAnsiEncoding");
    f.first_char = 32;
    f.widths = Some(vec![500.0; 10]);
    f.font_extra = "/LastChar 50".into();
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::FontUnsupported),
        "/Widths length"
    );
    f.widths = Some(vec![500.0]);
    f.font_extra.clear();
    f.descriptor_extra = "/Flags (x)".into();
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::FontUnsupported),
        "/Flags type"
    );
    let base = cid_font_with_widths(None, None);
    for (encoding, reason) in [
        ("/Identity-V", TextReason::Vertical),
        ("/90ms-RKSJ-V", TextReason::Vertical),
        ("/UniJIS-UCS2-H", TextReason::UnsupportedEncoding),
        ("(x)", TextReason::FontUnsupported),
    ] {
        let mut f = base.clone();
        f.encoding = encoding.into();
        let m = load_type0(&f);
        assert_eq!(m.refusal, Some(reason), "Type0 /Encoding {encoding}");
        assert_eq!(m.vertical, reason == TextReason::Vertical);
    }
    let mut f = base.clone();
    f.cid_extra = "/WMode 1".into();
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::Vertical),
        "CIDFont /WMode 1"
    );
    let mut f = base.clone();
    f.tounicode = None;
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::NoTounicode),
        "Type0 without ToUnicode"
    );
    let m = load_type0(&base);
    assert_eq!(
        m.split_codes(&[0, 1, 0]),
        Err(TextReason::AmbiguousUnicode),
        "odd 2-byte string"
    );
    assert_eq!(m.split_codes(&[0, 1, 0, 2]), Ok(vec![c2(1), c2(2)]));
    assert!(!m.is_word_space(c2(32)));
    let simple = load_simple(&std14("Helvetica", "/WinAnsiEncoding"));
    assert_eq!(simple.split_codes(b"a "), Ok(vec![c1(b'a'), c1(b' ')]));
    assert!(simple.is_word_space(c1(32)));
    assert_eq!(simple.code_bytes(c1(0x41)), vec![0x41]);
}

#[test]
fn font_31_faces() {
    let pairs = [
        ("TimesNewRomanPS-BoldMT", "TimesNewRomanPSMT"),
        ("Arial-BoldMT", "ArialMT"),
        ("Arial-ItalicMT", "ArialMT"),
        ("BAAAAA+Times New Roman-Bold", "CAAAAA+Times New Roman"),
        ("NZDDWO+Liberation-Sans-Bold", "VRMYHF+Liberation-Sans"),
        ("OSDNAA+Liberation-Sans-Italic", "VRMYHF+Liberation-Sans"),
        ("PHVFWU+Carlito-Bold", "GICQRP+Carlito"),
        ("BCDEEE+Aptos-Bold", "BCDFEE+Aptos"),
        ("AAAAAB+SourceSansPro-Bold", "AAAAAD+SourceSansPro-Regular"),
        ("MGAFOB+Helvetica-Oblique", "AHLTJX+Helvetica"),
        ("Helvetica-Bold", "Helvetica"),
        ("NILLFH+DejaVu-Sans-Mono-Bold", "IKCJUC+DejaVu-Sans-Mono"),
    ];
    for (styled, plain) in pairs {
        assert_eq!(
            family_key(styled),
            family_key(plain),
            "FONT-31 {styled} ~ {plain}"
        );
    }
    assert_ne!(family_key("Arial-BoldMT"), family_key("TimesNewRomanPSMT"));
    assert_ne!(family_key("Carlito-Bold"), family_key("Caladea-Bold"));
    assert_eq!(family_key("ABCDEF+"), "", "FONT-31 a bare subset tag");
    assert_eq!(
        words_of("TimesNewRomanPS-BoldMT"),
        ["times", "new", "roman", "ps", "bold", "mt"]
    );
    let none = FaceHints::default();
    assert!(is_bold("TimesNewRomanPS-BoldMT", &none));
    assert!(!is_bold("TimesNewRomanPSMT", &none));
    assert!(is_italic("MGAFOB+Helvetica-Oblique", &none));
    assert!(
        is_bold("WIVRPB+Arial,Bold", &none),
        "FONT-31 comma separator"
    );
    assert!(
        !is_bold("Boldoni", &none) && !is_bold("ABCDEF+Boldoni", &none),
        "FONT-31 Boldoni"
    );
    let flags = |f| FaceHints {
        flags: Some(f),
        ..none
    };
    let stem = |s| FaceHints {
        stem_v: Some(s),
        ..none
    };
    let angle = |a| FaceHints {
        italic_angle: Some(a),
        ..none
    };
    assert!(
        is_bold("VULAQV+SegoeUI", &flags(1 << 18)),
        "FONT-31 ForceBold"
    );
    assert!(is_bold("VULAQV+SegoeUI", &stem(165.0)), "FONT-31 StemV");
    assert!(
        is_italic("VULAQV+SegoeUI", &angle(-12.0)),
        "FONT-31 ItalicAngle"
    );
    assert!(!is_italic("VULAQV+SegoeUI", &angle(0.0)));
    assert!(
        is_italic("VULAQV+SegoeUI", &flags(1 << 6)),
        "FONT-31 Italic flag"
    );
    let thick_italic = FaceHints {
        stem_v: Some(140.0),
        italic_angle: Some(-15.0),
        ..none
    };
    assert!(is_italic("IPRNUP+TimesNewRomanPS-ItalicMT", &thick_italic));
    assert!(
        !is_bold("IPRNUP+TimesNewRomanPS-ItalicMT", &thick_italic),
        "FONT-31 italic stem"
    );
    assert!(
        !is_bold("AAAAAD+SourceSansPro-Regular", &stem(200.0)),
        "FONT-31 Regular wins"
    );
    assert!(!is_italic("AAAAAD+SourceSansPro-Regular", &angle(-12.0)));
    assert!(
        is_bold("Liberation-Sans-Bold-Italic", &none)
            && is_italic("Liberation-Sans-Bold-Italic", &none)
    );
    let bi = FaceHints {
        stem_v: Some(160.0),
        italic_angle: Some(-12.0),
        ..none
    };
    assert!(is_bold("AAAAAA+Liberation-Sans-BoldItalic", &bi));
    assert!(is_italic("AAAAAA+Liberation-Sans-BoldItalic", &bi));
    // Through a loaded model: descriptor flags drive the family hint and faces.
    let mut f = winansi_truetype("ABCDEF+SegoeUI", "A", "");
    f.flags = Some(32 | 1 << 18 | 2);
    let m = load_simple(&f);
    assert!(m.bold && !m.italic);
    assert_eq!(m.family_hint, FamilyHint::Serif);
    assert_eq!(m.display_name, "Segoe UI");
    assert_eq!(m.base_name, "SegoeUI");
}

/// A Type0 CIDFontType2 named `base` typing `chars` (GID = CID = 1…).
fn word_companion(base: &str, chars: &str) -> Type0Font {
    let (program, gids) = latin_truetype(chars, "");
    let entries: Vec<(u32, usize, String)> = gids
        .iter()
        .map(|(ch, gid)| (u32::from(*gid), 2, ch.to_string()))
        .collect();
    let refs: Vec<(u32, usize, &str)> = entries
        .iter()
        .map(|(c, l, t)| (*c, *l, t.as_str()))
        .collect();
    let mut f = Type0Font::new("CIDFontType2", base);
    f.flags = Some(32);
    f.program = Program::TrueType(program);
    f.tounicode = Some(tounicode_bfchar(&refs));
    f
}

fn page(fonts: Vec<(&str, FontSpec)>) -> Vec<(Vec<u8>, Arc<FontModel>)> {
    let mut b = PdfBuilder::new();
    let ids: Vec<(&str, u32)> = fonts
        .iter()
        .map(|(name, spec)| {
            let id = match spec {
                FontSpec::Simple(f) => add_simple(&mut b, f),
                FontSpec::Type0(f) => add_type0(&mut b, f),
            };
            (*name, id)
        })
        .collect();
    load_page_fonts(&snapshot(page_with_fonts(b, &ids)))
}

enum FontSpec {
    Simple(SimpleFont),
    Type0(Type0Font),
}

fn names(surface: &super::TypingSurface) -> Vec<String> {
    surface
        .fonts
        .iter()
        .map(|(n, _)| String::from_utf8_lossy(n).into_owned())
        .collect()
}

#[test]
fn font_32_sibling_groups_word_pattern() {
    let mut refused = winansi_truetype("BCDEFG+Calibri", "Sa", "");
    refused.widths = None;
    let fonts = page(vec![
        (
            "F1",
            FontSpec::Simple(winansi_truetype("ABCDEF+Calibri", "Salk B", "")),
        ),
        (
            "F2",
            FontSpec::Type0(word_companion("ABCDEF+Calibri", "ğış")),
        ),
        (
            "F3",
            FontSpec::Simple(winansi_truetype("ABCDEF+Calibri-Bold", "Sa", "")),
        ),
        (
            "F4",
            FontSpec::Type0(word_companion("ABCDEF+Calibri-Bold", "ğ")),
        ),
        (
            "F5",
            FontSpec::Simple(winansi_truetype("ABCDEF+Cambria", "Sa", "")),
        ),
        ("F6", FontSpec::Simple(refused)),
        (
            "F7",
            FontSpec::Type0(word_companion("ABCDEF+Calibri-Light", "ğ")),
        ),
    ]);
    assert_eq!(fonts[5].1.refusal, Some(TextReason::MissingWidths));
    let surface = typing_surface(&fonts, b"F1");
    assert_eq!(
        names(&surface),
        ["F1", "F2"],
        "FONT-32 Word pair, no bold/other family/refused"
    );
    assert_eq!(
        surface.alphabet().into_iter().collect::<String>(),
        " BSaklğış"
    );
    assert_eq!(surface.writer_for('a'), Some((0, c1(b'a'))));
    let (idx, code) = surface.writer_for('ğ').expect("ğ");
    assert_eq!(
        (idx, code.len),
        (1, 2),
        "FONT-32 ğ from the Type0 companion"
    );
    assert!(surface.has_space());
    assert_eq!(
        names(&typing_surface(&fonts, b"F2")),
        ["F2", "F1"],
        "FONT-32 Type0 primary"
    );
    assert!(
        typing_surface(&fonts, b"F9").fonts.is_empty(),
        "unknown primary"
    );
    assert_eq!(
        names(&typing_surface(&fonts, b"F6")),
        ["F6"],
        "refused primary types alone"
    );
    let bold = face_surface(&fonts, &fonts[0].1, Face::Bold).expect("FONT-32 bold group");
    assert_eq!(
        names(&bold),
        ["F3", "F4"],
        "FONT-32 the bold group, led by the simple font"
    );
    assert!(
        face_surface(&fonts, &fonts[0].1, Face::Italic).is_none(),
        "no italic face"
    );
    let regular = face_surface(&fonts, &fonts[2].1, Face::Regular).expect("regular");
    assert_eq!(names(&regular), ["F1", "F2"]);
    // A simple font is never swapped for a lone Type0 face of another name.
    let fonts = page(vec![
        (
            "F1",
            FontSpec::Simple(winansi_truetype("ABCDEF+Aptos", "Sa", "")),
        ),
        (
            "F2",
            FontSpec::Type0(word_companion("ABCDEF+Aptos-Bold", "ğ")),
        ),
    ]);
    assert!(
        face_surface(&fonts, &fonts[0].1, Face::Bold).is_none(),
        "FONT-32 no swap"
    );
    assert_eq!(names(&typing_surface(&fonts, b"F1")), ["F1"]);
}

#[test]
fn font_33_encode_preference_order() {
    let fonts = page(vec![
        (
            "F1",
            FontSpec::Simple(std14("Helvetica", "<< /Differences [65 /A 200 /A] >>")),
        ),
        (
            "F2",
            FontSpec::Simple(std14("Helvetica-Bold", "/WinAnsiEncoding")),
        ),
        (
            "F3",
            FontSpec::Simple(std14("Helvetica", "<< /Differences [66 /A] >>")),
        ),
    ]);
    let m = &fonts[0].1;
    assert_eq!(
        m.code_for('A', &[], &[]),
        Some(c1(65)),
        "FONT-33 lowest code"
    );
    assert_eq!(
        m.code_for('A', &[], &[('A', c1(200))]),
        Some(c1(200)),
        "FONT-33 page code"
    );
    assert_eq!(
        m.code_for('A', &[('A', c1(65))], &[('A', c1(200))]),
        Some(c1(65)),
        "FONT-33 run code beats page code"
    );
    assert_eq!(
        m.code_for('A', &[('A', c1(66))], &[]),
        Some(c1(65)),
        "FONT-33 invalid preference"
    );
    assert_eq!(
        m.code_for('A', &[('B', c1(200))], &[]),
        Some(c1(65)),
        "other char"
    );
    assert_eq!(m.code_for('ğ', &[], &[]), None);
    let surface = typing_surface(&fonts, b"F1");
    assert_eq!(names(&surface), ["F1", "F3"], "FONT-33 same face only");
    assert_eq!(surface.writer_for('A'), Some((0, c1(65))));
    assert_eq!(
        surface.writer_for_with('A', &[(1, 'A', c1(66))], &[]),
        Some((1, c1(66)))
    );
    assert_eq!(
        surface.writer_for_with('A', &[], &[(0, 'A', c1(200))]),
        Some((0, c1(200))),
        "FONT-33 page preference"
    );
    assert_eq!(
        surface.writer_for_with('A', &[(1, 'A', c1(200))], &[]),
        Some((0, c1(65))),
        "FONT-33 a preference the font cannot honour is ignored"
    );
}

#[test]
fn font_identity_hash_cache_and_budget() {
    let font = |padding: usize, flate: bool, width: f64| {
        let mut b = PdfBuilder::new();
        for _ in 0..padding {
            b.add("<< /Padding true >>");
        }
        let mut f = winansi_truetype("ABCDEF+Calibri", "AB", "");
        f.flate = flate;
        f.widths = Some(vec![width; 224]);
        let id = add_simple(&mut b, &f);
        let fonts = load_page_fonts(&snapshot(page_with_fonts(b, &[("F1", id)])));
        fonts[0].1.content_hash
    };
    let base = font(0, true, 500.0);
    assert_eq!(
        base,
        font(7, true, 500.0),
        "renumbered objects hash alike (D32)"
    );
    assert_eq!(base, font(0, false, 500.0), "streams hash by decoded bytes");
    assert_ne!(
        base,
        font(0, true, 501.0),
        "a changed width changes the hash"
    );
    // The cache returns the same model for the same key; a budget-starved load is not cached.
    let mut b = PdfBuilder::new();
    let id = add_simple(&mut b, &winansi_truetype("ABCDEF+Calibri", "AB", ""));
    let snap = snapshot(page_with_fonts(b, &[("F1", id)]));
    let font_id = snap
        .doc
        .objects
        .keys()
        .copied()
        .find(|k| k.0 == id)
        .expect("id");
    let dict = snap
        .doc
        .get_object(font_id)
        .and_then(lopdf::Object::as_dict)
        .expect("dict");
    let cache = FontCache::new();
    let starved = cache.get_or_load(
        &snap.doc,
        FontKey::Indirect(font_id),
        dict,
        &mut DecodeBudget::new(100),
    );
    assert_eq!(
        starved.refusal,
        Some(TextReason::PageTooComplex),
        "budget exhausted"
    );
    let mut budget = DecodeBudget::new(1 << 20);
    let first = cache.get_or_load(&snap.doc, FontKey::Indirect(font_id), dict, &mut budget);
    assert_eq!(first.refusal, None, "the starved model was not cached");
    assert_eq!(first.key, FontKey::Indirect(font_id));
    let used = (1 << 20) - budget.remaining();
    assert!(used > 0);
    let again = cache.get_or_load(&snap.doc, FontKey::Indirect(font_id), dict, &mut budget);
    assert!(Arc::ptr_eq(&first, &again), "cached");
    assert_eq!(
        (1 << 20) - budget.remaining(),
        used,
        "a cached model decodes nothing"
    );
    let direct = FontKey::direct(font_id, b"F1");
    assert_ne!(direct, FontKey::direct(font_id, b"F2"));
    let other = cache.get_or_load(&snap.doc, direct, dict, &mut budget);
    assert!(!Arc::ptr_eq(&first, &other), "keys are distinct entries");
    assert_eq!(first.content_hash, other.content_hash);
}

#[test]
fn family_hint_reads_the_name_when_the_flags_say_nothing() {
    // Live check B3: LibreOffice writes `/Flags 4` (Symbolic only), so every Liberation Serif
    // line was "sans" and the editor set it in Arial. The FixedPitch and Serif flags still win.
    use super::faces::family_hint;
    let cases: [(&str, Option<i64>, FamilyHint); 12] = [
        ("BAAAAA+LiberationSerif", Some(4), FamilyHint::Serif),
        ("CAAAAA+LiberationSerif-Bold", Some(4), FamilyHint::Serif),
        ("DAAAAA+LiberationSans", Some(4), FamilyHint::Sans),
        ("LiberationMono", Some(4), FamilyHint::Mono),
        ("DejaVuSansMono", None, FamilyHint::Mono),
        ("TimesNewRomanPSMT", Some(32), FamilyHint::Serif),
        ("Georgia,Italic", None, FamilyHint::Serif),
        ("Microsoft Sans Serif", None, FamilyHint::Sans),
        ("CenturyGothic", None, FamilyHint::Sans),
        ("Consolas", Some(32), FamilyHint::Mono),
        ("Arial", Some(2), FamilyHint::Serif),
        ("ABCDEF+Lato-Regular", Some(1), FamilyHint::Mono),
    ];
    for (name, flags, want) in cases {
        assert_eq!(family_hint(name, flags), want, "{name} {flags:?}");
    }
    let mut f = winansi_truetype("BAAAAA+LiberationSerif", "A", "");
    f.flags = Some(4);
    assert_eq!(
        load_simple(&f).family_hint,
        FamilyHint::Serif,
        "through a load"
    );
}
