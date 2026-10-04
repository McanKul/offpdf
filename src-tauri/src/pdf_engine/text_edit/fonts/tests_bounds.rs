//! Work and memory bounds of the font loader, and the reading fixes of the review-T2 fix pass:
//! the Type1 interpreter (FONT-24b–d), the outline pre-checks against ttf-parser (FONT-29c–f,
//! FONT-30b), ToUnicode keyed by value and its text budget (FONT-08b, FONT-14b/c), descriptor
//! and refusal-order fixes (FONT-28c/d), the Type3 hash budget (FONT-28e) and the character rules
//! (FONT-12b, FONT-15b).

use super::cff_layout::CffLayout;
use super::encodings::{reading_reason, writable_char};
use super::glyph_budget::{WorkMeter, GLYPH_WORK_MAX};
use super::program::{cff_cid_to_gid, CffNames, Outlines};
use super::tests::{alphabet, c1, c2, std14, typeable, winansi_truetype};
use super::tests_fuzz::thread_cpu;
use super::tounicode::parse_tounicode;
use super::type1::{parse_type1, GlyphProof};
use super::{FontCache, FontKey, FontModel};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::cff::{t2, CffBuilder};
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_type0, cmap, latin_truetype, load_simple, load_type0, page_with_fonts, snapshot,
    tounicode_bfchar, Program, SimpleFont, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder;
use crate::pdf_engine::text_edit::testkit::thread_peak;
use crate::pdf_engine::text_edit::testkit::ttf::{composite, TtfBuilder};
use crate::pdf_engine::text_edit::testkit::type1::{op, T1Glyph, Type1Builder, Type1File};
use lopdf::Object;
use std::sync::Arc;
use ttf_parser::{GlyphId, OutlineBuilder};

/// `f`'s result and the CPU seconds it took on this thread.
pub(super) fn cpu<R>(f: impl FnOnce() -> R) -> (R, f64) {
    let started = thread_cpu();
    let r = f();
    (r, thread_cpu().saturating_sub(started).as_secs_f64())
}

/// Loads object `font` of `b` (as the one font of a page) with `budget`.
fn load_with(b: PdfBuilder, font: u32, budget: &mut DecodeBudget) -> Arc<FontModel> {
    let snap = snapshot(page_with_fonts(b, &[("F1", font)]));
    let dict = snap
        .doc
        .get_object((font, 0))
        .and_then(Object::as_dict)
        .expect("font dict");
    FontCache::new().get_or_load(&snap.doc, FontKey::Indirect((font, 0)), dict, budget)
}

/// A Type1 charstring: `hsbw`, `n` calls of subroutine `subr`, a drawn segment, `endchar`.
fn calling(subr: i32, n: usize) -> Vec<u8> {
    let mut code = op(&[0, 600], &[13]);
    for _ in 0..n {
        code.extend(op(&[subr], &[10]));
    }
    code.extend(op(&[100, 0], &[21]));
    code.extend(op(&[400, 0], &[5]));
    code.push(14);
    code
}

fn type1_simple(base: &str, file: Type1File) -> SimpleFont {
    let mut f = SimpleFont::new("Type1", base);
    f.encoding = Some("/WinAnsiEncoding".into());
    f.first_char = 32;
    f.widths = Some(vec![600.0; 224]);
    f.flags = Some(32);
    f.program = Program::Type1(file);
    f.flate = false;
    f
}

const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

