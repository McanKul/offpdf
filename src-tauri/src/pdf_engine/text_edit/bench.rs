//! BENCH-01/02 (SPEC §E.7; `#[ignore]`, run locally): timings of open, inspect, preview and Save
//! on generated documents, with the peak resident set size (`getrusage`) of a child process per
//! configuration, printed as Markdown tables for `docs/EDIT_TEXT.md` §12.
//!
//! ```text
//! cargo test --lib -j 6 text_edit::bench -- --ignored --nocapture            # debug build
//! cargo test --release --lib -j 6 text_edit::bench -- --ignored --nocapture  # release build
//! ```
//!
//! BENCH-01: 10 / 100 / 1,000 pages of 40 Helvetica lines — open, inspect p50/p95 (cold: first
//! visit of a page; warm: the cached model), preview p50/p95, Save with 1 / 10 / 100 changed
//! lines. BENCH-02: a ~300 MB image-heavy file (Flate and DCT images, 20 text pages) — open to
//! first inspect, the background `qpdf --check`, one preview, Save with 1 change; budgets: first
//! inspect ≤ 3 s, Save ≤ 90 s, peak RSS ≤ 3 × file size.
#[cfg(test)]
// the module is test-only (declared under #[cfg(test)]); this line ends the API scan
use crate::models::PageGroup;
use crate::pdf_engine::edit_overlay::{export_edit_pdf_with_check_exe, EditDocumentIn};
use crate::pdf_engine::qpdf::resolve_qpdf_standalone;
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::service;
use crate::pdf_engine::text_edit::testkit::producers::{DocBuilder, PageSpec, HELVETICA};
use crate::pdf_engine::text_edit::testkit::{child_mode, run_child_test};
use crate::pdf_engine::text_edit::tests_e2e::preview::edit_in;
use crate::pdf_engine::text_edit::tests_e2e::{font_path, run_qpdf, E2e};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const LINES_PER_PAGE: usize = 40;
const INSPECT_SAMPLE: usize = 20;
const PREVIEW_SAMPLE: usize = 10;

