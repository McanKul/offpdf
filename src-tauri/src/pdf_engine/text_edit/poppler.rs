//! Independent-engine checks (SPEC §B.15, Phase A check A5): Poppler, not our model, reads and
//! renders the page before and after the edit. Words outside the edited bands must stay 1:1 (text
//! and box within `WORD_BBOX_TOL_PT`); inside them the new text must be there and the old text
//! gone (G-TEXT). Pixels outside the edited glyphs' masks must not change (G-RENDER); an edit that
//! changes no pixel inside its own mask is reported `EDIT_NOT_VISIBLE` (non-blocking). This breaks
//! the mobile B13 loop, where the gate verified the edit with the same width model that made it.
//!
//! Frames (pinned by IND-06, poppler 26.04): `pdftotext -bbox` and `pdftoppm` both use the
//! MediaBox with the display rotation applied, origin at the top left, y down — an offset CropBox
//! does not move them.

use crate::error::AppError;
use crate::pdf_engine::text_edit::apply::verify_failed;
use crate::pdf_engine::text_edit::engines::{run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::geometry::PageGeometry;
use crate::pdf_engine::text_edit::limits::{
    EDIT_BAND_PAD_PT, PDFTOTEXT_OUTPUT_MAX, RENDER_CHANNEL_TOL, RENDER_DPI_MAX,
    RENDER_KEPT_PIXELS_MAX, RENDER_MASK_PAD_PX, RENDER_OUTSIDE_PIXELS_MAX, RENDER_OWN_PAD_PX,
    RENDER_PIXELS_MAX,
};
use crate::pdf_engine::text_edit::rewrite::ExpectedRun;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use words::check_words;

mod ink;
pub(crate) mod near;
mod words;

pub use near::NearGlyphs;

/// Points per inch.
const PT_PER_INCH: f64 = 72.0;
/// Bytes of a PPM header we accept.
const PPM_HEADER_MAX: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Decodes the XML entities pdftotext writes.
fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or_default());
        let tail = rest.get(p..).unwrap_or_default();
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = tail.get(1..).unwrap_or_default();
            continue;
        };
        let ent = tail.get(1..end).unwrap_or_default();
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => out.push(c),
            None => out.push_str(tail.get(..=end).unwrap_or_default()),
        }
        rest = tail.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let key = format!("{name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = tag.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end)?
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite())
}

/// Parses the `<word xMin yMin xMax yMax>text</word>` elements of `pdftotext -bbox` output.
pub(crate) fn parse_words(xml: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(p) = rest.find("<word ") {
        let tail = rest.get(p..).unwrap_or_default();
        let Some(close) = tail.find('>') else { break };
        let tag = tail.get(..close).unwrap_or_default();
        let body = tail.get(close + 1..).unwrap_or_default();
        let Some(end) = body.find("</word>") else {
            break;
        };
        if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (
            attr(tag, "xMin"),
            attr(tag, "yMin"),
            attr(tag, "xMax"),
            attr(tag, "yMax"),
        ) {
            out.push(Word {
                text: entities(body.get(..end).unwrap_or_default()),
                x0,
                y0,
                x1,
                y1,
            });
        }
        rest = body.get(end + "</word>".len()..).unwrap_or_default();
    }
    out
}

