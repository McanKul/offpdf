//! FONT-08…15: the bounded ToUnicode parser (§B.9.4) and how ToUnicode and glyph names combine
//! when reading and writing (§A.3.1, §A.3.2).

use super::tests::{alphabet, c1, c2, std14, typeable};
use super::tounicode::{parse_tounicode, Lookup};
use crate::pdf_engine::text_edit::limits::CMAP_MAPPINGS_MAX;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::fonts::{
    cmap, load_simple, load_type0, tounicode_bfchar, Program, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;

fn text(tu: &super::tounicode::ToUnicode, code: super::Code) -> Option<String> {
    match tu.lookup(code.value) {
        Lookup::Text(t) => Some(t.to_string()),
        Lookup::Undecodable => Some("<undecodable>".into()),
        Lookup::Absent => None,
    }
}

#[test]
fn font_08_bfchar() {
    let tu = parse_tounicode(&cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         3 beginbfchar\n<0003> <0020>\n<0024> <011F>\n<0025> <0130>\nendbfchar",
    ))
    .expect("FONT-08 parses");
    assert_eq!(text(&tu, c2(3)).as_deref(), Some(" "));
    assert_eq!(text(&tu, c2(0x24)).as_deref(), Some("ğ"));
    assert_eq!(text(&tu, c2(0x25)).as_deref(), Some("İ"));
    assert_eq!(text(&tu, c2(0x26)), None, "FONT-08 unmapped code is absent");
    assert_eq!(
        text(&tu, c1(0x24)).as_deref(),
        Some("ğ"),
        "FONT-08 codes are keyed by value, whatever the source length (pdf.js, Poppler)"
    );
    // Literal-string sources and a later mapping of the same code (later wins).
    let tu = parse_tounicode(&cmap("2 beginbfchar\n(A) <0061>\n<41> <0062>\nendbfchar"))
        .expect("parses");
    assert_eq!(text(&tu, c1(0x41)).as_deref(), Some("b"));
}

#[test]
fn font_09_bfrange_incrementing() {
    let tu = parse_tounicode(&cmap(
        "2 beginbfrange\n<0041> <0043> <0061>\n<0001> <0003> <FFFE>\nendbfrange",
    ))
    .expect("FONT-09 parses");
    let abc: Vec<_> = (0x41..=0x43).map(|c| text(&tu, c2(c))).collect();
    assert_eq!(abc, [Some("a".into()), Some("b".into()), Some("c".into())]);
    assert_eq!(text(&tu, c2(1)).as_deref(), Some("\u{FFFE}"));
    assert_eq!(text(&tu, c2(2)).as_deref(), Some("\u{FFFF}"));
    assert_eq!(
        text(&tu, c2(3)),
        None,
        "FONT-09 overflow past 0xFFFF stops the range"
    );
    // The last UTF-16 unit is incremented: a ligature destination keeps its prefix.
    let tu =
        parse_tounicode(&cmap("1 beginbfrange\n<10> <11> <00660069>\nendbfrange")).expect("parses");
    assert_eq!(text(&tu, c1(0x10)).as_deref(), Some("fi"));
    assert_eq!(text(&tu, c1(0x11)).as_deref(), Some("fj"));
    for bad in [
        "1 beginbfrange\n<0043> <0041> <0061>\nendbfrange",
        "1 beginbfrange\n<0041> <43> <0061>\nendbfrange",
        "1 beginbfrange\n<0041> <0043> /a\nendbfrange",
        "1 beginbfrange\n<0041> <0043>",
        "1 beginbfchar\n<0041>\nendbfchar",
        "1 beginbfchar\n<0000000041> <0041>\nendbfchar",
        "1 begincodespacerange <00> <FFFF> endcodespacerange",
    ] {
        assert!(
            parse_tounicode(&cmap(bad)).is_err(),
            "FONT-09 syntax error: {bad}"
        );
    }
}

