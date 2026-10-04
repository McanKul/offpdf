//! FONT-01…40 (SPEC §E.3) and the loader fuzz. This file: shared helpers, base encodings, the
//! AGL, `/Differences`, built-in encodings (Type1, CFF incl. FONT-35/36) and MacRoman (FONT-37).
//! The other groups live in `tests_tounicode.rs`, `tests_presence.rs`, `tests_classes.rs` and
//! `tests_fuzz.rs` (all `#[cfg(test)]`, declared by `fonts/mod.rs`).

use super::agl::{agl_is_sorted, agl_len, agl_value, glyph_name_char};
use super::encodings::BaseEncoding;
use super::{Code, FontClass, FontModel};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::cff::{CffBuilder, CffEncodingSpec};
use crate::pdf_engine::text_edit::testkit::fonts::{
    latin_truetype, load_simple, tounicode_bfchar, Program, SimpleFont,
};
use crate::pdf_engine::text_edit::testkit::type1::{T1Encoding, T1Glyph, Type1Builder};

pub(super) fn c1(v: u8) -> Code {
    Code {
        value: u32::from(v),
        len: 1,
    }
}

pub(super) fn c2(v: u16) -> Code {
    Code {
        value: u32::from(v),
        len: 2,
    }
}

/// The typeable characters as a string, sorted.
pub(super) fn alphabet(m: &FontModel) -> String {
    m.alphabet().iter().map(|(c, _)| c).collect()
}

pub(super) fn typeable(m: &FontModel, code: Code) -> Option<char> {
    m.info(code).and_then(|i| i.typeable_as)
}

/// A WinAnsi simple TrueType font embedding `drawn` (outlined) and `blank` (empty) glyphs, with
/// width 500 for codes 32–255.
pub(super) fn winansi_truetype(base: &str, drawn: &str, blank: &str) -> SimpleFont {
    let (program, _) = latin_truetype(drawn, blank);
    let mut f = SimpleFont::new("TrueType", base);
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(32);
    f.program = Program::TrueType(program);
    f
}

/// A non-embedded Standard-14 font with an explicit `/Encoding`.
pub(super) fn std14(base: &str, encoding: &str) -> SimpleFont {
    let mut f = SimpleFont::new("Type1", base);
    f.encoding = Some(encoding.into());
    f
}

/// lopdf 0.34's own table for a named encoding, through its public `get_font_encoding`.
fn lopdf_table(name: &str) -> [Option<u16>; 256] {
    let doc = lopdf::Document::new();
    let dict = lopdf::dictionary! { "Type" => "Font", "Encoding" => lopdf::Object::Name(name.as_bytes().to_vec()) };
    match dict.get_font_encoding(&doc) {
        Ok(lopdf::Encoding::OneByteEncoding(table)) => *table,
        other => panic!("lopdf table for {name}: {other:?}"),
    }
}

const MAC_OS_ONLY: [u8; 15] = [
    0xAD, 0xB0, 0xB2, 0xB3, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBD, 0xC3, 0xC5, 0xC6, 0xD7, 0xF0,
];

