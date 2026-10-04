//! FONT-34 loader fuzz (§B.21): 1,000 deterministic xorshift mutations — byte flips,
//! truncations, inserted PostScript/PDF tokens, huge numbers, duplicated slices — over
//! ToUnicode, Type1, TrueType and CFF inputs, fed to the parsers directly and through the whole
//! loader (`FontCache::get_or_load` on an in-memory document); FONT-34b mutates the font
//! dictionaries themselves (`/W`, `/DW`, `/Widths`, `/Differences`, `/Encoding`, descriptor
//! values, references). No panic; < 2 s of CPU time per 1,000 cases (summed over the workers'
//! thread clocks, so a loaded machine or the worker count does not change the bound).

use super::cff_encoding::cff_builtin_encoding;
use super::glyph_budget::WorkMeter;
use super::program::{Outlines, TrueTypeLookup};
use super::tounicode::parse_tounicode;
use super::type1::parse_type1;
use super::{FontCache, FontKey, FontModel};
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::testkit::cff::{CffBuilder, CffEncodingSpec};
use crate::pdf_engine::text_edit::testkit::fonts::{cmap, latin_truetype};
use crate::pdf_engine::text_edit::testkit::type1::{T1Encoding, T1Glyph, Type1Builder};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use std::time::Duration;

/// CPU time used by the calling thread (wall time where the platform has no thread clock).
pub(super) fn thread_cpu() -> Duration {
    #[cfg(all(
        any(target_os = "macos", target_os = "linux"),
        target_pointer_width = "64"
    ))]
    {
        #[repr(C)]
        struct Timespec {
            tv_sec: i64,
            tv_nsec: i64,
        }
        extern "C" {
            fn clock_gettime(clock: i32, tp: *mut Timespec) -> i32;
        }
        #[cfg(target_os = "macos")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 16;
        #[cfg(target_os = "linux")]
        const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
        let mut ts = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: clock_gettime only writes the timespec it is handed, which outlives the call.
        if unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0 {
            return Duration::new(
                ts.tv_sec.max(0) as u64,
                ts.tv_nsec.clamp(0, 999_999_999) as u32,
            );
        }
    }
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed()
}

/// Runs cases `0..n` on four workers; returns the sum of `run`'s results and of the workers'
/// CPU times. A panic in any case fails the test.
fn run_cases(n: usize, what: &str, run: impl Fn(usize) -> usize + Sync) -> (usize, Duration) {
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|w| {
                let run = &run;
                scope.spawn(move || {
                    let started = thread_cpu();
                    let sum = (w..n).step_by(4).map(run).sum::<usize>();
                    (sum, thread_cpu().saturating_sub(started))
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| panic!("{what}: no panic in any case"))
            })
            .fold((0, Duration::ZERO), |(a, t), (b, u)| (a + b, t + u))
    })
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const INSERTS: &[&[u8]] = &[
    b"(",
    b"<",
    b"[",
    b"<<",
    b"BI",
    b"q",
    b"99999999999999999999",
    b"-2147483648",
    b"RD ",
    b" -| ",
    b"endbfchar",
    b"beginbfrange",
    b"<FFFF>",
    b"/Subrs",
    b"/CharStrings",
    b"/lenIV 9 ",
    b"\xff\xff\xff\xff",
    b"\x00\x00",
];

fn mutate(rng: &mut XorShift, input: &[u8]) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..=rng.below(4) {
        match rng.below(5) {
            0 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] ^= 1 << rng.below(8);
            }
            1 if !out.is_empty() => {
                let i = rng.below(out.len());
                out[i] = rng.next() as u8;
            }
            2 => {
                let cut = rng.below(out.len() + 1);
                out.truncate(cut);
            }
            3 => {
                let at = rng.below(out.len() + 1);
                let token = INSERTS[rng.below(INSERTS.len())];
                out.splice(at..at, token.iter().copied());
            }
            _ if !out.is_empty() => {
                let start = rng.below(out.len());
                let end = (start + rng.below(64)).min(out.len());
                let at = rng.below(out.len() + 1);
                let slice = out[start..end].to_vec();
                out.splice(at..at, slice);
            }
            _ => {}
        }
    }
    out
}

