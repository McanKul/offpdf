//! Glyph presence from the embedded program (§A.2, §B.9.5): FONT-16…19, 21…24, 29, 30, 38…40,
//! plus OpenType (glyf and CFF flavours) and the real FoxitSerif CFF program.

use super::tests::{alphabet, c1, c2, typeable, winansi_truetype};
use super::{FontClass, FontModel};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::cff::{CffBuilder, CffEncodingSpec};
use crate::pdf_engine::text_edit::testkit::fonts::{
    latin_truetype, load_simple, load_type0, tounicode_bfchar, Program, SimpleFont, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
use crate::pdf_engine::text_edit::testkit::type1::{op, T1Encoding, T1Glyph, Type1Builder};
use std::sync::Arc;

#[test]
fn font_16_b3_subset_without_y_is_not_typeable() {
    let m = load_simple(&winansi_truetype("ABCDEF+Calibri", "Helo", "Y "));
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleTrueType));
    assert!(m.subset, "FONT-16 subset tag");
    assert_eq!(
        alphabet(&m),
        " Helo",
        "FONT-16 B3: the zeroed Y glyph is not typeable"
    );
    let y = m.info(c1(b'Y')).expect("code Y");
    assert_eq!(y.text.as_deref(), Some("Y"), "FONT-16 Y still reads");
    assert!(
        y.gid.is_some() && !y.drawable,
        "FONT-16 Y has a GID but no outline"
    );
    assert!(!m.drawable(c1(b'Z')), "FONT-16 a code with no glyph at all");
    // The space has an empty glyph and width 500: whitespace with a width is drawable.
    assert!(m.drawable(c1(b' ')));
    assert_eq!(m.code_for(' ', &[], &[]), Some(c1(b' ')));
}

#[test]
fn font_17_zero_width_is_not_typeable() {
    let mut f = winansi_truetype("ABCDEF+Calibri", "AB ", "");
    let widths = f.widths.as_mut().expect("widths");
    widths[usize::from(b'A' - 32)] = 0.0;
    widths[usize::from(b' ' - 32)] = 0.0;
    let m = load_simple(&f);
    assert!(m.drawable(c1(b'A')), "FONT-17 the glyph itself is present");
    assert_eq!(typeable(&m, c1(b'A')), None, "FONT-17 width 0");
    assert_eq!(
        typeable(&m, c1(b' ')),
        None,
        "FONT-17 a zero-width space either"
    );
    assert_eq!(typeable(&m, c1(b'B')), Some('B'));
    assert_eq!(m.width(c1(b'A')), 0.0);
    // An empty space glyph is drawable only with a positive width (§A.2).
    let mut f = winansi_truetype("ABCDEF+Calibri", "AB", " ");
    f.widths.as_mut().expect("widths")[0] = 0.0;
    assert!(
        !load_simple(&f).drawable(c1(b' ')),
        "FONT-17 blank space, width 0"
    );
    f.widths.as_mut().expect("widths")[0] = 250.0;
    assert!(
        load_simple(&f).drawable(c1(b' ')),
        "FONT-17 blank space, width 250"
    );
}

#[test]
fn font_18_missing_width_only_when_present() {
    let mut f = winansi_truetype("ABCDEF+Calibri", "ABC", "");
    f.first_char = 65;
    f.widths = Some(vec![600.0, 650.0]);
    let m = load_simple(&f);
    assert_eq!(m.width(c1(b'B')), 650.0);
    assert_eq!(
        m.width(c1(b'C')),
        0.0,
        "FONT-18 outside the range, no /MissingWidth: 0"
    );
    assert_eq!(m.info(c1(b'C')).and_then(|i| i.width1000), None);
    assert_eq!(
        typeable(&m, c1(b'C')),
        None,
        "FONT-18 unknown width is never typeable"
    );
    f.descriptor_extra = "/MissingWidth 420".into();
    let m = load_simple(&f);
    assert_eq!(
        m.width(c1(b'C')),
        420.0,
        "FONT-18 /MissingWidth when present"
    );
    assert_eq!(typeable(&m, c1(b'C')), Some('C'));
}

