//! Secure redaction: rasterize affected pages in place and verify removal.
//!
//! Only pages with ≥1 region are rewritten. Fill (and an optional label) are
//! burned into the raster before encode. Leftover `/Annots`, field `/V`, and
//! attachments are warnings, not a strip. Page-content probes fail closed.

use crate::error::AppError;
use crate::pdf_engine::crop;
use crate::pdf_engine::edit_overlay::PdfRectIn;
use crate::pdf_engine::render;
use crate::utils::safe_output;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const RASTER_DPI: u32 = 72;

pub struct RedactRegion {
    pub page_index: u32,
    pub rect: PdfRectIn,
    pub fill: Option<String>,
    pub label: Option<String>,
}

/// Mutate `path` in place. Tests copy source → dest first.
pub fn apply_redactions(path: &Path, regions: &[RedactRegion]) -> Result<(), AppError> {
    apply_redactions_inner(path, regions, None)
}

/// Same as [`apply_redactions`], using the packaged `pdftoppm` preview uses.
pub(crate) fn apply_redactions_with_app(
    app: &tauri::AppHandle,
    path: &Path,
    regions: &[RedactRegion],
) -> Result<(), AppError> {
    apply_redactions_inner(path, regions, Some(render::resolve_pdftoppm(app)))
}