#[test]
fn font_01_encoding_tables_cross_check_and_spot_codes() {
    assert_eq!(agl_len(), 4_495, "FONT-01 AGL size (lopdf glyphnames.rs)");
    assert!(agl_is_sorted(), "FONT-01 AGL sorted for binary search");
    let mut mac_differences = Vec::new();
    for (base, lopdf_name) in [
        (BaseEncoding::WinAnsi, "WinAnsiEncoding"),
        (BaseEncoding::MacRoman, "MacRomanEncoding"),
        (BaseEncoding::Standard, "StandardEncoding"),
    ] {
        let theirs = lopdf_table(lopdf_name);
        for code in 0..=255u8 {
            let ours = base.name(code).map(|n| {
                u16::try_from(agl_value(n).unwrap_or_else(|| panic!("FONT-01 {n} in AGL")))
                    .expect("BMP")
            });
            let lopdf = theirs[usize::from(code)];
            if ours != lopdf {
                assert_eq!(
                    base,
                    BaseEncoding::MacRoman,
                    "FONT-01 {lopdf_name} code {code:#04X}"
                );
                assert!(
                    ours.is_none() && lopdf.is_some(),
                    "FONT-01 MacRoman {code:#04X}"
                );
                mac_differences.push(code);
            }
        }
    }
    assert_eq!(
        mac_differences, MAC_OS_ONLY,
        "FONT-37 the explicit 15-code difference"
    );
    let mac = BaseEncoding::MacRoman;
    let win = BaseEncoding::WinAnsi;
    let std = BaseEncoding::Standard;
    assert_eq!(mac.name(0x80), Some("Adieresis"));
    assert_eq!(mac.name(0x8E), Some("eacute"));
    assert_eq!(mac.name(0xA5), Some("bullet"));
    assert_eq!(mac.name(0xDB), Some("currency"));
    assert_eq!(mac.name(0xAD), None);
    assert_eq!(mac.name(0xF0), None);
    assert_eq!(win.name(0x80), Some("Euro"));
    assert_eq!(win.name(0x8A), Some("Scaron"));
    assert_eq!(win.name(0x9F), Some("Ydieresis"));
    assert_eq!(std.name(0x27), Some("quoteright"));
    assert_eq!(std.name(0xE8), Some("Lslash"));
    for code in 0x80..=0xA0u8 {
        assert_eq!(
            std.name(code),
            None,
            "FONT-01 Standard {code:#04X} undefined"
        );
    }
}

#[test]
fn font_02_agl_turkish_and_typographic_names() {
    for (name, ch) in [
        ("gbreve", 'ğ'),
        ("scedilla", 'ş'),
        ("dotlessi", 'ı'),
        ("Idotaccent", 'İ'),
        ("Scedilla", 'Ş'),
        ("fi", '\u{FB01}'),
        ("quoteright", '’'),
    ] {
        assert_eq!(glyph_name_char(name), Some(ch), "FONT-02 {name}");
    }
    // Through a font: Times (AFM has every one of them) with /Differences.
    let f = std14(
        "Times-Roman",
        "<< /Differences [128 /gbreve /scedilla /dotlessi /Idotaccent /Scedilla /fi /quoteright] >>",
    );
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    let texts: Vec<&str> = (128..=134).map(|c| m.text(c1(c)).unwrap_or("∅")).collect();
    assert_eq!(
        texts,
        ["ğ", "ş", "ı", "İ", "Ş", "fi", "’"],
        "FONT-02 display texts"
    );
    assert_eq!(typeable(&m, c1(128)), Some('ğ'));
    assert_eq!(
        typeable(&m, c1(133)),
        None,
        "FONT-02 the fi ligature is read, never typed"
    );
    assert_eq!(typeable(&m, c1(134)), Some('’'));
}

#[test]
fn font_03_uni_and_u_names_and_suffixed_names() {
    assert_eq!(glyph_name_char("uni011F"), Some('ğ'), "FONT-03 uniXXXX");
    assert_eq!(glyph_name_char("u1F600"), Some('😀'), "FONT-03 uXXXXX");
    assert_eq!(
        glyph_name_char("uni00410042"),
        None,
        "FONT-03 two groups name a sequence"
    );
    assert_eq!(
        glyph_name_char("uniD800"),
        None,
        "FONT-03 surrogates have no value"
    );
    assert_eq!(glyph_name_char("a.sc"), None, "FONT-03 suffixed names");
    assert_eq!(glyph_name_char("u12"), None);
    let mut f = SimpleFont::new("TrueType", "Calibri");
    f.encoding = Some("<< /Differences [65 /uni011F /u1F600 /a.sc] >>".into());
    f.first_char = 65;
    f.widths = Some(vec![500.0, 500.0, 500.0]);
    f.flags = Some(32);
    let m = load_simple(&f);
    assert_eq!(m.class, Some(FontClass::SimpleNonEmbedded));
    assert_eq!(m.text(c1(65)), Some("ğ"));
    assert_eq!(typeable(&m, c1(65)), Some('ğ'), "FONT-03 uni011F writable");
    assert_eq!(m.text(c1(66)), Some("😀"), "FONT-03 u1F600 readable");
    assert_eq!(typeable(&m, c1(66)), None, "FONT-03 non-BMP never typeable");
    assert_eq!(m.text(c1(67)), None, "FONT-03 a.sc has no Unicode");
}