/// One in-memory font: a TrueType/Type1/Type1C simple font or a Type0 font around `program`.
fn load(kind: usize, program: Vec<u8>, tounicode: Vec<u8>, l1: i64, l2: i64) -> usize {
    let mut doc = Document::with_version("1.7");
    let tu = doc.add_object(Stream::new(dictionary! {}, tounicode));
    let (key, file_dict) = match kind {
        0 => ("FontFile2", dictionary! {}),
        1 => (
            "FontFile",
            dictionary! { "Length1" => l1, "Length2" => l2, "Length3" => 0 },
        ),
        2 => ("FontFile3", dictionary! { "Subtype" => "Type1C" }),
        _ => ("FontFile3", dictionary! { "Subtype" => "CIDFontType0C" }),
    };
    let file = doc.add_object(Stream::new(file_dict, program));
    let desc = doc.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "ABCDEF+Fuzz", "Flags" => if kind == 0 { 32 } else { 4 },
        key => file, "Ascent" => 800, "Descent" => -200,
    });
    // A /Differences-only encoding (on a symbolic flag for the Type1/CFF kinds) keeps the name
    // path short, so the cases spend their time on the mutated programs and CMaps; TrueType is
    // non-symbolic so its glyphs are selected by name (AGL → cmap, post).
    let differences: Vec<Object> = [
        Object::Integer(32),
        "space".into(),
        Object::Integer(65),
        "A".into(),
        "B".into(),
        "C".into(),
        "Aacute".into(),
        Object::Integer(89),
        "Y".into(),
        Object::Integer(97),
        "a".into(),
        "b".into(),
        "c".into(),
    ]
    .into();
    let widths: Vec<Object> = (32..=255).map(|_| Object::Integer(500)).collect();
    let font = if kind == 3 {
        let cid = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType0", "BaseFont" => "ABCDEF+Fuzz",
            "FontDescriptor" => desc, "W" => vec![Object::Integer(1), Object::Array(vec![Object::Integer(500)])],
        });
        dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "ABCDEF+Fuzz",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(cid)], "ToUnicode" => tu,
        }
    } else {
        dictionary! {
            "Type" => "Font", "Subtype" => if kind == 0 { "TrueType" } else { "Type1" },
            "BaseFont" => "ABCDEF+Fuzz", "FirstChar" => 32, "LastChar" => 255, "Widths" => widths,
            "Encoding" => dictionary! { "Differences" => differences },
            "FontDescriptor" => desc, "ToUnicode" => tu,
        }
    };
    let id = doc.add_object(font.clone());
    let model = FontCache::new().get_or_load(
        &doc,
        FontKey::Indirect(id),
        &font,
        &mut DecodeBudget::new(PAGE_DECODE_BUDGET),
    );
    model.alphabet().len()
}

#[test]
fn font_34_loader_fuzz() {
    let tounicode = cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         2 beginbfchar <0041> <0041> <0042> <D83DDE00> endbfchar\n\
         2 beginbfrange <0050> <0060> <0061> <0070> <0072> [<0041> <00660069> <0043>] endbfrange",
    );
    let type1 = Type1Builder {
        encoding: T1Encoding::Custom(vec![(65, "A".into()), (66, "Aacute".into())]),
        ..Type1Builder::new("Fuzz")
    }
    .glyph("A", T1Glyph::Box { width: 500 })
    .glyph("B", T1Glyph::HintedBox { width: 500 })
    .glyph("acute", T1Glyph::Box { width: 300 })
    .glyph(
        "Aacute",
        T1Glyph::Seac {
            width: 500,
            base: 65,
            accent: 0xC2,
        },
    )
    .glyph("space", T1Glyph::Blank { width: 250 })
    .build();
    let (truetype, _) = latin_truetype("ABCabc", " Y");
    let cff = CffBuilder::new("Fuzz")
        .glyph("A", true)
        .glyph("B", false)
        .encoding(CffEncodingSpec::Format1(vec![(0x41, 1)]))
        .supplement(0x61, "A")
        .build();
    let cid_cff = CffBuilder::new("FuzzCID")
        .cid_glyph(65, true)
        .cid_glyph(66, false)
        .build();
    // Cases are independent and seeded by their index; four workers share them.
    let run_case = |case: usize| -> usize {
        let mut rng =
            XorShift(0x9E37_79B9_7F4A_7C15 ^ (case as u64 + 1).wrapping_mul(0x2545_F491_4F6C_DD1D));
        let tu = mutate(&mut rng, &tounicode);
        let _ = parse_tounicode(&tu).map(|t| t.codes().count());
        match case % 4 {
            0 => {
                let data = mutate(&mut rng, &truetype);
                if let Ok(face) = ttf_parser::Face::parse(&data, 0) {
                    let outlines = Outlines::of_face(&face);
                    let lookup = TrueTypeLookup::new(&face, &outlines);
                    if let super::program::GidLookup::Agree(g) = lookup.symbolic(0x41) {
                        let _ = outlines.drawn(g, &mut WorkMeter::new(PAGE_DECODE_BUDGET));
                    }
                    let _ = lookup.by_name("A");
                }
                load(0, data, tu, 0, 0)
            }
            1 => {
                let data = mutate(&mut rng, &type1.data);
                let l1 = (type1.length1 as i64 + rng.below(9) as i64 - 4).max(0);
                let l2 = if rng.below(10) == 0 {
                    data.len() as i64
                } else {
                    type1.length2 as i64
                };
                if let Ok(program) = parse_type1(&data, l1 as usize, l2 as usize) {
                    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
                    for name in ["A", "B", "Aacute", "space", ".notdef"] {
                        let _ = program.proof(name, &mut meter);
                    }
                }
                load(1, data, tu, l1, l2)
            }
            2 => {
                let data = mutate(&mut rng, &cff);
                let _ = cff_builtin_encoding(&data);
                if let Some(outlines) = Outlines::of_cff(&data) {
                    let mut meter = WorkMeter::new(PAGE_DECODE_BUDGET);
                    for gid in 0..16 {
                        let _ = outlines.drawn(gid, &mut meter);
                    }
                }
                load(2, data, tu, 0, 0)
            }
            _ => {
                let data = mutate(&mut rng, &cid_cff);
                let _ = cff_builtin_encoding(&data);
                load(3, data, tu, 0, 0)
            }
        }
    };
    let (loaded, cpu) = run_cases(1_000, "FONT-34", run_case);
    println!("FONT-34: 1,000 cases in {cpu:?} of CPU, {loaded} typeable characters in total");
    assert!(loaded > 0, "FONT-34 some mutants still load");
    assert!(
        cpu.as_secs_f64() < 2.0,
        "FONT-34 < 2 s of CPU per 1,000 cases: {cpu:?}"
    );
}

