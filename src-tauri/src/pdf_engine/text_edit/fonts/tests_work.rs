//! Work bounds of the second review-T2 fix pass: CFF DICTs are walked lazily, capped and charged
//! per glyph (FONT-29h); glyph walks stop at what the page budget can still pay and an exhausted
//! meter walks nothing (FONT-29i); the CMap and Type1 clear-text token caps (FONT-14d); a vertical
//! Type0 font reports `VERTICAL` whatever else is wrong (FONT-28f); CFF custom-encoding names
//! (FONT-35b).

use super::cff_layout::DICT_LEN_MAX;
use super::glyph_budget::WorkMeter;
use super::program::Outlines;
use super::tests::{c1, c2};
use super::tests_bounds::{cpu, Segs};
use super::tounicode::parse_tounicode;
use super::type1::{parse_type1, GlyphProof};
use super::{FontCache, FontKey, FontModel};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::limits::{PAGE_DECODE_BUDGET, TOUNICODE_MAX_DECODED};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::cff::{t2, CffBuilder, CffEncodingSpec};
use crate::pdf_engine::text_edit::testkit::fonts::{
    add_type0, cmap, load_one, load_simple, load_type0, page_with_fonts, snapshot, Program,
    SimpleFont, Type0Font,
};
use crate::pdf_engine::text_edit::testkit::pdf::PdfBuilder;
use crate::pdf_engine::text_edit::testkit::thread_peak;
use crate::pdf_engine::text_edit::testkit::ttf::{composite, TtfBuilder};
use crate::pdf_engine::text_edit::testkit::type1::{op, T1Glyph, Type1Builder};
use lopdf::Object;
use std::sync::Arc;
use ttf_parser::GlyphId;

/// Loads the one font of `f`'s page with a page budget of `budget` bytes, measuring only the
/// load: the model, its thread CPU seconds and the peak bytes it held on this thread.
fn measured_type0(f: &Type0Font, budget: usize) -> (Arc<FontModel>, f64, usize) {
    let mut b = PdfBuilder::new();
    let id = add_type0(&mut b, f);
    let snap = snapshot(page_with_fonts(b, &[("F1", id)]));
    let dict = snap
        .doc
        .get_object((id, 0))
        .and_then(Object::as_dict)
        .expect("font dict");
    let mut budget = DecodeBudget::new(budget);
    let ((model, peak), secs) = cpu(|| {
        thread_peak(|| {
            FontCache::new().get_or_load(&snap.doc, FontKey::Indirect((id, 0)), dict, &mut budget)
        })
    });
    (model, secs, peak)
}

/// `100 0 rmoveto`, `middle`, `endchar`.
fn t2_glyph(middle: Vec<u8>) -> Vec<u8> {
    [t2(100), t2(0), vec![21], middle, vec![14]].concat()
}

/// A CID-keyed CFF with one Font DICT: CID 1 draws a line without subroutines; CIDs 2..=n+1 draw
/// theirs through local subroutine 0. The Private DICT carries `padding` after its entries.
fn cid_local_cff(n: u16, padding: Vec<u8>) -> Vec<u8> {
    let mut b = CffBuilder {
        subrs: vec![[t2(400), t2(0), vec![5, 11]].concat()],
        private_padding: padding,
        ..CffBuilder::new("CIDDict")
    }
    .raw_cid_glyph(1, t2_glyph([t2(400), t2(0), vec![5]].concat()));
    for cid in 2..=n + 1 {
        b = b.raw_cid_glyph(cid, t2_glyph([t2(-107), vec![10]].concat()));
    }
    b.build()
}

/// Whether ttf-parser itself (no pre-check) draws `gid` of a bare CFF.
fn ttf_parser_draws(data: &[u8], gid: u16) -> bool {
    let mut segs = Segs::default();
    ttf_parser::cff::Table::parse(data)
        .is_some_and(|t| t.outline(GlyphId(gid), &mut segs).is_ok() && segs.0 > 0)
}