#[test]
fn font_19_strategy_disagreement_is_not_drawable() {
    // (3,1) maps 'A' to GID 1 ("A.alt"), while the post table names GID 2 "A".
    let mut t = TtfBuilder::new();
    let alt = t.glyph("Aalt", true, 500);
    t.glyph("A", true, 500);
    let b = t.glyph("B", true, 500);
    t.cmap31.push((u32::from('A'), alt));
    t.cmap31.push((u32::from('B'), b));
    let mut f = winansi_truetype("ABCDEF+Calibri", "", "");
    f.program = Program::TrueType(t.build());
    let m = load_simple(&f);
    assert!(!m.drawable(c1(b'A')), "FONT-19 strategies disagree");
    assert_eq!(m.info(c1(b'A')).and_then(|i| i.gid), None);
    assert_eq!(
        typeable(&m, c1(b'B')),
        Some('B'),
        "FONT-19 B: cmap and post agree"
    );
    // (1,0) by MacRoman code agreeing with (3,1).
    let mut t = TtfBuilder::new();
    let e = t.glyph("eacute", true, 500);
    t.cmap31.push((u32::from('é'), e));
    t.cmap10.push((0x8E, e));
    let mut f = winansi_truetype("ABCDEF+Calibri", "", "");
    f.program = Program::TrueType(t.build());
    assert_eq!(typeable(&load_simple(&f), c1(0xE9)), Some('é'));
    // (1,0) pointing at another glyph: disagreement.
    let mut t = TtfBuilder::new();
    let e = t.glyph("eacute", true, 500);
    let other = t.glyph("x", true, 500);
    t.cmap31.push((u32::from('é'), e));
    t.cmap10.push((0x8E, other));
    t.post_names = false;
    let mut f = winansi_truetype("ABCDEF+Calibri", "", "");
    f.program = Program::TrueType(t.build());
    assert!(
        !load_simple(&f).drawable(c1(0xE9)),
        "FONT-19 (3,1) vs (1,0)"
    );
}

#[test]
fn font_07_symbolic_truetype_3_0_cmap() {
    let mut t = TtfBuilder::new();
    let a = t.glyph("glyph1", true, 500);
    let b = t.glyph("glyph2", true, 500);
    let sp = t.glyph("glyph3", false, 250);
    t.cmap30.push((0xF001, a));
    t.cmap30.push((0xF002, b));
    t.cmap30.push((0x0003, sp)); // (3,0) at the bare code too
    let mut f = SimpleFont::new("TrueType", "ABCDEF+LiberationSerif");
    f.first_char = 1;
    f.widths = Some(vec![500.0, 500.0, 250.0]);
    f.flags = Some(4);
    f.program = Program::TrueType(t.build());
    f.tounicode = Some(tounicode_bfchar(&[(1, 1, "S"), (2, 1, "ğ"), (3, 1, " ")]));
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), " Sğ", "FONT-07 0xF000+code and code");
    assert_eq!(
        m.info(c1(1)).and_then(|i| i.glyph_name.clone()),
        None,
        "FONT-07 no names"
    );
    f.tounicode = None;
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::NoTounicode),
        "FONT-07"
    );
    f.tounicode = Some(b"garbage beginbfchar".to_vec());
    assert_eq!(load_simple(&f).refusal, Some(TextReason::AmbiguousUnicode));
}

/// A Type0 Identity-H CIDFontType2 over `program` with `tounicode` entries `(cid, text)`.
fn cid2(program: Vec<u8>, entries: &[(u32, &str)]) -> Type0Font {
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Calibri");
    f.program = Program::TrueType(program);
    let e: Vec<(u32, usize, &str)> = entries.iter().map(|(c, t)| (*c, 2, *t)).collect();
    f.tounicode = Some(tounicode_bfchar(&e));
    f
}

