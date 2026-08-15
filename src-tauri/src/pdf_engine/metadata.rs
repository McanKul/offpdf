//! Read and write a PDF's document information dictionary (/Info) — Title,
//! Author, Subject, Keywords, Creator, Producer — using lopdf.
//!
//! Strings are decoded the same way `outline.rs` decodes bookmark titles
//! (UTF-16BE with BOM, else PDFDocEncoding ≈ Latin-1) and written back as
//! plain ASCII or, for anything non-ASCII (e.g. Turkish "Başlık Ğüzel"),
//! as UTF-16BE with a BOM so every reader shows the right characters.
//!
//! lopdf 0.34 can write an xref some readers reject as "damaged", so the
//! result is saved to a temp file and normalised by qpdf (`poster.rs` pattern).

use crate::error::AppError;
use crate::models::JobHandle;
use crate::utils::temp;
use lopdf::{Dictionary, Document, Object, StringFormat};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// lopdf loads the whole file; refuse enormous inputs instead of risking RAM.
const MAX_META_BYTES: u64 = 400 * 1024 * 1024;

/// The /Info fields exposed to the UI. `None`/empty means "not set" on read
/// and "remove this entry" on write.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PdfMeta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
}

const FIELD_KEYS: [&[u8]; 6] = [
    b"Title", b"Author", b"Subject", b"Keywords", b"Creator", b"Producer",
];

