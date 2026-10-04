//! GENERATED DATA — do not edit by hand (SPEC §B.9.2). Base encodings as glyph names per code.
//!
//! Provenance: generated from `lopdf` 0.34.0 `src/encodings/mappings.rs` (`WIN_ANSI_ENCODING`,
//! `MAC_ROMAN_ENCODING`, `STANDARD_ENCODING`; each `Some(Glyph::name)` entry written as its glyph
//! name) and checked, code by code, to be identical to pdf.js 4.10.38 (`pdfjs-dist`,
//! `build/pdf.worker.mjs`: `WinAnsiEncoding`, `MacRomanEncoding`, `StandardEncoding`), which follow
//! ISO 32000-1 Annex D.2 ("Latin Character Set and Encodings", incl. its notes: WinAnsi 0xA0 =
//! space, 0xAD = hyphen, unused WinAnsi codes above 0x20 = bullet; MacRoman 0xCA = space).
//! `MAC_ROMAN_NAMES` is strict Annex D: the 15 Mac-OS-only codes that lopdf and pdf.js add
//! (0xAD notequal, 0xB0 infinity, 0xB2 lessequal, 0xB3 greaterequal, 0xB6 partialdiff,
//! 0xB7 summation, 0xB8 product, 0xB9 pi, 0xBA integral, 0xBD Omega, 0xC3 radical,
//! 0xC5 approxequal, 0xC6 Delta, 0xD7 lozenge, 0xF0 apple) are `None`; 0xDB is `currency`.
//! The test `encoding_tables_cross_check` (FONT-01/FONT-37) re-derives every code from lopdf's
//! Unicode tables through the AGL and asserts exactly that 15-code difference.
//!
//! Licences: lopdf is MIT (Copyright (c) 2016 Junfeng Liu); pdf.js is Apache-2.0 (Copyright
//! Mozilla Foundation), used only for the cross-check; the encodings themselves are tables of
//! ISO 32000-1 (Annex D).

/// WinAnsiEncoding (ISO 32000-1 Annex D.2).
#[rustfmt::skip]
pub(super) static WIN_ANSI_NAMES: [Option<&str>; 256] = [
    /* 0x00 */ None, None, None, None, None, None, None, None,
    /* 0x08 */ None, None, None, None, None, None, None, None,
    /* 0x10 */ None, None, None, None, None, None, None, None,
    /* 0x18 */ None, None, None, None, None, None, None, None,
    /* 0x20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quotesingle"),
    /* 0x28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 0x30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 0x38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 0x40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 0x48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 0x50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 0x58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 0x60 */ Some("grave"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 0x68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 0x70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 0x78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), Some("bullet"),
    /* 0x80 */ Some("Euro"), Some("bullet"), Some("quotesinglbase"), Some("florin"), Some("quotedblbase"), Some("ellipsis"), Some("dagger"), Some("daggerdbl"),
    /* 0x88 */ Some("circumflex"), Some("perthousand"), Some("Scaron"), Some("guilsinglleft"), Some("OE"), Some("bullet"), Some("Zcaron"), Some("bullet"),
    /* 0x90 */ Some("bullet"), Some("quoteleft"), Some("quoteright"), Some("quotedblleft"), Some("quotedblright"), Some("bullet"), Some("endash"), Some("emdash"),
    /* 0x98 */ Some("tilde"), Some("trademark"), Some("scaron"), Some("guilsinglright"), Some("oe"), Some("bullet"), Some("zcaron"), Some("Ydieresis"),
    /* 0xA0 */ Some("space"), Some("exclamdown"), Some("cent"), Some("sterling"), Some("currency"), Some("yen"), Some("brokenbar"), Some("section"),
    /* 0xA8 */ Some("dieresis"), Some("copyright"), Some("ordfeminine"), Some("guillemotleft"), Some("logicalnot"), Some("hyphen"), Some("registered"), Some("macron"),
    /* 0xB0 */ Some("degree"), Some("plusminus"), Some("twosuperior"), Some("threesuperior"), Some("acute"), Some("mu"), Some("paragraph"), Some("periodcentered"),
    /* 0xB8 */ Some("cedilla"), Some("onesuperior"), Some("ordmasculine"), Some("guillemotright"), Some("onequarter"), Some("onehalf"), Some("threequarters"), Some("questiondown"),
    /* 0xC0 */ Some("Agrave"), Some("Aacute"), Some("Acircumflex"), Some("Atilde"), Some("Adieresis"), Some("Aring"), Some("AE"), Some("Ccedilla"),
    /* 0xC8 */ Some("Egrave"), Some("Eacute"), Some("Ecircumflex"), Some("Edieresis"), Some("Igrave"), Some("Iacute"), Some("Icircumflex"), Some("Idieresis"),
    /* 0xD0 */ Some("Eth"), Some("Ntilde"), Some("Ograve"), Some("Oacute"), Some("Ocircumflex"), Some("Otilde"), Some("Odieresis"), Some("multiply"),
    /* 0xD8 */ Some("Oslash"), Some("Ugrave"), Some("Uacute"), Some("Ucircumflex"), Some("Udieresis"), Some("Yacute"), Some("Thorn"), Some("germandbls"),
    /* 0xE0 */ Some("agrave"), Some("aacute"), Some("acircumflex"), Some("atilde"), Some("adieresis"), Some("aring"), Some("ae"), Some("ccedilla"),
    /* 0xE8 */ Some("egrave"), Some("eacute"), Some("ecircumflex"), Some("edieresis"), Some("igrave"), Some("iacute"), Some("icircumflex"), Some("idieresis"),
    /* 0xF0 */ Some("eth"), Some("ntilde"), Some("ograve"), Some("oacute"), Some("ocircumflex"), Some("otilde"), Some("odieresis"), Some("divide"),
    /* 0xF8 */ Some("oslash"), Some("ugrave"), Some("uacute"), Some("ucircumflex"), Some("udieresis"), Some("yacute"), Some("thorn"), Some("ydieresis"),
];