fn apply_redactions_inner(
    path: &Path,
    regions: &[RedactRegion],
    pdftoppm: Option<PathBuf>,
) -> Result<(), AppError> {
    if regions.is_empty() {
        return Ok(());
    }
    if !path.is_file() {
        return Err(AppError::invalid_pdf(&path.to_string_lossy()));
    }

    let mut by_page: HashMap<u32, Vec<&RedactRegion>> = HashMap::new();
    for region in regions {
        by_page.entry(region.page_index).or_default().push(region);
    }

    let work = std::env::temp_dir().join(format!(
        "offpdf-redact-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;

    let result = (|| {
        let exe = match pdftoppm {
            Some(p) => p,
            None => resolve_pdftoppm_standalone(),
        };
        ensure_pdftoppm(&exe)?;

        let mut rasters: HashMap<u32, (u32, u32, Vec<u8>)> = HashMap::new();
        {
            let probe = Document::load(path).map_err(|e| {
                AppError::engine_failed(format!("Could not read the PDF: {e}"))
            })?;
            let pages = probe.get_pages();
            for &page_index in by_page.keys() {
                if pages.get(&(page_index + 1)).is_none() {
                    return Err(redaction_page_missing(page_index));
                }
            }
            let mut needs_unrotated = false;
            for &page_index in by_page.keys() {
                let Some(&page_id) = pages.get(&(page_index + 1)) else {
                    return Err(redaction_page_missing(page_index));
                };
                if crop::page_rotation(&probe, page_id) != 0 {
                    needs_unrotated = true;
                    break;
                }
            }
            let raster_src = if needs_unrotated {
                let sibling = work.join("unrotated.pdf");
                write_unrotated_copy(path, &sibling)?;
                sibling
            } else {
                path.to_path_buf()
            };
            for (&page_index, page_regions) in &by_page {
                let page_no = page_index + 1;
                let Some(&page_id) = pages.get(&page_no) else {
                    return Err(redaction_page_missing(page_index));
                };
                let media = crop::media_box(&probe, page_id);
                let user_unit = crop::page_user_unit(&probe, page_id);
                let (w, h, mut rgb) =
                    raster_page(&exe, &raster_src, page_no, media, user_unit, &work)?;
                for region in page_regions {
                    let color = parse_fill(region.fill.as_deref());
                    fill_pdf_rect(&mut rgb, w, h, media, user_unit, &region.rect, color);
                    if let Some(label) = region.label.as_deref() {
                        if !label.trim().is_empty() {
                            draw_label(&mut rgb, w, h, media, user_unit, &region.rect, label);
                        }
                    }
                }
                rasters.insert(page_index, (w, h, rgb));
            }
        }

        if rasters.len() != by_page.len() || by_page.keys().any(|p| !rasters.contains_key(p)) {
            let missing = by_page
                .keys()
                .copied()
                .find(|p| !rasters.contains_key(p))
                .unwrap_or(0);
            return Err(redaction_page_missing(missing));
        }

        let mut doc = Document::load(path).map_err(|e| {
            AppError::engine_failed(format!("Could not read the PDF: {e}"))
        })?;
        let pages = doc.get_pages();
        let mut redacted_ids = HashSet::new();
        for (page_index, (w, h, rgb)) in rasters {
            let page_no = page_index + 1;
            let Some(&page_id) = pages.get(&page_no) else {
                return Err(redaction_page_missing(page_index));
            };
            let media = crop::media_box(&doc, page_id);
            install_page_image(&mut doc, page_id, w, h, &rgb, media)?;
            redacted_ids.insert(page_id);
        }
        detach_inherited_resources(&mut doc, &redacted_ids);
        sweep_unreferenced(&mut doc);
        save_in_place(&mut doc, path)
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Page-content probe still in dest streams → `Err` (fail closed).
/// Empty probes + leftover intersecting `/Annots` `/Contents` or field `/V`
/// → `Ok(warnings)`; the leftover string may remain.
pub fn verify_redaction(
    dest: &Path,
    page_content_probes: &[&[u8]],
    regions: &[RedactRegion],
) -> Result<Vec<String>, AppError> {
    if !dest.is_file() {
        return Err(AppError::invalid_pdf(&dest.to_string_lossy()));
    }
    let mut doc = Document::load(dest).map_err(|e| {
        AppError::engine_failed(format!("Could not read the PDF: {e}"))
    })?;
    let _ = doc.decompress();

    let pages = doc.get_pages();
    for region in regions {
        if pages.get(&(region.page_index + 1)).is_none() {
            return Err(redaction_page_missing(region.page_index));
        }
    }

    if !page_content_probes.is_empty() {
        if probe_remains_on_redacted_pages(&doc, page_content_probes, regions) {
            return Err(AppError::new(
                "REDACTION_INCOMPLETE",
                "Redaction could not be verified",
                "Page content that should have been removed is still in the PDF. The file was not saved.",
            ));
        }
    }

    let mut warnings = Vec::new();
    warn_intersecting_annots(&doc, regions, &mut warnings);
    warn_intersecting_fields(&doc, regions, &mut warnings);
    warn_leftover_thumbs_and_struct(&doc, regions, &mut warnings);
    if has_attachments(&doc) {
        warnings.push(
            "This PDF has attachments that were not removed. They may still hold sensitive data."
                .into(),
        );
    }
    Ok(warnings)
}

fn save_in_place(doc: &mut Document, path: &Path) -> Result<(), AppError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".offpdf-redact-write-{}-{}.pdf.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    doc.save(&tmp)
        .map_err(|e| AppError::io("Could not write the redacted PDF.", e))?;
    if let Err(e) = safe_output::replace_file(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn redaction_page_missing(page_index: u32) -> AppError {
    AppError::new(
        "REDACTION_INCOMPLETE",
        "Redaction could not be verified",
        format!(
            "A redaction region targets page {} which is not in the PDF. The file was not saved.",
            page_index + 1
        ),
    )
}

/// Copy inherited `/Resources` onto unredacted siblings, then drop Pages-level
/// `/Resources` so Form/Image XObjects used only by redacted pages become
/// unreachable and can be swept.
fn detach_inherited_resources(doc: &mut Document, redacted: &HashSet<ObjectId>) {
    let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let mut copies: Vec<(ObjectId, Object)> = Vec::new();
    for page_id in &page_ids {
        if redacted.contains(page_id) {
            continue;
        }
        let has_own = doc
            .get_dictionary(*page_id)
            .ok()
            .is_some_and(|p| p.get(b"Resources").is_ok());
        if has_own {
            continue;
        }
        if let Some(res) = inherited_resources_clone(doc, *page_id) {
            copies.push((*page_id, res));
        }
    }
    for (page_id, res) in copies {
        if let Ok(page) = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut()) {
            if page.get(b"Resources").is_err() {
                page.set("Resources", res);
            }
        }
    }
    for id in collect_pages_tree_nodes(doc) {
        if let Ok(dict) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
            dict.remove(b"Resources");
        }
    }
}

fn inherited_resources_clone(doc: &Document, page_id: ObjectId) -> Option<Object> {
    let mut seen = HashSet::new();
    let mut current = page_id;
    loop {
        let dict = doc.get_dictionary(current).ok()?;
        let parent = dict.get(b"Parent").ok()?.as_reference().ok()?;
        if !seen.insert(parent) {
            return None;
        }
        let parent_dict = doc.get_dictionary(parent).ok()?;
        if let Ok(res) = parent_dict.get(b"Resources") {
            return Some(res.clone());
        }
        current = parent;
    }
}

fn collect_pages_tree_nodes(doc: &Document) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return out;
    };
    let Ok(catalog) = doc.get_dictionary(root) else {
        return out;
    };
    let Ok(pages_id) = catalog.get(b"Pages").and_then(Object::as_reference) else {
        return out;
    };
    let mut stack = vec![pages_id];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Ok(dict) = doc.get_dictionary(id) else {
            continue;
        };
        let typ = dict.get(b"Type").ok().and_then(|o| o.as_name().ok());
        if typ == Some(b"Page") {
            continue;
        }
        out.push(id);
        if let Ok(Object::Array(kids)) = dict.get(b"Kids") {
            for kid in kids {
                if let Ok(kid_id) = kid.as_reference() {
                    stack.push(kid_id);
                }
            }
        }
    }
    out
}

fn renderer_missing() -> AppError {
    AppError::new(
        "RENDERER_MISSING",
        "Page renderer not found",
        "Redaction needs pdftoppm (poppler) to rasterize the page. The file was not saved.",
    )
    .with_suggestion("Install poppler, or reinstall OffPDF so the bundled renderer is available.")
}

fn raster_failed(details: impl Into<String>) -> AppError {
    AppError::engine_failed(details).with_suggestion(
        "Redaction could not rasterize the page. The file was not saved.",
    )
}

/// Locate `pdftoppm` the same way preview/Compress do when no AppHandle exists.
fn resolve_pdftoppm_standalone() -> PathBuf {
    let exe = if cfg!(windows) {
        "pdftoppm.exe"
    } else {
        "pdftoppm"
    };
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            for candidate in [
                parent.join("binaries").join(exe),
                parent.join("resources").join("binaries").join(exe),
                parent.join("binaries").join("binaries").join(exe),
            ] {
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        for candidate in [
            "/opt/homebrew/bin/pdftoppm",
            "/usr/local/bin/pdftoppm",
            "/opt/local/bin/pdftoppm",
            "/usr/bin/pdftoppm",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from(exe)
}

fn ensure_pdftoppm(exe: &Path) -> Result<(), AppError> {
    let mut cmd = Command::new(exe);
    render::configure_poppler_command(&mut cmd, exe);
    cmd.arg("-v").stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    match cmd.status() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(renderer_missing()),
        Err(e) => Err(raster_failed(e.to_string())),
        Ok(_) => Ok(()),
    }
}

fn expected_raster_px(media: [f64; 4], user_unit: f64) -> (u32, u32) {
    let uu = if user_unit.is_finite() && user_unit > 0.0 {
        user_unit
    } else {
        1.0
    };
    let w = ((media[2] - media[0]).abs() * uu * (RASTER_DPI as f64) / 72.0)
        .round()
        .clamp(1.0, 8192.0) as u32;
    let h = ((media[3] - media[1]).abs() * uu * (RASTER_DPI as f64) / 72.0)
        .round()
        .clamp(1.0, 8192.0) as u32;
    (w, h)
}

fn write_unrotated_copy(src: &Path, dest: &Path) -> Result<(), AppError> {
    let mut doc = Document::load(src).map_err(|e| {
        AppError::engine_failed(format!("Could not read the PDF: {e}"))
    })?;
    let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for page_id in page_ids {
        if let Ok(page) = doc.get_object_mut(page_id).and_then(|o| o.as_dict_mut()) {
            page.set("Rotate", 0);
        }
    }
    doc.save(dest)
        .map_err(|e| AppError::io("Could not write an unrotated render copy.", e))?;
    Ok(())
}

fn raster_page(
    exe: &Path,
    pdf: &Path,
    page_no: u32,
    media: [f64; 4],
    user_unit: f64,
    work: &Path,
) -> Result<(u32, u32, Vec<u8>), AppError> {
    let expected = expected_raster_px(media, user_unit);
    let media_only = expected_raster_px(media, 1.0);
    // Scale -r so a UserUnit-unaware pdftoppm still emits the UU-aware size.
    let dpi = ((RASTER_DPI as f64) * if user_unit.is_finite() && user_unit > 0.0 {
        user_unit
    } else {
        1.0
    })
    .round()
    .clamp(1.0, 2400.0) as u32;
    let (w, h, rgb) = raster_with_pdftoppm(exe, pdf, page_no, work, dpi)?;
    if (w, h) == expected {
        return Ok((w, h, rgb));
    }
    // UU-aware pdftoppm already multiplied UserUnit; -r * UU then double-counts.
    if (user_unit - 1.0).abs() > 1e-9 {
        let retry = raster_with_pdftoppm(exe, pdf, page_no, work, RASTER_DPI)?;
        if (retry.0, retry.1) == expected || (retry.0, retry.1) == media_only {
            return Ok(retry);
        }
        if (w, h) == media_only {
            return Ok((w, h, rgb));
        }
    }
    Err(raster_failed(format!(
        "Raster size {w}×{h} does not match unrotated MediaBox {}×{} at {RASTER_DPI} DPI (UserUnit {user_unit}).",
        expected.0, expected.1
    )))
}

fn raster_with_pdftoppm(
    exe: &Path,
    pdf: &Path,
    page_no: u32,
    work: &Path,
    dpi: u32,
) -> Result<(u32, u32, Vec<u8>), AppError> {
    let prefix = work.join(format!("p{page_no}-{dpi}"));
    let mut cmd = Command::new(exe);
    render::configure_poppler_command(&mut cmd, exe);
    cmd.arg("-png")
        .arg("-r")
        .arg(dpi.to_string())
        .arg("-f")
        .arg(page_no.to_string())
        .arg("-l")
        .arg(page_no.to_string())
        .arg("-singlefile")
        .arg(pdf)
        .arg(&prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            renderer_missing()
        } else {
            raster_failed(e.to_string())
        }
    })?;
    let png = prefix.with_extension("png");
    if !out.status.success() || !png.exists() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(raster_failed(if stderr.trim().is_empty() {
            format!("pdftoppm failed to rasterize page {page_no}.")
        } else {
            stderr.trim().to_string()
        }));
    }
    let img = image::open(&png)
        .map_err(|e| raster_failed(format!("Could not read the rasterized page: {e}")))?
        .to_rgb8();
    let _ = std::fs::remove_file(&png);
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(raster_failed(format!(
            "pdftoppm produced an empty raster for page {page_no}."
        )));
    }
    Ok((w, h, img.into_raw()))
}