#[test]
fn font_24b_type1_subroutine_calls_decrypt_nothing() {
    // Subr 5: `return` and 60,000 filler bytes (inside the 65,535-byte limit); subr 6: `return`
    // and 4 MiB. Before the fix every call decrypted the whole subroutine again: 2,000 calls of
    // subr 5 per glyph, 52 glyphs, were 6 GB of decryption.
    let mut long = vec![11u8];
    long.resize(60_000, 0);
    let mut huge = vec![11u8];
    huge.resize(4 << 20, 0);
    let mut b = Type1Builder {
        extra_subrs: vec![long, huge],
        ..Type1Builder::new("Slow")
    };
    for ch in LETTERS.chars() {
        b = b.glyph(&ch.to_string(), T1Glyph::Raw(calling(5, 2_000)));
    }
    let file = b.glyph("huge", T1Glyph::Raw(calling(6, 1))).build();
    let (proofs, secs) = cpu(|| {
        let program = parse_type1(&file.data, file.length1, file.length2).expect("parses");
        let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
        let drawn = LETTERS
            .chars()
            .filter(|c| program.proof(&c.to_string(), &mut meter) == GlyphProof::Drawn)
            .count();
        (drawn, program.proof("huge", &mut meter))
    });
    assert_eq!(
        proofs,
        (52, GlyphProof::Unproven),
        "FONT-24b 2,000 calls of a 60 KB subroutine prove; a 4 MiB subroutine never runs"
    );
    assert!(secs < 2.0, "FONT-24b bounded ({secs:.2} s of CPU)");
    let (m, secs) = cpu(|| load_simple(&type1_simple("ABCDEF+Slow", file)));
    assert_eq!(alphabet(&m), LETTERS, "FONT-24b through the loader");
    assert!(secs < 3.0, "FONT-24b loader bounded ({secs:.2} s of CPU)");
}

#[test]
fn font_24c_type1_seac_components_share_the_glyph_budget() {
    // Subr 5 pushes 46 numbers and clears them: 48 tokens and 3 operators per call. A glyph of
    // 1,360 calls stays under 4,096 operators and spends ~68,000 tokens; a seac of two such
    // glyphs spends ~136,000 > GLYPH_WORK_MAX.
    let mut subr = Vec::new();
    for _ in 0..46 {
        subr.extend(op(&[1], &[]));
    }
    subr.extend([9, 11]);
    let file = Type1Builder {
        extra_subrs: vec![subr],
        ..Type1Builder::new("Heavy")
    }
    .glyph("A", T1Glyph::Raw(calling(5, 1_360)))
    .glyph("acute", T1Glyph::Raw(calling(5, 1_360)))
    .glyph("B", T1Glyph::Box { width: 600 })
    .glyph(
        "Aacute",
        T1Glyph::Seac {
            width: 600,
            base: 65,
            accent: 0xC2,
        },
    )
    .glyph(
        "Bacute",
        T1Glyph::Seac {
            width: 600,
            base: 66,
            accent: 0xC2,
        },
    )
    .build();
    let program = parse_type1(&file.data, file.length1, file.length2).expect("parses");
    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
    assert_eq!(program.proof("A", &mut meter), GlyphProof::Drawn);
    assert_eq!(program.proof("Bacute", &mut meter), GlyphProof::Drawn);
    assert_eq!(
        program.proof("Aacute", &mut meter),
        GlyphProof::Unproven,
        "FONT-24c the components' tokens count against one glyph budget"
    );
    // The meter is the page budget: a meter that cannot pay leaves the glyph unproven.
    let mut poor = WorkMeter::new(100);
    assert_eq!(program.proof("A", &mut poor), GlyphProof::Unproven);
    assert!(
        poor.exhausted(),
        "FONT-24c an exhausted meter marks the load"
    );
}

#[test]
fn font_24d_seac_takes_exactly_five_operands() {
    let seac = |args: &[i32]| [op(&[0, 600], &[13]), op(args, &[12, 6])].concat();
    let file = Type1Builder::new("Seac")
        .glyph("A", T1Glyph::Box { width: 600 })
        .glyph("acute", T1Glyph::Box { width: 300 })
        .glyph("five", T1Glyph::Raw(seac(&[0, 0, 0, 65, 194])))
        .glyph("six", T1Glyph::Raw(seac(&[0, 0, 65, 194, 65, 194])))
        .build();
    let program = parse_type1(&file.data, file.length1, file.length2).expect("parses");
    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
    assert_eq!(program.proof("five", &mut meter), GlyphProof::Drawn);
    assert_eq!(
        program.proof("six", &mut meter),
        GlyphProof::Unproven,
        "FONT-24d six operands are not a seac (the components would come from the wrong slots)"
    );
}

/// Segments of glyph `gid` as ttf-parser draws them, with no pre-check.
#[derive(Default)]
pub(super) struct Segs(pub(super) usize);

impl OutlineBuilder for Segs {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {}
}

