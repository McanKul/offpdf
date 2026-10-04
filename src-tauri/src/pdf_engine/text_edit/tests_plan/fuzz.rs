//! Plan fuzz (§B.21, T4): 10,000 deterministic xorshift mutations — byte flips, truncations,
//! inserted `(`, `<`, `[`, `BI`, `q`, huge numbers, stray text-object operators — of fixture content
//! streams, each modelled and then planned with random edits (text, size, face, colour, spacing,
//! stale ids, duplicates). No panic; every `Err` is a request or self-check error; every plan
//! re-verifies; < 2 s of thread CPU per 1,000 cases.

use super::ctx;
use crate::pdf_engine::text_edit::content::page_content;
use crate::pdf_engine::text_edit::decode::DecodeBudget;
use crate::pdf_engine::text_edit::limits::PAGE_DECODE_BUDGET;
use crate::pdf_engine::text_edit::reasons::Face;
use crate::pdf_engine::text_edit::rewrite::{plan_page, SourceTextStyleIn, TextEditIn};
use crate::pdf_engine::text_edit::runs::model_of;
use crate::pdf_engine::text_edit::testkit::producers::{
    cid_font, cid_hex, DocBuilder, PageSpec, HELVETICA,
};
use crate::pdf_engine::text_edit::verify::walk_and_verify;
use crate::pdf_engine::text_edit::walker::{walk_page, WalkMode};
use std::time::Duration;

/// CPU time of the calling thread (wall time where the platform has no thread clock).
pub(crate) fn thread_cpu() -> Duration {
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

const CID_CHARS: &str = "Fuzy ğış";

fn fixture() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = d.add(HELVETICA);
    let f2 = d.add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
    );
    let f3 = cid_font(&mut d.b, "ABCDEF+Arimo", CID_CHARS);
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (seed) Tj ET",
        &format!(
            "/Font << /F1 {f1} 0 R /F2 {f2} 0 R /F3 {f3} 0 R >> /ExtGState << /GS1 << /Font [{f1} 0 R 9] >> >> \
             /ColorSpace << /Cs1 /DeviceRGB >>"
        ),
    ));
    d.build()
}

fn seeds() -> Vec<Vec<u8>> {
    vec![
        b"BT /F1 12 Tf 72 700 Td [(Hel) -20 (lo) -333 (World)] TJ (tail) Tj ET BT /F2 12 Tf 72 680 Td (Bold) Tj ET"
            .to_vec(),
        format!(
            "q 0.5 g BT /F3 10 Tf 2 Tc 3 Tw 90 Tz 1 Ts 72 650 Td <{}> Tj ET Q 0 0 1 rg 72 600 20 20 re f",
            cid_hex(CID_CHARS, "Fuzy ğış Fuzy")
        )
        .into_bytes(),
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 14 TL 72 720 Td (One) ' 1 0.5 (Two) \" ET EMC \
          BT /F1 12 Tf 0.123 Tc /Cs1 cs 0.2 0.3 0.4 sc 72 560 Td (Coloured line) Tj ET"
            .to_vec(),
        b"/GS1 gs BT 72 500 Td (ExtGState font) Tj ET BT /F1 1 Tf 12 0 0 12 72 480 Tm (Tiny Tf) Tj ET \
          BT /F1 12 Tf 72 460 Td [(Name) -3000 (Value)] TJ ET"
            .to_vec(),
    ]
}

fn mutate(base: &[u8], rng: &mut XorShift) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..rng.below(4) {
        let at = rng.below(v.len() + 1);
        match rng.below(9) {
            0 => {
                if let Some(b) = v.get_mut(at) {
                    *b ^= 1 << rng.below(8);
                }
            }
            1 => v.truncate(at),
            2 => v.insert(at, b'('),
            3 => v.insert(at, b'<'),
            4 => v.insert(at, b'['),
            5 => v.splice(at..at, b" BI ".iter().copied()).for_each(drop),
            6 => v.splice(at..at, b" q ".iter().copied()).for_each(drop),
            7 => v
                .splice(at..at, b" 999999999 -1e9 0.00001 ".iter().copied())
                .for_each(drop),
            _ => v.splice(at..at, b" ET BT ".iter().copied()).for_each(drop),
        }
    }
    v
}

const SAFE: [&str; 9] = ["a", "e", "H", "l", "o", " ", "W", "\u{e9}", "7"];
const UNSAFE: [&str; 5] = ["ğ", "ı", "\n", "\u{1F600}", "  "];