#[test]
fn font_04_differences() {
    let m = load_simple(&std14(
        "Times-Roman",
        "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /gbreve /scedilla 128 /Idotaccent] >>",
    ));
    assert_eq!(m.text(c1(65)), Some("ğ"));
    assert_eq!(m.text(c1(66)), Some("ş"));
    assert_eq!(
        m.text(c1(67)),
        Some("C"),
        "FONT-04 base WinAnsi below the differences"
    );
    assert_eq!(m.text(c1(128)), Some("İ"));
    assert_eq!(
        m.info(c1(65)).and_then(|i| i.glyph_name.clone()).as_deref(),
        Some("gbreve")
    );
    assert!(alphabet(&m).contains('ğ'));
    // No /BaseEncoding on a non-embedded non-symbolic font: StandardEncoding underneath.
    let m = load_simple(&std14("Helvetica", "<< /Differences [65 /Aring] >>"));
    assert_eq!(
        m.text(c1(0x27)),
        Some("’"),
        "FONT-04 implicit Standard base"
    );
    assert_eq!(m.text(c1(65)), Some("Å"));
    for bad in [
        "<< /Differences [65 (A)] >>",
        "<< /Differences [/A 65] >>",
        "<< /Differences [255 /a /b] >>",
        "<< /Differences [-1 /a] >>",
        "<< /Differences 5 >>",
        "<< /BaseEncoding 5 >>",
    ] {
        let m = load_simple(&std14("Helvetica", bad));
        assert_eq!(
            m.refusal,
            Some(TextReason::FontUnsupported),
            "FONT-04 {bad}"
        );
    }
    let m = load_simple(&std14("Helvetica", "/MacExpertEncoding"));
    assert_eq!(
        m.refusal,
        Some(TextReason::UnsupportedEncoding),
        "FONT-04 MacExpert"
    );
    let m = load_simple(&std14(
        "Helvetica",
        "<< /BaseEncoding /MacExpertEncoding >>",
    ));
    assert_eq!(m.refusal, Some(TextReason::UnsupportedEncoding));
}

/// A Type1-subtype font embedding `program`, with /Widths 500 for 32–255 and no /Encoding.
fn embedded_type1(program: Program) -> SimpleFont {
    let mut f = SimpleFont::new("Type1", "ABCDEF+TestSerif");
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(32);
    f.program = program;
    f
}

#[test]
fn font_05_type1_builtin_encoding() {
    let t1 = Type1Builder {
        encoding: T1Encoding::Custom(vec![(65, "A".into()), (66, "gbreve".into())]),
        ..Type1Builder::new("TestSerif")
    }
    .glyph("A", T1Glyph::Box { width: 600 })
    .glyph("gbreve", T1Glyph::Box { width: 500 })
    .glyph("C", T1Glyph::Box { width: 600 })
    .build();
    let m = load_simple(&embedded_type1(Program::Type1(t1)));
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleType1));
    assert_eq!(typeable(&m, c1(65)), Some('A'), "FONT-05 dup 65 /A put");
    assert_eq!(
        typeable(&m, c1(66)),
        Some('ğ'),
        "FONT-05 dup 66 /gbreve put"
    );
    assert_eq!(
        m.text(c1(67)),
        None,
        "FONT-05 code 67 is .notdef in the built-in encoding"
    );
    assert_eq!(alphabet(&m), "Ağ");
    // `/Encoding StandardEncoding def`.
    let t1 = Type1Builder::new("TestSerif")
        .glyph("quoteright", T1Glyph::Box { width: 300 })
        .build();
    let m = load_simple(&embedded_type1(Program::Type1(t1)));
    assert_eq!(
        typeable(&m, c1(0x27)),
        Some('’'),
        "FONT-05 Standard built-in"
    );
}