#[test]
fn font_10_bfrange_array_form() {
    let tu = parse_tounicode(&cmap(
        "1 beginbfrange\n<10> <13> [<0041> <0042> <00660069>]\nendbfrange",
    ))
    .expect("FONT-10 parses");
    assert_eq!(text(&tu, c1(0x10)).as_deref(), Some("A"));
    assert_eq!(text(&tu, c1(0x11)).as_deref(), Some("B"));
    assert_eq!(text(&tu, c1(0x12)).as_deref(), Some("fi"));
    assert_eq!(
        text(&tu, c1(0x13)),
        None,
        "FONT-10 a short array maps fewer codes"
    );
    assert!(parse_tounicode(&cmap("1 beginbfrange\n<10> <11> [<0041> /B]\nendbfrange")).is_err());
}

#[test]
fn font_11_surrogates_and_bad_destinations() {
    let tu = parse_tounicode(&cmap(
        "4 beginbfchar\n<01> <D83DDE00>\n<02> <D83D>\n<03> <000041>\n<04> <>\nendbfchar",
    ))
    .expect("FONT-11 parses");
    assert_eq!(
        text(&tu, c1(1)).as_deref(),
        Some("😀"),
        "FONT-11 surrogate pair"
    );
    assert_eq!(
        text(&tu, c1(2)).as_deref(),
        Some("<undecodable>"),
        "FONT-11 lone surrogate"
    );
    assert_eq!(
        text(&tu, c1(3)).as_deref(),
        Some("<undecodable>"),
        "FONT-11 odd length"
    );
    assert_eq!(
        text(&tu, c1(4)).as_deref(),
        Some("<undecodable>"),
        "FONT-11 empty"
    );
    let long = format!("1 beginbfchar\n<05> <{}>\nendbfchar", "0041".repeat(257));
    assert!(
        parse_tounicode(&cmap(&long)).is_err(),
        "FONT-11 destination > 512 bytes"
    );
}

#[test]
fn font_12_ligatures_are_readable_not_writable() {
    let mut f = std14(
        "Helvetica",
        "<< /BaseEncoding /WinAnsiEncoding /Differences [150 /fi 151 /fl] >>",
    );
    f.tounicode = Some(tounicode_bfchar(&[
        (150, 1, "fi"),
        (151, 1, "\u{FB02}"),
        (65, 1, "A"),
    ]));
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(
        m.text(c1(150)),
        Some("fi"),
        "FONT-12 ToUnicode 'fi' agrees with /fi"
    );
    assert_eq!(
        m.text(c1(151)),
        Some("fl"),
        "FONT-12 U+FB02 displayed as fl"
    );
    assert_eq!(
        typeable(&m, c1(150)),
        None,
        "FONT-12 two scalars: not typeable"
    );
    assert_eq!(
        typeable(&m, c1(151)),
        None,
        "FONT-12 ligature code point: not typeable"
    );
    assert_eq!(typeable(&m, c1(65)), Some('A'));
}

#[test]
fn font_13_usecmap_makes_unmapped_codes_undecodable() {
    let body = "/Adobe-Identity-UCS2 usecmap\n1 beginbfchar\n<41> <0041>\nendbfchar";
    let tu = parse_tounicode(&cmap(body)).expect("FONT-13 parses");
    assert_eq!(text(&tu, c1(0x41)).as_deref(), Some("A"));
    assert_eq!(text(&tu, c1(0x42)).as_deref(), Some("<undecodable>"));
    let mut f = std14("Helvetica", "/WinAnsiEncoding");
    f.tounicode = Some(cmap(body));
    let m = load_simple(&f);
    assert_eq!(typeable(&m, c1(0x41)), Some('A'));
    assert_eq!(
        m.text(c1(0x42)),
        None,
        "FONT-13 the parent CMap might map B differently"
    );
    assert_eq!(typeable(&m, c1(0x42)), None);
}