#[test]
fn font_29c_truetype_composite_fan_out_is_bounded() {
    let mut t = TtfBuilder::new();
    let a = t.unicode_glyph('A', "A", true);
    let c = t.raw_glyph("C", composite(&[a, a]), 500);
    t.cmap31.push((u32::from('C'), c));
    // 30 levels of two references to the level below: 2^30 triangles for ttf-parser.
    let mut level = a;
    for depth in 0..30 {
        level = t.raw_glyph(&format!("level{depth}"), composite(&[level, level]), 500);
    }
    t.cmap31.push((u32::from('B'), level));
    let data = t.build();
    let face = ttf_parser::Face::parse(&data, 0).expect("face");
    let outlines = Outlines::of_face(&face);
    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
    let (drawn, secs) = cpu(|| {
        (
            outlines.drawn(a, &mut meter),
            outlines.drawn(c, &mut meter),
            outlines.drawn(level, &mut meter),
        )
    });
    assert_eq!(
        drawn,
        (true, true, false),
        "FONT-29c the fan-out never draws"
    );
    assert!(secs < 1.0, "FONT-29c bounded ({secs:.2} s of CPU)");
    assert!(
        meter.used() <= 3 * GLYPH_WORK_MAX as usize,
        "FONT-29c work charged"
    );
    let mut f = winansi_truetype("ABCDEF+Fan", "", "");
    f.program = Program::TrueType(data);
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(alphabet(&m), "AC", "FONT-29c through the loader");
}

/// A name-keyed CFF: a box `A`, an `acute` box, and glyphs that use global and local
/// subroutines, hint masks and `seac`; `B` fans out 8 calls per level over 9 levels.
pub(super) fn subroutine_cff() -> Vec<u8> {
    let call_global = |j: i32| [t2(j - 107), vec![29]].concat();
    let mut gsubrs: Vec<Vec<u8>> = (0..9)
        .map(|k| {
            let mut s: Vec<u8> = (0..8).flat_map(|_| call_global(k + 1)).collect();
            s.push(11);
            s
        })
        .collect();
    gsubrs.push(vec![11]);
    let moveto = [t2(100), t2(0), vec![21]].concat();
    let lineto = [t2(400), t2(0), vec![5]].concat();
    let glyph = |middle: Vec<u8>| [moveto.clone(), middle, lineto.clone(), vec![14]].concat();
    CffBuilder {
        gsubrs,
        subrs: vec![[t2(0), t2(500), vec![5, 11]].concat()],
        ..CffBuilder::new("Subrs")
    }
    .glyph("A", true)
    .raw_glyph("B", glyph(call_global(0)))
    .raw_glyph("C", glyph(call_global(9)))
    .raw_glyph("D", glyph([t2(-107), vec![10]].concat()))
    .raw_glyph(
        "E",
        [t2(0), t2(50), vec![18, 19, 0x80], glyph(Vec::new())].concat(),
    )
    .glyph("acute", true)
    .raw_glyph("Aacute", [t2(0), t2(0), t2(65), t2(194), vec![14]].concat())
    // `seac` runs its base one level deeper: G's fan-out (from gsubr 1) still fits ttf-parser's
    // nesting limit there, so Gacute is a bomb for ttf-parser unless the pre-check follows seac.
    .raw_glyph("Gacute", [t2(0), t2(0), t2(71), t2(194), vec![14]].concat())
    // A hint mask byte that reads as `callgsubr` unless the mask is skipped as ttf-parser does.
    .raw_glyph(
        "F",
        [t2(0), t2(50), vec![18, 19, 29], glyph(Vec::new())].concat(),
    )
    .raw_glyph("G", glyph(call_global(1)))
    .build()
}

