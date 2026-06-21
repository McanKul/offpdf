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

/// Resolve a page's effective MediaBox, walking up Parent for inheritance.
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
                        return v;
                    }
                }
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    [0.0, 0.0, 612.0, 792.0]
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
            let (w, h) = (mb[2] - mb[0], mb[3] - mb[1]);
            let nx0 = mb[0] + w * left / 100.0;
            let nx1 = mb[2] - w * right / 100.0;
            let ny0 = mb[1] + h * bottom / 100.0;
            let ny1 = mb[3] - h * top / 100.0;
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
                dict.set(b"MediaBox".to_vec(), rect.clone());
                dict.set(b"CropBox".to_vec(), rect);
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
