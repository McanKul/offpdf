//! N-up / booklet ("print layout"): place several source pages per sheet.
//! 2-up and 4-up keep the page order and lay pages out on a grid; booklet
//! applies a saddle-stitch imposition (n,1 | 2,n-1 | …, padded to a multiple
//! of 4 with blanks) so the folded, stapled stack reads in order.
//!
//! Implementation: each source page is converted to a Form XObject (page
//! content + resources behind a /BBox), then drawn 2 or 4 per sheet, scaled to
//! fit its slot with a small gutter while preserving the aspect ratio.
//! Fully vector and lossless. Like poster.rs, the lopdf output is normalised
//! by a final qpdf pass (lopdf 0.34 writes an xref some readers reject).
//!
//! The whole combined document is loaded into memory (via `lopdf`), so it is
//! gated by file size like poster.rs.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::utils::temp;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::sync::Arc;
use tauri::Emitter;

/// Loading the whole combined document into `lopdf` beyond this size would
/// risk exhausting memory; refused with a friendly error.
const MAX_INPUT_BYTES: u64 = 2_000_000_000;
/// Safety cap on the number of generated sheets.
const MAX_SHEETS: usize = 2_000;
/// Margin around each placed page inside its slot (points).
const GUTTER: f64 = 12.0;

/// Layout mode. `TwoUp`/`FourUp` keep page order; `Booklet` reorders pages
/// for saddle-stitch printing (2 per printed side, fold + staple in the middle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    TwoUp,
    FourUp,
    Booklet,
}

fn parse_mode(s: &str) -> Result<Mode, AppError> {
    match s {
        "2up" => Ok(Mode::TwoUp),
        "4up" => Ok(Mode::FourUp),
        "booklet" => Ok(Mode::Booklet),
        other => Err(AppError::new(
            "INVALID_MODE",
            "Unknown layout",
            format!("\"{other}\" is not a valid layout. Use 2up, 4up or booklet."),
        )),
    }
}

// ---------------------------------------------------------------------------
// Pure layout math (unit-tested below; no lopdf/tauri types)
// ---------------------------------------------------------------------------

/// Saddle-stitch imposition for `n` pages: one `[left, right]` pair per printed
/// side, `0` = blank. Padded to a multiple of 4 so the booklet folds cleanly.
/// Sequence: (n,1), (2,n-1), (n-2,3), (4,n-3), … — print double-sided, flip on
/// the short edge, fold the stack in half and the pages read 1..n.
fn booklet_pairs(n: usize) -> Vec<[usize; 2]> {
    let padded = n.div_ceil(4) * 4;
    let blank_if_padding = |p: usize| if p <= n { p } else { 0 };
    (0..padded / 2)
        .map(|i| {
            let (left, right) = if i % 2 == 0 { (padded - i, i + 1) } else { (i + 1, padded - i) };
            [blank_if_padding(left), blank_if_padding(right)]
        })
        .collect()
}

/// The source pages (1-based, `0` = blank slot) on each output sheet, in
/// output order. Slot order matches `slot_rects` (reading order).
fn sheet_plan(mode: Mode, n: usize) -> Vec<Vec<usize>> {
    let sequential = |per: usize| -> Vec<Vec<usize>> {
        (1..=n)
            .collect::<Vec<usize>>()
            .chunks(per)
            .map(|c| {
                let mut v = c.to_vec();
                v.resize(per, 0);
                v
            })
            .collect()
    };
    match mode {
        Mode::TwoUp => sequential(2),
        Mode::FourUp => sequential(4),
        Mode::Booklet => booklet_pairs(n).into_iter().map(|p| p.to_vec()).collect(),
    }
}