#[test]
fn font_29d_cff_subroutine_fan_out_is_bounded() {
    let data = subroutine_cff();
    let outlines = Outlines::of_cff(&data).expect("CFF");
    assert!(
        matches!(outlines, Outlines::Cff { .. }),
        "FONT-29d laid out"
    );
    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
    let (drawn, secs) = cpu(|| {
        (1..=10)
            .map(|g| outlines.drawn(g, &mut meter))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        drawn,
        [true, false, true, true, true, true, true, false, true, false],
        "FONT-29d A, C (global), D (local), E (hint mask), acute, Aacute (seac) and F (mask byte \
         29) draw; B, G (fan-outs) and Gacute (a seac whose base fans out) never"
    );
    assert!(secs < 1.0, "FONT-29d bounded ({secs:.2} s of CPU)");
    // Bare CFF (Type1C) through the loader.
    let mut f = SimpleFont::new("Type1", "ABCDEF+Subrs");
    f.encoding = Some("<< /Differences [65 /A /B /C /D /E 193 /Aacute] >>".into());
    f.first_char = 32;
    f.widths = Some(vec![500.0; 224]);
    f.flags = Some(32);
    f.program = Program::Cff(data.clone());
    let typed = |m: &FontModel| -> String {
        [65u8, 66, 67, 68, 69, 193]
            .iter()
            .filter_map(|c| typeable(m, c1(*c)))
            .collect()
    };
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(typed(&m), "ACDEÁ", "FONT-29d Type1C");
    // The same program as OpenType-CFF.
    let mut t = TtfBuilder::new();
    for (ch, name) in [('A', "A"), ('B', "B"), ('C', "C"), ('D', "D"), ('E', "E")] {
        t.unicode_glyph(ch, name, true);
    }
    t.glyph("acute", true, 500);
    t.unicode_glyph('Á', "Aacute", true);
    t.glyph("Gacute", true, 500);
    t.glyph("F", true, 500);
    t.glyph("G", true, 500);
    t.cff = Some(data);
    let mut f = winansi_truetype("ABCDEF+SubrsOT", "", "");
    f.program = Program::OpenType(t.build());
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(typed(&m), "ACDEÁ", "FONT-29d OpenType-CFF");
}

/// Real fonts of the repository: (path, bare CFF?).
fn real_fonts() -> Vec<(String, bool)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut fonts: Vec<(String, bool)> = [
        "FoxitDingbats",
        "FoxitFixed",
        "FoxitFixedBold",
        "FoxitFixedBoldItalic",
        "FoxitFixedItalic",
        "FoxitSerif",
        "FoxitSerifBold",
        "FoxitSerifBoldItalic",
        "FoxitSerifItalic",
        "FoxitSymbol",
    ]
    .iter()
    .map(|n| (format!("../public/pdfjs/standard_fonts/{n}.pfb"), true))
    .collect();
    for n in ["Regular", "Bold", "Italic", "BoldItalic"] {
        fonts.push((
            format!("../public/pdfjs/standard_fonts/LiberationSans-{n}.ttf"),
            false,
        ));
    }
    fonts.push(("resources/fonts/NotoSans-Regular.ttf".into(), false));
    fonts
        .into_iter()
        .map(|(p, cff)| (root.join(p).to_string_lossy().into_owned(), cff))
        .collect()
}

#[test]
fn font_29e_pre_check_never_rejects_a_glyph_ttf_parser_draws() {
    let mut glyphs = 0usize;
    for (path, bare_cff) in real_fonts() {
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let face = (!bare_cff).then(|| ttf_parser::Face::parse(&data, 0).expect("face"));
        let outlines = match &face {
            Some(face) => Outlines::of_face(face),
            None => Outlines::of_cff(&data).expect("CFF"),
        };
        let mut meter = WorkMeter::new(usize::MAX);
        let count = match (&outlines, &face) {
            (Outlines::Glyf { .. }, Some(face)) => face.number_of_glyphs(),
            (Outlines::Cff { table, .. }, _) => table.number_of_glyphs(),
            _ => panic!("FONT-29e {path}: outlines readable"),
        };
        for gid in 0..count {
            let mut segs = Segs::default();
            let direct = match &outlines {
                Outlines::Glyf { table, .. } => table.outline(GlyphId(gid), &mut segs).is_some(),
                Outlines::Cff { table, .. } => table.outline(GlyphId(gid), &mut segs).is_ok(),
                _ => false,
            };
            assert_eq!(
                outlines.drawn(gid, &mut meter),
                direct && segs.0 > 0,
                "FONT-29e {path} GID {gid}"
            );
        }
        assert!(!meter.exhausted());
        glyphs += usize::from(count);
        if let Outlines::Cff { guard, table } = &outlines {
            names_match_ttf_parser(guard.layout(), table, &path);
        }
    }
    assert!(glyphs > 8_000, "FONT-29e {glyphs} glyphs compared");
}

