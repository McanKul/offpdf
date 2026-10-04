//! Save integration, step 1–2 (SPEC §B.18): which source page every destination page is, and
//! the edits grouped by edited source (`STALE` when the editor's page is now another source page,
//! `TEXT_EDIT_DUPLICATE_PAGE` when an edited source page is listed more than once).

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_overlay::expand_page_spec;
use crate::pdf_engine::text_edit::cache::file_name;
use crate::pdf_engine::text_edit::context::invalid_page;
use crate::pdf_engine::text_edit::engines::{run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::export::SourceTextSpec;
use crate::pdf_engine::text_edit::reasons;
use crate::pdf_engine::text_edit::snapshot::read_snapshot;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn duplicate_page(source_page: u32, name: &str, times: usize) -> AppError {
    let p = u64::from(source_page) + 1;
    AppError::new(
        "TEXT_EDIT_DUPLICATE_PAGE",
        "This page appears twice",
        format!("Page {p} of \u{201c}{name}\u{201d} is in the list more than once and has a text change."),
    )
    .with_suggestion("Remove the extra copy of the page, then save again.")
    .with_details(format!("source page {p} appears {times} times"))
}

/// One destination page: its group, the canonical source path and the source page (0-based).
pub(super) struct DestPage {
    group: usize,
    key: PathBuf,
    source_page: u32,
}

pub(super) fn key_of(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// `qpdf --show-npages` (no policy refusals: an unedited signed or XFA file may be appended).
fn page_count(engines: &Engines, path: &str, opts: &RunOpts<'_>) -> Result<u32, AppError> {
    let args = [OsString::from("--show-npages"), OsString::from(path)];
    let out = run_tool(&engines.qpdf, &args, false, opts)?;
    let text = String::from_utf8_lossy(&out.stdout);
    if let (0 | 3, Ok(n)) = (out.code, text.trim().parse::<u32>()) {
        return Ok(n);
    }
    // qpdf could not count the pages: our own read names the reason when it has one
    // (ENCRYPTED, FILE_TOO_LARGE, INVALID_PDF …).
    read_snapshot(Path::new(path))?;
    Err(AppError::invalid_pdf(path).with_details(format!(
        "qpdf --show-npages exited with code {}: {}",
        out.code,
        out.stderr.trim()
    )))
}

pub(super) fn dest_pages(
    groups: &[PageGroup],
    engines: &Engines,
    opts: &RunOpts<'_>,
) -> Result<Vec<DestPage>, AppError> {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    let mut dest = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let n = match counts.get(g.path.as_str()) {
            Some(n) => *n,
            None => {
                let n = page_count(engines, &g.path, opts)?;
                counts.insert(g.path.as_str(), n);
                n
            }
        };
        let key = key_of(&g.path);
        for p in expand_page_spec(&g.pages, n)? {
            dest.push(DestPage {
                group: gi,
                key: key.clone(),
                source_page: p.saturating_sub(1),
            });
        }
    }
    Ok(dest)
}

/// An edited source: the group path, and per edited source page its dest page and specs.
pub(super) struct SourceEdits<'a> {
    pub path: String,
    pub key: PathBuf,
    /// Per edited source page: its destination page and its edits.
    pub pages: BTreeMap<u32, (u32, Vec<&'a SourceTextSpec>)>,
}

/// Maps every spec to its source (`STALE` when the dest page is now another source page,
/// `TEXT_EDIT_DUPLICATE_PAGE` when an edited source page is listed more than once).
pub(super) fn group_specs<'a>(
    groups: &[PageGroup],
    dest: &[DestPage],
    specs: &'a [SourceTextSpec],
) -> Result<Vec<SourceEdits<'a>>, AppError> {
    let mut listed: HashMap<(&Path, u32), usize> = HashMap::new();
    for d in dest {
        *listed.entry((d.key.as_path(), d.source_page)).or_insert(0) += 1;
    }
    let mut sources: Vec<SourceEdits<'a>> = Vec::new();
    for s in specs {
        let d = usize::try_from(s.page_index)
            .ok()
            .and_then(|i| dest.get(i))
            .ok_or_else(|| invalid_page(s.page_index, dest.len()))?;
        let path = groups
            .get(d.group)
            .map(|g| g.path.clone())
            .unwrap_or_default();
        let name = file_name(Path::new(&path));
        if d.source_page != s.source_page_index {
            return Err(reasons::stale(&name));
        }
        let times = listed
            .get(&(d.key.as_path(), d.source_page))
            .copied()
            .unwrap_or(0);
        if times != 1 {
            return Err(duplicate_page(d.source_page, &name, times));
        }
        let pos = match sources.iter().position(|e| e.key == d.key) {
            Some(pos) => pos,
            None => {
                sources.push(SourceEdits {
                    path,
                    key: d.key.clone(),
                    pages: BTreeMap::new(),
                });
                sources.len() - 1
            }
        };
        if let Some(src) = sources.get_mut(pos) {
            src.pages
                .entry(d.source_page)
                .or_insert_with(|| (s.page_index, Vec::new()))
                .1
                .push(s);
        }
    }
    Ok(sources)
}
