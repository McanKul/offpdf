//! Parse Tesseract `--list-langs` output and validate a `-l` language string.

use crate::error::AppError;
use std::collections::HashSet;

/// Split `tesseract --list-langs` stdout/stderr into language codes.
/// Drops the header, empty lines, and `Error` / `exception:` lines. Dedups,
/// preserving Tesseract's order.
pub fn parse_tesseract_list_langs(stdout: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in stdout.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("List of available languages") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("error") || lower.starts_with("exception:") {
            continue;
        }
        if seen.insert(line.to_string()) {
            out.push(line.to_string());
        }
    }
    out
}

/// Reject empty / whitespace / empty `+` parts (`OCR_LANG_EMPTY`) and any
/// code not in `installed` (`OCR_LANG_MISSING`).
pub fn validate_ocr_lang(lang: &str, installed: &[&str]) -> Result<(), AppError> {
    let trimmed = lang.trim();
    if trimmed.is_empty() {
        return Err(empty_lang());
    }
    let parts: Vec<&str> = trimmed.split('+').map(str::trim).collect();
    if parts.iter().any(|p| p.is_empty()) {
        return Err(empty_lang());
    }
    for part in parts {
        if !installed.iter().any(|code| *code == part) {
            return Err(missing_lang(part));
        }
    }
    Ok(())
}

fn empty_lang() -> AppError {
    AppError::new(
        "OCR_LANG_EMPTY",
        "No language selected",
        "Choose at least one installed OCR language before starting.",
    )
    .with_suggestion(
        "Select one or more languages from the packs installed on this machine.",
    )
}

fn missing_lang(code: &str) -> AppError {
    AppError::new(
        "OCR_LANG_MISSING",
        "OCR language pack not installed",
        format!("The language pack \"{code}\" is not installed for this Tesseract."),
    )
    .with_suggestion(format!(
        "Install the \"{code}\" pack locally (macOS: brew install tesseract-lang). OffPDF does not download language packs."
    ))
}