/// CffNames (one charset walk) gives every name ttf-parser's `glyph_name` gives, first GID first.
fn names_match_ttf_parser(layout: &CffLayout<'_>, table: &ttf_parser::cff::Table<'_>, what: &str) {
    let names = CffNames::new(layout, table);
    let mut first = std::collections::HashMap::new();
    for gid in 0..table.number_of_glyphs() {
        if let Some(name) = table.glyph_name(GlyphId(gid)) {
            first.entry(name.to_string()).or_insert(gid);
        }
    }
    for (name, gid) in &first {
        assert_eq!(names.gid(name), Some(*gid), "{what}: {name}");
    }
}

#[test]
fn font_29f_charset_maps_are_linear() {
    // 40,000 glyphs, one charset range each: ttf-parser's per-glyph walk is 8 × 10^8 steps.
    let mut b = CffBuilder {
        charset_format: 2,
        ..CffBuilder::new("Many")
    };
    for i in 0..40_000 {
        b = b.glyph(&format!("g{i:05}"), i % 2 == 0);
    }
    let data = b.build();
    let table = ttf_parser::cff::Table::parse(&data).expect("CFF");
    let layout = CffLayout::parse(&data).expect("layout");
    let (names, secs) = cpu(|| CffNames::new(&layout, &table));
    assert!(secs < 1.0, "FONT-29f O(glyphs) ({secs:.2} s of CPU)");
    for gid in [1u16, 2, 777, 20_000, 39_999, 40_000] {
        let name = table.glyph_name(GlyphId(gid)).expect("named");
        assert_eq!(names.gid(name), Some(gid), "FONT-29f {name}");
    }
    // CID-keyed, formats 1 and 2: the CID → GID map equals ttf-parser's glyph_cid inverse.
    for format in [1, 2] {
        let mut b = CffBuilder {
            charset_format: format,
            ..CffBuilder::new("ManyCID")
        };
        for cid in (0..600).map(|i| (i * 7 + 3) % 1_000 + 1) {
            b = b.cid_glyph(cid, true);
        }
        let data = b.build();
        let table = ttf_parser::cff::Table::parse(&data).expect("CFF");
        let map = cff_cid_to_gid(&CffLayout::parse(&data).expect("layout")).expect("CID");
        for gid in 0..table.number_of_glyphs() {
            let cid = table.glyph_cid(GlyphId(gid)).expect("cid");
            assert_eq!(
                map.get(&cid).copied(),
                Some(gid),
                "FONT-29f format {format}"
            );
        }
    }
}

/// A minimal CFF2 table: Top DICT with CharStrings, empty Global Subr INDEX, one charstring.
fn minimal_cff2() -> Vec<u8> {
    let mut d = vec![2, 0, 5, 0, 6];
    d.extend([29, 0, 0, 0, 15, 17]); // CharStrings at 15
    d.extend([0, 0, 0, 0]); // Global Subr INDEX (u32 count 0)
    d.extend([0, 0, 0, 1, 1, 1, 2, 139]); // CharStrings: one charstring
    d
}

#[test]
fn font_30b_cff2_only_programs_are_unsupported() {
    let mut t = TtfBuilder::new();
    t.unicode_glyph('A', "A", true);
    t.cff2 = Some(minimal_cff2());
    let data = t.build();
    let face = ttf_parser::Face::parse(&data, 0).expect("face");
    assert!(
        face.tables().cff2.is_some(),
        "FONT-30b the fixture has a CFF2 table"
    );
    let mut f = winansi_truetype("ABCDEF+Variable", "", "");
    f.program = Program::OpenType(data);
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::FontProgramUnsupported),
        "FONT-30b CFF2 charstrings are not pre-checked, so never outlined"
    );
}

