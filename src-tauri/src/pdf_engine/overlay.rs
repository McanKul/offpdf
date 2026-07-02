//! Page-number stamping. Builds a tiny text-only overlay PDF (standard
//! Helvetica font, no embedding) with one numbered page per document page, then
//! stamps it onto the assembled document with `qpdf --overlay`. Overlay pages
//! are sized to each destination page's DISPLAYED size (post-/Rotate) so qpdf
//! maps them 1:1 instead of scaling/centering a mismatched page.

use crate::error::AppError;
use crate::models::{JobHandle, PageGroup};
use crate::pdf_engine::{crop, qpdf};
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use lopdf::Document;
use std::sync::Arc;

const PAGE_W: f64 = 612.0;
const PAGE_H: f64 = 792.0;
const FS: f64 = 12.0;

/// Helvetica with WinAnsi text encoding, plus the six Turkish glyphs WinAnsi
/// lacks (İ ı Ş ş Ğ ğ) mapped onto unused codes via /Differences.
pub(crate) const FONT_DICT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
     /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding \
     /Differences [127 /Idotaccent 129 /dotlessi 141 /Scedilla 143 /scedilla 144 /Gbreve 157 /gbreve] >> >>";

/// Map one char to its byte under WinAnsi + the Turkish /Differences above.
fn winansi_byte(ch: char) -> Option<u8> {
    let cp = ch as u32;
    match ch {
        // Turkish glyphs on unused WinAnsi codes (must match FONT_DICT).
        'İ' => Some(0x7F),
        'ı' => Some(0x81),
        'Ş' => Some(0x8D),
        'ş' => Some(0x8F),
        'Ğ' => Some(0x90),
        'ğ' => Some(0x9D),
        // CP1252 0x80–0x9F specials.
        '€' => Some(0x80),
        '‚' => Some(0x82),
        'ƒ' => Some(0x83),
        '„' => Some(0x84),
        '…' => Some(0x85),
        '†' => Some(0x86),
        '‡' => Some(0x87),
        'ˆ' => Some(0x88),
        '‰' => Some(0x89),
        'Š' => Some(0x8A),
        '‹' => Some(0x8B),
        'Œ' => Some(0x8C),
        'Ž' => Some(0x8E),
        '\u{2018}' => Some(0x91),
        '\u{2019}' => Some(0x92),
        '\u{201C}' => Some(0x93),
        '\u{201D}' => Some(0x94),
        '•' => Some(0x95),
        '–' => Some(0x96),
        '—' => Some(0x97),
        '˜' => Some(0x98),
        '™' => Some(0x99),
        'š' => Some(0x9A),
        '›' => Some(0x9B),
        'œ' => Some(0x9C),
        'ž' => Some(0x9E),
        'Ÿ' => Some(0x9F),
        _ if (0x20..=0x7E).contains(&cp) => Some(cp as u8),
        // Latin-1 supplement matches WinAnsi byte-for-byte here.
        _ if (0xA0..=0xFF).contains(&cp) => Some(cp as u8),
        _ => None,
    }
}

/// Closest-ASCII fallback for characters Helvetica/WinAnsi cannot show.
fn ascii_fallback(ch: char) -> &'static str {
    match ch {
        'Ā' | 'Ă' | 'Ą' => "A",
        'ā' | 'ă' | 'ą' => "a",
        'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "C",
        'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'Ď' | 'Đ' => "D",
        'ď' | 'đ' => "d",
        'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => "E",
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'Ĝ' | 'Ġ' | 'Ģ' => "G",
        'ĝ' | 'ġ' | 'ģ' => "g",
        'Ĥ' | 'Ħ' => "H",
        'ĥ' | 'ħ' => "h",
        'Ĩ' | 'Ī' | 'Ĭ' | 'Į' => "I",
        'ĩ' | 'ī' | 'ĭ' | 'į' => "i",
        'Ĳ' => "IJ",
        'ĳ' => "ij",
        'Ķ' => "K",
        'ķ' => "k",
        'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "L",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'Ń' | 'Ņ' | 'Ň' => "N",
        'ń' | 'ņ' | 'ň' => "n",
        'Ō' | 'Ŏ' | 'Ő' => "O",
        'ō' | 'ŏ' | 'ő' => "o",
        'Ŕ' | 'Ŗ' | 'Ř' => "R",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'Ś' | 'Ŝ' => "S",
        'ś' | 'ŝ' => "s",
        'Ţ' | 'Ť' | 'Ŧ' => "T",
        'ţ' | 'ť' | 'ŧ' => "t",
        'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "U",
        'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'Ŵ' => "W",
        'ŵ' => "w",
        'Ŷ' => "Y",
        'ŷ' => "y",
        'Ź' | 'Ż' => "Z",
        'ź' | 'ż' => "z",
        _ => "?",
    }
}