/// MacRomanEncoding, strictly as ISO 32000-1 Annex D.2 (no Mac-OS-only codes).
#[rustfmt::skip]
pub(super) static MAC_ROMAN_NAMES: [Option<&str>; 256] = [
    /* 0x00 */ None, None, None, None, None, None, None, None,
    /* 0x08 */ None, None, None, None, None, None, None, None,
    /* 0x10 */ None, None, None, None, None, None, None, None,
    /* 0x18 */ None, None, None, None, None, None, None, None,
    /* 0x20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quotesingle"),
    /* 0x28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 0x30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 0x38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 0x40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 0x48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 0x50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 0x58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 0x60 */ Some("grave"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 0x68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 0x70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 0x78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), None,
    /* 0x80 */ Some("Adieresis"), Some("Aring"), Some("Ccedilla"), Some("Eacute"), Some("Ntilde"), Some("Odieresis"), Some("Udieresis"), Some("aacute"),
    /* 0x88 */ Some("agrave"), Some("acircumflex"), Some("adieresis"), Some("atilde"), Some("aring"), Some("ccedilla"), Some("eacute"), Some("egrave"),
    /* 0x90 */ Some("ecircumflex"), Some("edieresis"), Some("iacute"), Some("igrave"), Some("icircumflex"), Some("idieresis"), Some("ntilde"), Some("oacute"),
    /* 0x98 */ Some("ograve"), Some("ocircumflex"), Some("odieresis"), Some("otilde"), Some("uacute"), Some("ugrave"), Some("ucircumflex"), Some("udieresis"),
    /* 0xA0 */ Some("dagger"), Some("degree"), Some("cent"), Some("sterling"), Some("section"), Some("bullet"), Some("paragraph"), Some("germandbls"),
    /* 0xA8 */ Some("registered"), Some("copyright"), Some("trademark"), Some("acute"), Some("dieresis"), None, Some("AE"), Some("Oslash"),
    /* 0xB0 */ None, Some("plusminus"), None, None, Some("yen"), Some("mu"), None, None,
    /* 0xB8 */ None, None, None, Some("ordfeminine"), Some("ordmasculine"), None, Some("ae"), Some("oslash"),
    /* 0xC0 */ Some("questiondown"), Some("exclamdown"), Some("logicalnot"), None, Some("florin"), None, None, Some("guillemotleft"),
    /* 0xC8 */ Some("guillemotright"), Some("ellipsis"), Some("space"), Some("Agrave"), Some("Atilde"), Some("Otilde"), Some("OE"), Some("oe"),
    /* 0xD0 */ Some("endash"), Some("emdash"), Some("quotedblleft"), Some("quotedblright"), Some("quoteleft"), Some("quoteright"), Some("divide"), None,
    /* 0xD8 */ Some("ydieresis"), Some("Ydieresis"), Some("fraction"), Some("currency"), Some("guilsinglleft"), Some("guilsinglright"), Some("fi"), Some("fl"),
    /* 0xE0 */ Some("daggerdbl"), Some("periodcentered"), Some("quotesinglbase"), Some("quotedblbase"), Some("perthousand"), Some("Acircumflex"), Some("Ecircumflex"), Some("Aacute"),
    /* 0xE8 */ Some("Edieresis"), Some("Egrave"), Some("Iacute"), Some("Icircumflex"), Some("Idieresis"), Some("Igrave"), Some("Oacute"), Some("Ocircumflex"),
    /* 0xF0 */ None, Some("Ograve"), Some("Uacute"), Some("Ucircumflex"), Some("Ugrave"), Some("dotlessi"), Some("circumflex"), Some("tilde"),
    /* 0xF8 */ Some("macron"), Some("breve"), Some("dotaccent"), Some("ring"), Some("cedilla"), Some("hungarumlaut"), Some("ogonek"), Some("caron"),
];