#[test]
fn font_06_cff_builtin_encoding() {
    let cff = CffBuilder::new("TestSans")
        .glyph("A", true)
        .glyph("B", true)
        .glyph("C", true)
        .encoding(CffEncodingSpec::Format0(vec![0x41, 0x42]))
        .build();
    let m = load_simple(&embedded_type1(Program::Cff(cff)));
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleCff));
    assert_eq!(
        alphabet(&m),
        "AB",
        "FONT-06 format 0: 0x41 → GID 1, 0x42 → GID 2"
    );
    assert_eq!(m.info(c1(0x41)).and_then(|i| i.gid), Some(1));
    let cff = CffBuilder::new("TestSans")
        .glyph("a", true)
        .glyph("b", true)
        .glyph("c", true)
        .encoding(CffEncodingSpec::Format1(vec![(0x61, 2)]))
        .build();
    let m = load_simple(&embedded_type1(Program::Cff(cff)));
    assert_eq!(alphabet(&m), "abc", "FONT-06 format 1 range");
    let cff = CffBuilder::new("TestSans")
        .glyph("quoteright", true)
        .build();
    let m = load_simple(&embedded_type1(Program::Cff(cff)));
    assert_eq!(
        alphabet(&m),
        "’",
        "FONT-06 Standard built-in → names → GIDs"
    );
    // A PDF /Encoding overrides the built-in one: names → glyph_index_by_name.
    let cff = CffBuilder::new("TestSans")
        .glyph("A", true)
        .glyph("gbreve", true)
        .encoding(CffEncodingSpec::Format0(vec![0x41, 0x42]))
        .build();
    let mut f = embedded_type1(Program::Cff(cff));
    f.encoding = Some("<< /BaseEncoding /WinAnsiEncoding /Differences [200 /gbreve] >>".into());
    let m = load_simple(&f);
    assert_eq!(typeable(&m, c1(200)), Some('ğ'));
    assert_eq!(typeable(&m, c1(0x41)), Some('A'));
    assert_eq!(
        typeable(&m, c1(0x42)),
        None,
        "FONT-06 WinAnsi B has no glyph"
    );
    // /Differences without /BaseEncoding sit on the built-in encoding, and a name they set
    // replaces the built-in code → GID mapping.
    let cff = CffBuilder::new("TestSans")
        .glyph("A", true)
        .glyph("B", true)
        .encoding(CffEncodingSpec::Format0(vec![0x41, 0x42]))
        .build();
    let mut f = embedded_type1(Program::Cff(cff));
    f.encoding = Some("<< /Differences [65 /B] >>".into());
    let m = load_simple(&f);
    assert_eq!(
        m.text(c1(0x41)),
        Some("B"),
        "FONT-06 the difference names code 65 B"
    );
    assert_eq!(
        m.info(c1(0x41)).and_then(|i| i.gid),
        Some(2),
        "FONT-06 B's own GID"
    );
    assert_eq!(
        m.info(c1(0x42)).and_then(|i| i.gid),
        Some(2),
        "FONT-06 built-in 0x42"
    );
}

