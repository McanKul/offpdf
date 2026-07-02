//! Blank-page detection. Each page is rendered to a tiny grayscale PNG with
//! poppler's `pdftoppm` (-gray -r 40 — a Letter page becomes ~340×440 px, so a
//! whole scan batch stays cheap), then analysed with the `image` crate:
//!
//! * a page is blank when the fraction of "dark" pixels (luma < 240) is below
//!   the sensitivity threshold — real content (text, lines, stamps) always
//!   produces a visible dark fraction, while paper texture does not;
//! * a page of near-uniform light gray (a scanner reading an empty sheet often
//!   yields flat noise around ~200-230 instead of white) is also treated as
//!   blank via the standard deviation of the luma histogram.
//!
//! Detection only *reports* pages — removal is done by the existing
//! delete/assemble commands, so there is no new write path here.

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate};
use crate::pdf_engine::{qpdf, render};
use crate::utils::temp;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Render resolution for detection. 40 DPI is enough: a single word on a page
/// still covers dozens of pixels, while pages render in a few milliseconds.
const DETECT_DPI: u32 = 40;

/// Pixels with a luma below this count as "content" (ink).
const DARK_LUMA: u8 = 240;

/// A near-uniform page (flat scanner gray) has a luma stddev below this.
const UNIFORM_STDDEV: f64 = 4.0;

/// The uniform-page rule only applies to *light* pages — a solid dark page
/// (e.g. a full-bleed photo or a black cover) is uniform but not blank.
const UNIFORM_MIN_MEAN: f64 = 160.0;

/// Map a sensitivity preset to the maximum dark-pixel fraction of a blank page.
/// Unknown values fall back to "normal".
pub(crate) fn threshold_for(sensitivity: &str) -> f64 {
    match sensitivity {
        "strict" => 0.0005,    // 0.05 % — only truly empty pages
        "aggressive" => 0.01,  // 1 %    — also catches specks / punch holes
        _ => 0.003,            // 0.3 %  — "normal"
    }
}