#[test]
fn font_29h_cff_dicts_are_capped_lazy_and_charged() {
    // 1,000 glyphs through one Font DICT whose Private DICT is 4 MiB of numbers or of operators:
    // before the fix each glyph walked it (and built one 24-byte entry per operator byte).
    for (what, pad) in [("numbers", 139u8), ("operators", 0u8)] {
        let data = cid_local_cff(1_000, vec![pad; 4 << 20]);
        let mut f = Type0Font::new("CIDFontType0", "ABCDEF+BigDict");
        // CJK destinations: no code reads as a space (a blank glyph with a width would count).
        f.tounicode = Some(cmap("1 beginbfrange\n<0001> <03E9> <4E00>\nendbfrange"));
        f.program = Program::CidCff(data.clone());
        let (m, secs, peak) = measured_type0(&f, PAGE_DECODE_BUDGET);
        println!("FONT-29h {what}: {secs:.3} s of CPU, {peak} bytes peak");
        assert_eq!(m.refusal, None, "FONT-29h {what}");
        assert!(
            m.drawable(c2(1)),
            "FONT-29h {what}: no local subroutine, draws"
        );
        assert!(
            (2..=1_001).all(|cid| !m.drawable(c2(cid))),
            "FONT-29h {what}: a Private DICT past DICT_LEN_MAX gives no local subroutines"
        );
        assert!(secs < 1.0, "FONT-29h {what}: {secs:.2} s of CPU");
        assert!(
            peak < data.len() + (4 << 20),
            "FONT-29h {what}: {peak} bytes peak for a {}-byte program",
            data.len()
        );
    }
    // Just past the cap the guard does not follow the Private DICT although ttf-parser would;
    // just under it, it does.
    for (pad, drawn) in [(DICT_LEN_MAX, false), (DICT_LEN_MAX - 64, true)] {
        let data = cid_local_cff(1, vec![139; pad]);
        assert!(ttf_parser_draws(&data, 2), "FONT-29h precondition");
        let outlines = Outlines::of_cff(&data).expect("CFF");
        let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
        assert_eq!(
            outlines.drawn(2, &mut meter),
            drawn,
            "FONT-29h a {}-byte Private DICT",
            pad + 10
        );
    }
    // A name-keyed Private DICT or a Top DICT past the cap: the program is unreadable.
    for (what, builder) in [
        (
            "Private",
            CffBuilder {
                private_padding: vec![139; DICT_LEN_MAX],
                ..CffBuilder::new("BigPrivate")
            },
        ),
        (
            "Top",
            CffBuilder {
                top_padding: vec![139; DICT_LEN_MAX],
                ..CffBuilder::new("BigTop")
            },
        ),
    ] {
        let data = builder.glyph("A", true).build();
        assert!(ttf_parser_draws(&data, 1), "FONT-29h {what} precondition");
        let outlines = Outlines::of_cff(&data).expect("ttf-parser reads it");
        assert_eq!(
            outlines.refusal(),
            Some(TextReason::FontProgramUnreadable),
            "FONT-29h a {what} DICT past DICT_LEN_MAX"
        );
    }
    // Every glyph calling a local subroutine pays the Font DICT (11 bytes) and Private DICT bytes
    // ttf-parser walks again for it; the guard walks them once (per-Font-DICT cache).
    let (n, pad) = (64u16, 4_000usize);
    let data = cid_local_cff(n, vec![139; pad]);
    let outlines = Outlines::of_cff(&data).expect("CFF");
    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
    assert!(
        (2..=n + 1).all(|gid| outlines.drawn(gid, &mut meter)),
        "FONT-29h under the cap they draw"
    );
    let dicts = (pad + 10 + 11) as u64;
    let used = meter.used() as u64;
    println!("FONT-29h {n} glyphs × {dicts} DICT bytes: {used} units charged");
    assert!(
        used >= u64::from(n) * dicts,
        "FONT-29h ttf-parser's per-glyph DICT walk is charged ({used} units)"
    );
    assert!(
        used <= u64::from(n + 1) * dicts + u64::from(n) * 64,
        "FONT-29h the guard walks the DICTs once ({used} units)"
    );
}

/// A TrueType program: glyph `A` draws; the returned GID is 30 levels of two references to the
/// level below (2^30 leaves for ttf-parser).
fn glyf_fan_out() -> (Vec<u8>, u16) {
    let mut t = TtfBuilder::new();
    let mut level = t.unicode_glyph('A', "A", true);
    for depth in 0..30 {
        level = t.raw_glyph(&format!("level{depth}"), composite(&[level, level]), 500);
    }
    (t.build(), level)
}

/// Global subroutines: 0 pushes 48 numbers (5 bytes each) and draws a line; 1 calls 0 eight
/// times. `heavy_charstring` calls 1 seventy times: ~137,000 units in ~1,800 operators, past
/// `GLYPH_WORK_MAX` before the operator cap.
fn heavy_gsubrs() -> Vec<Vec<u8>> {
    let mut numbers: Vec<u8> = (0..48).flat_map(|_| [255u8, 0, 1, 0, 0]).collect();
    numbers.extend([5, 11]);
    let mut caller: Vec<u8> = (0..8).flat_map(|_| [t2(-107), vec![29]].concat()).collect();
    caller.push(11);
    vec![numbers, caller]
}