/// A random PDF value: wrong types, extreme numbers, references to fonts, to nothing or to
/// themselves, nested arrays and dictionaries.
fn random_object(rng: &mut XorShift, depth: usize, ids: &[ObjectId]) -> Object {
    const INTS: [i64; 10] = [
        0,
        -1,
        1,
        255,
        256,
        65_535,
        65_536,
        i64::MAX,
        i64::MIN,
        1 << 40,
    ];
    const REALS: [f32; 6] = [0.0, -0.5, 1e30, f32::NAN, f32::INFINITY, 0.001];
    const NAMES: [&str; 8] = [
        "Identity-H",
        "WinAnsiEncoding",
        "MacExpertEncoding",
        "A",
        "",
        "Type1C",
        "space",
        "Identity",
    ];
    match rng.below(if depth >= 2 { 8 } else { 10 }) {
        0 => Object::Null,
        1 => Object::Boolean(rng.below(2) == 0),
        2 => Object::Integer(INTS[rng.below(INTS.len())]),
        3 => Object::Real(REALS[rng.below(REALS.len())]),
        4 => Object::Name(NAMES[rng.below(NAMES.len())].as_bytes().to_vec()),
        5 => Object::String(b"/A/B/space".to_vec(), StringFormat::Literal),
        6 => Object::Reference(ids[rng.below(ids.len())]),
        7 => Object::Reference((999_999, 0)),
        8 => Object::Array(
            (0..rng.below(6))
                .map(|_| random_object(rng, depth + 1, ids))
                .collect(),
        ),
        _ => Object::Dictionary(dictionary! {
            "Differences" => random_object(rng, depth + 1, ids),
            "BaseEncoding" => random_object(rng, depth + 1, ids),
        }),
    }
}

/// Changes `dict`: one key set to a random value or dropped, or (for an array value) one item
/// replaced, the array cut, or an item added.
fn mutate_dict(rng: &mut XorShift, dict: &mut Dictionary, keys: &[&str], ids: &[ObjectId]) {
    let key = keys[rng.below(keys.len())].as_bytes().to_vec();
    let choice = rng.below(10);
    if choice == 0 {
        let _ = dict.remove(&key);
        return;
    }
    if let (1..=4, Ok(Object::Array(items))) = (choice, dict.get_mut(&key)) {
        let i = rng.below(items.len() + 1);
        match rng.below(3) {
            0 if i < items.len() => items[i] = random_object(rng, 1, ids),
            1 => items.truncate(i),
            _ => items.insert(i, random_object(rng, 1, ids)),
        }
        return;
    }
    dict.set(key, random_object(rng, 0, ids));
}

/// Touches every query a later task makes of a model.
fn exercise(model: &FontModel) -> usize {
    let codes = model.split_codes(b"\x00\x41\x20\x42").unwrap_or_default();
    for code in &codes {
        let _ = (model.text(*code), model.width(*code), model.drawable(*code));
    }
    let alphabet = model.alphabet();
    for (ch, _) in &alphabet {
        let _ = model.code_for(*ch, &[], &[]).map(|c| model.code_bytes(c));
    }
    alphabet.len()
}

