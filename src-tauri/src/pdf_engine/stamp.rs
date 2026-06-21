//! Text stamp / typed signature: place a line of text (name, date, "APPROVED",
//! …) at a chosen anchor on ONE page, in a chosen colour and size. Built as a
//! single-page text overlay sized to the target page, then stamped with
//! `qpdf --overlay --to=<page>` so it lands exactly (no scaling distortion).

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::pdf_engine::qpdf;
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use lopdf::{Document, Object, ObjectId};
use std::process::{Command, Stdio};
use std::sync::Arc;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

fn num(o: &Object, doc: &Document) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        Object::Reference(r) => doc.get_object(*r).ok().and_then(|x| num(x, doc)),
        _ => None,
    }
}

fn media_box(doc: &Document, page_id: ObjectId) -> (f64, f64) {
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
                    let x0 = num(&arr[0], doc).unwrap_or(0.0);
                    let y0 = num(&arr[1], doc).unwrap_or(0.0);
                    let x1 = num(&arr[2], doc).unwrap_or(612.0);
                    let y1 = num(&arr[3], doc).unwrap_or(792.0);
                    return ((x1 - x0).abs(), (y1 - y0).abs());
                }
            }
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    (612.0, 792.0)
}

/// Size of `page` (1-based) in points, by extracting it to a tiny PDF (safe even
/// for huge sources) and reading its MediaBox.
fn page_size(app: &tauri::AppHandle, merged: &str, page: u32, dir: &std::path::Path) -> (f64, f64) {
    let one = dir.join("one.pdf");
    let one_str = one.to_string_lossy().to_string();
    let mut cmd = Command::new(qpdf::resolve_qpdf(app));
    cmd.args(["--empty", "--pages", merged, &page.to_string(), "--", &one_str]);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let _ = cmd.status();
    if !one.exists() {
        return (612.0, 792.0);
    }
    match Document::load(&one) {
        Ok(doc) => doc
            .get_pages()
            .values()
            .next()
            .map(|id| media_box(&doc, *id))
            .unwrap_or((612.0, 792.0)),
        Err(_) => (612.0, 792.0),
    }
}

fn build_overlay(out: &str, w: f64, h: f64, text: &str, x: f64, y: f64, fs: f64, color: [f64; 3]) -> Result<(), AppError> {
    let content = format!(
        "{:.3} {:.3} {:.3} rg\nBT\n/F1 {fs:.1} Tf\n{x:.1} {y:.1} Td\n({}) Tj\nET\n",
        color[0], color[1], color[2], esc(text)
    );
    let mut buf: Vec<u8> = Vec::new();
    let mut off = [0usize; 6];
    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
    off[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    off[2] = buf.len();
    buf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [5 0 R] /Count 1 >>\nendobj\n");
    off[3] = buf.len();
    buf.extend_from_slice(b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
    off[4] = buf.len();
    buf.extend_from_slice(format!("4 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n", content.len()).as_bytes());
    off[5] = buf.len();
    buf.extend_from_slice(
        format!(
            "5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.1} {h:.1}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>\nendobj\n"
        )
        .as_bytes(),
    );
    let xref = buf.len();
    buf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for n in 1..=5 {
        buf.extend_from_slice(format!("{:010} 00000 n \n", off[n]).as_bytes());
    }
    buf.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    std::fs::write(out, &buf).map_err(|e| AppError::output_not_writable(&format!("{out} ({e})")))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn stamp_text(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    page: u32,
    anchor: &str,
    text: &str,
    color: [f64; 3],
    size_pct: f64,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    if text.trim().is_empty() {
        return Err(AppError::new("NO_TEXT", "No text", "Type the stamp text."));
    }
    for g in groups {
        super::require_input(&g.path)?;
    }
    super::ensure_output_dir(output)?;

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work).map_err(|e| AppError::io("Could not create a temp directory.", e))?;
    let merged = work.join("merged.pdf").to_string_lossy().to_string();
    let overlay = work.join("overlay.pdf").to_string_lossy().to_string();

    let result = (|| -> Result<Vec<String>, AppError> {
        super::assemble_groups(app, handle, job_id, groups, &merged, "Preparing", None)?;
        let total = qpdf::npages(app, &merged)?;
        let page = page.clamp(1, total.max(1));

        let (w, h) = page_size(app, &merged, page, &work);
        let fs = (size_pct.clamp(0.015, 0.2)) * h;
        let tw = text.chars().count() as f64 * 0.52 * fs;
        let mx = 0.04 * w;
        let my = 0.04 * h;

        let x = if anchor.ends_with("left") {
            mx
        } else if anchor.ends_with("right") {
            (w - tw - mx).max(mx)
        } else {
            ((w - tw) / 2.0).max(mx)
        };
        let y = if anchor.starts_with("top") {
            h - my - fs
        } else if anchor.starts_with("bottom") {
            my
        } else {
            (h - fs) / 2.0
        };

        build_overlay(&overlay, w, h, text, x, y, fs, color)?;
        run_qpdf(
            app,
            handle,
            job_id,
            &[merged.clone(), "--overlay".into(), overlay.clone(), format!("--to={page}"), "--".into(), output.to_string()],
            "Stamping",
            None,
        )?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}