fn heavy_charstring() -> Vec<u8> {
    let mut code: Vec<u8> = (0..70)
        .flat_map(|_| [t2(1 - 107), vec![29]].concat())
        .collect();
    code.push(14);
    code
}

/// A Type1 program whose glyph `fan` calls subr 6 175 times; subr 6 calls subr 5 eight times;
/// subr 5 pushes 46 numbers and draws a line (~63,000 tokens before the operator cap).
fn type1_fan_out() -> super::type1::Type1Program {
    let mut numbers = op(&[1; 46], &[5]);
    numbers.push(11);
    let mut caller: Vec<u8> = (0..8).flat_map(|_| op(&[5], &[10])).collect();
    caller.push(11);
    let mut fan = op(&[0, 600], &[13]);
    for _ in 0..175 {
        fan.extend(op(&[6], &[10]));
    }
    fan.push(14);
    let file = Type1Builder {
        extra_subrs: vec![numbers, caller],
        ..Type1Builder::new("Fan")
    }
    .glyph("fan", T1Glyph::Raw(fan))
    .build();
    parse_type1(&file.data, file.length1, file.length2).expect("parses")
}

#[test]
fn font_29i_an_exhausted_meter_stops_glyph_work() {
    // One over-budget glyph checked 3,000 times (no memo) against 1 MiB of budget: once the meter
    // runs out nothing more is walked. Before the fix each check walked up to GLYPH_WORK_MAX.
    const CHECKS: usize = 3_000;
    let (ttf, fan) = glyf_fan_out();
    let face = ttf_parser::Face::parse(&ttf, 0).expect("face");
    let glyf = Outlines::of_face(&face);
    let cff_data = CffBuilder {
        gsubrs: heavy_gsubrs(),
        ..CffBuilder::new("Heavy")
    }
    .raw_glyph("A", heavy_charstring())
    .build();
    let cff = Outlines::of_cff(&cff_data).expect("CFF");
    let type1 = type1_fan_out();
    let (meters, secs) = cpu(|| {
        let mut meters = [(); 3].map(|_| WorkMeter::new(1 << 20));
        let [m_glyf, m_cff, m_t1] = &mut meters;
        for _ in 0..CHECKS {
            assert!(!glyf.drawn(fan, m_glyf));
            assert!(!cff.drawn(1, m_cff));
            assert_eq!(type1.proof("fan", m_t1), GlyphProof::Unproven);
        }
        meters
    });
    println!("FONT-29i {CHECKS} × 3 checks: {secs:.3} s of CPU");
    assert!(
        meters.iter().all(WorkMeter::exhausted),
        "FONT-29i the meters ran out"
    );
    assert!(
        secs < 1.0,
        "FONT-29i {secs:.2} s of CPU for {CHECKS} × 3 checks"
    );
    // Through a Type0 load: 4,000 such glyphs under a 4 MiB page budget.
    let mut b = CffBuilder {
        gsubrs: heavy_gsubrs(),
        ..CffBuilder::new("HeavyCID")
    };
    for cid in 1..=4_000 {
        b = b.raw_cid_glyph(cid, heavy_charstring());
    }
    let mut f = Type0Font::new("CIDFontType0", "ABCDEF+HeavyCID");
    f.program = Program::CidCff(b.build());
    f.tounicode = Some(cmap("1 beginbfrange\n<0001> <0FA0> <0041>\nendbfrange"));
    let (m, secs, _) = measured_type0(&f, 4 << 20);
    println!("FONT-29i Type0 load: {secs:.3} s of CPU");
    assert_eq!(m.refusal, Some(TextReason::PageTooComplex), "FONT-29i");
    assert!(secs < 1.0, "FONT-29i Type0 load: {secs:.2} s of CPU");
}