#[test]
fn font_08b_tounicode_codes_match_by_value() {
    let two_byte = cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         3 beginbfchar\n<0041> <0042>\n<0042> <0042>\n<0043> <0106>\nendbfchar",
    );
    let mut f = std14("Helvetica", "/WinAnsiEncoding");
    f.tounicode = Some(two_byte);
    let m = load_simple(&f);
    assert_eq!(m.text(c1(0x41)), None, "FONT-08b name A vs ToUnicode B");
    assert_eq!(typeable(&m, c1(0x41)), None);
    assert_eq!(typeable(&m, c1(0x42)), Some('B'), "FONT-08b agreement");
    assert_eq!(m.text(c1(0x43)), None, "FONT-08b C vs Ć");
    // A Type0 font's ToUnicode with 1-byte sources maps the 2-byte codes of the same value.
    let (program, gids) = latin_truetype("A", "");
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Short");
    f.program = Program::TrueType(program);
    let mut map = vec![0u8; 0x42 * 2];
    map[0x41 * 2 + 1] = gids[0].1 as u8;
    f.cid_to_gid = Some(Some(map));
    f.tounicode = Some(cmap(
        "1 begincodespacerange <00> <FF> endcodespacerange\n1 beginbfchar\n<41> <0041>\nendbfchar",
    ));
    let m = load_type0(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.text(c2(0x41)), Some("A"), "FONT-08b <41> maps CID 0x0041");
    assert_eq!(typeable(&m, c2(0x41)), Some('A'));
}

#[test]
fn font_14b_tounicode_text_is_bounded_before_expansion() {
    // 65,536 codes × a 512-byte destination is 32 MiB of text from 1 KB of CMap.
    let dst = "0041".repeat(256);
    let body = format!("1 beginbfrange\n<0000> <FFFF> <{dst}>\nendbfrange");
    let (parsed, peak) = thread_peak(|| parse_tounicode(&cmap(&body)).is_ok());
    assert!(!parsed, "FONT-14b destinations past TOUNICODE_MAX_DECODED");
    assert!(
        peak < 8 << 20,
        "FONT-14b nothing expanded ({peak} bytes peak)"
    );
    // A short destination over the same range still parses.
    let body = "1 beginbfrange\n<0000> <FFFF> <0020>\nendbfrange";
    assert!(parse_tounicode(&cmap(body)).is_ok());
}

#[test]
fn font_14c_model_memory_is_paid_from_the_page_budget() {
    // Each font maps all 65,536 codes from one bfrange line (~9 MB of model). Five of them under
    // a 32 MiB budget: the first loads, later ones run the budget out.
    let mut b = PdfBuilder::new();
    let (program, _) = latin_truetype("A", "");
    let ids: Vec<u32> = (0..5)
        .map(|i| {
            let mut f = Type0Font::new("CIDFontType2", &format!("ABCDEF+Wide{i}"));
            f.program = Program::TrueType(program.clone());
            f.tounicode = Some(cmap("1 beginbfrange\n<0000> <FFFF> <0020>\nendbfrange"));
            add_type0(&mut b, &f)
        })
        .collect();
    let names: Vec<String> = (0..ids.len()).map(|i| format!("F{i}")).collect();
    let fonts: Vec<(&str, u32)> = names
        .iter()
        .map(String::as_str)
        .zip(ids.iter().copied())
        .collect();
    let snap = snapshot(page_with_fonts(b, &fonts));
    let cache = FontCache::new();
    let mut budget = DecodeBudget::new(32 << 20);
    let refusals: Vec<Option<TextReason>> = ids
        .iter()
        .map(|id| {
            let dict = snap
                .doc
                .get_object((*id, 0))
                .and_then(Object::as_dict)
                .expect("font");
            cache
                .get_or_load(&snap.doc, FontKey::Indirect((*id, 0)), dict, &mut budget)
                .refusal
        })
        .collect();
    assert_eq!(refusals[0], None, "FONT-14c one such font fits");
    assert_eq!(
        refusals[4],
        Some(TextReason::PageTooComplex),
        "FONT-14c the page budget bounds the models' memory: {refusals:?}"
    );
}