fn text(rng: &mut XorShift, base: &str) -> String {
    match rng.below(5) {
        0 => base.to_string(),
        1 => String::new(),
        2 => base
            .chars()
            .take(rng.below(base.chars().count() + 1))
            .collect(),
        _ => {
            let mut s: String = base
                .chars()
                .take(rng.below(base.chars().count() + 1))
                .collect();
            for _ in 0..rng.below(6) {
                s.push_str(SAFE[rng.below(SAFE.len())]);
            }
            if rng.below(10) == 0 {
                s.push_str(UNSAFE[rng.below(UNSAFE.len())]);
            }
            s
        }
    }
}

/// A random style; one in eight is malformed (BAD_EDIT), the others are in range.
fn style(rng: &mut XorShift) -> SourceTextStyleIn {
    let faces = [Face::Regular, Face::Bold, Face::Italic, Face::BoldItalic];
    if rng.below(8) == 0 {
        return match rng.below(3) {
            0 => SourceTextStyleIn {
                size_pt: Some([3.0, 200.0, f64::NAN][rng.below(3)]),
                ..Default::default()
            },
            1 => SourceTextStyleIn {
                fill: Some("nope".into()),
                ..Default::default()
            },
            _ => SourceTextStyleIn {
                letter_spacing_pt: Some([-3.0, 11.0][rng.below(2)]),
                ..Default::default()
            },
        };
    }
    SourceTextStyleIn {
        size_pt: (rng.below(4) == 0).then(|| [4.0, 12.0, 13.5, 144.0][rng.below(4)]),
        face: (rng.below(4) == 0).then(|| faces[rng.below(4)]),
        fill: (rng.below(4) == 0)
            .then(|| ["#ff0000", "#000000", "#c71c1c"][rng.below(3)].to_string()),
        letter_spacing_pt: (rng.below(4) == 0).then(|| [-2.0, 0.0, 0.7, 10.0][rng.below(4)]),
    }
}

#[test]
fn plan_fuzz() {
    let c = ctx(fixture());
    let page = c.page_id(0).expect("page");
    let base =
        page_content(c.doc(), page, &mut DecodeBudget::new(PAGE_DECODE_BUDGET)).expect("content");
    let seeds = seeds();
    let mut rng = XorShift(0xD1B5_4A32_D192_ED03);
    let (mut planned, mut refused) = (0usize, 0usize);
    for chunk in 0..10 {
        let started = thread_cpu();
        for i in 0..1_000 {
            let seed = &seeds[(chunk * 1_000 + i) % seeds.len()];
            let content = base.with_replaced_parts(&[(0, mutate(seed, &mut rng))]);
            let walk = walk_page(&c, 0, &content, WalkMode::Edit, None);
            let m = model_of(&c, 0, content, walk);
            let mut edits = Vec::new();
            for _ in 0..1 + usize::from(rng.below(5) == 0) {
                let (id, old) = match m.runs.get(rng.below(m.runs.len() + 1)) {
                    Some(r) => (r.id.clone(), r.text.clone()),
                    None => ("t1:stale".to_string(), "stale".to_string()),
                };
                edits.push(TextEditIn {
                    run_id: id,
                    original_text: old.clone(),
                    text: text(&mut rng, &old),
                    style: style(&mut rng),
                });
            }
            match plan_page(&c, &m, &edits) {
                Ok(out) => {
                    assert_eq!(out.verdicts.len(), edits.len(), "one verdict per edit");
                    if let Some(p) = out.plan {
                        planned += 1;
                        let after = p.expected_content(&m.content);
                        assert!(
                            walk_and_verify(&c, 0, &after, &m.walk, &m.runs, &p, None).is_ok(),
                            "every returned plan re-verifies"
                        );
                    } else {
                        refused += 1;
                    }
                }
                Err(e) => {
                    assert!(
                        matches!(
                            e.code.as_str(),
                            "BAD_EDIT" | "TOO_MANY_TEXT_EDITS" | "EDIT_VERIFY_FAILED"
                        ),
                        "unexpected error {}",
                        e.code
                    );
                }
            }
        }
        let spent = thread_cpu().saturating_sub(started);
        assert!(
            spent < Duration::from_secs(2),
            "plan fuzz chunk {chunk}: {spent:?} for 1,000 cases"
        );
    }
    println!("plan fuzz: {planned} planned, {refused} refused or no-ops");
    assert!(
        planned > 1_000 && refused > 1_000,
        "both outcomes reached: {planned} planned, {refused} refused"
    );
}