#[test]
fn font_14_tounicode_bombs_are_bounded() {
    let over = CMAP_MAPPINGS_MAX / 0x10000 + 1;
    let ranges: String = (0..over)
        .map(|i| format!("<{i:02X}0000> <{i:02X}FFFF> <0041>\n"))
        .collect();
    let started = std::time::Instant::now();
    assert!(
        parse_tounicode(&cmap(&format!("{over} beginbfrange\n{ranges}endbfrange"))).is_err(),
        "FONT-14 more than CMAP_MAPPINGS_MAX mappings"
    );
    // Exactly at the cap still parses; one bfchar more does not.
    let full = "<000000> <00FFFF> <0000>\n<010000> <01FFFF> <0000>\n";
    let ok = parse_tounicode(&cmap(&format!("2 beginbfrange\n{full}endbfrange")));
    assert_eq!(ok.map(|t| t.codes().count()).ok(), Some(CMAP_MAPPINGS_MAX));
    let past_cap = cmap(&format!(
        "2 beginbfrange\n{full}endbfrange\n1 beginbfchar\n<020000> <0041>\nendbfchar"
    ));
    assert!(
        parse_tounicode(&past_cap).is_err(),
        "FONT-14 one bfchar past the cap"
    );
    assert!(started.elapsed().as_secs() < 30, "FONT-14 bounded time");
    // In a Type0 font a broken ToUnicode is the font's refusal.
    let mut t = TtfBuilder::new();
    t.glyph("A", true, 500);
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Test");
    f.program = Program::TrueType(t.build());
    f.tounicode = Some(cmap(&format!("{over} beginbfrange\n{ranges}endbfrange")));
    assert_eq!(load_type0(&f).refusal, Some(TextReason::AmbiguousUnicode));
    f.tounicode = Some(b"begincmap 1 beginbfchar <0001> endcmap".to_vec());
    assert_eq!(load_type0(&f).refusal, Some(TextReason::AmbiguousUnicode));
    // A valid CMap padded past TOUNICODE_MAX_DECODED (it inflates from a few KiB): capped while
    // inflating, refused.
    let mut padded = vec![b' '; crate::pdf_engine::text_edit::limits::TOUNICODE_MAX_DECODED];
    padded.extend(tounicode_bfchar(&[(1, 2, "A")]));
    f.tounicode = Some(padded);
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::AmbiguousUnicode),
        "FONT-14 cap"
    );
    f.tounicode = Some(tounicode_bfchar(&[(1, 2, "A")]));
    assert_eq!(
        load_type0(&f).refusal,
        None,
        "FONT-14 the same CMap unpadded loads"
    );
}

#[test]
fn font_15_tounicode_and_name_disagreement_is_undecodable() {
    let mut f = std14("Helvetica", "/WinAnsiEncoding");
    f.tounicode = Some(tounicode_bfchar(&[
        (0x41, 1, "B"),
        (0x42, 1, "B"),
        (0x43, 1, "Ç"),
    ]));
    let m = load_simple(&f);
    assert_eq!(m.text(c1(0x41)), None, "FONT-15 ToUnicode B vs name A");
    assert_eq!(typeable(&m, c1(0x41)), None);
    assert_eq!(m.text(c1(0x42)), Some("B"), "FONT-15 agreement");
    assert_eq!(typeable(&m, c1(0x42)), Some('B'));
    assert_eq!(m.text(c1(0x43)), None, "FONT-15 C vs Ç");
    assert_eq!(
        m.text(c1(0x44)),
        Some("D"),
        "FONT-15 names alone still read"
    );
    // A broken ToUnicode on a simple font: names still read, nothing types.
    let mut f = std14("Helvetica", "/WinAnsiEncoding");
    f.tounicode = Some(b"beginbfchar <41> endbfchar".to_vec());
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.text(c1(0x41)), Some("A"));
    assert_eq!(alphabet(&m), "", "FONT-15 agreement cannot hold");
}