#[test]
fn font_28c_descriptor_null_and_charset_and_dw() {
    // /FontDescriptor null (or a reference to nothing) is an absent descriptor.
    for extra in ["/FontDescriptor null", "/FontDescriptor 9999 0 R"] {
        let mut f = std14("Helvetica", "/WinAnsiEncoding");
        f.font_extra = extra.into();
        let m = load_simple(&f);
        assert_eq!(m.refusal, None, "FONT-28c {extra}");
        assert_eq!(typeable(&m, c1(b'A')), Some('A'));
    }
    // /CharSet present but not a string: no Type1 glyph is proven.
    let file = Type1Builder::new("CS")
        .glyph("A", T1Glyph::Box { width: 600 })
        .build();
    let mut f = type1_simple("ABCDEF+CS", file);
    assert_eq!(alphabet(&load_simple(&f)), "A");
    f.descriptor_extra = "/CharSet 5".into();
    assert_eq!(
        alphabet(&load_simple(&f)),
        "",
        "FONT-28c malformed /CharSet"
    );
    // /DW present but not a number.
    let (program, _) = latin_truetype("A", "");
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+DW");
    f.program = Program::TrueType(program);
    f.tounicode = Some(tounicode_bfchar(&[(1, 2, "A")]));
    assert_eq!(load_type0(&f).refusal, None);
    f.cid_extra = "/DW /Wide".into();
    assert_eq!(
        load_type0(&f).refusal,
        Some(TextReason::FontUnsupported),
        "FONT-28c malformed /DW"
    );
}

#[test]
fn font_28d_structural_refusals_report_the_lowest_code() {
    // A FontFile3 that does not match the font type (FONT_PROGRAM_UNSUPPORTED) and malformed
    // /Widths (FONT_UNSUPPORTED, earlier in §A.10): the earlier code is reported.
    let mut f = SimpleFont::new("TrueType", "ABCDEF+Both");
    f.flags = Some(32);
    f.program = Program::Raw {
        key: "FontFile3",
        subtype: Some("Type1C"),
        data: vec![1, 0, 4, 4],
    };
    f.font_extra = "/FirstChar 32 /LastChar 40 /Widths [500]".into();
    assert_eq!(load_simple(&f).refusal, Some(TextReason::FontUnsupported));
    f.font_extra = "/FirstChar 32 /LastChar 32 /Widths [500]".into();
    assert_eq!(
        load_simple(&f).refusal,
        Some(TextReason::FontProgramUnsupported)
    );
}

#[test]
fn font_28e_type3_hash_decodes_a_bounded_amount() {
    let type3 = |b: &mut PdfBuilder, procs: &[Vec<u8>]| -> u32 {
        let ids: Vec<u32> = procs.iter().map(|p| b.add_flate("", p)).collect();
        b.add(format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a {} 0 R /b {} 0 R /c {} 0 R >> \
             /Encoding << /Type /Encoding /Differences [97 /a /b /c] >> /FirstChar 97 \
             /LastChar 99 /Widths [500 500 500] /Resources << >> >>",
            ids[0], ids[1], ids[2]
        ))
    };
    let big = vec![b' '; 3 << 20];
    let mut b = PdfBuilder::new();
    let id = type3(&mut b, &[big.clone(), big.clone(), big]);
    let mut budget = DecodeBudget::new(PAGE_DECODE_BUDGET);
    let m = load_with(b, id, &mut budget);
    assert_eq!(
        m.refusal,
        Some(TextReason::Type3),
        "FONT-28e still a run-level refusal"
    );
    let spent = PAGE_DECODE_BUDGET - budget.remaining();
    assert!(
        spent < 5 << 20,
        "FONT-28e ≤ 4 MiB of hash-only decoding ({spent} bytes)"
    );
    // Small glyph procedures still hash by content, the same across renumbered files.
    let procs = |c: &str| vec![c.as_bytes().to_vec(), b"0 0 m".to_vec(), b"1 1 l".to_vec()];
    let hash = |pad: usize, c: &str| {
        let mut b = PdfBuilder::new();
        for _ in 0..pad {
            b.add("<< >>");
        }
        let id = type3(&mut b, &procs(c));
        load_with(b, id, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).content_hash
    };
    assert_eq!(
        hash(0, "0 0 m 5 5 l f"),
        hash(3, "0 0 m 5 5 l f"),
        "FONT-28e identity"
    );
    assert_ne!(
        hash(0, "0 0 m 5 5 l f"),
        hash(0, "0 0 m 6 6 l f"),
        "FONT-28e content"
    );
}