/// Slot rectangles `[x0,y0,x1,y1]` on a `sw × sh` sheet, in reading order
/// (2-up/booklet: left, right; 4-up: top-left, top-right, bottom-left,
/// bottom-right).
fn slot_rects(mode: Mode, sw: f64, sh: f64) -> Vec<[f64; 4]> {
    match mode {
        Mode::TwoUp | Mode::Booklet => vec![[0.0, 0.0, sw / 2.0, sh], [sw / 2.0, 0.0, sw, sh]],
        Mode::FourUp => vec![
            [0.0, sh / 2.0, sw / 2.0, sh],
            [sw / 2.0, sh / 2.0, sw, sh],
            [0.0, 0.0, sw / 2.0, sh / 2.0],
            [sw / 2.0, 0.0, sw, sh / 2.0],
        ],
    }
}

/// The `cm` matrix `[a b c d e f]` that draws a page's Form XObject (whose
/// /BBox is `mb`, displayed with `rotate`) centred inside `slot`, scaled to
/// fit with `GUTTER` margin while preserving aspect ratio. The /Rotate of the
/// source page is baked into the matrix so sheets themselves are unrotated.
fn place_matrix(mb: [f64; 4], rotate: i64, slot: [f64; 4]) -> Option<[f64; 6]> {
    let (x0, y0, x1, y1) = (mb[0], mb[1], mb[2], mb[3]);
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 1.0 || h <= 1.0 {
        return None;
    }
    let r = rotate.rem_euclid(360);
    // Displayed (post-/Rotate) size.
    let (dw, dh) = if r % 180 == 90 { (h, w) } else { (w, h) };

    let slot_w = slot[2] - slot[0];
    let slot_h = slot[3] - slot[1];
    let avail_w = (slot_w - 2.0 * GUTTER).max(1.0);
    let avail_h = (slot_h - 2.0 * GUTTER).max(1.0);
    let s = (avail_w / dw).min(avail_h / dh);
    let cx = slot[0] + (slot_w - s * dw) / 2.0;
    let cy = slot[1] + (slot_h - s * dh) / 2.0;

    // R maps the page box (x0..x1, y0..y1) onto (0..dw, 0..dh) in displayed
    // orientation: x' = a·x + c·y + e, y' = b·x + d·y + f.
    let (a, b, c, d, e, f) = match r {
        90 => (0.0, -1.0, 1.0, 0.0, -y0, x1),
        180 => (-1.0, 0.0, 0.0, -1.0, x1, y1),
        270 => (0.0, 1.0, -1.0, 0.0, y1, -x0),
        _ => (1.0, 0.0, 0.0, 1.0, -x0, -y0),
    };
    Some([s * a, s * b, s * c, s * d, s * e + cx, s * f + cy])
}

// ---------------------------------------------------------------------------
// Page geometry helpers (self-contained, mirroring poster.rs)
// ---------------------------------------------------------------------------

fn num(o: &Object, doc: &Document) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        Object::Reference(r) => doc.get_object(*r).ok().and_then(|x| num(x, doc)),
        _ => None,
    }
}