fn install_page_image(
    doc: &mut Document,
    page_id: ObjectId,
    w: u32,
    h: u32,
    rgb: &[u8],
    media: [f64; 4],
) -> Result<(), AppError> {
    let mut img_dict = Dictionary::new();
    img_dict.set("Type", "XObject");
    img_dict.set("Subtype", "Image");
    img_dict.set("Width", w as i64);
    img_dict.set("Height", h as i64);
    img_dict.set("ColorSpace", "DeviceRGB");
    img_dict.set("BitsPerComponent", 8);
    let mut img_stream = Stream::new(img_dict, rgb.to_vec());
    let _ = img_stream.compress();
    let img_id = doc.add_object(Object::Stream(img_stream));

    let pw = media[2] - media[0];
    let ph = media[3] - media[1];
    let ops = format!(
        "q\n{pw:.4} 0 0 {ph:.4} {:.4} {:.4} cm\n/ImR Do\nQ\n",
        media[0], media[1]
    );
    let content_id = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        ops.into_bytes(),
    )));

    let mut xobjects = Dictionary::new();
    xobjects.set("ImR", Object::Reference(img_id));
    let mut resources = Dictionary::new();
    resources.set("XObject", Object::Dictionary(xobjects));

    let page = doc
        .get_object_mut(page_id)
        .and_then(|o| o.as_dict_mut())
        .map_err(|e| AppError::engine_failed(format!("Could not update the page: {e}")))?;
    page.set("Contents", content_id);
    page.set("Resources", Object::Dictionary(resources));
    Ok(())
}