/// Append one encoded byte to a PDF literal string body, escaping the string
/// delimiters and emitting non-ASCII bytes as octal escapes (keeps the content
/// stream pure ASCII, so byte length == char length).
fn push_string_byte(out: &mut String, b: u8) {
    match b {
        b'\\' => out.push_str("\\\\"),
        b'(' => out.push_str("\\("),
        b')' => out.push_str("\\)"),
        0x20..=0x7E => out.push(b as char),
        _ => out.push_str(&format!("\\{b:03o}")),
    }
}

/// Encode arbitrary text as the body of a `( ... )` literal for a font using
/// `FONT_DICT`: WinAnsi + Turkish /Differences, closest-ASCII for the rest.
/// Writing raw UTF-8 bytes instead would garble every non-ASCII character.
pub(crate) fn encode_pdf_text(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match winansi_byte(ch) {
            Some(b) => push_string_byte(&mut out, b),
            None => {
                for c in ascii_fallback(ch).chars() {
                    push_string_byte(&mut out, c as u8);
                }
            }
        }
    }
    out
}

/// Optional label formatting for page-number stamping (Bates numbering /
/// header-footer), e.g. prefix "DAVA-" + pad 6 → "DAVA-000123".
#[derive(Debug, Clone, Default)]
pub struct NumberFormat {
    /// Text placed before the counter (Turkish-safe via `encode_pdf_text`).
    pub prefix: String,
    /// Zero-pad the counter to this many digits (0 = no padding).
    pub pad_width: usize,
    /// Append today's date (dd.MM.yyyy) after the counter.
    pub with_date: bool,
}

/// Render one page label: prefix + zero-padded counter + optional date.
fn format_label(n: i64, fmt: &NumberFormat, date: Option<&str>) -> String {
    let mut out = format!("{}{:0width$}", fmt.prefix, n, width = fmt.pad_width);
    if let Some(d) = date {
        out.push_str(" \u{2013} "); // en dash separator
        out.push_str(d);
    }
    out
}

/// Civil date from days since the Unix epoch (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Today as "dd.MM.yyyy" (UTC — good enough for a document stamp).
fn today_dmy() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{d:02}.{m:02}.{y:04}")
}

/// Displayed (post-/Rotate) size of every page of `path`, in points. Falls
/// back to Letter-sized pages if the file cannot be parsed.
fn page_view_sizes(app: &tauri::AppHandle, path: &str) -> Result<Vec<(f64, f64)>, AppError> {
    if let Ok(doc) = Document::load(path) {
        let sizes: Vec<(f64, f64)> = doc
            .get_pages()
            .values()
            .map(|id| {
                let mb = crop::media_box(&doc, *id);
                let (w, h) = (mb[2] - mb[0], mb[3] - mb[1]);
                if crop::page_rotation(&doc, *id) % 180 == 90 { (h, w) } else { (w, h) }
            })
            .collect();
        if !sizes.is_empty() {
            return Ok(sizes);
        }
    }
    let n = qpdf::npages(app, path)?;
    Ok(vec![(PAGE_W, PAGE_H); n as usize])
}