#[test]
fn font_34b_font_dictionary_fuzz() {
    let (truetype, _) = latin_truetype("ABCabc", " ");
    let tounicode = cmap(
        "1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
         1 beginbfrange <0041> <0043> <0041> endbfrange",
    );
    let run_case = |case: usize| -> usize {
        let mut rng =
            XorShift(0xD1B5_4A32_D192_ED03 ^ (case as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut doc = Document::with_version("1.7");
        let program = doc.add_object(Stream::new(dictionary! {}, truetype.clone()));
        let tu = doc.add_object(Stream::new(dictionary! {}, tounicode.clone()));
        let gid_map = doc.add_object(Stream::new(dictionary! {}, vec![0, 0, 0, 1, 0, 2, 0, 3]));
        let (font_id, desc_id, cid_id) = (
            doc.add_object(Object::Null),
            doc.add_object(Object::Null),
            doc.add_object(Object::Null),
        );
        let ids = [font_id, desc_id, cid_id, program, tu, gid_map];
        let mut desc = dictionary! {
            "Type" => "FontDescriptor", "FontName" => "ABCDEF+Fuzz", "Flags" => 32,
            "FontFile2" => program, "Ascent" => 800, "Descent" => -200, "ItalicAngle" => 0,
            "StemV" => 80, "MissingWidth" => 250, "CharSet" => Object::string_literal("/A/B"),
        };
        let mut encoding = dictionary! {
            "BaseEncoding" => "WinAnsiEncoding",
            "Differences" => vec![
                Object::Integer(65), "A".into(), "B".into(), Object::Integer(32), "space".into(),
            ],
        };
        let type0 = case % 2 == 1;
        let mut cid = dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "ABCDEF+Fuzz",
            "FontDescriptor" => desc_id, "CIDToGIDMap" => gid_map, "DW" => 1000,
            "W" => vec![
                Object::Integer(1), Object::Array(vec![500.into(), 600.into()]),
                3.into(), 4.into(), 700.into(),
            ],
        };
        let widths: Vec<Object> = (32..=127).map(|_| Object::Integer(500)).collect();
        let mut font = if type0 {
            dictionary! {
                "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "ABCDEF+Fuzz",
                "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(cid_id)],
                "ToUnicode" => tu,
            }
        } else {
            dictionary! {
                "Type" => "Font", "Subtype" => "TrueType", "BaseFont" => "ABCDEF+Fuzz",
                "FirstChar" => 32, "LastChar" => 127, "Widths" => widths,
                "FontDescriptor" => desc_id, "ToUnicode" => tu,
            }
        };
        if !type0 && rng.below(2) == 0 {
            font.set("Encoding", Object::Dictionary(encoding.clone()));
        }
        for _ in 0..=rng.below(4) {
            match rng.below(4) {
                0 => mutate_dict(
                    &mut rng,
                    &mut font,
                    &[
                        "Subtype",
                        "FirstChar",
                        "LastChar",
                        "Widths",
                        "Encoding",
                        "FontDescriptor",
                        "ToUnicode",
                        "DescendantFonts",
                        "BaseFont",
                    ],
                    &ids,
                ),
                1 => mutate_dict(
                    &mut rng,
                    &mut desc,
                    &[
                        "Flags",
                        "Ascent",
                        "Descent",
                        "ItalicAngle",
                        "StemV",
                        "MissingWidth",
                        "CharSet",
                        "FontFile2",
                        "FontFile3",
                    ],
                    &ids,
                ),
                2 => mutate_dict(
                    &mut rng,
                    &mut cid,
                    &[
                        "W",
                        "DW",
                        "CIDToGIDMap",
                        "Subtype",
                        "WMode",
                        "FontDescriptor",
                    ],
                    &ids,
                ),
                _ => {
                    mutate_dict(
                        &mut rng,
                        &mut encoding,
                        &["BaseEncoding", "Differences"],
                        &ids,
                    );
                    if !type0 {
                        font.set("Encoding", Object::Dictionary(encoding.clone()));
                    }
                }
            }
        }
        doc.objects.insert(desc_id, Object::Dictionary(desc));
        doc.objects.insert(cid_id, Object::Dictionary(cid));
        doc.objects
            .insert(font_id, Object::Dictionary(font.clone()));
        let model = FontCache::new().get_or_load(
            &doc,
            FontKey::Indirect(font_id),
            &font,
            &mut DecodeBudget::new(PAGE_DECODE_BUDGET),
        );
        exercise(&model)
    };
    let (typeable, cpu) = run_cases(1_000, "FONT-34b", run_case);
    println!("FONT-34b: 1,000 cases in {cpu:?} of CPU, {typeable} typeable characters in total");
    assert!(typeable > 0, "FONT-34b some mutants still type");
    assert!(
        cpu.as_secs_f64() < 2.0,
        "FONT-34b < 2 s of CPU per 1,000 cases: {cpu:?}"
    );
}
