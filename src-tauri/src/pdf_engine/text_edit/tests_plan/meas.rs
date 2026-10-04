//! MEAS-01: the editor's width estimate (`fit.rs`) against the golden file the frontend's
//! `estimateDeltaPt`/`estimateCaretOffsets` are tested with (T7). `OFFPDF_UPDATE_GOLDEN=1`
//! rewrites `src/lib/editor/__fixtures__/text-measure-golden.json`; without it the file must equal
//! what the code computes now.

use super::{ctx, edit, model, plan, run_with, sized, style};
use crate::pdf_engine::text_edit::fit::{
    estimate_caret_offsets, estimate_delta_pt, golden, write_measure_golden,
};
use crate::pdf_engine::text_edit::testkit::producers::{self as fx, helvetica_page};
use std::path::PathBuf;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../src/lib/editor/__fixtures__/text-measure-golden.json")
}

#[test]
fn meas_01_estimate_golden() {
    let path = golden_path();
    if std::env::var("OFFPDF_UPDATE_GOLDEN").as_deref() == Ok("1") {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("fixtures dir");
        }
        write_measure_golden(&path).expect("write the golden file");
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "MEAS-01 {}: {e} (run with OFFPDF_UPDATE_GOLDEN=1)",
            path.display()
        )
    });
    let stored: serde_json::Value = serde_json::from_str(&text).expect("MEAS-01 JSON");
    // serde_json reads floats to within an ulp (no `float_roundtrip` feature): compare numbers
    // within 1e-9, everything else exactly.
    if let Err(at) = same_json(&stored, &golden::build(), "$") {
        panic!("MEAS-01 the golden file is stale at {at}; regenerate it (OFFPDF_UPDATE_GOLDEN=1)");
    }
    let cases = stored["cases"].as_array().expect("cases");
    let names: Vec<&str> = cases.iter().filter_map(|c| c["name"].as_str()).collect();
    for want in [
        "std14-helvetica-12",
        "tz-80",
        "tc-0.5",
        "kern-space",
        "sibling-surface",
        "size-change",
        "letter-spacing",
        "face-bold",
    ] {
        assert!(names.contains(&want), "MEAS-01 case {want}");
    }
    for c in cases {
        let offsets = c["caretOffsets"].as_array().expect("offsets");
        let chars = c["text"].as_str().expect("text").chars().count();
        assert_eq!(offsets.len(), chars + 1, "MEAS-01 {} chars + 1", c["name"]);
    }
}

#[test]
fn meas_01_estimate_matches_the_plan_without_kerns() {
    // Helvetica 12, no kerns, no Tc: the estimate equals the planned width change (with Tc the
    // estimate also counts the spacing after the last glyph, which the ink extent does not).
    let c = ctx(helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"));
    let m = model(&c, 0);
    let run = run_with(&m, "Hello");
    for (text, s) in [
        ("Help me", style()),
        ("Hello", sized(14.0)),
        ("Hel", style()),
    ] {
        let est = estimate_delta_pt(run, &m.walk.page_fonts, text, &s);
        let out = plan(&c, &m, &[edit(&m, "Hello", text, s)]);
        let planned = out.verdicts[0].delta_pt;
        assert!(
            (est - planned).abs() < 1e-9,
            "MEAS-01 {text:?}: estimate {est} vs plan {planned}"
        );
    }
    let offsets = estimate_caret_offsets(run, &m.walk.page_fonts, "Hi", &style());
    // H = 722, i = 222 (×12/1000).
    assert_eq!(offsets.len(), 3);
    assert!(
        (offsets[1] - 8.664).abs() < 1e-9 && (offsets[2] - 11.328).abs() < 1e-9,
        "{offsets:?}"
    );
    // Kern mode: a typed space is the run's median gap (−333 ⇒ 0.333 em).
    let c = ctx(fx::pdftex());
    let m = model(&c, 0);
    let run = run_with(&m, "Hello World");
    let d = estimate_delta_pt(run, &m.walk.page_fonts, "Hello  World", &style());
    assert!((d - 0.333 * 9.9626).abs() < 1e-6, "MEAS-01 kern space {d}");
}

fn same_json(a: &serde_json::Value, b: &serde_json::Value, at: &str) -> Result<(), String> {
    use serde_json::Value as V;
    match (a, b) {
        (V::Number(x), V::Number(y)) => {
            let (x, y) = (
                x.as_f64().unwrap_or(f64::NAN),
                y.as_f64().unwrap_or(f64::NAN),
            );
            if (x - y).abs() <= 1e-9 * 1f64.max(x.abs()) {
                Ok(())
            } else {
                Err(format!("{at}: {x} vs {y}"))
            }
        }
        (V::Array(x), V::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .try_for_each(|(i, (p, q))| same_json(p, q, &format!("{at}[{i}]"))),
        (V::Object(x), V::Object(y)) if x.len() == y.len() => {
            x.iter().try_for_each(|(k, p)| match y.get(k) {
                Some(q) => same_json(p, q, &format!("{at}.{k}")),
                None => Err(format!("{at}.{k} missing")),
            })
        }
        _ if a == b => Ok(()),
        _ => Err(format!("{at}: {a} vs {b}")),
    }
}