/// Peak resident set size of this process in MiB (`getrusage(RUSAGE_SELF)`).
fn peak_rss_mib() -> f64 {
    #[repr(C)]
    struct Rusage {
        words: [i64; 18], // 2 × timeval (16 bytes each on macOS and Linux) + 14 longs
    }
    extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = Rusage { words: [0; 18] };
    // SAFETY: `usage` is a writable buffer at least as large as `struct rusage` on the 64-bit
    // macOS and Linux targets (144 bytes); RUSAGE_SELF = 0.
    let ok = unsafe { getrusage(0, &mut usage) } == 0;
    assert!(ok, "getrusage failed");
    let maxrss = usage.words[4] as f64; // ru_maxrss: bytes on macOS, KiB on Linux
    if cfg!(target_os = "macos") {
        maxrss / (1024.0 * 1024.0)
    } else {
        maxrss / 1024.0
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// (p50, p95) of `samples` in ms.
fn percentiles(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| {
        let i = ((samples.len() as f64 - 1.0) * q).round() as usize;
        samples.get(i).copied().unwrap_or(f64::NAN)
    };
    (at(0.5), at(0.95))
}

fn line_text(page: usize, line: usize) -> String {
    format!("Line {line} on page {page}: the quick brown fox")
}

/// `pages` pages of `LINES_PER_PAGE` Helvetica lines (one shared font).
fn text_doc(d: &mut DocBuilder, font: u32, pages: usize) {
    for p in 0..pages {
        let mut content = String::new();
        for l in 0..LINES_PER_PAGE {
            let y = 760 - 18 * l;
            let text = line_text(p, l);
            content.push_str(&format!("BT /F1 10 Tf 60 {y} Td ({text}) Tj ET\n"));
        }
        d.page(PageSpec::new(
            content.as_bytes(),
            &format!("/Font << /F1 {font} 0 R >>"),
        ));
    }
}

/// A `sourceText` export object changing line `line` of page `page` ("fox" → "dog").
fn edit_object(t: &E2e, src: &Path, page: u32, line: usize) -> Value {
    let old = line_text(page as usize, line);
    t.edit(src, page, page, &old, &old.replace("fox", "dog"))
}

/// One save through the real export; returns its duration.
fn timed_save(t: &E2e, src: &Path, objects: Vec<Value>, name: &str) -> Duration {
    let doc: EditDocumentIn =
        serde_json::from_value(json!({ "version": 1, "objects": objects })).expect("document");
    let groups = [PageGroup {
        path: src.to_string_lossy().into_owned(),
        pages: "1-z".into(),
    }];
    let work = t.scratch.path(&format!("{name}-work"));
    std::fs::create_dir_all(&work).expect("work");
    let dest = t.scratch.path(&format!("{name}-out.pdf"));
    let qpdf = resolve_qpdf_standalone();
    let started = Instant::now();
    let r = export_edit_pdf_with_check_exe(
        &groups,
        &dest.to_string_lossy(),
        &doc,
        &font_path(),
        &work,
        name,
        None,
        &qpdf,
        None,
        None,
        &[],
        &[],
        false,
        false,
        |args| run_qpdf(&qpdf, args),
    );
    let took = started.elapsed();
    r.unwrap_or_else(|e| panic!("{name}: {e} {:?}", e.details));
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_file(&dest);
    took
}

/// Preview of one changed line on `page`; returns its duration.
fn timed_preview(t: &E2e, src: &Path, fp: &str, page: u32) -> Duration {
    let dto = t.try_inspect(src, fp, page).expect("inspect");
    let old = line_text(page as usize, 0);
    let run = dto.runs.iter().find(|r| r.text == old).expect("line 0");
    let edits = [edit_in(
        &run.id,
        &old,
        &old.replace("fox", "dog"),
        SourceTextStyleIn::default(),
    )];
    let started = Instant::now();
    let p = service::preview_edits(
        &t.cache,
        &t.engines,
        &t.temp_root(),
        &src.to_string_lossy(),
        fp,
        page,
        &edits,
    )
    .expect("preview");
    let took = started.elapsed();
    assert!(
        p.page_pdf.is_some() && p.verdicts.iter().all(|v| v.ok),
        "{:?}",
        p.verdicts
    );
    took
}

fn bench_01_child(pages: usize) {
    let t = E2e::new("bench_01").expect("engines");
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    text_doc(&mut d, f, pages);
    let pdf = d.build();
    let src = t.file("bench.pdf", &pdf);
    let started = Instant::now();
    let info = t.open(&src);
    let open = started.elapsed();
    let k = INSPECT_SAMPLE.min(pages);
    let sample: Vec<u32> = (0..k).map(|i| (i * pages / k) as u32).collect();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    for pass in [&mut cold, &mut warm] {
        for page in &sample {
            let started = Instant::now();
            t.try_inspect(&src, &info.fingerprint, *page)
                .expect("inspect");
            pass.push(ms(started.elapsed()));
        }
    }
    let mut previews: Vec<f64> = sample
        .iter()
        .take(PREVIEW_SAMPLE)
        .map(|p| ms(timed_preview(&t, &src, &info.fingerprint, *p)))
        .collect();
    let mut saves = Vec::new();
    for n in [1usize, 10, 100] {
        let objects = (0..n)
            .map(|i| edit_object(&t, &src, (i % pages) as u32, i / pages))
            .collect();
        saves.push(ms(timed_save(&t, &src, objects, &format!("save-{n}"))));
    }
    let (c50, c95) = percentiles(&mut cold);
    let (w50, w95) = percentiles(&mut warm);
    let (p50, p95) = percentiles(&mut previews);
    println!(
        "BENCH01|{pages}|{:.1}|{:.0}|{c50:.0}|{c95:.0}|{w50:.0}|{w95:.0}|{p50:.0}|{p95:.0}|{:.0}|{:.0}|{:.0}|{:.0}",
        pdf.len() as f64 / 1024.0,
        ms(open),
        saves[0],
        saves[1],
        saves[2],
        peak_rss_mib()
    );
}

/// BENCH-01 (§E.7): one child process per document size; prints the Markdown table.
#[test]
#[ignore]
fn bench_01_open_inspect_preview_save_by_page_count() {
    if let Some(mode) = child_mode() {
        if let Some(n) = mode.strip_prefix("bench01:").and_then(|n| n.parse().ok()) {
            bench_01_child(n);
        }
        return;
    }
    let mut rows = Vec::new();
    for n in [10, 100, 1000] {
        let out = run_child_test(
            "pdf_engine::text_edit::bench::bench_01_open_inspect_preview_save_by_page_count",
            &format!("bench01:{n}"),
        );
        let row = out
            .lines()
            .find_map(|l| l.split("BENCH01|").nth(1))
            .unwrap_or_else(|| panic!("no BENCH01 row:\n{out}"))
            .replace('|', " | ");
        rows.push(format!("| {row} |"));
    }
    println!(
        "\nBENCH-01 ({} build)\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!("| pages | file KiB | open ms | inspect cold p50 | cold p95 | warm p50 | warm p95 | preview p50 | preview p95 | save 1 edit ms | save 10 | save 100 | peak RSS MiB |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for r in rows {
        println!("{r}");
    }
}

/// Deterministic incompressible bytes.
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    let mut v = Vec::with_capacity(n + 8);
    while v.len() < n {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        v.extend_from_slice(&state.to_le_bytes());
    }
    v.truncate(n);
    v
}

/// zlib with stored (uncompressed) deflate blocks: valid Flate data as large as its input.
fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    z.write_all(raw).expect("zlib");
    z.finish().expect("zlib")
}

/// A baseline JPEG of `side` × `side` noise pixels (valid DCT data: `qpdf --check` decodes it).
fn noise_jpeg(side: u32, seed: u64) -> Vec<u8> {
    let pixels = noise((side * side * 3) as usize, seed);
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
        .encode(&pixels, side, side, image::ExtendedColorType::Rgb8)
        .expect("jpeg");
    out
}

/// About `total_mib` MiB: 20 text pages, then image pages each holding a Flate image
/// (incompressible data in stored deflate blocks, 5 MiB) and a DCT image (a noise JPEG).
fn heavy_doc(total_mib: usize) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    text_doc(&mut d, f, 20);
    let side = 1_024usize;
    let rows = (5usize << 20) / (side * 3);
    let jpeg_side = 1_400u32;
    let jpeg = noise_jpeg(jpeg_side, 0xD0C0);
    let mut total = 0usize;
    let mut i = 0u64;
    while total < total_mib << 20 {
        let raw = noise(side * rows * 3, 0x5EED_0000 + i);
        let flate = zlib_stored(&raw);
        total += flate.len() + jpeg.len();
        let fl = d.b.add_stream(
            &format!("/Type /XObject /Subtype /Image /Width {side} /Height {rows} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"),
            &flate,
        );
        let dct = d.b.add_stream(
            &format!("/Type /XObject /Subtype /Image /Width {jpeg_side} /Height {jpeg_side} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode"),
            &jpeg,
        );
        d.page(PageSpec::new(
            b"q 300 0 0 300 50 400 cm /Im0 Do Q q 300 0 0 300 50 50 cm /Im1 Do Q",
            &format!("/XObject << /Im0 {fl} 0 R /Im1 {dct} 0 R >>"),
        ));
        i += 1;
    }
    d.build()
}