#[test]
fn font_12b_bidi_isolates_and_deprecated_controls_are_excluded() {
    for cp in 0x2060..=0x206F {
        let ch = char::from_u32(cp).expect("BMP");
        assert!(!writable_char(ch), "FONT-12b U+{cp:04X} never typeable");
        assert!(
            reading_reason(ch).is_some(),
            "FONT-12b U+{cp:04X} not readable"
        );
    }
    assert!(
        writable_char('\u{205F}'),
        "FONT-12b the medium space before them stays"
    );
}

#[test]
fn font_15b_annex_d_duplicates_agree_with_tounicode() {
    // Word writes WinAnsi 0xA0 (`space`) as U+00A0 and 0xAD (`hyphen`) as U+00AD.
    let mut f = winansi_truetype("ABCDEF+Word", "A-", " ");
    f.tounicode = Some(tounicode_bfchar(&[
        (0xA0, 1, "\u{a0}"),
        (0xAD, 1, "\u{ad}"),
        (0x41, 1, "\u{a0}"),
        (0x20, 1, "\u{a0}"),
    ]));
    let m = load_simple(&f);
    assert_eq!(m.refusal, None);
    assert_eq!(m.text(c1(0xA0)), Some("\u{a0}"), "FONT-15b no-break space");
    assert_eq!(typeable(&m, c1(0xA0)), Some('\u{a0}'));
    assert_eq!(m.text(c1(0xAD)), Some("\u{ad}"), "FONT-15b soft hyphen");
    assert_eq!(
        m.text(c1(0x41)),
        None,
        "FONT-15b only the Annex D pairs agree"
    );
    assert_eq!(
        m.text(c1(0x20)),
        None,
        "FONT-15b `space` at 0x20 is U+0020, not U+00A0 (only the Annex D codes)"
    );
}

#[test]
fn font_29g_cid_keyed_cff_local_subroutines_through_fdselect() {
    let local_call = |j: i32| [t2(j - 107), vec![10]].concat();
    let glyph = |middle: Vec<u8>| [t2(100), t2(0), vec![21], middle, vec![14]].concat();
    // Local subr 0 draws a line; 1–9 each call the next one 8 times; 10 returns.
    let mut subrs = vec![[t2(400), t2(0), vec![5, 11]].concat()];
    for k in 1..10 {
        let mut s: Vec<u8> = (0..8).flat_map(|_| local_call(k + 1)).collect();
        s.push(11);
        subrs.push(s);
    }
    subrs.push(vec![11]);
    for format in [0u8, 3] {
        let data = CffBuilder {
            subrs: subrs.clone(),
            fdselect_format: format,
            ..CffBuilder::new("CIDSubrs")
        }
        .raw_cid_glyph(10, glyph(local_call(0)))
        .raw_cid_glyph(11, glyph([local_call(1), local_call(0)].concat()))
        .raw_cid_glyph(12, glyph([t2(400), t2(0), vec![5]].concat()))
        .build();
        let outlines = Outlines::of_cff(&data).expect("CFF");
        let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
        let (drawn, secs) = cpu(|| {
            (1..=3)
                .map(|g| outlines.drawn(g, &mut meter))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            drawn,
            [true, false, true],
            "FONT-29g FDSelect format {format}"
        );
        assert!(secs < 1.0, "FONT-29g bounded ({secs:.2} s of CPU)");
        let table = ttf_parser::cff::Table::parse(&data).expect("CFF");
        for gid in [1u16, 3] {
            let mut segs = Segs::default();
            assert!(table.outline(GlyphId(gid), &mut segs).is_ok() && segs.0 > 0);
        }
        let mut f = Type0Font::new("CIDFontType0", "ABCDEF+CIDSubrs");
        f.program = Program::CidCff(data);
        f.tounicode = Some(tounicode_bfchar(&[
            (10, 2, "A"),
            (11, 2, "B"),
            (12, 2, "C"),
        ]));
        let m = load_type0(&f);
        assert_eq!(m.refusal, None);
        assert_eq!(alphabet(&m), "AC", "FONT-29g through a Type0 font");
    }
}