/// Resolve a page's effective MediaBox `[x0,y0,x1,y1]`, walking up Parent.
fn media_box(doc: &Document, page_id: ObjectId) -> [f64; 4] {
    let mut cur = Some(page_id);
    let mut steps = 0;
    while let Some(id) = cur {
        if steps > 32 {
            break;
        }
        steps += 1;
        let Ok(dict) = doc.get_dictionary(id) else { break };
        if let Ok(obj) = dict.get(b"MediaBox") {
            let resolved = if let Ok(r) = obj.as_reference() { doc.get_object(r).ok() } else { Some(obj) };
            if let Some(arr) = resolved.and_then(|o| o.as_array().ok()) {
                if arr.len() == 4 {
                    let mut v = [0.0; 4];
                    let mut ok = true;
                    for (i, e) in arr.iter().enumerate() {
                        match num(e, doc) {
                            Some(n) => v[i] = n,
                            None => ok = false,
                        }
                    }
                    if ok {
                        return [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
                    }
                }
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    [0.0, 0.0, 612.0, 792.0]
}

/// Find an inherited page attribute (Resources, Rotate) by walking parents.
fn inherited(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut cur = Some(page_id);
    let mut steps = 0;
    while let Some(id) = cur {
        if steps > 32 {
            break;
        }
        steps += 1;
        let Ok(dict) = doc.get_dictionary(id) else { break };
        if let Ok(obj) = dict.get(key) {
            return Some(obj.clone());
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    None
}

// ---------------------------------------------------------------------------
// Core (lopdf only — verified standalone in a scratch cargo project)
// ---------------------------------------------------------------------------

/// Rewrite `doc` in place: its pages become `sheet_w × sheet_h` sheets with 2
/// or 4 of the original pages drawn on each (as Form XObjects). Returns the
/// number of sheets produced. Errors are plain strings so the core stays free
/// of app-level types.
fn build_nup(doc: &mut Document, mode: Mode, sheet_w: f64, sheet_h: f64) -> Result<usize, String> {
    let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let n = page_ids.len();
    if n == 0 {
        return Err("The document has no pages.".to_string());
    }
    let pages_id = doc
        .get_dictionary(page_ids[0])
        .ok()
        .and_then(|d| d.get(b"Parent").ok())
        .and_then(|o| o.as_reference().ok())
        .ok_or_else(|| "The document's page tree could not be read.".to_string())?;

    // Convert every source page into a Form XObject. Content streams are
    // decompressed + concatenated by lopdf; qpdf recompresses on the final pass.
    let mut xobjects: Vec<(ObjectId, [f64; 4], i64)> = Vec::with_capacity(n);
    for &pid in &page_ids {
        let content = doc
            .get_page_content(pid)
            .map_err(|e| format!("Could not read page content: {e}"))?;
        let mb = media_box(doc, pid);
        let rotate = inherited(doc, pid, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0);
        let resources = inherited(doc, pid, b"Resources").unwrap_or(Object::Dictionary(Dictionary::new()));

        let mut d = Dictionary::new();
        d.set(b"Type".to_vec(), Object::Name(b"XObject".to_vec()));
        d.set(b"Subtype".to_vec(), Object::Name(b"Form".to_vec()));
        d.set(
            b"BBox".to_vec(),
            Object::Array(vec![
                Object::Real(mb[0] as f32),
                Object::Real(mb[1] as f32),
                Object::Real(mb[2] as f32),
                Object::Real(mb[3] as f32),
            ]),
        );
        d.set(b"Resources".to_vec(), resources);
        let xid = doc.add_object(Object::Stream(Stream::new(d, content)));
        xobjects.push((xid, mb, rotate));
    }

    let plan = sheet_plan(mode, n);
    let slots = slot_rects(mode, sheet_w, sheet_h);
    let sheet_box = Object::Array(vec![
        Object::Real(0.0),
        Object::Real(0.0),
        Object::Real(sheet_w as f32),
        Object::Real(sheet_h as f32),
    ]);

    let mut sheet_ids: Vec<ObjectId> = Vec::with_capacity(plan.len());
    for pages in &plan {
        let mut ops = String::new();
        let mut xdict = Dictionary::new();
        for (slot_idx, &page_no) in pages.iter().enumerate() {
            if page_no == 0 {
                continue; // blank (booklet padding / odd tail)
            }
            let (xid, mb, rotate) = xobjects[page_no - 1];
            let Some(m) = place_matrix(mb, rotate, slots[slot_idx]) else { continue };
            let name = format!("P{slot_idx}");
            ops.push_str(&format!(
                "q\n{:.4} {:.4} {:.4} {:.4} {:.2} {:.2} cm\n/{name} Do\nQ\n",
                m[0], m[1], m[2], m[3], m[4], m[5],
            ));
            xdict.set(name.into_bytes(), Object::Reference(xid));
        }
        let content_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), ops.into_bytes())));

        let mut res = Dictionary::new();
        res.set(b"XObject".to_vec(), Object::Dictionary(xdict));
        let mut page = Dictionary::new();
        page.set(b"Type".to_vec(), Object::Name(b"Page".to_vec()));
        page.set(b"Parent".to_vec(), Object::Reference(pages_id));
        page.set(b"MediaBox".to_vec(), sheet_box.clone());
        page.set(b"Resources".to_vec(), Object::Dictionary(res));
        page.set(b"Rotate".to_vec(), Object::Integer(0)); // never inherit a source /Rotate
        page.set(b"Contents".to_vec(), Object::Reference(content_id));
        sheet_ids.push(doc.add_object(Object::Dictionary(page)));
    }

    // Point the page tree at the sheets and drop stale inheritable attributes.
    let pd = doc
        .get_object_mut(pages_id)
        .ok()
        .and_then(|o| o.as_dict_mut().ok())
        .ok_or_else(|| "The document's page tree could not be updated.".to_string())?;
    pd.set(
        b"Kids".to_vec(),
        Object::Array(sheet_ids.iter().map(|id| Object::Reference(*id)).collect()),
    );
    pd.set(b"Count".to_vec(), Object::Integer(sheet_ids.len() as i64));
    pd.remove(b"MediaBox");
    pd.remove(b"Rotate");
    Ok(sheet_ids.len())
}

// ---------------------------------------------------------------------------
// Tauri-facing operation
// ---------------------------------------------------------------------------

/// Assemble the combined document, then lay its pages out `mode`-up on
/// `sheet_w × sheet_h` sheets (points). `mode`: "2up" | "4up" | "booklet".
#[allow(clippy::too_many_arguments)]
pub fn nup(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    mode: &str,
    sheet_w: f64,
    sheet_h: f64,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    let mode = parse_mode(mode)?;
    if sheet_w < 72.0 || sheet_h < 72.0 {
        return Err(AppError::new(
            "SHEET_TOO_SMALL",
            "Sheet size too small",
            "Choose a valid paper size for the sheets.",
        ));
    }
    for g in groups {
        super::require_input(&g.path)?;
    }
    super::ensure_output_dir(output)?;

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work).map_err(|e| AppError::io("Could not create a temp directory.", e))?;
    let merged = work.join("merged.pdf").to_string_lossy().to_string();

    let result = (|| -> Result<Vec<String>, AppError> {
        super::assemble_groups(app, handle, job_id, groups, &merged, "Preparing", None)?;
        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }

        let size = std::fs::metadata(&merged).map(|m| m.len()).unwrap_or(0);
        if size > MAX_INPUT_BYTES {
            return Err(AppError::new(
                "NUP_TOO_LARGE",
                "Document is too large for this layout",
                "This layout needs to load the document into memory. Try compressing it first.",
            ));
        }

        let mut doc = Document::load(&merged)
            .map_err(|e| AppError::engine_failed(format!("Could not read the document: {e}")))?;
        let n = doc.get_pages().len();
        let sheets = sheet_plan(mode, n).len();
        if sheets > MAX_SHEETS {
            return Err(AppError::new(
                "TOO_MANY_SHEETS",
                "Too many sheets",
                format!("This would make {sheets} sheets."),
            ));
        }
        let _ = app.emit(
            "job:update",
            crate::models::JobUpdate::new(
                job_id,
                "running",
                &format!("Laying out {n} pages onto {sheets} sheets"),
            ),
        );

        build_nup(&mut doc, mode, sheet_w, sheet_h).map_err(AppError::engine_failed)?;
        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }

        // lopdf writes a valid object set but an xref table some readers
        // reject as "damaged" — save to a temp file and let qpdf rewrite a
        // clean, normalised PDF (exactly like poster.rs).
        let laid = work.join("nup.pdf").to_string_lossy().to_string();
        doc.save(&laid).map_err(|e| AppError::io("Could not write the layout PDF.", e))?;
        drop(doc);
        crate::utils::process::run_qpdf(app, handle, job_id, &[laid, output.to_string()], "Finalizing", None)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::{booklet_pairs, place_matrix, sheet_plan, slot_rects, Mode, GUTTER};

    #[test]
    fn booklet_order_for_multiple_of_four() {
        assert_eq!(booklet_pairs(8), vec![[8, 1], [2, 7], [6, 3], [4, 5]]);
        assert_eq!(booklet_pairs(4), vec![[4, 1], [2, 3]]);
    }

    #[test]
    fn booklet_pads_with_blanks_to_multiple_of_four() {
        // 5 pages pad to 8; pages 6-8 become blanks (0).
        assert_eq!(booklet_pairs(5), vec![[0, 1], [2, 0], [0, 3], [4, 5]]);
        // 6 pages pad to 8.
        assert_eq!(booklet_pairs(6), vec![[0, 1], [2, 0], [6, 3], [4, 5]]);
    }

    #[test]
    fn sequential_plans_pad_the_last_sheet() {
        assert_eq!(sheet_plan(Mode::TwoUp, 5), vec![vec![1, 2], vec![3, 4], vec![5, 0]]);
        assert_eq!(sheet_plan(Mode::FourUp, 6), vec![vec![1, 2, 3, 4], vec![5, 6, 0, 0]]);
        assert_eq!(sheet_plan(Mode::TwoUp, 4).len(), 2);
    }

    #[test]
    fn four_up_slots_read_top_left_to_bottom_right() {
        let s = slot_rects(Mode::FourUp, 100.0, 200.0);
        assert_eq!(s.len(), 4);
        assert_eq!(s[0], [0.0, 100.0, 50.0, 200.0]); // top-left
        assert_eq!(s[3], [50.0, 0.0, 100.0, 100.0]); // bottom-right
        let two = slot_rects(Mode::Booklet, 100.0, 200.0);
        assert_eq!(two, vec![[0.0, 0.0, 50.0, 200.0], [50.0, 0.0, 100.0, 200.0]]);
    }

    #[test]
    fn place_matrix_centres_and_preserves_aspect() {
        // US Letter into the left half of an A4 landscape sheet.
        let m = place_matrix([0.0, 0.0, 612.0, 792.0], 0, [0.0, 0.0, 421.0, 595.0]).unwrap();
        let s = (421.0 - 2.0 * GUTTER) / 612.0; // width-limited? no: height wins
        let s_h = (595.0 - 2.0 * GUTTER) / 792.0;
        let expect = s.min(s_h);
        assert!((m[0] - expect).abs() < 1e-9);
        assert_eq!(m[1], 0.0);
        assert_eq!(m[2], 0.0);
        assert!((m[3] - expect).abs() < 1e-9);
        // Centred: left offset == (slot_w - s*w) / 2.
        assert!((m[4] - (421.0 - expect * 612.0) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn place_matrix_bakes_in_page_rotation() {
        // A /Rotate 90 A4 page displays landscape: its corners must land in a
        // 0..dh × 0..dw box (before scale/offset). Check corner mapping.
        let mb = [0.0, 0.0, 595.0, 842.0];
        let m = place_matrix(mb, 90, [0.0, 0.0, 842.0, 595.0]).unwrap();
        let map = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        // Displayed size is 842 × 595 (w/h swapped) → all corners inside slot.
        for (x, y) in [(0.0, 0.0), (595.0, 0.0), (0.0, 842.0), (595.0, 842.0)] {
            let (px, py) = map(x, y);
            assert!(px >= GUTTER - 1e-6 && px <= 842.0 - GUTTER + 1e-6, "x out of slot: {px}");
            assert!(py >= GUTTER - 1e-6 && py <= 595.0 - GUTTER + 1e-6, "y out of slot: {py}");
        }
    }

    #[test]
    fn degenerate_page_boxes_are_skipped() {
        assert!(place_matrix([0.0, 0.0, 0.5, 792.0], 0, [0.0, 0.0, 421.0, 595.0]).is_none());
    }
}