#[test]
fn font_21_cid_to_gid_map_stream() {
    let (program, gids) = latin_truetype("AB", "");
    assert_eq!(gids, [('A', 1), ('B', 2)]);
    let mut f = cid2(program, &[(5, "B"), (6, "A"), (7, "C"), (40, "A")]);
    let mut map = vec![0u8; 16];
    map[10..12].copy_from_slice(&2u16.to_be_bytes()); // CID 5 → GID 2
    map[12..14].copy_from_slice(&1u16.to_be_bytes()); // CID 6 → GID 1
    f.cid_to_gid = Some(Some(map));
    let m = load_type0(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::Type0Cid2));
    assert_eq!(m.info(c2(5)).and_then(|i| i.gid), Some(2));
    assert_eq!(typeable(&m, c2(5)), Some('B'));
    assert_eq!(typeable(&m, c2(6)), Some('A'));
    assert_eq!(typeable(&m, c2(7)), None, "FONT-21 CID 7 → GID 0");
    assert_eq!(typeable(&m, c2(40)), None, "FONT-21 CID past the map");
    assert_eq!(m.code_for('A', &[], &[]), Some(c2(6)));
    assert_eq!(m.code_bytes(c2(6)), vec![0, 6]);
    f.cid_to_gid = Some(Some(vec![0; 131_074]));
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::FontUnsupported),
        "FONT-21 cap"
    );
    f.cid_to_gid = Some(None);
    let m = load_type0(&f);
    assert_eq!(
        typeable(&m, c2(5)),
        None,
        "FONT-21 /Identity: CID 5 has no glyph"
    );
}

#[test]
fn font_22_cid_font_type0_cid_keyed_and_name_keyed() {
    let cff = CffBuilder::new("TestCID")
        .cid_glyph(100, true)
        .cid_glyph(200, false)
        .build();
    let mut f = Type0Font::new("CIDFontType0", "ABCDEF+SourceHanSans");
    f.program = Program::CidCff(cff.clone());
    f.dw = Some(500.0);
    f.tounicode = Some(tounicode_bfchar(&[
        (100, 2, "A"),
        (200, 2, " "),
        (300, 2, "B"),
    ]));
    let m = load_type0(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::Type0Cid0));
    assert_eq!(
        m.info(c2(100)).and_then(|i| i.gid),
        Some(1),
        "FONT-22 CID 100 → GID 1"
    );
    assert_eq!(alphabet(&m), " A", "FONT-22 CID-keyed: B has no CID");
    // The same CFF wrapped as OpenType.
    let mut t = TtfBuilder::new();
    t.cff = Some(cff);
    t.glyph("cid100", true, 500);
    t.glyph("cid200", false, 500);
    f.program = Program::OpenType(t.build());
    let m = load_type0(&f);
    assert_eq!(m.refusal, None, "FONT-22 OpenType CIDFontType0");
    assert_eq!(alphabet(&m), " A");
    // Name-keyed CFF: GID = CID.
    let cff = CffBuilder::new("TestNamed")
        .glyph("A", true)
        .glyph("B", true)
        .build();
    let mut f = Type0Font::new("CIDFontType0", "ABCDEF+TestNamed");
    f.program = Program::CidCff(cff);
    f.tounicode = Some(tounicode_bfchar(&[(1, 2, "A"), (2, 2, "B"), (3, 2, "C")]));
    let m = load_type0(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), "AB", "FONT-22 name-keyed: GID = CID");
}