fn sweep_unreferenced(doc: &mut Document) {
    let mut reachable: HashSet<ObjectId> = HashSet::new();
    let mut stack: Vec<ObjectId> = Vec::new();
    push_refs(doc.trailer.get(b"Root").ok(), &mut reachable, &mut stack);
    push_refs(doc.trailer.get(b"Info").ok(), &mut reachable, &mut stack);
    push_refs(doc.trailer.get(b"Encrypt").ok(), &mut reachable, &mut stack);
    while let Some(id) = stack.pop() {
        if let Ok(obj) = doc.get_object(id) {
            collect_refs(obj, &mut reachable, &mut stack);
        }
    }
    doc.objects.retain(|id, _| reachable.contains(id));
}

fn push_refs(obj: Option<&Object>, reachable: &mut HashSet<ObjectId>, stack: &mut Vec<ObjectId>) {
    if let Some(obj) = obj {
        collect_refs(obj, reachable, stack);
    }
}

fn collect_refs(obj: &Object, reachable: &mut HashSet<ObjectId>, stack: &mut Vec<ObjectId>) {
    match obj {
        Object::Reference(id) => {
            if reachable.insert(*id) {
                stack.push(*id);
            }
        }
        Object::Array(items) => {
            for item in items {
                collect_refs(item, reachable, stack);
            }
        }
        Object::Dictionary(dict) => {
            for (_, value) in dict.iter() {
                collect_refs(value, reachable, stack);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter() {
                collect_refs(value, reachable, stack);
            }
        }
        _ => {}
    }
}

fn unique_redact_pages(regions: &[RedactRegion]) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut pages = Vec::new();
    for region in regions {
        if seen.insert(region.page_index) {
            pages.push(region.page_index);
        }
    }
    pages
}

fn haystack_contains(haystack: &[u8], probe: &[u8]) -> bool {
    !probe.is_empty() && haystack.windows(probe.len()).any(|w| w == probe)
}

/// Search each probe only in dest streams of the redacted page it belongs to.
/// Unredacted siblings are not scanned (identical leftover text must not fail-close).
fn probe_remains_on_redacted_pages(
    doc: &Document,
    probes: &[&[u8]],
    regions: &[RedactRegion],
) -> bool {
    let pages = unique_redact_pages(regions);
    if pages.is_empty() {
        let haystack = page_content_haystack(doc);
        return probes.iter().any(|p| haystack_contains(&haystack, p));
    }
    if probes.len() == pages.len() {
        return probes.iter().zip(pages.iter()).any(|(probe, &page_index)| {
            haystack_contains(&page_content_haystack_for(doc, page_index), probe)
        });
    }
    let haystacks: Vec<Vec<u8>> = pages
        .iter()
        .map(|&page_index| page_content_haystack_for(doc, page_index))
        .collect();
    probes
        .iter()
        .any(|probe| haystacks.iter().any(|h| haystack_contains(h, probe)))
}

fn page_content_haystack(doc: &Document) -> Vec<u8> {
    let mut out = Vec::new();
    for &page_id in doc.get_pages().values() {
        append_page_streams(doc, page_id, &mut out);
    }
    out
}

