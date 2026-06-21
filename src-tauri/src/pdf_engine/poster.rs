//! Poster / tile: split ONE large page (e.g. an A1 plan) into a grid of
//! printable tiles (A4 / A3 / Letter). Each output page is a copy of the source
//! page whose MediaBox/CropBox is a sub-rectangle of the original — lossless,
//! fully vector, and the heavy page content is *shared* between tiles (so the
//! output file barely grows). Print each sheet at 100 % (actual size) and tape
//! them together.
//!
//! This needs the chosen page loaded into memory (via `lopdf`), so it is gated
//! by file size: a single page beyond ~2 GB is refused with a clear message.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::pdf_engine::qpdf;
use crate::utils::temp;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::sync::Arc;
use tauri::Emitter;

/// Loading a single page beyond this size into `lopdf` would risk exhausting
/// memory. Tiling is refused above it with a friendly error.
const MAX_PAGE_BYTES: u64 = 2_000_000_000;
/// Safety cap on the number of generated sheets (catches near-zero tile sizes).
const MAX_TILES: usize = 600;

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
                        // Normalise so x0<x1, y0<y1.
                        return [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
                    }
                }
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    [0.0, 0.0, 612.0, 792.0]
}

/// Find an inherited dictionary attribute (e.g. Resources) by walking parents.
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

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Object {
    Object::Array(vec![
        Object::Real(x0 as f32),
        Object::Real(y0 as f32),
        Object::Real(x1 as f32),
        Object::Real(y1 as f32),
    ])
}

/// Dashed "cut line" just inside a tile's edges, drawn in absolute page coords.
fn cut_marks(x0: f64, y0: f64, w: f64, h: f64) -> Stream {
    let s = format!(
        "q\n0.5 0.5 0.5 RG\n0.6 w\n[4 4] 0 d\n{:.2} {:.2} {:.2} {:.2} re\nS\nQ\n",
        x0 + 0.3,
        y0 + 0.3,
        (w - 0.6).max(0.0),
        (h - 0.6).max(0.0),
    );
    Stream::new(Dictionary::new(), s.into_bytes())
}

/// Compute how many tiles span `total` length, given a tile and overlap.
fn tile_count(total: f64, tile: f64, overlap: f64) -> usize {
    if total <= tile + 0.5 {
        return 1;
    }
    let step = (tile - overlap).max(1.0);
    (((total - tile) / step).ceil() as usize) + 1
}