#[test]
fn font_14d_token_caps_bound_transient_memory() {
    // 2 MiB of one-byte tokens: the scan stops at the token cap (the old 2^21-token cap held all
    // of them, 48 bytes each: 100 MB).
    let brackets = "[]".repeat(TOUNICODE_MAX_DECODED / 2);
    let (parsed, peak) = thread_peak(|| parse_tounicode(brackets.as_bytes()).is_ok());
    println!("FONT-14d CMap: {peak} bytes peak");
    assert!(peak < 48 << 20, "FONT-14d CMap tokens: {peak} bytes peak");
    assert!(!parsed, "FONT-14d past the token cap");
    // The densest real form, one-destination bfrange arrays, still parses for every 2-byte code.
    let mut body = String::new();
    let codes: Vec<u32> = (0..=0xFFFF).collect();
    for chunk in codes.chunks(100) {
        body.push_str(&format!("{} beginbfrange\n", chunk.len()));
        for c in chunk {
            body.push_str(&format!("<{c:04X}> <{c:04X}> [<0041>]\n"));
        }
        body.push_str("endbfrange\n");
    }
    let dense = cmap(&body);
    assert!(dense.len() <= TOUNICODE_MAX_DECODED);
    assert_eq!(
        parse_tounicode(&dense).map(|t| t.codes().count()).ok(),
        Some(0x10000),
        "FONT-14d real CMaps fit the cap"
    );
    // A Type1 clear text of 4 MiB of names: the built-in encoding is not read past the cap.
    let file = Type1Builder::new("Pad")
        .glyph("A", T1Glyph::Box { width: 600 })
        .build();
    let mut data = b"/a ".repeat((4 << 20) / 3);
    let pad = data.len();
    data.extend_from_slice(&file.data);
    let (program, peak) =
        thread_peak(|| parse_type1(&data, pad + file.length1, file.length2).map(|p| p.builtin));
    println!("FONT-14d Type1 clear text: {peak} bytes peak");
    assert_eq!(program, Ok(None), "FONT-14d no encoding past the cap");
    assert!(
        peak < 16 << 20,
        "FONT-14d clear-text tokens: {peak} bytes peak"
    );
}

#[test]
fn font_28f_type0_vertical_is_reported_before_early_returns() {
    // `/FontDescriptor 5` (not a dictionary) is FONT_UNSUPPORTED (#19) and returns early;
    // VERTICAL (#8) is still found and reported.
    let mut f = Type0Font::new("CIDFontType2", "ABCDEF+Vert");
    f.flags = None;
    f.encoding = "/Identity-V".into();
    f.cid_extra = "/FontDescriptor 5".into();
    let m = load_type0(&f);
    assert_eq!((m.refusal, m.vertical), (Some(TextReason::Vertical), true));
    f.encoding = "/Identity-H".into();
    f.cid_extra = "/WMode 1 /FontDescriptor 5".into();
    let m = load_type0(&f);
    assert_eq!(
        (m.refusal, m.vertical),
        (Some(TextReason::Vertical), true),
        "FONT-28f CIDFont WMode"
    );
    // No /DescendantFonts at all.
    let mut b = PdfBuilder::new();
    let id = b.add("<< /Type /Font /Subtype /Type0 /BaseFont /NoKids /Encoding /Identity-V >>");
    let m = load_one(b, id);
    assert_eq!(
        (m.refusal, m.vertical),
        (Some(TextReason::Vertical), true),
        "FONT-28f no descendant"
    );
}

#[test]
fn font_35b_cff_custom_encoding_names_match_ttf_parser() {
    // Names come from one charset walk; they equal ttf-parser's per-GID `glyph_name` for every
    // charset format, standard and custom (String INDEX) names alike.
    let codes = [0x41u8, 0x42, 0x43, 0xE9];
    for charset_format in [0u8, 1, 2] {
        let mut b = CffBuilder {
            charset_format,
            ..CffBuilder::new("Names")
        }
        .encoding(CffEncodingSpec::Format0(codes.to_vec()));
        for name in ["A", "B", "Cfoo", "eacute"] {
            b = b.glyph(name, true);
        }
        let data = b.build();
        let table = ttf_parser::cff::Table::parse(&data).expect("CFF");
        let mut f = SimpleFont::new("Type1", "ABCDEF+Names");
        f.first_char = 32;
        f.widths = Some(vec![500.0; 224]);
        f.flags = Some(4);
        f.program = Program::Cff(data.clone());
        let m = load_simple(&f);
        assert_eq!(m.refusal, None);
        for (gid, code) in (1u16..).zip(codes) {
            assert_eq!(
                m.info(c1(code)).and_then(|i| i.glyph_name.as_deref()),
                table.glyph_name(GlyphId(gid)),
                "FONT-35b charset format {charset_format}, code {code:#04X}"
            );
        }
    }
}