fn type1_font(builder: Type1Builder, encoding: &str, extra: &str) -> Arc<FontModel> {
    let mut f = SimpleFont::new("Type1", "ABCDEF+CMR10");
    f.encoding = Some(encoding.into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(4);
    f.descriptor_extra = extra.into();
    f.program = Program::Type1(builder.build());
    load_simple(&f)
}

fn latin_type1() -> Type1Builder {
    Type1Builder::new("CMR10")
        .glyph("A", T1Glyph::Box { width: 750 })
        .glyph("B", T1Glyph::HintedBox { width: 700 })
        .glyph("C", T1Glyph::Box { width: 700 })
        .glyph("space", T1Glyph::Blank { width: 333 })
}

#[test]
fn font_23_type1_charstrings_and_charset() {
    let m = type1_font(latin_type1(), "/WinAnsiEncoding", "");
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleType1));
    assert_eq!(
        alphabet(&m),
        " ABC",
        "FONT-23 drawn, hinted and whitespace glyphs"
    );
    let m = type1_font(latin_type1(), "/WinAnsiEncoding", "/CharSet (/A/B/space)");
    assert_eq!(
        alphabet(&m),
        " AB",
        "FONT-23 C is in /CharStrings but not in /CharSet"
    );
    let m = type1_font(latin_type1(), "/WinAnsiEncoding", "/CharSet (/A/D)");
    assert_eq!(
        alphabet(&m),
        "A",
        "FONT-23 D is in /CharSet but has no charstring"
    );
}

#[test]
fn font_23b_real_cff_program_foxit_serif() {
    // public/pdfjs/standard_fonts/FoxitSerif.pfb is a bare CFF ("ChromSerifOTF"), not a Type1
    // PFB: it exercises the Type1C path with a real program (see DEVIATIONS).
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../public/pdfjs/standard_fonts/FoxitSerif.pfb");
    let data = std::fs::read(&path).expect("FoxitSerif.pfb in the repo");
    assert_eq!(&data[..3], &[1, 0, 4], "FONT-23b FoxitSerif is CFF");
    let mut f = SimpleFont::new("Type1", "ABCDEF+FoxitSerif");
    f.encoding =
        Some("<< /BaseEncoding /WinAnsiEncoding /Differences [128 /dotlessi /gbreve] >>".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(34);
    f.program = Program::Cff(data);
    let m = load_simple(&f);
    assert_eq!(m.refusal, None, "FONT-23b the real program loads");
    let abc = alphabet(&m);
    assert!(
        !abc.contains('ğ'),
        "FONT-23b FoxitSerif has no gbreve: not typeable"
    );
    for ch in "ABCXYZabcxyz0189 .,;:!?()-ÄÖÜäöüçéßıŒ".chars() {
        assert!(
            abc.contains(ch),
            "FONT-23b {ch:?} typeable in FoxitSerif: {abc}"
        );
    }
}

#[test]
fn font_24_tiny_type1_lacking_a_glyph() {
    let t1 = Type1Builder::new("Tiny")
        .glyph("A", T1Glyph::Box { width: 600 })
        .glyph(
            "Q",
            // an unknown operator (15) before a fully drawn box
            T1Glyph::Raw(
                [
                    op(&[0, 600], &[13]),
                    op(&[1, 2], &[15]),
                    op(&[100, 0], &[21]),
                    op(&[400, 0], &[5]),
                    op(&[0, 500], &[5]),
                    vec![9, 14],
                ]
                .concat(),
            ),
        )
        .glyph("Z", T1Glyph::Raw(op(&[100, 0], &[21])));
    let m = type1_font(t1, "/WinAnsiEncoding", "");
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), "A", "FONT-24 B has no charstring");
    assert!(
        !m.drawable(c1(b'Q')),
        "FONT-24 an unknown operator leaves Q unproven"
    );
    assert!(!m.drawable(c1(b'Z')), "FONT-24 no hsbw first / no endchar");
    assert_eq!(m.text(c1(b'B')), Some("B"), "FONT-24 B still reads");
}

#[test]
fn font_38_type1_hex_eexec() {
    let t1 = Type1Builder {
        hex: true,
        ..latin_type1()
    };
    let file = t1.build();
    assert!(file.data[file.length1..file.length1 + 4]
        .iter()
        .all(u8::is_ascii_hexdigit));
    let m = type1_font(t1, "/WinAnsiEncoding", "");
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), " ABC", "FONT-38 hex eexec decoded first");
}