/// Build an overlay PDF (one page per entry in `sizes`, matching dimensions),
/// each page showing its number at `position`, optionally rendered through a
/// `NumberFormat` (Bates prefix + zero-padding + date).
fn build_overlay(
    out_path: &str,
    sizes: &[(f64, f64)],
    start: i64,
    position: &str,
    fmt: Option<&NumberFormat>,
) -> Result<(), AppError> {
    let date = fmt.filter(|f| f.with_date).map(|_| today_dmy());
    let count = sizes.len();
    let total_objs = 3 + count * 2; // catalog, pages, font, (content,page)*N
    let mut buf: Vec<u8> = Vec::new();
    let mut off = vec![0usize; total_objs + 1];

    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");

    off[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    let kids: String = (0..count).map(|i| format!("{} 0 R", 5 + i * 2)).collect::<Vec<_>>().join(" ");
    off[2] = buf.len();
    buf.extend_from_slice(format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {count} >>\nendobj\n").as_bytes());

    off[3] = buf.len();
    buf.extend_from_slice(format!("3 0 obj\n{FONT_DICT}\nendobj\n").as_bytes());

    for (i, &(w, h)) in sizes.iter().enumerate() {
        let n = start + i as i64;
        let label = match fmt {
            Some(f) => format_label(n, f, date.as_deref()),
            None => n.to_string(),
        };
        let safe = encode_pdf_text(&label);
        let text_w = label.chars().count() as f64 * 0.56 * FS; // approx Helvetica width
        let (x, y) = match position {
            "bottom-right" => (w - 40.0 - text_w, 28.0),
            "bottom-left" => (40.0, 28.0),
            "top-right" => (w - 40.0 - text_w, h - 40.0),
            "top-center" => ((w - text_w) / 2.0, h - 40.0),
            "top-left" => (40.0, h - 40.0),
            _ => ((w - text_w) / 2.0, 28.0), // bottom-center
        };
        let x = x.max(8.0); // long Bates labels must not slide off the left edge
        let content = format!("BT\n/F1 {FS:.0} Tf\n{x:.1} {y:.1} Td\n({safe}) Tj\nET\n");
        let content_obj = 4 + i * 2;
        let page_obj = 5 + i * 2;

        off[content_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{content_obj} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );

        off[page_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.1} {h:.1}] \
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

/// Build an overlay (one page per entry in `sizes`, matching dimensions) where
/// every page shows the same diagonal, semi-transparent watermark text.
fn build_watermark(out_path: &str, sizes: &[(f64, f64)], text: &str, opacity: f64) -> Result<(), AppError> {
    let safe = encode_pdf_text(text);
    let c = std::f64::consts::FRAC_1_SQRT_2; // cos/sin 45°
    let op = opacity.clamp(0.05, 1.0);

    let count = sizes.len();
    let total_objs = 4 + count * 2; // catalog, pages, font, gstate, (content,page)*N
    let mut buf: Vec<u8> = Vec::new();
    let mut off = vec![0usize; total_objs + 1];

    buf.extend_from_slice(b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n");
    off[1] = buf.len();
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let kids: String = (0..count).map(|i| format!("{} 0 R", 6 + i * 2)).collect::<Vec<_>>().join(" ");
    off[2] = buf.len();
    buf.extend_from_slice(format!("2 0 obj\n<< /Type /Pages /Kids [{kids}] /Count {count} >>\nendobj\n").as_bytes());
    off[3] = buf.len();
    buf.extend_from_slice(format!("3 0 obj\n{FONT_DICT}\nendobj\n").as_bytes());
    off[4] = buf.len();
    buf.extend_from_slice(format!("4 0 obj\n<< /Type /ExtGState /ca {op:.2} /CA {op:.2} >>\nendobj\n").as_bytes());

    for (i, &(w, h)) in sizes.iter().enumerate() {
        // Scale the mark with the page, and roughly center the rotated (45°)
        // text around the page center.
        let fs = 56.0 * (w.min(h) / PAGE_W);
        let tw = text.chars().count() as f64 * 0.5 * fs;
        let tx = w / 2.0 - (tw / 2.0) * c;
        let ty = h / 2.0 - (tw / 2.0) * c;
        let content = format!(
            "q\n/GS1 gs\n0.5 0.5 0.5 rg\nBT\n/F1 {fs:.1} Tf\n{c:.4} {c:.4} -{c:.4} {c:.4} {tx:.1} {ty:.1} Tm\n({safe}) Tj\nET\nQ\n"
        );
        let content_obj = 5 + i * 2;
        let page_obj = 6 + i * 2;
        off[content_obj] = buf.len();
        buf.extend_from_slice(
            format!("{content_obj} 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n", content.len()).as_bytes(),
        );
        off[page_obj] = buf.len();
        buf.extend_from_slice(
            format!(
                "{page_obj} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.1} {h:.1}] \
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
        let sizes = page_view_sizes(app, &merged)?;
        build_watermark(&overlay, &sizes, text, opacity)?;
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

/// Assemble the combined document and stamp formatted page labels onto it
/// (Bates numbering / header-footer). `format: None` = plain numbers.
#[allow(clippy::too_many_arguments)]
pub fn add_page_numbers_formatted(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    position: &str,
    start: i64,
    format: Option<NumberFormat>,
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
        let sizes = page_view_sizes(app, &merged)?;
        build_overlay(&overlay, &sizes, start, position, format.as_ref())?;
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

#[cfg(test)]
mod tests {
    use super::{civil_from_days, encode_pdf_text, format_label, NumberFormat};

    #[test]
    fn format_label_pads_and_prefixes() {
        let f = NumberFormat { prefix: "DAVA-".into(), pad_width: 6, with_date: false };
        assert_eq!(format_label(123, &f, None), "DAVA-000123");
        assert_eq!(format_label(123, &f, Some("02.07.2026")), "DAVA-000123 \u{2013} 02.07.2026");
        // No padding, no prefix behaves like a plain number.
        assert_eq!(format_label(7, &NumberFormat::default(), None), "7");
        // Counter wider than the pad is never truncated.
        let narrow = NumberFormat { prefix: "".into(), pad_width: 2, with_date: false };
        assert_eq!(format_label(12345, &narrow, None), "12345");
    }

    #[test]
    fn turkish_prefix_encodes_via_differences() {
        let f = NumberFormat { prefix: "EK-Şğı ".into(), pad_width: 3, with_date: false };
        let label = format_label(5, &f, None);
        // İ Ş ğ ı map onto the /Differences codes; output stays pure ASCII.
        assert!(encode_pdf_text(&label).is_ascii());
        assert_eq!(encode_pdf_text(&label), "EK-\\215\\235\\201 005");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(365), (1971, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29)); // leap day
        assert_eq!(civil_from_days(-1), (1969, 12, 31)); // pre-epoch
    }

    #[test]
    fn ascii_passes_through_with_escapes() {
        assert_eq!(encode_pdf_text("Page 1"), "Page 1");
        assert_eq!(encode_pdf_text("(a) \\ b"), "\\(a\\) \\\\ b");
    }

    #[test]
    fn turkish_chars_map_to_differences_codes() {
        // İ=127, ı=129, Ş=141, ş=143, Ğ=144, ğ=157 (octal escaped)
        assert_eq!(encode_pdf_text("GİZLİ"), "G\\177ZL\\177");
        assert_eq!(encode_pdf_text("Şğı"), "\\215\\235\\201");
    }

    #[test]
    fn latin1_and_cp1252_encode_as_octal() {
        assert_eq!(encode_pdf_text("ç"), "\\347"); // Latin-1 0xE7
        assert_eq!(encode_pdf_text("€"), "\\200"); // CP1252 0x80
    }

    #[test]
    fn unmappable_chars_transliterate_not_garble() {
        let out = encode_pdf_text("中");
        assert!(out.is_ascii());
        assert!(!out.is_empty());
    }
}