#[test]
fn font_35_cff_custom_encoding_never_falls_back_to_standard() {
    // GID 1 = "B", GID 2 = "A"; the custom encoding maps only 0x42 → GID 1. "A" is in the
    // charset, and StandardEncoding has "A" at 0x41, but no code of this font maps to it.
    let cff = CffBuilder::new("TestSans")
        .glyph("B", true)
        .glyph("A", true)
        .encoding(CffEncodingSpec::Format0(vec![0x42]))
        .build();
    let table = ttf_parser::cff::Table::parse(&cff).expect("CFF parses");
    assert_eq!(
        table.glyph_index(0x41).map(|g| g.0),
        Some(2),
        "FONT-35 precondition: ttf-parser's glyph_index falls back to StandardEncoding"
    );
    let m = load_simple(&embedded_type1(Program::Cff(cff)));
    assert_eq!(m.refusal, None);
    assert_eq!(typeable(&m, c1(0x41)), None, "FONT-35 A is not typeable");
    assert!(!m.drawable(c1(0x41)), "FONT-35 code 0x41 is not drawable");
    assert_eq!(typeable(&m, c1(0x42)), Some('B'));
    assert_eq!(alphabet(&m), "B");
}

#[test]
fn font_36_cff_supplements_and_expert_are_not_typeable() {
    let cff = CffBuilder::new("TestSans")
        .glyph("A", true)
        .glyph("B", true)
        .encoding(CffEncodingSpec::Format0(vec![0x41]))
        .supplement(0x61, "B")
        .build();
    let table = ttf_parser::cff::Table::parse(&cff).expect("CFF parses");
    assert_eq!(
        table.glyph_index(0x61).map(|g| g.0),
        Some(2),
        "precondition: supplement"
    );
    let m = load_simple(&embedded_type1(Program::Cff(cff)));
    assert_eq!(m.refusal, None);
    assert_eq!(
        typeable(&m, c1(0x61)),
        None,
        "FONT-36 supplement code ignored"
    );
    assert_eq!(alphabet(&m), "A", "FONT-36 only the format-0 code types");
    let cff = CffBuilder::new("TestSans")
        .glyph("A", true)
        .encoding(CffEncodingSpec::Expert)
        .build();
    let mut f = embedded_type1(Program::Cff(cff));
    f.tounicode = Some(tounicode_bfchar(&[(0x41, 1, "A")]));
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(
        m.text(c1(0x41)),
        Some("A"),
        "FONT-36 Expert codes readable via ToUnicode"
    );
    assert_eq!(alphabet(&m), "", "FONT-36 Expert encoding types nothing");
}

#[test]
fn font_37_mac_roman_is_strict_annex_d() {
    let m = load_simple(&std14("Helvetica", "/MacRomanEncoding"));
    assert_eq!(m.refusal, None);
    assert_eq!(m.text(c1(0xDB)), Some("¤"), "FONT-37 0xDB is currency");
    assert_eq!(typeable(&m, c1(0xDB)), Some('¤'));
    assert_eq!(
        m.text(c1(0xCA)),
        Some(" "),
        "FONT-37 0xCA is a second space"
    );
    for code in MAC_OS_ONLY {
        assert_eq!(
            m.text(c1(code)),
            None,
            "FONT-37 {code:#04X} undecodable without ToUnicode"
        );
        assert_eq!(
            typeable(&m, c1(code)),
            None,
            "FONT-37 {code:#04X} never typeable"
        );
    }
    // With ToUnicode the code reads, but has no glyph name, so it never types.
    let mut f = std14("Helvetica", "/MacRomanEncoding");
    f.tounicode = Some(tounicode_bfchar(&[(0xAD, 1, "≠"), (0xB9, 1, "π")]));
    let m = load_simple(&f);
    assert_eq!(m.text(c1(0xAD)), Some("≠"));
    assert_eq!(
        typeable(&m, c1(0xAD)),
        None,
        "FONT-37 ToUnicode alone never makes it typeable"
    );
    assert_eq!(m.text(c1(0xB9)), Some("π"));
    assert_eq!(typeable(&m, c1(0xB9)), None);
}