#[test]
fn font_39_type1_len_iv_minus_one() {
    let t1 = Type1Builder {
        len_iv: -1,
        ..latin_type1()
    };
    let m = type1_font(t1, "/WinAnsiEncoding", "");
    assert_eq!(alphabet(&m), " ABC", "FONT-39 plaintext charstrings");
    let t1 = Type1Builder {
        len_iv: 0,
        ..latin_type1()
    };
    assert_eq!(
        alphabet(&type1_font(t1, "/WinAnsiEncoding", "")),
        " ABC",
        "lenIV 0"
    );
}

#[test]
fn font_40_seac_needs_both_components() {
    // StandardEncoding: 0x41 = A, 0xC2 = acute.
    let with = Type1Builder::new("Accents")
        .glyph("A", T1Glyph::Box { width: 600 })
        .glyph("acute", T1Glyph::Box { width: 300 })
        .glyph(
            "Aacute",
            T1Glyph::Seac {
                width: 600,
                base: 0x41,
                accent: 0xC2,
            },
        );
    let m = type1_font(with, "/WinAnsiEncoding", "");
    assert_eq!(
        typeable(&m, c1(0xC1)),
        Some('Á'),
        "FONT-40 both components present"
    );
    let without = Type1Builder::new("Accents")
        .glyph("A", T1Glyph::Box { width: 600 })
        .glyph(
            "Aacute",
            T1Glyph::Seac {
                width: 600,
                base: 0x41,
                accent: 0xC2,
            },
        );
    let m = type1_font(without, "/WinAnsiEncoding", "");
    assert_eq!(typeable(&m, c1(0xC1)), None, "FONT-40 acute missing");
    assert_eq!(typeable(&m, c1(b'A')), Some('A'));
}

#[test]
fn font_29_corrupt_programs_are_unreadable_without_panic() {
    let (program, _) = latin_truetype("AB", "");
    let cases: Vec<(&str, SimpleFont)> = vec![
        ("garbage FontFile2", {
            let mut f = winansi_truetype("ABCDEF+X", "", "");
            f.program = Program::TrueType(b"not a font at all".to_vec());
            f
        }),
        ("truncated FontFile2", {
            let mut f = winansi_truetype("ABCDEF+X", "", "");
            f.program = Program::TrueType(program[..program.len() / 3].to_vec());
            f
        }),
        ("garbage Type1C", {
            let mut f = SimpleFont::new("Type1", "ABCDEF+X");
            f.encoding = Some("/WinAnsiEncoding".into());
            f.first_char = 32;
            f.widths = Some(vec![500.0; 224]);
            f.program = Program::Cff(vec![1, 0, 4, 4, 0, 1, 1, 1, 255]);
            f
        }),
        ("Type1 without Length2", {
            let mut f = SimpleFont::new("Type1", "ABCDEF+X");
            f.first_char = 32;
            f.widths = Some(vec![500.0; 224]);
            let built = latin_type1().build();
            f.program = Program::Raw {
                key: "FontFile",
                subtype: None,
                data: built.data,
            };
            f
        }),
        ("Type1 Length2 out of range", {
            let mut f = SimpleFont::new("Type1", "ABCDEF+X");
            f.first_char = 32;
            f.widths = Some(vec![500.0; 224]);
            let mut built = latin_type1().build();
            built.length2 = built.data.len() * 2;
            f.program = Program::Type1(built);
            f
        }),
        ("FontFile is not a stream", {
            let mut f = winansi_truetype("ABCDEF+X", "", "");
            f.program = Program::None;
            f.descriptor_extra = "/FontFile2 [1 2 3]".into();
            f
        }),
    ];
    for (what, f) in cases {
        let m = load_simple(&f);
        assert_eq!(
            m.refusal,
            Some(TextReason::FontProgramUnreadable),
            "FONT-29 {what}"
        );
        assert_eq!(m.class, None);
        assert_eq!(alphabet(&m), "");
    }
    let mut f = cid2(b"\x00\x01\x00\x00garbage".to_vec(), &[(1, "A")]);
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::FontProgramUnreadable),
        "FONT-29 CID"
    );
    f.program = Program::None;
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::FontNotEmbedded),
        "FONT-29 no program"
    );
}