#[allow(clippy::too_many_arguments)]
pub fn poster(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    page: u32,
    tile_w: f64,
    tile_h: f64,
    overlap: f64,
    marks: bool,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    if tile_w < 36.0 || tile_h < 36.0 {
        return Err(AppError::new(
            "TILE_TOO_SMALL",
            "Sheet size too small",
            "Choose a valid paper size for the tiles.",
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
        // Assemble the combined document, then isolate just the chosen page so
        // we never load more than one page into memory.
        super::assemble_groups(app, handle, job_id, groups, &merged, "Preparing", None)?;
        let total = qpdf::npages(app, &merged)?;
        let page = page.clamp(1, total.max(1));

        let one = if total == 1 {
            merged.clone()
        } else {
            let one = work.join("one.pdf").to_string_lossy().to_string();
            crate::utils::process::run_qpdf(
                app,
                handle,
                job_id,
                &["--empty".into(), "--pages".into(), merged.clone(), page.to_string(), "--".into(), one.clone()],
                "Selecting page",
                None,
            )?;
            one
        };

        if handle.is_cancelled() {
            return Err(AppError::cancelled());
        }

        let size = std::fs::metadata(&one).map(|m| m.len()).unwrap_or(0);
        if size > MAX_PAGE_BYTES {
            return Err(AppError::new(
                "POSTER_TOO_LARGE",
                "Page is too large to tile",
                "This page is extremely large; tiling needs to load it into memory. Try compressing it first.",
            ));
        }

        let mut doc = Document::load(&one)
            .map_err(|e| AppError::engine_failed(format!("Could not read the page: {e}")))?;

        let orig_id = *doc
            .get_pages()
            .values()
            .next()
            .ok_or_else(|| AppError::engine_failed("The selected page could not be read.".to_string()))?;

        // Read everything we need from the source page before mutating it.
        let mb = media_box(&doc, orig_id);
        let (px0, py0, px1, py1) = (mb[0], mb[1], mb[2], mb[3]);
        let (pw, ph) = (px1 - px0, py1 - py0);

        let contents_obj = doc
            .get_dictionary(orig_id)
            .ok()
            .and_then(|d| d.get(b"Contents").ok().cloned())
            .unwrap_or(Object::Array(vec![]));
        let resources_obj = inherited(&doc, orig_id, b"Resources").unwrap_or(Object::Dictionary(Dictionary::new()));
        let rotate = inherited(&doc, orig_id, b"Rotate").and_then(|o| o.as_i64().ok()).unwrap_or(0);
        let pages_id = doc
            .get_dictionary(orig_id)
            .ok()
            .and_then(|d| d.get(b"Parent").ok())
            .and_then(|o| o.as_reference().ok());

        // Clamp overlap so a tile still advances.
        let overlap = overlap.max(0.0).min(tile_w.min(tile_h) - 12.0).max(0.0);
        let step_x = (tile_w - overlap).max(1.0);
        let step_y = (tile_h - overlap).max(1.0);
        let cols = tile_count(pw, tile_w, overlap).max(1);
        let rows = tile_count(ph, tile_h, overlap).max(1);
        if cols * rows > MAX_TILES {
            return Err(AppError::new(
                "TOO_MANY_TILES",
                "Too many sheets",
                format!("This would make {} sheets. Use a larger sheet size or less overlap.", cols * rows),
            ));
        }

        let _ = app.emit(
            "job:update",
            crate::models::JobUpdate::new(job_id, "running", &format!("Tiling into {cols} × {rows} = {} sheets", cols * rows)),
        );

        // Base content stream ids (shared by every tile).
        let base_ids: Vec<ObjectId> = match &contents_obj {
            Object::Reference(id) => vec![*id],
            Object::Array(arr) => arr.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => vec![],
        };
        // Shared q / Q wrappers so cut marks draw in a clean graphics state.
        let (q_id, qq_id) = if marks {
            (
                Some(doc.add_object(Object::Stream(Stream::new(Dictionary::new(), b"q\n".to_vec())))),
                Some(doc.add_object(Object::Stream(Stream::new(Dictionary::new(), b"Q\n".to_vec())))),
            )
        } else {
            (None, None)
        };

        // Build the tiles top-to-bottom, left-to-right (reading order).
        let mut tile_ids: Vec<ObjectId> = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            if handle.is_cancelled() {
                return Err(AppError::cancelled());
            }
            let ty1 = (py1 - r as f64 * step_y).min(py1);
            let ty0 = (ty1 - tile_h).max(py0);
            for c in 0..cols {
                let tx0 = (px0 + c as f64 * step_x).max(px0);
                let tx1 = (tx0 + tile_w).min(px1);
                let (tw, th) = (tx1 - tx0, ty1 - ty0);
                if tw < 2.0 || th < 2.0 {
                    continue;
                }
                let tile_rect = rect(tx0, ty0, tx1, ty1);

                let contents_for_tile = if marks {
                    let marks_id =
                        doc.add_object(Object::Stream(cut_marks(tx0, ty0, tw, th)));
                    let mut arr: Vec<Object> = Vec::with_capacity(base_ids.len() + 3);
                    arr.push(Object::Reference(q_id.unwrap()));
                    for b in &base_ids {
                        arr.push(Object::Reference(*b));
                    }
                    arr.push(Object::Reference(qq_id.unwrap()));
                    arr.push(Object::Reference(marks_id));
                    Object::Array(arr)
                } else {
                    contents_obj.clone()
                };

                let is_first = tile_ids.is_empty();
                if is_first {
                    // Reuse the original page object for the first tile.
                    if let Ok(dict) = doc.get_object_mut(orig_id).and_then(|o| o.as_dict_mut()) {
                        dict.set(b"MediaBox".to_vec(), tile_rect.clone());
                        dict.set(b"CropBox".to_vec(), tile_rect);
                        dict.set(b"Resources".to_vec(), resources_obj.clone());
                        dict.set(b"Contents".to_vec(), contents_for_tile);
                        if rotate != 0 {
                            dict.set(b"Rotate".to_vec(), Object::Integer(rotate));
                        }
                    }
                    tile_ids.push(orig_id);
                } else {
                    let mut d = Dictionary::new();
                    d.set(b"Type".to_vec(), Object::Name(b"Page".to_vec()));
                    if let Some(pid) = pages_id {
                        d.set(b"Parent".to_vec(), Object::Reference(pid));
                    }
                    d.set(b"MediaBox".to_vec(), tile_rect.clone());
                    d.set(b"CropBox".to_vec(), tile_rect);
                    d.set(b"Resources".to_vec(), resources_obj.clone());
                    if rotate != 0 {
                        d.set(b"Rotate".to_vec(), Object::Integer(rotate));
                    }
                    d.set(b"Contents".to_vec(), contents_for_tile);
                    tile_ids.push(doc.add_object(Object::Dictionary(d)));
                }
            }
        }

        if tile_ids.is_empty() {
            return Err(AppError::new("NO_TILES", "Nothing to tile", "The page produced no tiles."));
        }

        // Point the page tree at the new tiles.
        if let Some(pid) = pages_id {
            if let Ok(pd) = doc.get_object_mut(pid).and_then(|o| o.as_dict_mut()) {
                pd.set(
                    b"Kids".to_vec(),
                    Object::Array(tile_ids.iter().map(|id| Object::Reference(*id)).collect()),
                );
                pd.set(b"Count".to_vec(), Object::Integer(tile_ids.len() as i64));
                // Each tile sets its own box; drop any stale full-page box on the parent.
                pd.remove(b"MediaBox");
            }
        }

        // `lopdf` writes a valid object set but an xref table some readers reject
        // as "damaged". Save to a temp file, then let qpdf rewrite a clean,
        // normalised PDF (object sharing — so file size — is preserved).
        let tiled = work.join("tiled.pdf").to_string_lossy().to_string();
        doc.save(&tiled).map_err(|e| AppError::io("Could not write the poster PDF.", e))?;
        drop(doc);
        crate::utils::process::run_qpdf(app, handle, job_id, &[tiled, output.to_string()], "Finalizing", None)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}