fn page_content_haystack_for(doc: &Document, page_index: u32) -> Vec<u8> {
    let Some(&page_id) = doc.get_pages().get(&(page_index + 1)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    append_page_streams(doc, page_id, &mut out);
    out
}

fn append_page_streams(doc: &Document, page_id: ObjectId, out: &mut Vec<u8>) {
    if let Ok(bytes) = doc.get_page_content(page_id) {
        out.extend_from_slice(&bytes);
        out.push(b'\n');
    }
    for id in doc.get_page_contents(page_id) {
        if let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) {
            out.extend_from_slice(&plain_stream(stream));
            out.push(b'\n');
        }
    }
    append_page_xobjects(doc, page_id, out);
}

fn append_page_xobjects(doc: &Document, page_id: ObjectId, out: &mut Vec<u8>) {
    let mut seen = HashSet::new();
    for resources in ancestor_resource_dicts(doc, page_id) {
        append_xobject_streams(doc, resources, out, &mut seen);
    }
}

fn ancestor_resource_dicts<'a>(doc: &'a Document, start: ObjectId) -> Vec<&'a Dictionary> {
    let mut dicts = Vec::new();
    let mut current = Some(start);
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        let Ok(node) = doc.get_dictionary(id) else {
            break;
        };
        if let Ok(res) = node.get(b"Resources") {
            if let Some(d) = dict_from(doc, res) {
                dicts.push(d);
            }
        }
        current = node.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    dicts
}

fn append_xobject_streams(
    doc: &Document,
    resources: &Dictionary,
    out: &mut Vec<u8>,
    seen: &mut HashSet<ObjectId>,
) {
    let Ok(xo) = resources.get(b"XObject") else {
        return;
    };
    let Some(xobjects) = dict_from(doc, xo) else {
        return;
    };
    for (_, obj) in xobjects.iter() {
        let Ok(id) = obj.as_reference() else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        out.extend_from_slice(&plain_stream(stream));
        out.push(b'\n');
        if stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name().ok())
            == Some(b"Form")
        {
            if let Ok(res) = stream.dict.get(b"Resources") {
                if let Some(d) = dict_from(doc, res) {
                    append_xobject_streams(doc, d, out, seen);
                }
            }
        }
    }
}

/// Page Contents plus decoded Form XObject streams (page-local and inherited).
pub(crate) fn collect_redact_probes_for_pages(
    doc: &Document,
    regions: &[RedactRegion],
) -> Result<Vec<Vec<u8>>, AppError> {
    let pages = doc.get_pages();
    let mut seen = HashSet::new();
    let mut probes = Vec::new();
    for region in regions {
        if !seen.insert(region.page_index) {
            continue;
        }
        let Some(&id) = pages.get(&(region.page_index + 1)) else {
            return Err(redaction_page_missing(region.page_index));
        };
        if let Ok(bytes) = doc.get_page_content(id) {
            if !bytes.is_empty() {
                probes.push(bytes);
            }
        }
        collect_form_xobject_probes(doc, id, &mut probes);
    }
    Ok(probes)
}

fn collect_form_xobject_probes(doc: &Document, page_id: ObjectId, probes: &mut Vec<Vec<u8>>) {
    let mut seen = HashSet::new();
    for resources in ancestor_resource_dicts(doc, page_id) {
        collect_form_xobjects_from_dict(doc, resources, probes, &mut seen);
    }
}

fn collect_form_xobjects_from_dict(
    doc: &Document,
    resources: &Dictionary,
    probes: &mut Vec<Vec<u8>>,
    seen: &mut HashSet<ObjectId>,
) {
    let Ok(xo) = resources.get(b"XObject") else {
        return;
    };
    let Some(xobjects) = dict_from(doc, xo) else {
        return;
    };
    for (_, obj) in xobjects.iter() {
        let Ok(id) = obj.as_reference() else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) else {
            continue;
        };
        let subtype = stream.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok());
        if subtype != Some(b"Form") {
            continue;
        }
        let bytes = plain_stream(stream);
        if !bytes.is_empty() {
            probes.push(bytes);
        }
        if let Ok(res) = stream.dict.get(b"Resources") {
            if let Some(d) = dict_from(doc, res) {
                collect_form_xobjects_from_dict(doc, d, probes, seen);
            }
        }
    }
}

fn dict_from<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn plain_stream(stream: &Stream) -> Vec<u8> {
    stream
        .get_plain_content()
        .or_else(|_| stream.decompressed_content())
        .unwrap_or_else(|_| stream.content.clone())
}

fn warn_intersecting_annots(doc: &Document, regions: &[RedactRegion], warnings: &mut Vec<String>) {
    for (page_no, page_id) in doc.get_pages() {
        let page_index = page_no.saturating_sub(1);
        let page_regions: Vec<&RedactRegion> = regions
            .iter()
            .filter(|r| r.page_index == page_index)
            .collect();
        if page_regions.is_empty() {
            continue;
        }
        for annot_id in page_annot_ids(doc, page_id) {
            let Ok(annot) = doc.get_dictionary(annot_id) else {
                continue;
            };
            let Some(contents) = dict_string(doc, annot, b"Contents") else {
                continue;
            };
            if contents.trim().is_empty() {
                continue;
            }
            let Some(rect) = dict_rect(doc, annot, b"Rect") else {
                continue;
            };
            if page_regions.iter().any(|r| rects_intersect(rect, &r.rect)) {
                warnings.push(format!(
                    "An annotation on page {page_no} still contains text that intersects a redaction region."
                ));
            }
        }
    }
}