#[test]
fn font_30_fontfile3_subtype_mismatch_and_opentype() {
    let (tt, _) = latin_truetype("AB", "");
    let cff = CffBuilder::new("X").glyph("A", true).build();
    let mismatches: Vec<(&str, &'static str, Program)> = vec![
        (
            "Type1 + CIDFontType0C",
            "Type1",
            Program::CidCff(cff.clone()),
        ),
        ("TrueType + Type1C", "TrueType", Program::Cff(cff.clone())),
        ("Type1 + FontFile2", "Type1", Program::TrueType(tt.clone())),
        (
            "FontFile3 without subtype",
            "Type1",
            Program::Raw {
                key: "FontFile3",
                subtype: None,
                data: cff.clone(),
            },
        ),
        (
            "TrueType + FontFile",
            "TrueType",
            Program::Raw {
                key: "FontFile",
                subtype: None,
                data: tt.clone(),
            },
        ),
    ];
    for (what, subtype, program) in mismatches {
        let mut f = SimpleFont::new(subtype, "ABCDEF+X");
        f.encoding = Some("/WinAnsiEncoding".into());
        f.first_char = 32;
        f.widths = Some(vec![500.0; 224]);
        f.program = program;
        assert_eq!(
            load_simple(&f).refusal,
            Some(TextReason::FontProgramUnsupported),
            "FONT-30 {what}"
        );
    }
    let mut f = cid2(tt.clone(), &[(1, "A")]);
    f.program = Program::CidCff(cff.clone());
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::FontProgramUnsupported),
        "CIDFontType2 + CIDFontType0C"
    );
    // OpenType, glyf flavour, in a simple TrueType font …
    let mut f = winansi_truetype("ABCDEF+X", "", "");
    f.program = Program::OpenType(tt);
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.class, Some(FontClass::SimpleOpenType));
    assert_eq!(alphabet(&m), "AB", "FONT-30 OpenType (glyf)");
    // … and CFF flavour (OTTO) in a simple Type1 font.
    let mut t = TtfBuilder::new();
    let a = t.unicode_glyph('A', "A", true);
    t.cff = Some(
        CffBuilder::new("X")
            .glyph("A", true)
            .encoding(CffEncodingSpec::Standard)
            .build(),
    );
    assert_eq!(a, 1);
    let mut f = SimpleFont::new("Type1", "ABCDEF+X");
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(32);
    f.program = Program::OpenType(t.build());
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), "A", "FONT-30 OpenType (CFF)");
}

#[test]
fn font_30b_an_sfnt_with_both_glyf_and_cff_is_refused() {
    // Review T4 r2 M-2: presence was proven on `glyf` while viewers draw an `OTTO` program's CFF
    // outlines, so "Hello" → "Hellj" passed with "j" drawn empty. Both tables: refused.
    let mut t = TtfBuilder::new();
    for (ch, name) in [('H', "H"), ('e', "e"), ('l', "l"), ('o', "o"), ('j', "j")] {
        t.unicode_glyph(ch, name, true);
    }
    let cff = |empty: &str| {
        ["H", "e", "l", "o", "j"]
            .iter()
            .fold(CffBuilder::new("X"), |b, g| b.glyph(g, *g != empty))
            .encoding(CffEncodingSpec::Standard)
            .build()
    };
    let font = |program: Vec<u8>| {
        let mut f = SimpleFont::new("Type1", "ABCDEF+X");
        f.encoding = Some("/WinAnsiEncoding".into());
        f.first_char = 32;
        f.widths = Some(vec![500.0; 224]);
        f.flags = Some(32);
        f.program = Program::OpenType(program);
        load_simple(&f)
    };
    for (tag, magic) in [("OTTO", 0x4F54_544F_u32), ("1.0", 0x0001_0000)] {
        let m = font(t.build_glyf_and_cff(cff("j"), magic));
        assert_eq!(m.refusal, Some(TextReason::FontProgramUnsupported), "{tag}");
        assert_eq!(alphabet(&m), "", "{tag}: nothing typeable");
    }
    // Control: the same program without `glyf` is read as before, from its CFF outlines (the
    // glyf-only flavour is FONT-30's).
    let only_cff = TtfBuilder {
        cff: Some(cff("j")),
        ..t.clone()
    };
    let m = font(only_cff.build());
    assert_eq!(m.refusal, None, "OTTO, CFF only");
    assert_eq!(alphabet(&m), "Helo", "\"j\" draws nothing in CFF");
}