/// Decode a PDF text string via lopdf: UTF-16BE with a BOM, otherwise
/// PDFDocEncoding.
pub(crate) fn decode_pdf_string(bytes: &[u8]) -> String {
    lopdf::decode_text_string(&Object::String(bytes.to_vec(), StringFormat::Literal))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Encode a text value for a PDF string object: plain bytes when pure ASCII,
/// else UTF-16BE with a leading FE FF BOM (survives Turkish and any Unicode).
pub(crate) fn encode_pdf_string(s: &str) -> Vec<u8> {
    if s.is_ascii() {
        return s.as_bytes().to_vec();
    }
    let mut out = Vec::with_capacity(2 + s.len() * 2);
    out.push(0xFE);
    out.push(0xFF);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn size_gate(input: &str) -> Result<(), AppError> {
    let meta = std::fs::metadata(input).map_err(|e| AppError::io("Could not read the file.", e))?;
    if meta.len() > MAX_META_BYTES {
        return Err(AppError::new(
            "FILE_TOO_LARGE",
            "File too large for metadata editing",
            "Editing metadata needs the document loaded into memory, and this file is over 400 MB.",
        ));
    }
    Ok(())
}

fn load_doc(input: &str) -> Result<Document, AppError> {
    Document::load(input).map_err(|e| {
        AppError::invalid_pdf(input).with_details(format!("lopdf: {e}"))
    })
}

/// Resolve the trailer's /Info to a Dictionary (either a reference or inline).
fn info_dict(doc: &Document) -> Option<Dictionary> {
    match doc.trailer.get(b"Info").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
        Object::Dictionary(d) => Some(d.clone()),
        _ => None,
    }
}

fn field(dict: &Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;
    let bytes = match obj {
        Object::String(b, _) => b.clone(),
        _ => return None,
    };
    let s = decode_pdf_string(&bytes);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Read the /Info dictionary of `input`.
pub fn read_meta(input: &str) -> Result<PdfMeta, AppError> {
    super::require_input(input)?;
    size_gate(input)?;
    let doc = load_doc(input)?;
    let Some(info) = info_dict(&doc) else {
        return Ok(PdfMeta::default());
    };
    Ok(PdfMeta {
        title: field(&info, b"Title"),
        author: field(&info, b"Author"),
        subject: field(&info, b"Subject"),
        keywords: field(&info, b"Keywords"),
        creator: field(&info, b"Creator"),
        producer: field(&info, b"Producer"),
    })
}

/// Apply `fields` to a dictionary: non-empty value → set, empty/None → remove.
fn apply_fields(dict: &mut Dictionary, fields: &PdfMeta) {
    let values = [
        &fields.title,
        &fields.author,
        &fields.subject,
        &fields.keywords,
        &fields.creator,
        &fields.producer,
    ];
    for (key, value) in FIELD_KEYS.iter().zip(values) {
        match value.as_deref().map(str::trim) {
            Some(v) if !v.is_empty() => dict.set(
                key.to_vec(),
                Object::String(encode_pdf_string(v), lopdf::StringFormat::Literal),
            ),
            _ => {
                dict.remove(key);
            }
        }
    }
}

/// Write `fields` into the /Info dict of `input`, saving the result to
/// `output`. With `clear_all`, the existing /Info is dropped entirely and
/// replaced by an empty one — qpdf's rewrite then discards the old, now
/// unreferenced strings ("sanitize").
pub fn write_meta(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    fields: &PdfMeta,
    clear_all: bool,
) -> Result<Vec<String>, AppError> {
    super::require_input(input)?;
    super::ensure_output_dir(output)?;
    size_gate(input)?;

    let mut doc = load_doc(input)?;

    // Build the new Info dictionary as a fresh object (never mutate a possibly
    // shared one in place) and point the trailer at it.
    let info = if clear_all {
        Dictionary::new()
    } else {
        let mut d = info_dict(&doc).unwrap_or_default();
        apply_fields(&mut d, fields);
        d
    };
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set(b"Info".to_vec(), Object::Reference(info_id));

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work)
        .map_err(|e| AppError::io("Could not create a temp directory.", e))?;
    let tmp = work.join("meta.pdf").to_string_lossy().to_string();

    let result = (|| -> Result<Vec<String>, AppError> {
        doc.save(&tmp)
            .map_err(|e| AppError::io("Could not write the updated PDF.", e))?;
        drop(doc);
        // lopdf 0.34 xref quirk: qpdf rewrites a clean, normalised file. A
        // plain `qpdf in out` copy preserves the trailer /Info.
        crate::utils::process::run_qpdf(
            app,
            handle,
            job_id,
            &[tmp.clone(), output.to_string()],
            "Saving metadata",
            None,
        )?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::{decode_pdf_string, encode_pdf_string};

    #[test]
    fn ascii_roundtrips_without_bom() {
        let enc = encode_pdf_string("Plain Title 42");
        assert_eq!(enc, b"Plain Title 42".to_vec());
        assert_eq!(decode_pdf_string(&enc), "Plain Title 42");
    }

    #[test]
    fn turkish_roundtrips_via_utf16be() {
        let s = "Başlık Ğüzel";
        let enc = encode_pdf_string(s);
        assert_eq!(&enc[..2], &[0xFE, 0xFF], "must carry a UTF-16BE BOM");
        assert_eq!(decode_pdf_string(&enc), s);
    }

    #[test]
    fn utf16_surrogate_pairs_roundtrip() {
        let s = "emoji 🙂 test";
        assert_eq!(decode_pdf_string(&encode_pdf_string(s)), s);
    }

    #[test]
    fn latin1_bytes_decode() {
        // PDFDocEncoding ≈ Latin-1: 0xE9 = é
        assert_eq!(decode_pdf_string(&[0x63, 0x61, 0x66, 0xE9]), "café");
    }

    #[test]
    fn pdf_doc_encoding_special_bytes() {
        // 0x8B is U+2030 PER MILLE SIGN, not a control character.
        assert_eq!(decode_pdf_string(&[0x8B]), "‰");
        // 0xA0 is U+20AC EURO SIGN, not NBSP.
        assert_eq!(decode_pdf_string(&[0xA0]), "€");
        // 0x80 is U+2022 BULLET, another byte that differs from Latin-1.
        assert_eq!(decode_pdf_string(&[0x80]), "•");
    }
}
