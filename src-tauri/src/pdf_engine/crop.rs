//! Crop: trim margins from every page by shrinking each page's MediaBox and
//! CropBox. Lossless (vectors/text untouched) — only the visible window changes.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use lopdf::{Document, Object, ObjectId};
use std::sync::Arc;

fn num(o: &Object, doc: &Document) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        Object::Reference(r) => doc.get_object(*r).ok().and_then(|x| num(x, doc)),
        _ => None,
    }
}

/// Resolve a page's box entry (`MediaBox`, `CropBox`, …), walking up Parent for
/// inheritance. Returns a normalized rect (x0 < x1, y0 < y1), or None.
fn inherited_box(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    let mut cur = Some(page_id);
    let mut steps = 0;
    while let Some(id) = cur {
        if steps > 32 {
            break;
        }
        steps += 1;
        let Ok(dict) = doc.get_dictionary(id) else { break };
        if let Ok(obj) = dict.get(key) {
            let resolved = if let Ok(r) = obj.as_reference() {
                doc.get_object(r).ok()
            } else {
                Some(obj)
            };
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
                        return Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]);
                    }
                }
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    None
}

/// Resolve a page's effective MediaBox, walking up Parent for inheritance.
pub(crate) fn media_box(doc: &Document, page_id: ObjectId) -> [f64; 4] {
    inherited_box(doc, page_id, b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0])
}

/// Effective /Rotate of a page (walking up Parent), normalized to 0/90/180/270.
pub(crate) fn page_rotation(doc: &Document, page_id: ObjectId) -> i64 {
    let mut cur = Some(page_id);
    let mut steps = 0;
    while let Some(id) = cur {
        if steps > 32 {
            break;
        }
        steps += 1;
        let Ok(dict) = doc.get_dictionary(id) else { break };
        if let Ok(obj) = dict.get(b"Rotate") {
            let resolved = if let Ok(r) = obj.as_reference() {
                doc.get_object(r).ok()
            } else {
                Some(obj)
            };
            if let Some(n) = resolved.and_then(|o| o.as_i64().ok()) {
                return ((n % 360) + 360) % 360;
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    0
}

/// Crop the combined document by trimming `left/top/right/bottom` percent of
/// each page's size from the respective edges.
#[allow(clippy::too_many_arguments)]
pub fn crop(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
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

        let mut doc = Document::load(&merged)
            .map_err(|e| AppError::engine_failed(format!("Could not read the PDF: {e}")))?;

        let page_ids: Vec<ObjectId> = doc.get_pages().values().cloned().collect();
        for id in page_ids {
            let mb = media_box(&doc, id);
            // Trim relative to the page area the user actually SEES: the
            // CropBox (clipped to the MediaBox) when present, else the MediaBox.
            // Basing it on the MediaBox alone would grow the visible window on
            // documents whose CropBox is smaller.
            let eff = match inherited_box(&doc, id, b"CropBox") {
                Some(cb) => {
                    let ix0 = cb[0].max(mb[0]);
                    let iy0 = cb[1].max(mb[1]);
                    let ix1 = cb[2].min(mb[2]);
                    let iy1 = cb[3].min(mb[3]);
                    if ix1 - ix0 > 1.0 && iy1 - iy0 > 1.0 { [ix0, iy0, ix1, iy1] } else { mb }
                }
                None => mb,
            };
            // The UI margins are visual; /Rotate 90/180/270 shuffles which
            // physical box edge each visual edge is (CW: left edge shows as top).
            let (l, t, r, b) = match page_rotation(&doc, id) {
                90 => (top, right, bottom, left),
                180 => (right, bottom, left, top),
                270 => (bottom, left, top, right),
                _ => (left, top, right, bottom),
            };
            let (w, h) = (eff[2] - eff[0], eff[3] - eff[1]);
            let nx0 = eff[0] + w * l / 100.0;
            let nx1 = eff[2] - w * r / 100.0;
            let ny0 = eff[1] + h * b / 100.0;
            let ny1 = eff[3] - h * t / 100.0;
            if nx1 - nx0 < 1.0 || ny1 - ny0 < 1.0 {
                return Err(AppError::new(
                    "CROP_TOO_MUCH",
                    "Crop is too large",
                    "The margins leave no visible page. Reduce them.",
                ));
            }
            let rect = Object::Array(vec![
                Object::Real(nx0 as f32),
                Object::Real(ny0 as f32),
                Object::Real(nx1 as f32),
                Object::Real(ny1 as f32),
            ]);
            if let Ok(dict) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
                dict.set(b"CropBox".to_vec(), rect.clone());
                // Shrink the MediaBox too, but never grow it past the original.
                if nx0 >= mb[0] - 0.01 && ny0 >= mb[1] - 0.01 && nx1 <= mb[2] + 0.01 && ny1 <= mb[3] + 0.01 {
                    dict.set(b"MediaBox".to_vec(), rect);
                }
                // Print boxes from the source could poke outside the new window.
                dict.remove(b"TrimBox");
                dict.remove(b"BleedBox");
                dict.remove(b"ArtBox");
            }
        }

        // `lopdf` writes a valid object set but an xref table some readers reject
        // as "damaged". Save to a temp file, then let qpdf rewrite a clean,
        // normalised PDF before handing it back.
        let cropped = work.join("cropped.pdf").to_string_lossy().to_string();
        doc.save(&cropped).map_err(|e| AppError::io("Could not write the cropped PDF.", e))?;
        drop(doc);
        run_qpdf(app, handle, job_id, &[cropped, output.to_string()], "Finalizing", None)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}
