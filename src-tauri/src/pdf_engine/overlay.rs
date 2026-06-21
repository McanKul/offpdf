//! Page-number stamping. Builds a tiny text-only overlay PDF (standard
//! Helvetica font, no embedding) with one numbered page per document page, then
//! stamps it onto the assembled document with `qpdf --overlay`.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::pdf_engine::qpdf;
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use std::sync::Arc;

const PAGE_W: f64 = 612.0;
const PAGE_H: f64 = 792.0;
const FS: f64 = 12.0;

/// Build an N-page overlay PDF, each page showing its number at `position`.
fn build_overlay(out_path: &str, count: u32, start: i64, position: &str) -> Result<(), AppError> {
    let total_objs = 3 + (count as usize) * 2; // catalog, pages, font, (content,page)*N
    let mut buf: Vec<u8> = Vec::new();
    let mut off = vec![0usize; total_objs + 1];

    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");

    off[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let kids: String = (0..count).map(|i| format!("{} 0 R", 5 + i * 2)).collect::<Vec<_>>().join(" ");
    off[2] = buf.len();
    buf.extend_from_slice(format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {count} >>\nendobj\n").as_bytes());

    off[3] = buf.len();
    buf.extend_from_slice(b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");

    for i in 0..count {
        let num = (start + i as i64).to_string();
        let text_w = num.len() as f64 * 0.56 * FS; // approx Helvetica digit width
        let (x, y) = match position {
            "bottom-right" => (PAGE_W - 40.0 - text_w, 28.0),
            "bottom-left" => (40.0, 28.0),
            "top-right" => (PAGE_W - 40.0 - text_w, PAGE_H - 40.0),
            "top-center" => ((PAGE_W - text_w) / 2.0, PAGE_H - 40.0),
            "top-left" => (40.0, PAGE_H - 40.0),
            _ => ((PAGE_W - text_w) / 2.0, 28.0), // bottom-center
        };
        let content = format!("BT\n/F1 {FS:.0} Tf\n{x:.1} {y:.1} Td\n({num}) Tj\nET\n");
        let content_obj = 4 + i * 2;
        let page_obj = 5 + i * 2;

        off[content_obj as usize] = buf.len();
        buf.extend_from_slice(
            format!(
                "{content_obj} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );

        off[page_obj as usize] = buf.len();
        buf.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {content_obj} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );
    }

    let xref = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for n in 1..=total_objs {
        buf.extend_from_slice(format!("{:010} 00000 n \n", off[n]).as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", total_objs + 1).as_bytes(),
    );

    std::fs::write(out_path, &buf).map_err(|e| AppError::output_not_writable(&format!("{out_path} ({e})")))?;
    Ok(())
}

fn pdf_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('(', "\\(").replace(')', "\\)")
}

/// Build an N-page overlay where every page shows the same diagonal, semi-
/// transparent watermark text (standard Helvetica).
fn build_watermark(out_path: &str, count: u32, text: &str, opacity: f64) -> Result<(), AppError> {
    let safe = pdf_escape(text);
    let fs = 56.0f64;
    // Rough centering of the rotated (45°) text around the page center.
    let tw = text.chars().count() as f64 * 0.5 * fs;
    let c = std::f64::consts::FRAC_1_SQRT_2; // cos/sin 45°
    let tx = PAGE_W / 2.0 - (tw / 2.0) * c;
    let ty = PAGE_H / 2.0 - (tw / 2.0) * c;
    let op = opacity.clamp(0.05, 1.0);

    let total_objs = 4 + (count as usize) * 2; // catalog, pages, font, gstate, (content,page)*N
    let mut buf: Vec<u8> = Vec::new();
    let mut off = vec![0usize; total_objs + 1];

    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
    off[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let kids: String = (0..count).map(|i| format!("{} 0 R", 6 + i * 2)).collect::<Vec<_>>().join(" ");
    off[2] = buf.len();
    buf.extend_from_slice(format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {count} >>\nendobj\n").as_bytes());
    off[3] = buf.len();
    buf.extend_from_slice(b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n");
    off[4] = buf.len();
    buf.extend_from_slice(format!("4 0 obj\n<< /Type /ExtGState /ca {op:.2} /CA {op:.2} >>\nendobj\n").as_bytes());

    for i in 0..count {
        let content = format!(
            "q\n/GS1 gs\n0.5 0.5 0.5 rg\nBT\n/F1 {fs:.0} Tf\n{c:.4} {c:.4} -{c:.4} {c:.4} {tx:.1} {ty:.1} Tm\n({safe}) Tj\nET\nQ\n"
        );
        let content_obj = 5 + i * 2;
        let page_obj = 6 + i * 2;
        off[content_obj as usize] = buf.len();
        buf.extend_from_slice(
            format!("{content_obj} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n", content.len()).as_bytes(),
        );
        off[page_obj as usize] = buf.len();
        buf.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
                 /Resources << /Font << /F1 3 0 R >> /ExtGState << /GS1 4 0 R >> >> /Contents {content_obj} 0 R >>\nendobj\n"
            )
            .as_bytes(),
        );
    }

    let xref = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for n in 1..=total_objs {
        buf.extend_from_slice(format!("{:010} 00000 n \n", off[n]).as_bytes());
    }
    buf.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", total_objs + 1).as_bytes(),
    );
    std::fs::write(out_path, &buf).map_err(|e| AppError::output_not_writable(&format!("{out_path} ({e})")))?;
    Ok(())
}

/// Assemble the combined document and stamp a diagonal watermark on every page.
pub fn add_watermark(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    text: &str,
    opacity: f64,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(AppError::new("NO_PAGES", "No pages", "Add a PDF first."));
    }
    if text.trim().is_empty() {
        return Err(AppError::new("NO_TEXT", "No watermark text", "Type the watermark text."));
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
        let n = qpdf::npages(app, &merged)?;
        build_watermark(&overlay, n, text, opacity)?;
        run_qpdf(
            app,
            handle,
            job_id,
            &[merged.clone(), "--overlay".into(), overlay.clone(), "--".into(), output.to_string()],
            "Adding watermark",
            None,
        )?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Assemble the combined document and stamp page numbers onto it.
pub fn add_page_numbers(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    position: &str,
    start: i64,
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
    let overlay = work.join("overlay.pdf").to_string_lossy().to_string();

    let result = (|| -> Result<Vec<String>, AppError> {
        super::assemble_groups(app, handle, job_id, groups, &merged, "Preparing", None)?;
        let n = qpdf::npages(app, &merged)?;
        build_overlay(&overlay, n, start, position)?;
        run_qpdf(
            app,
            handle,
            job_id,
            &[merged.clone(), "--overlay".into(), overlay.clone(), "--".into(), output.to_string()],
            "Adding page numbers",
            None,
        )?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}
