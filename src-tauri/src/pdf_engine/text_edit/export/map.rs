//! Save integration, step 1–2 (SPEC §B.18): which source page every destination page is, and
//! the edits grouped by edited source (`STALE` when the editor's page is now another source page,
//! `TEXT_EDIT_DUPLICATE_PAGE` when an edited source page is listed more than once).

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_overlay::expand_page_spec;
use crate::pdf_engine::text_edit::cache::file_name;
use crate::pdf_engine::text_edit::context::invalid_page;
use crate::pdf_engine::text_edit::engines::{run_tool, Engines, RunOpts};
use crate::pdf_engine::text_edit::export::{SourceImageSpec, SourceTextSpec};
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
        format!("Page {p} of \u{201c}{name}\u{201d} is in the list more than once and has a text or image change."),
    )
    .with_suggestion("Remove the extra copy of the page, then save again.")
    .with_details(format!("source page {p} appears {times} times"))
}

/// One destination page: its group, the canonical source path and the source page (0-based).
pub(crate) struct DestPage {
    pub(crate) group: usize,
    pub(crate) key: PathBuf,
    pub(crate) source_page: u32,
}

pub(crate) fn key_of(path: &str) -> PathBuf {
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

pub(crate) fn dest_pages(
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

/// One edited source page: where it goes, its text changes and its image moves.
pub(super) struct PageEdits<'a> {
    pub dest: u32,
    pub text: Vec<&'a SourceTextSpec>,
    pub images: Vec<&'a SourceImageSpec>,
}

/// An edited source: the group path, and per edited source page its destination and edits.
pub(super) struct SourceEdits<'a> {
    pub path: String,
    pub key: PathBuf,
    /// Per edited source page: its destination page and its edits.
    pub pages: BTreeMap<u32, PageEdits<'a>>,
}

/// The source holding `page_index`, adding it on first use. `STALE` when the destination page is
/// now another source page, `TEXT_EDIT_DUPLICATE_PAGE` when the source page is listed twice.
fn source_for<'a>(
    groups: &[PageGroup],
    dest: &[DestPage],
    listed: &HashMap<(&Path, u32), usize>,
    sources: &mut Vec<SourceEdits<'a>>,
    page_index: u32,
    source_page_index: u32,
) -> Result<usize, AppError> {
    let d = usize::try_from(page_index)
        .ok()
        .and_then(|i| dest.get(i))
        .ok_or_else(|| invalid_page(page_index, dest.len()))?;
    let path = groups
        .get(d.group)
        .map(|g| g.path.clone())
        .unwrap_or_default();
    let name = file_name(Path::new(&path));
    if d.source_page != source_page_index {
        return Err(reasons::stale(&name));
    }
    let times = listed
        .get(&(d.key.as_path(), d.source_page))
        .copied()
        .unwrap_or(0);
    if times != 1 {
        return Err(duplicate_page(d.source_page, &name, times));
    }
    match sources.iter().position(|e| e.key == d.key) {
        Some(pos) => Ok(pos),
        None => {
            sources.push(SourceEdits {
                path,
                key: d.key.clone(),
                pages: BTreeMap::new(),
            });
            Ok(sources.len() - 1)
        }
    }
}

/// Maps every text spec and image spec to its source page (the checks above apply to both).
pub(super) fn group_specs<'a>(
    groups: &[PageGroup],
    dest: &[DestPage],
    specs: &'a [SourceTextSpec],
    images: &'a [SourceImageSpec],
) -> Result<Vec<SourceEdits<'a>>, AppError> {
    let mut listed: HashMap<(&Path, u32), usize> = HashMap::new();
    for d in dest {
        *listed.entry((d.key.as_path(), d.source_page)).or_insert(0) += 1;
    }
    let mut sources: Vec<SourceEdits<'a>> = Vec::new();
    for s in specs {
        let pos = source_for(
            groups,
            dest,
            &listed,
            &mut sources,
            s.page_index,
            s.source_page_index,
        )?;
        if let Some(src) = sources.get_mut(pos) {
            src.pages
                .entry(s.source_page_index)
                .or_insert_with(|| PageEdits {
                    dest: s.page_index,
                    text: Vec::new(),
                    images: Vec::new(),
                })
                .text
                .push(s);
        }
    }
    for s in images {
        let pos = source_for(
            groups,
            dest,
            &listed,
            &mut sources,
            s.page_index,
            s.source_page_index,
        )?;
        if let Some(src) = sources.get_mut(pos) {
            src.pages
                .entry(s.source_page_index)
                .or_insert_with(|| PageEdits {
                    dest: s.page_index,
                    text: Vec::new(),
                    images: Vec::new(),
                })
                .images
                .push(s);
        }
    }
    Ok(sources)
}