fn warn_intersecting_fields(doc: &Document, regions: &[RedactRegion], warnings: &mut Vec<String>) {
    let mut warned: HashSet<ObjectId> = HashSet::new();
    for (page_no, page_id) in doc.get_pages() {
        let page_index = page_no.saturating_sub(1);
        let page_regions: Vec<&RedactRegion> = regions
            .iter()
            .filter(|r| r.page_index == page_index)
            .collect();
        if page_regions.is_empty() {
            continue;
        }
        for annot_id in page_annot_ids(doc, page_id) {
            let Ok(annot) = doc.get_dictionary(annot_id) else {
                continue;
            };
            if !is_widget(annot) {
                continue;
            }
            let Some(value) = field_value(doc, annot_id) else {
                continue;
            };
            if value.trim().is_empty() {
                continue;
            }
            let Some(rect) = dict_rect(doc, annot, b"Rect") else {
                continue;
            };
            if page_regions.iter().any(|r| rects_intersect(rect, &r.rect)) {
                warned.insert(annot_id);
                warnings.push(format!(
                    "A form field on page {page_no} still holds a value that intersects a redaction region."
                ));
            }
        }
    }
    // After flatten-before-burn the widget is gone from /Annots; leftover /V
    // still lives on catalog /AcroForm /Fields (and parents).
    for field_id in acroform_field_ids(doc) {
        if !warned.insert(field_id) {
            continue;
        }
        let Some(value) = field_value(doc, field_id) else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        if let Some(page_no) = field_intersects_regions(doc, field_id, regions) {
            warnings.push(format!(
                "A form field on page {page_no} still holds a value that intersects a redaction region."
            ));
        }
    }
}

fn acroform_field_ids(doc: &Document) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return out;
    };
    let Ok(catalog) = doc.get_dictionary(root) else {
        return out;
    };
    let Ok(acro_obj) = catalog.get(b"AcroForm") else {
        return out;
    };
    let Some(acro) = dict_from(doc, acro_obj) else {
        return out;
    };
    let Ok(fields) = acro.get(b"Fields") else {
        return out;
    };
    let resolved = match fields {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    };
    let Some(Object::Array(items)) = resolved else {
        return out;
    };
    let mut stack: Vec<ObjectId> = items.iter().filter_map(|o| o.as_reference().ok()).collect();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        if let Ok(dict) = doc.get_dictionary(id) {
            if let Ok(Object::Array(kids)) = dict.get(b"Kids") {
                for kid in kids {
                    if let Ok(kid_id) = kid.as_reference() {
                        stack.push(kid_id);
                    }
                }
            }
        }
    }
    out
}

fn field_intersects_regions(
    doc: &Document,
    field_id: ObjectId,
    regions: &[RedactRegion],
) -> Option<u32> {
    let mut rects = Vec::new();
    collect_field_widget_rects(doc, field_id, &mut rects, &mut HashSet::new());
    for (page_index, rect) in rects {
        for region in regions {
            if page_index != u32::MAX && region.page_index != page_index {
                continue;
            }
            if rects_intersect(rect, &region.rect) {
                return Some(if page_index == u32::MAX {
                    region.page_index + 1
                } else {
                    page_index + 1
                });
            }
        }
    }
    None
}

fn collect_field_widget_rects(
    doc: &Document,
    id: ObjectId,
    out: &mut Vec<(u32, [f64; 4])>,
    seen: &mut HashSet<ObjectId>,
) {
    if !seen.insert(id) {
        return;
    }
    let Ok(dict) = doc.get_dictionary(id) else {
        return;
    };
    if let Some(rect) = dict_rect(doc, dict, b"Rect") {
        let page_index = dict
            .get(b"P")
            .ok()
            .and_then(|o| o.as_reference().ok())
            .and_then(|pid| page_index_of(doc, pid))
            .unwrap_or(u32::MAX);
        out.push((page_index, rect));
    }
    if let Ok(Object::Array(kids)) = dict.get(b"Kids") {
        for kid in kids {
            if let Ok(kid_id) = kid.as_reference() {
                collect_field_widget_rects(doc, kid_id, out, seen);
            }
        }
    }
}

fn page_index_of(doc: &Document, page_id: ObjectId) -> Option<u32> {
    doc.get_pages()
        .iter()
        .find(|(_, &id)| id == page_id)
        .map(|(&page_no, _)| page_no.saturating_sub(1))
}

fn is_widget(annot: &Dictionary) -> bool {
    if annot
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .is_some_and(|n| n == b"Widget")
    {
        return true;
    }
    annot.get(b"FT").is_ok() || annot.get(b"Parent").is_ok()
}