/// Dark-pixel fraction, mean and standard deviation of grayscale pixels.
pub(crate) fn luma_stats(pixels: &[u8]) -> (f64, f64, f64) {
    if pixels.is_empty() {
        return (0.0, 255.0, 0.0);
    }
    let n = pixels.len() as f64;
    let mut dark: u64 = 0;
    let mut sum: f64 = 0.0;
    for &p in pixels {
        if p < DARK_LUMA {
            dark += 1;
        }
        sum += p as f64;
    }
    let mean = sum / n;
    let var = pixels
        .iter()
        .map(|&p| {
            let d = p as f64 - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    (dark as f64 / n, mean, var.sqrt())
}

/// Blank-page decision from the page's luma statistics.
pub(crate) fn is_blank(dark_fraction: f64, mean: f64, stddev: f64, threshold: f64) -> bool {
    dark_fraction < threshold || (stddev < UNIFORM_STDDEV && mean > UNIFORM_MIN_MEAN)
}

/// Detect blank pages of `input` (1-based page numbers). `sensitivity` is
/// "strict" | "normal" | "aggressive". Cancellable between pages via the
/// `JobHandle`, like `ocr::ocr`.
pub fn detect_blank_pages(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    sensitivity: &str,
) -> Result<Vec<u32>, AppError> {
    super::require_input(input)?;
    let threshold = threshold_for(sensitivity);
    let n = qpdf::npages(app, input)?;
    if n == 0 {
        return Ok(vec![]);
    }

    let exe = render::resolve_pdftoppm(app);
    let work = temp::root(app)?
        .join("work")
        .join(job_id)
        .join(format!("blank-{}", render::fnv1a_hex(input)));
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;

    let result = (|| -> Result<Vec<u32>, AppError> {
        let mut blanks: Vec<u32> = Vec::new();
        for page in 1..=n {
            if handle.is_cancelled() {
                return Err(AppError::cancelled());
            }
            let _ = app.emit(
                "job:update",
                JobUpdate::new(
                    job_id,
                    "running",
                    &format!("Scanning page {page} of {n}"),
                )
                .percent((page - 1) as f32 / n as f32 * 100.0),
            );

            // Tiny grayscale render of just this page.
            let prefix = work.join(format!("p{page}"));
            let mut cmd = Command::new(&exe);
            render::configure_poppler_command(&mut cmd, &exe);
            cmd.args([
                "-png",
                "-gray",
                "-r",
                &DETECT_DPI.to_string(),
                "-f",
                &page.to_string(),
                "-l",
                &page.to_string(),
                "-singlefile",
                input,
                &prefix.to_string_lossy(),
            ]);
            cmd.stdout(Stdio::null()).stderr(Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(0x08000000);

            let (_status, stderr) = crate::utils::process::run_tracked(handle, cmd)?;
            let png = prefix.with_extension("png");
            if !png.exists() {
                return Err(AppError::engine_failed(stderr.trim().to_string()));
            }

            let img = image::open(&png)
                .map_err(|e| AppError::engine_failed(format!("read page {page}: {e}")))?
                .to_luma8();
            let (dark_fraction, mean, stddev) = luma_stats(img.as_raw());
            if is_blank(dark_fraction, mean, stddev, threshold) {
                blanks.push(page);
            }
            let _ = std::fs::remove_file(&png);
        }
        Ok(blanks)
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::{is_blank, luma_stats, threshold_for};

    #[test]
    fn thresholds_match_presets() {
        assert_eq!(threshold_for("strict"), 0.0005);
        assert_eq!(threshold_for("normal"), 0.003);
        assert_eq!(threshold_for("aggressive"), 0.01);
        assert_eq!(threshold_for("anything else"), 0.003);
    }

    #[test]
    fn pure_white_page_is_blank() {
        let pixels = vec![255u8; 10_000];
        let (dark, mean, stddev) = luma_stats(&pixels);
        assert_eq!(dark, 0.0);
        assert!(is_blank(dark, mean, stddev, threshold_for("strict")));
    }

    #[test]
    fn page_with_text_is_not_blank() {
        // 2% dark pixels — above every preset threshold, plenty of variance.
        let mut pixels = vec![255u8; 10_000];
        for p in pixels.iter_mut().take(200) {
            *p = 0;
        }
        let (dark, mean, stddev) = luma_stats(&pixels);
        assert!((dark - 0.02).abs() < 1e-9);
        assert!(!is_blank(dark, mean, stddev, threshold_for("aggressive")));
    }

    #[test]
    fn uniform_scanner_gray_is_blank() {
        // Flat light gray: every pixel counts as "dark" (215 < 240) so the
        // fraction rule alone would keep it — the stddev rule must catch it.
        let pixels = vec![215u8; 10_000];
        let (dark, mean, stddev) = luma_stats(&pixels);
        assert_eq!(dark, 1.0);
        assert!(is_blank(dark, mean, stddev, threshold_for("normal")));
    }

    #[test]
    fn uniform_dark_page_is_not_blank() {
        // A solid black page is uniform but must never be called blank.
        let pixels = vec![10u8; 10_000];
        let (dark, mean, stddev) = luma_stats(&pixels);
        assert!(!is_blank(dark, mean, stddev, threshold_for("aggressive")));
    }

    #[test]
    fn sensitivity_ordering_holds() {
        // A page with 0.5% dark pixels: blank for aggressive, kept for strict.
        let mut pixels = vec![255u8; 10_000];
        for p in pixels.iter_mut().take(50) {
            *p = 0;
        }
        let (dark, mean, stddev) = luma_stats(&pixels);
        assert!(is_blank(dark, mean, stddev, threshold_for("aggressive")));
        assert!(!is_blank(dark, mean, stddev, threshold_for("strict")));
    }
}