/// StandardEncoding (ISO 32000-1 Annex D.2); codes 0x80–0xA0 are undefined.
#[rustfmt::skip]
pub(super) static STANDARD_NAMES: [Option<&str>; 256] = [
    /* 0x00 */ None, None, None, None, None, None, None, None,
    /* 0x08 */ None, None, None, None, None, None, None, None,
    /* 0x10 */ None, None, None, None, None, None, None, None,
    /* 0x18 */ None, None, None, None, None, None, None, None,
    /* 0x20 */ Some("space"), Some("exclam"), Some("quotedbl"), Some("numbersign"), Some("dollar"), Some("percent"), Some("ampersand"), Some("quoteright"),
    /* 0x28 */ Some("parenleft"), Some("parenright"), Some("asterisk"), Some("plus"), Some("comma"), Some("hyphen"), Some("period"), Some("slash"),
    /* 0x30 */ Some("zero"), Some("one"), Some("two"), Some("three"), Some("four"), Some("five"), Some("six"), Some("seven"),
    /* 0x38 */ Some("eight"), Some("nine"), Some("colon"), Some("semicolon"), Some("less"), Some("equal"), Some("greater"), Some("question"),
    /* 0x40 */ Some("at"), Some("A"), Some("B"), Some("C"), Some("D"), Some("E"), Some("F"), Some("G"),
    /* 0x48 */ Some("H"), Some("I"), Some("J"), Some("K"), Some("L"), Some("M"), Some("N"), Some("O"),
    /* 0x50 */ Some("P"), Some("Q"), Some("R"), Some("S"), Some("T"), Some("U"), Some("V"), Some("W"),
    /* 0x58 */ Some("X"), Some("Y"), Some("Z"), Some("bracketleft"), Some("backslash"), Some("bracketright"), Some("asciicircum"), Some("underscore"),
    /* 0x60 */ Some("quoteleft"), Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), Some("f"), Some("g"),
    /* 0x68 */ Some("h"), Some("i"), Some("j"), Some("k"), Some("l"), Some("m"), Some("n"), Some("o"),
    /* 0x70 */ Some("p"), Some("q"), Some("r"), Some("s"), Some("t"), Some("u"), Some("v"), Some("w"),
    /* 0x78 */ Some("x"), Some("y"), Some("z"), Some("braceleft"), Some("bar"), Some("braceright"), Some("asciitilde"), None,
    /* 0x80 */ None, None, None, None, None, None, None, None,
    /* 0x88 */ None, None, None, None, None, None, None, None,
    /* 0x90 */ None, None, None, None, None, None, None, None,
    /* 0x98 */ None, None, None, None, None, None, None, None,
    /* 0xA0 */ None, Some("exclamdown"), Some("cent"), Some("sterling"), Some("fraction"), Some("yen"), Some("florin"), Some("section"),
    /* 0xA8 */ Some("currency"), Some("quotesingle"), Some("quotedblleft"), Some("guillemotleft"), Some("guilsinglleft"), Some("guilsinglright"), Some("fi"), Some("fl"),
    /* 0xB0 */ None, Some("endash"), Some("dagger"), Some("daggerdbl"), Some("periodcentered"), None, Some("paragraph"), Some("bullet"),
    /* 0xB8 */ Some("quotesinglbase"), Some("quotedblbase"), Some("quotedblright"), Some("guillemotright"), Some("ellipsis"), Some("perthousand"), None, Some("questiondown"),
    /* 0xC0 */ None, Some("grave"), Some("acute"), Some("circumflex"), Some("tilde"), Some("macron"), Some("breve"), Some("dotaccent"),
    /* 0xC8 */ Some("dieresis"), None, Some("ring"), Some("cedilla"), None, Some("hungarumlaut"), Some("ogonek"), Some("caron"),
    /* 0xD0 */ Some("emdash"), None, None, None, None, None, None, None,
    /* 0xD8 */ None, None, None, None, None, None, None, None,
    /* 0xE0 */ None, Some("AE"), None, Some("ordfeminine"), None, None, None, None,
    /* 0xE8 */ Some("Lslash"), Some("Oslash"), Some("OE"), Some("ordmasculine"), None, None, None, None,
    /* 0xF0 */ None, Some("ae"), None, None, None, Some("dotlessi"), None, None,
    /* 0xF8 */ Some("lslash"), Some("oslash"), Some("oe"), Some("germandbls"), None, None, None, None,
];
