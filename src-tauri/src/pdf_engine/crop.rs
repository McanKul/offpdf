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

/// Resolve the effective CropBox, preserving whether the source had one.
pub(crate) fn crop_box(doc: &Document, page_id: ObjectId) -> Option<[f64; 4]> {
    inherited_box(doc, page_id, b"CropBox")
}

fn intersect_box(inner: [f64; 4], media: [f64; 4]) -> [f64; 4] {
    let ix0 = inner[0].max(media[0]);
    let iy0 = inner[1].max(media[1]);
    let ix1 = inner[2].min(media[2]);
    let iy1 = inner[3].min(media[3]);
    if ix1 - ix0 > 1.0 && iy1 - iy0 > 1.0 {
        [ix0, iy0, ix1, iy1]
    } else {
        media
    }
}

/// Visible page box = CropBox ∩ MediaBox (else MediaBox). Matches pdf.js paint.
pub(crate) fn visible_box(doc: &Document, page_id: ObjectId) -> [f64; 4] {
    let mb = media_box(doc, page_id);
    match crop_box(doc, page_id) {
        Some(cb) => intersect_box(cb, mb),
        None => mb,
    }
}

/// Box on the page dict only (qpdf overlay does not inherit `/TrimBox`).
fn page_dict_box(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f64; 4]> {
    let Ok(dict) = doc.get_dictionary(page_id) else {
        return None;
    };
    let Ok(obj) = dict.get(key) else {
        return None;
    };
    let resolved = if let Ok(r) = obj.as_reference() {
        doc.get_object(r).ok()
    } else {
        Some(obj)
    };
    let arr = resolved.and_then(|o| o.as_array().ok())?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0.0; 4];
    for (i, e) in arr.iter().enumerate() {
        v[i] = num(e, doc)?;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// Resolve a page-level TrimBox. qpdf overlay alignment does not inherit it.
pub(crate) fn page_trim_box(doc: &Document, page_id: ObjectId) -> Option<[f64; 4]> {
    page_dict_box(doc, page_id, b"TrimBox")
}

/// qpdf overlay alignment: raw **page** TrimBox (not inherited, not clipped to
/// Media) → inherited CropBox → MediaBox. Matches `getTrimBox()` used by `--overlay`.
pub(crate) fn align_box(doc: &Document, page_id: ObjectId) -> [f64; 4] {
    if let Some(trim) = page_trim_box(doc, page_id) {
        return trim;
    }
    crop_box(doc, page_id).unwrap_or_else(|| media_box(doc, page_id))
}

/// Page `/UserUnit` (default 1). Not inherited.
pub(crate) fn page_user_unit(doc: &Document, page_id: ObjectId) -> f64 {
    let Ok(dict) = doc.get_dictionary(page_id) else {
        return 1.0;
    };
    let Ok(obj) = dict.get(b"UserUnit") else {
        return 1.0;
    };
    let resolved = if let Ok(r) = obj.as_reference() {
        doc.get_object(r).ok()
    } else {
        Some(obj)
    };
    let n = resolved.and_then(|o| match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    });
    match n {
        Some(v) if v.is_finite() && v > 0.0 => v,
        _ => 1.0,
    }
}

/// Displayed width/height after /Rotate from the **visible** box. Stamp and
/// watermark overlays use this; Edit PDF overlay uses the align box instead.
pub(crate) fn displayed_size(doc: &Document, page_id: ObjectId) -> (f64, f64) {
    let b = visible_box(doc, page_id);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if page_rotation(doc, page_id) % 180 == 90 {
        (h, w)
    } else {
        (w, h)
    }
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
            // Trim relative to the page area the user actually SEES.
            let eff = visible_box(&doc, id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Document, Object, Stream};

    fn rect_obj(b: [i64; 4]) -> Object {
        Object::Array(b.into_iter().map(Object::Integer).collect())
    }

    fn one_page(
        pages_extra: &[(&[u8], Object)],
        page_extra: &[(&[u8], Object)],
    ) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), Vec::new())));

        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set("Contents", content_id);
        for (k, v) in page_extra {
            page.set(k.to_vec(), v.clone());
        }
        let page_id = doc.add_object(Object::Dictionary(page));

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", vec![page_id.into()]);
        pages.set("Count", 1);
        for (k, v) in pages_extra {
            pages.set(k.to_vec(), v.clone());
        }
        doc.objects.insert(pages_id, Object::Dictionary(pages));

        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", pages_id);
        let cat_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", cat_id);
        (doc, page_id)
    }

    #[test]
    fn parent_trim_is_ignored_align_follows_crop() {
        let (doc, pid) = one_page(
            &[
                (b"MediaBox", rect_obj([0, 0, 612, 792])),
                (b"TrimBox", rect_obj([0, 0, 612, 792])),
            ],
            &[(b"CropBox", rect_obj([72, 72, 540, 720]))],
        );
        assert_eq!(visible_box(&doc, pid), [72.0, 72.0, 540.0, 720.0]);
        assert_eq!(align_box(&doc, pid), [72.0, 72.0, 540.0, 720.0]);
    }

    #[test]
    fn page_trim_differs_from_crop() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[
                (b"CropBox", rect_obj([72, 72, 540, 720])),
                (b"TrimBox", rect_obj([0, 0, 612, 792])),
            ],
        );
        assert_eq!(visible_box(&doc, pid), [72.0, 72.0, 540.0, 720.0]);
        assert_eq!(align_box(&doc, pid), [0.0, 0.0, 612.0, 792.0]);
    }

    #[test]
    fn no_trim_align_equals_visible() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[(b"CropBox", rect_obj([72, 72, 540, 720]))],
        );
        assert_eq!(align_box(&doc, pid), visible_box(&doc, pid));
    }

    #[test]
    fn page_trim_outside_media_is_not_clipped() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[(b"TrimBox", rect_obj([-10, -10, 700, 700]))],
        );
        assert_eq!(align_box(&doc, pid), [-10.0, -10.0, 700.0, 700.0]);
    }

    #[test]
    fn user_unit_on_page_only() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[(b"UserUnit", Object::Real(2.0))],
        );
        assert!((page_user_unit(&doc, pid) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn user_unit_on_parent_is_not_inherited() {
        let (doc, pid) = one_page(
            &[
                (b"MediaBox", rect_obj([0, 0, 612, 792])),
                (b"UserUnit", Object::Real(2.0)),
            ],
            &[],
        );
        assert!((page_user_unit(&doc, pid) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn user_unit_10000_real_is_accepted() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[(b"UserUnit", Object::Real(10000.0))],
        );
        assert!(
            (page_user_unit(&doc, pid) - 10000.0).abs() < 1e-6,
            "UserUnit 10000 Real must not clamp to 1, got {}",
            page_user_unit(&doc, pid)
        );
    }

    #[test]
    fn user_unit_10000_integer_is_accepted() {
        let (doc, pid) = one_page(
            &[(b"MediaBox", rect_obj([0, 0, 612, 792]))],
            &[(b"UserUnit", Object::Integer(10000))],
        );
        assert!(
            (page_user_unit(&doc, pid) - 10000.0).abs() < 1e-6,
            "UserUnit 10000 Integer must not clamp to 1, got {}",
            page_user_unit(&doc, pid)
        );
    }
}