fn field_value(doc: &Document, mut id: ObjectId) -> Option<String> {
    for _ in 0..16 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Some(v) = dict_string(doc, dict, b"V") {
            return Some(v);
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn page_annot_ids(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Ok(annots) = page.get(b"Annots") else {
        return Vec::new();
    };
    let resolved = match annots {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    };
    match resolved {
        Some(Object::Array(items)) => items.iter().filter_map(|o| o.as_reference().ok()).collect(),
        Some(Object::Reference(id)) => vec![*id],
        _ => Vec::new(),
    }
}

fn warn_leftover_thumbs_and_struct(
    doc: &Document,
    regions: &[RedactRegion],
    warnings: &mut Vec<String>,
) {
    let pages = doc.get_pages();
    let mut thumb = false;
    let mut seen = HashSet::new();
    for region in regions {
        if !seen.insert(region.page_index) {
            continue;
        }
        let Some(&page_id) = pages.get(&(region.page_index + 1)) else {
            continue;
        };
        let Ok(page) = doc.get_dictionary(page_id) else {
            continue;
        };
        if page.get(b"Thumb").is_ok() {
            thumb = true;
            break;
        }
    }
    if thumb {
        warnings.push(
            "A redacted page still has a thumbnail (/Thumb) that was not removed. It may still hold sensitive data."
                .into(),
        );
    }
    if has_struct_tree_root(doc) {
        warnings.push(
            "This PDF has a structure tree (/StructTreeRoot) that was not removed. It may still hold sensitive data."
                .into(),
        );
    }
}

fn has_struct_tree_root(doc: &Document) -> bool {
    let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return false;
    };
    let Ok(catalog) = doc.get_dictionary(root) else {
        return false;
    };
    catalog.get(b"StructTreeRoot").is_ok()
}

fn has_attachments(doc: &Document) -> bool {
    if catalog_has_attachments(doc) {
        return true;
    }
    for (_, page_id) in doc.get_pages() {
        for annot_id in page_annot_ids(doc, page_id) {
            let Ok(annot) = doc.get_dictionary(annot_id) else {
                continue;
            };
            if is_file_attachment(doc, annot) {
                return true;
            }
        }
    }
    false
}

fn catalog_has_attachments(doc: &Document) -> bool {
    let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return false;
    };
    let Ok(catalog) = doc.get_dictionary(root) else {
        return false;
    };
    if catalog.get(b"AF").is_ok() {
        return true;
    }
    let Ok(names) = catalog.get(b"Names") else {
        return false;
    };
    let Some(names) = dict_from(doc, names) else {
        return false;
    };
    names.get(b"EmbeddedFiles").is_ok()
}

fn is_file_attachment(doc: &Document, annot: &Dictionary) -> bool {
    if annot
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        .is_some_and(|n| n == b"FileAttachment")
    {
        return true;
    }
    let Ok(fs_obj) = annot.get(b"FS") else {
        return false;
    };
    let Some(fs) = dict_from(doc, fs_obj) else {
        return false;
    };
    fs.get(b"EF").is_ok()
}

fn dict_string(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    let obj = resolve_obj(doc, dict.get(key).ok()?)?;
    match obj {
        Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

fn dict_rect(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<[f64; 4]> {
    let obj = resolve_obj(doc, dict.get(key).ok()?)?;
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0.0; 4];
    for (i, item) in arr.iter().enumerate() {
        v[i] = num(doc, item)?;
    }
    Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

fn resolve_obj<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn num(doc: &Document, obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| num(doc, o)),
        _ => None,
    }
}

fn rects_intersect(annot: [f64; 4], region: &PdfRectIn) -> bool {
    let rx0 = region.x;
    let ry0 = region.y;
    let rx1 = region.x + region.w;
    let ry1 = region.y + region.h;
    annot[0] < rx1 && annot[2] > rx0 && annot[1] < ry1 && annot[3] > ry0
}

fn parse_fill(fill: Option<&str>) -> [u8; 3] {
    let Some(raw) = fill else {
        return [0, 0, 0];
    };
    let t = raw.trim().trim_start_matches('#');
    if t.len() == 6 {
        if let Ok(n) = u32::from_str_radix(t, 16) {
            return [
                ((n >> 16) & 255) as u8,
                ((n >> 8) & 255) as u8,
                (n & 255) as u8,
            ];
        }
    }
    if t.len() == 3 {
        if let Ok(n) = u32::from_str_radix(t, 16) {
            let r = ((n >> 8) & 0xf) as u8;
            let g = ((n >> 4) & 0xf) as u8;
            let b = (n & 0xf) as u8;
            return [r * 17, g * 17, b * 17];
        }
    }
    [0, 0, 0]
}

fn fill_pdf_rect(
    rgb: &mut [u8],
    iw: u32,
    ih: u32,
    media: [f64; 4],
    user_unit: f64,
    rect: &PdfRectIn,
    color: [u8; 3],
) {
    let (x0, y0, x1, y1) = pdf_rect_to_pixels(iw, ih, media, user_unit, rect);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y as u32 * iw + x as u32) * 3) as usize;
            if i + 2 < rgb.len() {
                rgb[i] = color[0];
                rgb[i + 1] = color[1];
                rgb[i + 2] = color[2];
            }
        }
    }
}