/// `pdftotext -f n -l n -bbox <pdf> -` (stdout ≤ `PDFTOTEXT_OUTPUT_MAX`).
pub fn pdftotext_words(
    engines: &Engines,
    pdf: &Path,
    page_1: u32,
    opts: &RunOpts<'_>,
) -> Result<Vec<Word>, AppError> {
    let n = page_1.to_string();
    let args: Vec<OsString> = vec![
        "-f".into(),
        n.clone().into(),
        "-l".into(),
        n.into(),
        "-bbox".into(),
        pdf.as_os_str().to_os_string(),
        "-".into(),
    ];
    let capped = RunOpts {
        handle: opts.handle,
        cancel: opts.cancel,
        timeout: opts.timeout,
        stdout_cap: PDFTOTEXT_OUTPUT_MAX,
    };
    let out = run_tool(&engines.pdftotext, &args, true, &capped)?;
    if out.code != 0 {
        return Err(verify_failed(&format!(
            "check=words pdftotext exited with code {}: {}",
            out.code,
            out.stderr.trim()
        )));
    }
    Ok(parse_words(&String::from_utf8_lossy(&out.stdout)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

/// Reads one whitespace-delimited header token of a PPM.
fn ppm_token(data: &[u8], at: &mut usize) -> Option<u64> {
    while data.get(*at).is_some_and(u8::is_ascii_whitespace) {
        *at += 1;
    }
    let start = *at;
    while data.get(*at).is_some_and(u8::is_ascii_digit) {
        *at += 1;
    }
    std::str::from_utf8(data.get(start..*at)?)
        .ok()?
        .parse()
        .ok()
}

/// Parses a binary (P6, maxval 255) PPM with a checked header.
pub(crate) fn parse_ppm(data: &[u8]) -> Option<Raster> {
    if data.get(..2) != Some(b"P6") {
        return None;
    }
    let mut at = 2usize;
    let w = ppm_token(data, &mut at)?;
    let h = ppm_token(data, &mut at)?;
    let max = ppm_token(data, &mut at)?;
    if max != 255 || at > PPM_HEADER_MAX || !data.get(at).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let pixels = w.checked_mul(h)?;
    if w == 0 || h == 0 || pixels > RENDER_PIXELS_MAX {
        return None;
    }
    let len = usize::try_from(pixels.checked_mul(3)?).ok()?;
    let body = data.get(at + 1..)?;
    if body.len() != len {
        return None;
    }
    Some(Raster {
        w: u32::try_from(w).ok()?,
        h: u32::try_from(h).ok()?,
        rgb: body.to_vec(),
    })
}

/// `pdftoppm -r dpi -f n -l n -singlefile <pdf> <work>/<prefix>`, then the PPM read back
/// (bounded by `RENDER_PIXELS_MAX`).
pub fn render_page(
    engines: &Engines,
    pdf: &Path,
    page_1: u32,
    dpi: u32,
    work: &Path,
    opts: &RunOpts<'_>,
) -> Result<Raster, AppError> {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let prefix = work.join(format!("render-{}-{n}", std::process::id()));
    let page = page_1.to_string();
    let args: Vec<OsString> = vec![
        "-r".into(),
        dpi.to_string().into(),
        "-f".into(),
        page.clone().into(),
        "-l".into(),
        page.into(),
        "-singlefile".into(),
        pdf.as_os_str().to_os_string(),
        prefix.as_os_str().to_os_string(),
    ];
    let out = run_tool(&engines.pdftoppm, &args, true, opts)?;
    let file = prefix.with_extension("ppm");
    let result = if out.code != 0 {
        Err(verify_failed(&format!(
            "check=render pdftoppm exited with code {}: {}",
            out.code,
            out.stderr.trim()
        )))
    } else {
        let cap = RENDER_PIXELS_MAX
            .saturating_mul(3)
            .saturating_add(PPM_HEADER_MAX as u64 + 1);
        let mut data = Vec::new();
        std::fs::File::open(&file)
            .and_then(|f| f.take(cap.saturating_add(1)).read_to_end(&mut data))
            .map_err(|e| verify_failed(&format!("check=render could not read the render: {e}")))
            .and_then(|_| {
                parse_ppm(&data)
                    .ok_or_else(|| verify_failed("check=render the render is not a usable PPM"))
            })
    };
    let _ = std::fs::remove_file(&file);
    result
}

/// Lowest render resolution A5 verifies at (SPEC §B.15: "≥ 25 for any legal page").
pub(crate) const RENDER_DPI_MIN: u32 = 25;

/// Pixels of a pdftoppm render of the media box at `dpi` (each side rounded up).
fn render_pixels(w_in: f64, h_in: f64, dpi: u32) -> f64 {
    (w_in * f64::from(dpi)).ceil() * (h_in * f64::from(dpi)).ceil()
}

/// `min(RENDER_DPI_MAX, floor(sqrt(RENDER_PIXELS_MAX / (w_in · h_in))))` over the media box,
/// lowered while the rounded-up render would still pass `RENDER_PIXELS_MAX`. `None` when that is
/// below `RENDER_DPI_MIN` (a page larger than the largest legal one, 14,400 pt square): A5's pads
/// and threshold are in pixels, so below it a same-line follower's shift would go unseen.
pub fn render_dpi(geom: &PageGeometry) -> Option<u32> {
    let w_in = (geom.media[2] - geom.media[0]).abs() / PT_PER_INCH;
    let h_in = (geom.media[3] - geom.media[1]).abs() / PT_PER_INCH;
    let area = w_in * h_in;
    if !(area.is_finite() && area > 0.0) {
        return None;
    }
    let budget = RENDER_PIXELS_MAX as f64;
    let ideal = (budget / area).sqrt().floor();
    let mut dpi = if ideal.is_finite() && ideal >= f64::from(RENDER_DPI_MIN) {
        (ideal.min(f64::from(RENDER_DPI_MAX))) as u32
    } else {
        return None;
    };
    while dpi >= RENDER_DPI_MIN && render_pixels(w_in, h_in, dpi) > budget {
        dpi -= 1;
    }
    (dpi >= RENDER_DPI_MIN).then_some(dpi)
}

/// A user-space point in the Poppler frame (MediaBox, display rotation, top-left origin, y down).
fn to_frame(x: f64, y: f64, geom: &PageGeometry) -> (f64, f64) {
    let [mx0, my0, mx1, my1] = geom.media;
    match geom.rotate.rem_euclid(360) {
        90 => (y - my0, x - mx0),
        180 => (mx1 - x, y - my0),
        270 => (my1 - y, mx1 - x),
        _ => (x - mx0, my1 - y),
    }
}

/// `[x0, y0, x1, y1]` user rect → the same rect in the pdftotext frame.
pub fn user_rect_to_text_frame(rect: [f64; 4], geom: &PageGeometry) -> [f64; 4] {
    let a = to_frame(rect[0], rect[1], geom);
    let b = to_frame(rect[2], rect[3], geom);
    [a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)]
}

/// `[x0, y0, x1, y1]` user rect → pixels of a pdftoppm render at `dpi` (outward rounding).
pub fn user_rect_to_pixels(rect: [f64; 4], geom: &PageGeometry, dpi: u32) -> [i64; 4] {
    let f = user_rect_to_text_frame(rect, geom);
    let s = f64::from(dpi) / PT_PER_INCH;
    let clamp = |v: f64| {
        if v.is_finite() {
            v.clamp(-1e9, 1e9) as i64
        } else {
            0
        }
    };
    [
        clamp((f[0] * s).floor()),
        clamp((f[1] * s).floor()),
        clamp((f[2] * s).ceil()),
        clamp((f[3] * s).ceil()),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndependentReport {
    /// Per edit: some pixel inside its mask changed (false ⇒ `EDIT_NOT_VISIBLE`).
    pub edit_visible: Vec<bool>,
}

pub(super) fn dilate(r: [f64; 4], pad: f64) -> [f64; 4] {
    [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]
}

pub(super) fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn differs(a: &[u8], b: &[u8]) -> bool {
    a.iter()
        .zip(b)
        .any(|(x, y)| x.abs_diff(*y) > RENDER_CHANNEL_TOL)
}

/// A mask box in raster pixels, half-open (`x0..x1`, `y0..y1`), clamped to the raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelBox {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

/// A user rect in the pixels of a `w × h` render at `dpi` (outward rounding), grown by `pad`
/// pixels and clamped to the raster; `None` when nothing of it is left.
fn pixel_box(
    rect: [f64; 4],
    geom: &PageGeometry,
    dpi: u32,
    pad: i64,
    (w, h): (usize, usize),
) -> Option<PixelBox> {
    let clamp = |v: i64, max: usize| usize::try_from(v.max(0)).map_or(max, |u| u.min(max));
    let p = user_rect_to_pixels(rect, geom, dpi);
    let m = PixelBox {
        x0: clamp(p[0].saturating_sub(pad), w),
        y0: clamp(p[1].saturating_sub(pad), h),
        x1: clamp(p[2].saturating_add(pad), w),
        y1: clamp(p[3].saturating_add(pad), h),
    };
    (m.x0 < m.x1 && m.y0 < m.y1).then_some(m)
}

/// Each edit's mask boxes (§B.15: glyph boxes dilated by `EDIT_BAND_PAD_PT`, then
/// `RENDER_MASK_PAD_PX`) in the pixels of a `w × h` render; boxes outside the raster are dropped.
fn pixel_masks(
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    geom: &PageGeometry,
    dpi: u32,
    w: usize,
    h: usize,
) -> Vec<Vec<PixelBox>> {
    edits
        .iter()
        .map(|(exp, _, _, _)| {
            let grown = exp.mask_boxes.iter().map(|b| dilate(*b, EDIT_BAND_PAD_PT));
            grown
                .filter_map(|b| pixel_box(b, geom, dpi, RENDER_MASK_PAD_PX, (w, h)))
                .collect()
        })
        .collect()
}

/// What G-RENDER and the ink check tell apart under the masks (review-verify HIGH-A), in raster
/// pixels: `kept`, the boxes of glyphs no edit changes that some edit's masks reach; `own`, the
/// edited glyphs' own boxes (the new glyphs' masks, undilated, and the old glyphs' ink boxes,
/// each grown by `RENDER_OWN_PAD_PX`); `old`, the old glyphs' ink boxes alone (grown alike). A
/// pixel in a kept box and in no own box is checked even under the masks: the edit draws and
/// removes ink only within its own glyphs' boxes.
struct Layers {
    kept: Vec<PixelBox>,
    own: Vec<PixelBox>,
    old: Vec<PixelBox>,
}

impl Layers {
    fn new(
        edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
        near: &NearGlyphs,
        masks: &[Vec<PixelBox>],
        geom: &PageGeometry,
        dpi: u32,
        size: (usize, usize),
    ) -> Layers {
        let px = |r: &[f64; 4]| pixel_box(*r, geom, dpi, 0, size);
        let own_px = |r: &[f64; 4]| pixel_box(*r, geom, dpi, RENDER_OWN_PAD_PX, size);
        // Each edit's masks as one envelope: a kept glyph matters where some mask reaches it.
        let envelopes: Vec<PixelBox> = masks
            .iter()
            .filter_map(|boxes| {
                boxes.iter().copied().reduce(|a, b| PixelBox {
                    x0: a.x0.min(b.x0),
                    y0: a.y0.min(b.y0),
                    x1: a.x1.max(b.x1),
                    y1: a.y1.max(b.y1),
                })
            })
            .collect();
        let kept = near
            .kept
            .iter()
            .filter_map(|k| px(&k.rect))
            .filter(|b| envelopes.iter().any(|e| overlaps(e, b)))
            .collect();
        let old: Vec<PixelBox> = edits
            .iter()
            .flat_map(|(exp, _, _, _)| exp.old_ink_boxes.iter().filter_map(own_px))
            .collect();
        let mut own: Vec<PixelBox> = edits
            .iter()
            .flat_map(|(exp, _, _, _)| exp.new_glyph_masks().iter().filter_map(own_px))
            .collect();
        own.extend_from_slice(&old);
        Layers { kept, own, old }
    }
}

fn overlaps(a: &PixelBox, b: &PixelBox) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// Coverage layers of `check_pixels`'s row sweep.
const SLACK: usize = 0;
const KEPT: usize = 1;
const OWN: usize = 2;

fn add_coverage(cover: &mut [i64], at: usize, v: i64) {
    if let Some(c) = cover.get_mut(at) {
        *c = c.saturating_add(v);
    }
}

/// G-RENDER: differing pixels (any channel off by more than `RENDER_CHANNEL_TOL`) outside every
/// mask fail the check once there are more than `RENDER_OUTSIDE_PIXELS_MAX`; per edit, whether a
/// differing pixel lies inside one of its own masks. A pixel of a glyph no edit changes (`near`)
/// outside the edited glyphs' own boxes is outside the masks too (review-verify HIGH-A: a narrow
/// follower lies within the masks' slack), and more than `RENDER_KEPT_PIXELS_MAX` of them fail
/// the check on their own.
///
/// One pass over the rows: the union of the masks (and of the kept and own boxes) covering a row
/// is a coverage difference array updated only where a box starts or ends (O(pixels +
/// boxes·log boxes) in all), and a row's prefix count of differing pixels answers "does this mask
/// hold a changed pixel" in O(1), asked only on rows with a change, for edits not yet seen whose
/// masks span that row. The scan stops at the first row that pushes a count over its limit (the
/// details then carry the count so far).
pub(crate) fn check_pixels(
    src: &Raster,
    dst: &Raster,
    geom: &PageGeometry,
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    near: &NearGlyphs,
    dpi: u32,
) -> Result<Vec<bool>, (&'static str, String)> {
    if src.w != dst.w || src.h != dst.h {
        return Err((
            "render",
            format!("render size {}x{} vs {}x{}", src.w, src.h, dst.w, dst.h),
        ));
    }
    let (w, h) = (src.w as usize, src.h as usize);
    let row_bytes = w.saturating_mul(3);
    let mut visible = vec![false; edits.len()];
    if row_bytes == 0 || h == 0 {
        return Ok(visible);
    }
    let masks = pixel_masks(edits, geom, dpi, w, h);
    let layers = Layers::new(edits, near, &masks, geom, dpi, (w, h));
    // Rows each edit's masks span (a quick reject for rows far from the edit).
    let spans: Vec<(usize, usize)> = masks
        .iter()
        .map(|boxes| {
            boxes
                .iter()
                .fold((usize::MAX, 0), |(lo, hi), m| (lo.min(m.y0), hi.max(m.y1)))
        })
        .collect();
    // Coverage events: (row, column, layer, ±1) where a box starts (y0) or ends (y1).
    let boxes = (masks.iter().flatten().map(|m| (SLACK, m)))
        .chain(layers.kept.iter().map(|m| (KEPT, m)))
        .chain(layers.own.iter().map(|m| (OWN, m)));
    let mut events: Vec<(usize, usize, usize, i64)> = boxes
        .flat_map(|(l, m)| {
            [
                (m.y0, m.x0, l, 1),
                (m.y0, m.x1, l, -1),
                (m.y1, m.x0, l, -1),
                (m.y1, m.x1, l, 1),
            ]
        })
        .collect();
    events.sort_unstable();
    let mut next_event = 0usize;
    let mut cover = [(); 3].map(|_| vec![0i64; w.saturating_add(1)]);
    let mut prefix: Vec<u32> = Vec::with_capacity(w.saturating_add(1));
    let (mut outside, mut kept) = (0u64, 0u64);
    let rows = src
        .rgb
        .chunks_exact(row_bytes)
        .zip(dst.rgb.chunks_exact(row_bytes))
        .take(h);
    for (y, (row_a, row_b)) in rows.enumerate() {
        while let Some(&(ey, x, l, v)) = events.get(next_event) {
            if ey > y {
                break;
            }
            if let Some(layer) = cover.get_mut(l) {
                add_coverage(layer, x, v);
            }
            next_event = next_event.saturating_add(1);
        }
        prefix.clear();
        prefix.push(0);
        let (mut changed, mut row_outside, mut row_kept) = (0u32, 0u64, 0u64);
        let mut covered = [0i64; 3];
        let pixels = row_a.chunks_exact(3).zip(row_b.chunks_exact(3));
        for (x, (a, b)) in pixels.enumerate() {
            for (sum, layer) in covered.iter_mut().zip(&cover) {
                *sum = sum.saturating_add(layer.get(x).copied().unwrap_or(0));
            }
            if differs(a, b) {
                changed = changed.saturating_add(1);
                let in_kept = covered[KEPT] > 0 && covered[OWN] <= 0;
                if in_kept {
                    row_kept = row_kept.saturating_add(1);
                }
                if covered[SLACK] <= 0 || in_kept {
                    row_outside = row_outside.saturating_add(1);
                }
            }
            prefix.push(changed);
        }
        if changed == 0 {
            continue;
        }
        outside = outside.saturating_add(row_outside);
        kept = kept.saturating_add(row_kept);
        if kept > RENDER_KEPT_PIXELS_MAX {
            return Err((
                "render",
                format!("next to the edited line: a glyph no edit changes changed, pixels={kept}"),
            ));
        }
        if outside > RENDER_OUTSIDE_PIXELS_MAX {
            return Err(("render", format!("pixels={outside}")));
        }
        let changed_in = |m: &PixelBox| {
            m.y0 <= y
                && y < m.y1
                && prefix.get(m.x1).copied().unwrap_or(0) > prefix.get(m.x0).copied().unwrap_or(0)
        };
        for ((seen, boxes), (lo, hi)) in visible.iter_mut().zip(&masks).zip(&spans) {
            if !*seen && *lo <= y && y < *hi && boxes.iter().any(changed_in) {
                *seen = true;
            }
        }
    }
    Ok(visible)
}

/// §B.15 on one page: words (G-TEXT, `poppler/words.rs`) then pixels (G-RENDER); a neighbour of
/// an edited line that is no longer extracted at its place fails after the pixels, and one that
/// lost its ink under the edit's masks after that (`poppler/ink.rs`). `edits`: per edited run its
/// expectations, old and new user rects (`[x0, y0, x1, y1]`) and old text; `near`: the glyphs our
/// model draws around them (`poppler/near.rs`), which only say where to look harder. An edit
/// whose glyphs did not change is always visible (only its style changed, or nothing).
#[allow(clippy::too_many_arguments)]
pub fn check_independent(
    src_words: &[Word],
    dst_words: &[Word],
    src_raster: &Raster,
    dst_raster: &Raster,
    geom: &PageGeometry,
    edits: &[(ExpectedRun, [f64; 4], [f64; 4], String)],
    near: &NearGlyphs,
    dpi: u32,
) -> Result<IndependentReport, (&'static str, String)> {
    let words = check_words(src_words, dst_words, geom, edits, near)?;
    let visible = check_pixels(src_raster, dst_raster, geom, edits, near, dpi)?;
    if let Some(fail) = words.gone {
        return Err(fail);
    }
    let raster = (src_raster, dst_raster);
    ink::check_neighbour_ink(raster, geom, edits, near, &words.neighbours, dpi)?;
    Ok(IndependentReport {
        edit_visible: visible
            .iter()
            .zip(edits)
            .map(|(v, (exp, _, _, _))| *v || !exp.glyphs_changed)
            .collect(),
    })
}