#[test]
fn font_29b_t1_and_unusual_encodings_report_their_class() {
    // A Type1 whose clear text has no /Encoding: no names, nothing typeable, no refusal.
    let t1 = Type1Builder {
        encoding: T1Encoding::Custom(Vec::new()),
        ..Type1Builder::new("NoEnc")
    }
    .glyph("A", T1Glyph::Box { width: 500 });
    let mut f = SimpleFont::new("Type1", "ABCDEF+NoEnc");
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.program = Program::Type1(t1.build());
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), "");
    assert_eq!(m.text(c1(b'A')), None);
}

#[test]
fn font_vertical_metrics() {
    let m = load_simple(&winansi_truetype("ABCDEF+Calibri", "A", ""));
    assert_eq!(
        (m.ascent, m.descent),
        (0.8, -0.2),
        "descriptor /Ascent /Descent"
    );
    let mut f = winansi_truetype("ABCDEF+Calibri", "A", "");
    f.descriptor_extra = "/Ascent 0 /Descent 0".into();
    let mut t = TtfBuilder::new();
    t.ascender = 900;
    t.descender = -300;
    t.unicode_glyph('A', "A", true);
    f.program = Program::TrueType(t.build());
    let m = load_simple(&f);
    assert_eq!(
        (m.ascent, m.descent),
        (0.9, -0.3),
        "zero descriptor values → hhea"
    );
    f.descriptor_extra = "/Ascent 5000 /Descent -3000".into();
    let m = load_simple(&f);
    assert_eq!((m.ascent, m.descent), (1.5, -0.8), "clamped");
}

#[test]
fn font_hash_recursion_is_bounded() {
    // A font dictionary carrying a 3,000-deep array: the hash stops descending at its nesting
    // cap (deterministically), so two trees that differ only below it hash alike.
    let deep = |leaf: i64| {
        let mut obj = lopdf::Object::Integer(leaf);
        for _ in 0..3_000 {
            obj = lopdf::Object::Array(vec![obj]);
        }
        obj
    };
    let hash = |leaf: i64| {
        let doc = lopdf::Document::with_version("1.7");
        let mut dict = lopdf::dictionary! { "Type" => "Font", "Subtype" => "Type3" };
        dict.set("Deep", deep(leaf));
        let model = std::thread::scope(|s| {
            s.spawn(|| {
                super::FontCache::new().get_or_load(
                    &doc,
                    super::FontKey::Indirect((1, 0)),
                    &dict,
                    &mut crate::pdf_engine::text_edit::decode::DecodeBudget::new(1 << 20),
                )
            })
            .join()
            .expect("no stack overflow")
        });
        assert_eq!(model.refusal, Some(TextReason::Type3));
        let h = model.content_hash;
        // Drop the deep tree iteratively-ish: unwrap level by level.
        let mut obj = dict.remove(b"Deep");
        while let Some(lopdf::Object::Array(mut items)) = obj {
            obj = items.pop();
        }
        h
    };
    assert_eq!(hash(1), hash(2), "below the nesting cap nothing is hashed");
}