fn pdf_rect_to_pixels(
    iw: u32,
    ih: u32,
    media: [f64; 4],
    user_unit: f64,
    rect: &PdfRectIn,
) -> (i32, i32, i32, i32) {
    // Map in UserUnit-aware expected pixels, then scale to the actual raster
    // (some pdftoppm builds ignore /UserUnit and emit MediaBox-only size).
    let (ew, eh) = expected_raster_px(media, user_unit);
    let sx = iw as f64 / ew as f64;
    let sy = ih as f64 / eh as f64;
    let mw = (media[2] - media[0]).abs().max(1.0);
    let mh = (media[3] - media[1]).abs().max(1.0);
    let x0 = ((rect.x.min(rect.x + rect.w) - media[0]) / mw * ew as f64 * sx).floor() as i32;
    let x1 = ((rect.x.max(rect.x + rect.w) - media[0]) / mw * ew as f64 * sx).ceil() as i32;
    let top = rect.y.max(rect.y + rect.h);
    let bot = rect.y.min(rect.y + rect.h);
    let y0 = ((media[3] - top) / mh * eh as f64 * sy).floor() as i32;
    let y1 = ((media[3] - bot) / mh * eh as f64 * sy).ceil() as i32;
    (
        x0.clamp(0, iw as i32),
        y0.clamp(0, ih as i32),
        x1.clamp(0, iw as i32),
        y1.clamp(0, ih as i32),
    )
}

fn draw_label(
    rgb: &mut [u8],
    iw: u32,
    ih: u32,
    media: [f64; 4],
    user_unit: f64,
    rect: &PdfRectIn,
    label: &str,
) {
    let (x0, y0, x1, y1) = pdf_rect_to_pixels(iw, ih, media, user_unit, rect);
    if x1 - x0 < 8 || y1 - y0 < 8 {
        return;
    }
    let ink = contrast_ink(parse_fill(None), rgb, iw, x0, y0);
    let scale = ((y1 - y0 - 6) / 7).clamp(1, 4);
    let mut x = x0 + 3;
    let y = y0 + ((y1 - y0 - 7 * scale) / 2).max(2);
    for ch in label.chars() {
        if x + 6 * scale >= x1 {
            break;
        }
        blit_glyph(rgb, iw, x, y, scale, ch, ink);
        x += 6 * scale;
    }
}

fn contrast_ink(fill: [u8; 3], rgb: &[u8], iw: u32, x: i32, y: i32) -> [u8; 3] {
    let i = ((y.max(0) as u32 * iw + x.max(0) as u32) * 3) as usize;
    let sample = if i + 2 < rgb.len() {
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    } else {
        fill
    };
    let luma = (sample[0] as u32 * 3 + sample[1] as u32 * 6 + sample[2] as u32) / 10;
    if luma > 128 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    }
}

fn blit_glyph(rgb: &mut [u8], iw: u32, x: i32, y: i32, scale: i32, ch: char, ink: [u8; 3]) {
    let bits = glyph5x7(ch);
    for row in 0..7 {
        for col in 0..5 {
            if (bits[row] >> (4 - col)) & 1 == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let px = x + col * scale + dx;
                    let py = y + row as i32 * scale + dy;
                    if px < 0 || py < 0 {
                        continue;
                    }
                    let i = ((py as u32 * iw + px as u32) * 3) as usize;
                    if i + 2 < rgb.len() {
                        rgb[i] = ink[0];
                        rgb[i + 1] = ink[1];
                        rgb[i + 2] = ink[2];
                    }
                }
            }
        }
    }
}

fn glyph5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0E],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0E, 0x11, 0x10, 0x0E, 0x01, 0x11, 0x0E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x0E, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x04, 0x00, 0x04, 0x00, 0x00],
    }
}

#[cfg(test)]
mod r2_user_unit_gate {
    use super::*;

    /// R-UU: the pdftoppm size gate must use page `/UserUnit`.
    /// Letter MediaBox × UserUnit 2 at 72 DPI is 1224×1584. Today
    /// `expected_raster_px` sees only MediaBox, so a UU-aware pdftoppm
    /// 2× PNG fails the gate (`raster_failed`).
    #[test]
    fn expected_raster_px_includes_user_unit_2() {
        let media = [0.0, 0.0, 612.0, 792.0];
        let user_unit = 2.0;
        let got = expected_raster_px(media, user_unit);
        let want_w = ((media[2] - media[0]).abs() * user_unit * (RASTER_DPI as f64) / 72.0)
            .round()
            .clamp(1.0, 8192.0) as u32;
        let want_h = ((media[3] - media[1]).abs() * user_unit * (RASTER_DPI as f64) / 72.0)
            .round()
            .clamp(1.0, 8192.0) as u32;
        assert_eq!(
            got,
            (want_w, want_h),
            "R-UU: expected_raster_px must multiply MediaBox by /UserUnit (want {want_w}×{want_h} for UU=2, got {}×{})",
            got.0,
            got.1
        );
    }
}