fn bench_02_child() {
    let t = E2e::new("bench_02").expect("engines");
    let pdf = heavy_doc(300);
    let size_mib = pdf.len() as f64 / (1024.0 * 1024.0);
    let src = t.file("heavy.pdf", &pdf);
    drop(pdf);
    let started = Instant::now();
    let info = t.open(&src);
    let opened = started.elapsed();
    t.try_inspect(&src, &info.fingerprint, 0)
        .expect("first inspect");
    let first_inspect = started.elapsed();
    {
        // Held only for the wait: a file over CACHE_SNAPSHOT_BYTES_MAX is not kept by the cache,
        // and nothing in the app holds its snapshot during a preview or a Save.
        let src_ref = t
            .cache
            .open(&src, &t.temp_root(), &t.engines)
            .expect("source");
        t.cache
            .source_check(&src_ref, &t.engines)
            .expect("check")
            .wait(None)
            .expect("check result");
    }
    let check = started.elapsed();
    let preview = timed_preview(&t, &src, &info.fingerprint, 3);
    let save = timed_save(&t, &src, vec![edit_object(&t, &src, 5, 7)], "heavy-save");
    let rss = peak_rss_mib();
    println!(
        "BENCH02|{size_mib:.0}|{:.0}|{:.0}|{:.0}|{:.0}|{:.0}|{rss:.0}|{:.2}",
        ms(opened),
        ms(first_inspect - opened),
        ms(check),
        ms(preview),
        ms(save),
        rss / size_mib
    );
}

/// BENCH-02 (§E.7): the ~300 MB image-heavy file, in a child process; prints the table.
#[test]
#[ignore]
fn bench_02_image_heavy_300_mb() {
    if let Some(mode) = child_mode() {
        if mode == "bench02" {
            bench_02_child();
        }
        return;
    }
    let out = run_child_test(
        "pdf_engine::text_edit::bench::bench_02_image_heavy_300_mb",
        "bench02",
    );
    let row = out
        .lines()
        .find_map(|l| l.split("BENCH02|").nth(1))
        .unwrap_or_else(|| panic!("no BENCH02 row:\n{out}"))
        .replace('|', " | ");
    println!("\nBENCH-02 ({} build; budgets: first inspect ≤ 3,000 ms, save ≤ 90,000 ms, peak RSS ≤ 3 × file)\n", if cfg!(debug_assertions) { "debug" } else { "release" });
    println!("| file MiB | open ms | first inspect ms (after open) | qpdf --check done ms (from the start of open) | preview ms | save 1 edit ms | peak RSS MiB | RSS / file |");
    println!("|---|---|---|---|---|---|---|---|");
    println!("| {row} |");
}
