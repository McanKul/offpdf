//! Walker fuzz (§B.21, T3): 10,000 deterministic xorshift mutations — byte flips, truncations,
//! inserted `(`, `<`, `[`, `BI`, `q`, huge numbers — of fixture content streams through
//! lex → walk (Edit and Classify) → runs. No panic; < 2 s of CPU per 1,000 cases.

use super::{content, ctx};
use crate::pdf_engine::text_edit::runs::model_of;
use crate::pdf_engine::text_edit::testkit::producers::{cid_font, cid_hex};
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::walker::{walk_page, WalkMode};
use std::time::Duration;

/// CPU time of the calling thread (wall time where the platform has no thread clock).
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

/// A page whose resources cover every operator family the seeds use.
fn fixture() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = d.add(HELVETICA);
    let f2 = cid_font(&mut d.b, "ABCDEF+Arimo", "Fuzzy ");
    let img = d.b.add_stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[128],
    );
    let form = d.b.add_stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 {f1} 0 R >> >>"),
        b"BT /F1 9 Tf 5 5 Td (form) Tj ET",
    );
    let ocg = d.add("<< /Type /OCG /Name (L) >>");
    d.catalog_extra = format!("/OCProperties << /OCGs [{ocg} 0 R] /D << >> >>");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (seed) Tj ET",
        &format!(
            "/Font << /F1 {f1} 0 R /F2 {f2} 0 R >> /XObject << /Im0 {img} 0 R /Fm0 {form} 0 R >> \
             /ExtGState << /GS1 << /ca 0.5 /LW 2 >> >> /Properties << /L1 {ocg} 0 R >> \
             /ColorSpace << /Cs1 /DeviceRGB >>"
        ),
    ));
    d.build()
}

fn seeds() -> Vec<Vec<u8>> {
    vec![
        b"BT /F1 12 Tf 72 700 Td [(Hel) -20 (lo) -333 (World)] TJ (tail) Tj ET".to_vec(),
        format!(
            "q 1 0 0 1 10 10 cm /GS1 gs 0.5 g BT /F2 10 Tf 2 Tc 3 Tw 90 Tz 1 Ts 72 650 Td <{}> Tj ET Q",
            cid_hex("Fuzzy ", "Fuzzy Fuzz")
        )
        .into_bytes(),
        b"q 0 0 612 792 re W n /Cs1 cs 1 0 0 sc 10 10 m 20 20 l 30 10 25 5 15 15 c h f Q \
          q 40 0 0 40 72 400 cm /Im0 Do Q q 1 0 0 1 300 300 cm /Fm0 Do Q"
            .to_vec(),
        b"/P <</MCID 0>> BDC BT /F1 12 Tf 14 TL 72 720 Td (One) ' 1 0.5 (Two) \" ET EMC \
          /OC /L1 BDC BT 3 Tr /F1 8 Tf 1 0 0.2 1 72 500 Tm (Layer) Tj ET EMC"
            .to_vec(),
        b"q 24 0 0 12 72 400 cm BI /W 2 /H 1 /CS /RGB /BPC 8 ID abcdef EI Q \
          BT /F1 12 Tf 0 1 -1 0 300 300 Tm (After) Tj T* (Next) Tj ET [3 2] 0 d 2 J"
            .to_vec(),
    ]
}

fn mutate(base: &[u8], rng: &mut XorShift) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..1 + rng.below(4) {
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
                .splice(at..at, b" 999999999 -1e9 999999999.5 ".iter().copied())
                .for_each(drop),
            _ => v
                .splice(at..at, b" Q ET BT ".iter().copied())
                .for_each(drop),
        }
    }
    v
}

#[test]
fn walker_fuzz() {
    let c = ctx(fixture());
    let base = content(&c, 0);
    let seeds = seeds();
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut refused = 0usize;
    for chunk in 0..10 {
        let started = thread_cpu();
        for i in 0..1_000 {
            let seed = &seeds[(chunk * 1_000 + i) % seeds.len()];
            let bytes = mutate(seed, &mut rng);
            let page = base.with_replaced_parts(&[(0, bytes)]);
            let classify = walk_page(&c, 0, &page, WalkMode::Classify, None);
            let edit = walk_page(&c, 0, &page, WalkMode::Edit, None);
            refused += usize::from(edit.page_reason.is_some());
            let m = model_of(&c, 0, page, edit);
            assert!(m
                .runs
                .iter()
                .all(|r| r.caret_offsets.len() == r.text.chars().count() + 1));
            assert_eq!(
                classify.page_reason.is_some() && classify.records.is_empty()
                    || classify.page_reason.is_none(),
                true,
                "never a partial list"
            );
        }
        let spent = thread_cpu().saturating_sub(started);
        assert!(
            spent < Duration::from_secs(2),
            "walker fuzz chunk {chunk}: {spent:?} for 1,000 cases"
        );
    }
    assert!(
        refused > 0 && refused < 10_000,
        "the fuzz reaches both outcomes: {refused}"
    );
}
