//! Applying a page plan through qpdf (SPEC §B.13): the decoded data of each edited content stream
//! is replaced with `qpdf --update-from-json` (an empty stream dictionary drops `/Filter` and
//! `/DecodeParms`; qpdf recomputes `/Length`). `--decode-level=none --compress-streams=n` keeps
//! every untouched stream's bytes exactly as they were: with qpdf's default
//! `--compress-streams=y`, qpdf 12.3 still converts LZW streams to Flate at decode level none
//! (APP-06), which the whole-graph check (A2) would rightly refuse; the edited streams are
//! written unfiltered (later Save passes compress them). lopdf never writes. qpdf renumbers
//! objects in its output, so everything that compares the result is id-free.

use crate::error::AppError;
use crate::pdf_engine::render;
use crate::pdf_engine::text_edit::content::PageContent;
use crate::pdf_engine::text_edit::engines::{
    classify_check, run_tool, Engines, RunOpts, SourceCheck,
};
use crate::pdf_engine::text_edit::limits::UPDATE_JSON_MAX_BYTES;
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode, ProblemCtx};
use crate::pdf_engine::text_edit::rewrite::PagePlan;
use lopdf::ObjectId;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::path::Path;

/// One content stream to replace: its id in qpdf's input and its new decoded bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct PartUpdate {
    pub object_id: ObjectId,
    pub decoded: Vec<u8>,
}

/// `EDIT_VERIFY_FAILED` with a technical detail (the apply step is part of verification).
pub(crate) fn verify_failed(detail: &str) -> AppError {
    let p = EditProblem::new(EditProblemCode::EditVerifyFailed, Some(detail.to_string()));
    let ctx = ProblemCtx {
        page_number: None,
        file_name: None,
        face: None,
        reason: None,
    };
    EditProblemCode::EditVerifyFailed.to_app_error(&p, &ctx)
}

/// The edited parts of `plan` against `content`'s stream ids; a part the page or the plan does
/// not have is an internal `EDIT_VERIFY_FAILED` (never skipped).
pub fn updates_for_plan(
    content: &PageContent,
    plan: &PagePlan,
) -> Result<Vec<PartUpdate>, AppError> {
    plan.edited_parts
        .iter()
        .map(
            |i| match (content.parts.get(*i), plan.expected_parts.get(*i)) {
                (Some(part), Some(decoded)) => Ok(PartUpdate {
                    object_id: part.stream_id,
                    decoded: decoded.clone(),
                }),
                _ => Err(verify_failed(&format!(
                    "edited part {i} is not on the page ({} parts, {} planned)",
                    content.parts.len(),
                    plan.expected_parts.len()
                ))),
            },
        )
        .collect()
}

/// The qpdf JSON v2 update document for `updates` (`max_object_id`: the input's highest id).
pub(crate) fn update_json(updates: &[PartUpdate], max_object_id: u32) -> Value {
    let mut objects = Map::new();
    for u in updates {
        objects.insert(
            format!("obj:{} {} R", u.object_id.0, u.object_id.1),
            json!({ "stream": { "dict": {}, "data": render::base64(&u.decoded) } }),
        );
    }
    json!({
        "qpdf": [
            {
                "jsonversion": 2,
                "pushedinheritedpageresources": false,
                "calledgetallpages": false,
                "maxobjectid": max_object_id,
            },
            Value::Object(objects),
        ]
    })
}

/// Writes the update JSON (≤ `UPDATE_JSON_MAX_BYTES`, else "too large to verify").
pub fn write_update_json(
    updates: &[PartUpdate],
    max_object_id: u32,
    out: &Path,
) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(&update_json(updates, max_object_id))
        .map_err(|e| verify_failed(&format!("update JSON: {e}")))?;
    if bytes.len() > UPDATE_JSON_MAX_BYTES {
        return Err(verify_failed("too large to verify: update JSON"));
    }
    std::fs::write(out, bytes).map_err(|e| AppError::io("OffPDF could not write a work file.", e))
}

/// The warning lines of a qpdf run that exited 3, when every one is on the benign allow-list
/// (`engines::classify_check`); `None` when any is not.
pub(crate) fn benign_warnings(stdout: &[u8], stderr: &str) -> Option<Vec<String>> {
    match classify_check(3, &String::from_utf8_lossy(stdout), stderr) {
        SourceCheck::Benign(lines) if !lines.is_empty() => Some(lines),
        _ => None,
    }
}

/// A warning line without the file name qpdf puts in front of it (so a source warning and the
/// same warning about a copy compare equal). qpdf writes `WARNING: <file>: …` or
/// `WARNING: <file> (…): …`; the file name ends at the last `.pdf` followed by `:` or ` (`, so a
/// directory or file name that merely contains `.pdf` does not cut the line short.
pub(crate) fn warning_text(line: &str) -> &str {
    let line = line.strip_prefix("WARNING: ").unwrap_or(line);
    let end = line
        .match_indices(".pdf")
        .map(|(p, _)| p + ".pdf".len())
        .filter(|e| {
            let rest = line.get(*e..).unwrap_or_default();
            rest.starts_with(':') || rest.starts_with(" (")
        })
        .last();
    match end {
        Some(e) => line.get(e..).unwrap_or(line).trim_start_matches([':', ' ']),
        None => line,
    }
}

/// Every warning is one the source itself already had.
pub(crate) fn warnings_known(lines: &[String], source_benign: &[String]) -> bool {
    lines.iter().all(|l| {
        source_benign
            .iter()
            .any(|s| warning_text(s) == warning_text(l))
    })
}

/// `qpdf <input> <output> --decode-level=none --compress-streams=n --update-from-json=<update>`.
/// Exit 0 → no warnings; exit 3 → every warning must be benign **and** one the source itself had
/// (`source_benign`), and they are returned; anything else → `EDIT_VERIFY_FAILED` (stderr in
/// details). `run_qpdf` (exit 3 = success) is not used here.
pub fn apply_update(
    engines: &Engines,
    input: &Path,
    update: &Path,
    output: &Path,
    source_benign: &[String],
    opts: &RunOpts<'_>,
) -> Result<Vec<String>, AppError> {
    let mut update_arg = OsString::from("--update-from-json=");
    update_arg.push(update.as_os_str());
    let args = [
        input.as_os_str().to_os_string(),
        output.as_os_str().to_os_string(),
        OsString::from("--decode-level=none"),
        OsString::from("--compress-streams=n"),
        update_arg,
    ];
    let out = run_tool(&engines.qpdf, &args, false, opts)?;
    match out.code {
        0 => Ok(Vec::new()),
        3 => {
            let benign = benign_warnings(&out.stdout, &out.stderr);
            if let Some(lines) = benign.filter(|l| warnings_known(l, source_benign)) {
                Ok(lines)
            } else {
                Err(verify_failed(&format!(
                    "qpdf update warnings: {}",
                    out.stderr.trim()
                )))
            }
        }
        code => Err(verify_failed(&format!(
            "qpdf update exited with code {code}: {}",
            out.stderr.trim()
        ))),
    }
}
