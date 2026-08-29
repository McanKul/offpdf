//! High-level PDF operations. Each function is synchronous and is meant to run
//! inside `tauri::async_runtime::spawn_blocking` from the `commands::pdf` layer.
//!
//! Every op validates inputs, ensures the output directory is writable, builds a
//! qpdf argv array (never a shell string) and invokes the engine. They return
//! the produced output path(s) so the command can report them to the frontend.

pub mod blank;
pub mod compress;
pub mod crop;
pub mod edit_image;
pub mod edit_links;
pub mod edit_annots;
pub mod edit_overlay;
#[cfg(test)]
mod edit_overlay_integ;
#[cfg(test)]
mod source_edit_fixtures;
pub mod metadata;
pub mod nup;
pub mod ocr;
pub mod office;
pub mod outline;
pub mod overlay;
pub mod poster;
pub mod qpdf;
pub mod render;
pub mod stamp;
pub mod textexport;
pub mod validate_output;

use crate::error::AppError;
use crate::models::{JobHandle, JobUpdate, PageGroup, PagePick, RotateGroup, SplitMode};
use crate::utils::process::run_qpdf;
use crate::utils::temp;
use std::path::Path;
use std::sync::Arc;
use tauri::Emitter;

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

/// Ensure an input file exists on disk; otherwise it cannot be a valid PDF.
fn require_input(path: &str) -> Result<(), AppError> {
    if Path::new(path).is_file() {
        Ok(())
    } else {
        Err(AppError::invalid_pdf(path))
    }
}

/// Ensure the parent directory of `output` exists and is writable.
fn ensure_output_dir(output: &str) -> Result<(), AppError> {
    let parent = Path::new(output).parent();
    let dir = match parent {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        // No parent component -> treat current dir as the target.
        _ => Path::new(".").to_path_buf(),
    };

    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|_| AppError::output_not_writable(&dir.to_string_lossy()))?;
    }

    if !is_writable(&dir) {
        return Err(AppError::output_not_writable(&dir.to_string_lossy()));
    }
    Ok(())
}

/// Ensure a directory (the destination folder itself) exists and is writable.
fn ensure_dir(dir: &str) -> Result<(), AppError> {
    let p = Path::new(dir);
    if !p.exists() {
        std::fs::create_dir_all(p).map_err(|_| AppError::output_not_writable(dir))?;
    }
    if !is_writable(p) {
        return Err(AppError::output_not_writable(dir));
    }
    Ok(())
}

/// Probe writability by creating and removing a temporary marker file.
fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".offpdf-write-test");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// File stem (name without extension) of `input`, defaulting to "output".
fn file_stem(input: &str) -> String {
    Path::new(input)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string())
}

// ---------------------------------------------------------------------------
// Page-range parsing (for delete)
// ---------------------------------------------------------------------------

/// Parse a page selection string ("1,3,5-8") into a sorted, de-duplicated set
/// of 1-based page numbers. Whitespace is ignored. Out-of-range values are kept
/// here and filtered by the caller against the real page count.
fn parse_pages(spec: &str) -> Result<Vec<u32>, AppError> {
    let mut out: Vec<u32> = Vec::new();
    for raw in spec.split(',') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            let start: u32 = a.trim().parse().map_err(|_| bad_pages(spec))?;
            let end: u32 = b.trim().parse().map_err(|_| bad_pages(spec))?;
            let (lo, hi) = if start <= end { (start, end) } else { (end, start) };
            for n in lo..=hi {
                out.push(n);
            }
        } else {
            out.push(part.parse().map_err(|_| bad_pages(spec))?);
        }
    }
    if out.is_empty() {
        return Err(bad_pages(spec));
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

fn bad_pages(spec: &str) -> AppError {
    AppError::new(
        "INVALID_PAGES",
        "Invalid page selection",
        format!("\"{spec}\" is not a valid page selection."),
    )
    .with_suggestion("Use page numbers and ranges like \"1,3,5-8\".")
}

/// Format an ascending list of page numbers compactly into a qpdf range string
/// (e.g. `[1,3,4,6] -> "1,3-4,6"`).
fn format_ranges(pages: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        if start == end {
            parts.push(start.to_string());
        } else {
            parts.push(format!("{start}-{end}"));
        }
        i += 1;
    }
    parts.join(",")
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Merge multiple PDFs into one, preserving the given order.
pub fn merge(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    inputs: &[String],
    output: &str,
) -> Result<Vec<String>, AppError> {
    if inputs.is_empty() {
        return Err(AppError::new(
            "NO_INPUT",
            "No files to merge",
            "Select at least one PDF to merge.",
        ));
    }
    for input in inputs {
        require_input(input)?;
    }
    ensure_output_dir(output)?;

    let mut args: Vec<String> = vec!["--empty".into(), "--pages".into()];
    for input in inputs {
        args.push(input.clone());
    }
    args.push("--".into());
    args.push(output.to_string());

    let step = format!("Merging {} file(s)", inputs.len());
    run_qpdf(app, handle, job_id, &args, &step, None)?;
    Ok(vec![output.to_string()])
}

/// Assemble pages picked from one or more files, in the given order, into a
/// single output. Powers cross-document reorder / delete / extract via qpdf's
/// multi-file `--pages` (which preserves the order the groups are listed in).
pub fn assemble(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(empty_pages());
    }
    for g in groups {
        require_input(&g.path)?;
    }
    ensure_output_dir(output)?;
    assemble_groups(app, handle, job_id, groups, output, "Assembling pages", None)?;
    Ok(vec![output.to_string()])
}

fn empty_pages() -> AppError {
    AppError::new(
        "NO_PAGES",
        "No pages selected",
        "Choose at least one page for the result.",
    )
}

/// Low-level: assemble `groups` into `output` via qpdf multi-file `--pages`,
/// optionally prefixing extra output flags (e.g. linearize). Order preserved.
fn assemble_groups(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
    step: &str,
    percent: Option<f32>,
) -> Result<(), AppError> {
    let mut args: Vec<String> = vec!["--empty".into(), "--pages".into()];
    for g in groups {
        args.push(g.path.clone());
        args.push(g.pages.clone());
    }
    args.push("--".into());
    args.push(output.to_string());
    run_qpdf(app, handle, job_id, &args, step, percent)
}

/// Group an ordered list of picks into qpdf page-groups (consecutive picks from
/// the same file merge into one group; order preserved).
fn picks_to_groups(picks: &[PagePick]) -> Vec<PageGroup> {
    let mut groups: Vec<PageGroup> = Vec::new();
    for p in picks {
        if let Some(last) = groups.last_mut() {
            if last.path == p.path {
                last.pages.push(',');
                last.pages.push_str(&p.page.to_string());
                continue;
            }
        }
        groups.push(PageGroup {
            path: p.path.clone(),
            pages: p.page.to_string(),
        });
    }
    groups
}

/// Organize: assemble `groups` (kept pages, in order) and apply per-page
/// `rotations` in a single qpdf pass. Powers the page editor (reorder + delete +
/// rotate). `rotations[*].pages` are output-page numbers on the assembled doc.
pub fn edit(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    rotations: &[RotateGroup],
    output: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(empty_pages());
    }
    for g in groups {
        require_input(&g.path)?;
    }
    ensure_output_dir(output)?;

    let mut args: Vec<String> = Vec::new();
    for r in rotations {
        if matches!(r.angle, 90 | 180 | 270) && !r.pages.is_empty() {
            args.push(format!("--rotate=+{}:{}", r.angle, r.pages));
        }
    }
    args.push("--empty".into());
    args.push("--pages".into());
    for g in groups {
        args.push(g.path.clone());
        args.push(g.pages.clone());
    }
    args.push("--".into());
    args.push(output.to_string());

    run_qpdf(app, handle, job_id, &args, "Saving pages", None)?;
    Ok(vec![output.to_string()])
}

/// Password-protect the combined document: assemble into a temp file, then
/// encrypt (AES-256) to `output`. `user_password` is needed to open; an empty
/// user password lets anyone open while `owner_password` still restricts edits.
pub fn protect(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    user_password: &str,
    owner_password: &str,
    output: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(empty_pages());
    }
    if user_password.is_empty() && owner_password.is_empty() {
        return Err(AppError::new(
            "NO_PASSWORD",
            "Enter a password",
            "Set a password to protect the PDF.",
        ));
    }
    for g in groups {
        require_input(&g.path)?;
    }
    ensure_output_dir(output)?;

    let work = temp::root(app)?.join("work").join(job_id);
    std::fs::create_dir_all(&work).map_err(|e| AppError::io("Could not create a temp directory.", e))?;
    let merged = work.join("merged.pdf");
    let merged_str = merged.to_string_lossy().to_string();

    let result = (|| -> Result<Vec<String>, AppError> {
        assemble_groups(app, handle, job_id, groups, &merged_str, "Preparing", None)?;
        // Owner password defaults to the user password when omitted.
        let owner = if owner_password.is_empty() { user_password } else { owner_password };
        let args: Vec<String> = vec![
            "--encrypt".into(),
            user_password.to_string(),
            owner.to_string(),
            "256".into(),
            "--".into(),
            merged_str.clone(),
            output.to_string(),
        ];
        run_qpdf(app, handle, job_id, &args, "Encrypting", None)?;
        Ok(vec![output.to_string()])
    })();

    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Remove a password from an encrypted PDF (given the correct password).
pub fn unlock(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    password: &str,
) -> Result<Vec<String>, AppError> {
    require_input(input)?;
    ensure_output_dir(output)?;
    let args: Vec<String> = vec![
        "--decrypt".into(),
        format!("--password={password}"),
        input.to_string(),
        output.to_string(),
    ];
    match run_qpdf(app, handle, job_id, &args, "Removing password", None) {
        Ok(()) => Ok(vec![output.to_string()]),
        Err(e) => {
            let d = format!("{} {}", e.message, e.details.clone().unwrap_or_default()).to_lowercase();
            if d.contains("password") || d.contains("invalid") {
                Err(AppError::new(
                    "WRONG_PASSWORD",
                    "Wrong password",
                    "The password is incorrect, or the file isn't password-protected.",
                ))
            } else {
                Err(e)
            }
        }
    }
}

/// Extract an explicit page selection ("1,4,8-12") into one output file.
pub fn extract(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    pages: &str,
) -> Result<Vec<String>, AppError> {
    require_input(input)?;
    ensure_output_dir(output)?;

    let args: Vec<String> = vec![
        "--empty".into(),
        "--pages".into(),
        input.to_string(),
        pages.to_string(),
        "--".into(),
        output.to_string(),
    ];
    run_qpdf(app, handle, job_id, &args, "Extracting pages", None)?;
    Ok(vec![output.to_string()])
}

/// Reorder pages according to `order` ("1,3,2,4-10"). qpdf preserves order.
pub fn reorder(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    order: &str,
) -> Result<Vec<String>, AppError> {
    require_input(input)?;
    ensure_output_dir(output)?;

    let args: Vec<String> = vec![
        "--empty".into(),
        "--pages".into(),
        input.to_string(),
        order.to_string(),
        "--".into(),
        output.to_string(),
    ];
    run_qpdf(app, handle, job_id, &args, "Reordering pages", None)?;
    Ok(vec![output.to_string()])
}

/// Delete the given pages, keeping the remainder. At least one page must remain.
pub fn delete(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    input: &str,
    output: &str,
    pages: &str,
) -> Result<Vec<String>, AppError> {
    require_input(input)?;
    ensure_output_dir(output)?;

    let n = qpdf::npages(app, input)?;
    let remove: std::collections::HashSet<u32> =
        parse_pages(pages)?.into_iter().filter(|p| *p >= 1 && *p <= n).collect();

    let keep: Vec<u32> = (1..=n).filter(|p| !remove.contains(p)).collect();
    if keep.is_empty() {
        return Err(AppError::new(
            "DELETE_ALL",
            "Cannot delete every page",
            "At least one page must remain.",
        ));
    }

    let keep_str = format_ranges(&keep);
    let args: Vec<String> = vec![
        "--empty".into(),
        "--pages".into(),
        input.to_string(),
        keep_str,
        "--".into(),
        output.to_string(),
    ];
    run_qpdf(app, handle, job_id, &args, "Deleting pages", None)?;
    Ok(vec![output.to_string()])
}

/// Rotate pages of the combined document by a relative angle (90/180/270).
/// `rotate_pages` is "all"/"" for every page, or qpdf output-page numbers like
/// "1,3,5-8" (global positions in the assembled document). Assembles + rotates
/// in a single qpdf pass (no temp file).
pub fn rotate(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    angle: i32,
    rotate_pages: &str,
    output: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(empty_pages());
    }
    if !matches!(angle, 90 | 180 | 270) {
        return Err(AppError::new(
            "INVALID_ANGLE",
            "Invalid rotation angle",
            "Rotation must be 90, 180 or 270 degrees.",
        ));
    }
    for g in groups {
        require_input(&g.path)?;
    }
    ensure_output_dir(output)?;

    // `+` makes the rotation relative to the page's current orientation.
    let rotate_flag = if rotate_pages.is_empty() || rotate_pages.eq_ignore_ascii_case("all") {
        format!("--rotate=+{angle}")
    } else {
        format!("--rotate=+{angle}:{rotate_pages}")
    };

    let mut args: Vec<String> = vec![rotate_flag, "--empty".into(), "--pages".into()];
    for g in groups {
        args.push(g.path.clone());
        args.push(g.pages.clone());
    }
    args.push("--".into());
    args.push(output.to_string());

    run_qpdf(app, handle, job_id, &args, "Rotating pages", None)?;
    Ok(vec![output.to_string()])
}

/// True if `spec` expands, in order, to exactly pages 1..=n — i.e. the whole
/// document in natural order ("1-z", "1-N" or "1,2,…,N"). Conservative: any
/// unparseable part returns false.
pub(crate) fn spec_is_full_range(spec: &str, n: u32) -> bool {
    let trimmed = spec.trim();
    if trimmed == "1-z" {
        return true;
    }
    let mut expect: u32 = 1;
    for raw in trimmed.split(',') {
        let part = raw.trim();
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => match (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                (Ok(a), Ok(b)) => (a, b),
                _ => return false,
            },
            None => match part.parse::<u32>() {
                Ok(p) => (p, p),
                Err(_) => return false,
            },
        };
        if lo != expect || hi < lo {
            return false;
        }
        expect = hi + 1;
    }
    expect == n + 1
}

/// Non-destructive optimization of the combined document: assemble + linearize
/// + generate object streams in one qpdf pass. Keeps text and vectors intact.
/// The first group's file is qpdf's primary input (its `--pages` slot is `.`),
/// which preserves document-level data — outline/bookmarks, Info metadata —
/// that the `--empty` form would strip. If the input is a single unmodified
/// file and qpdf's rewrite comes out larger (linearization overhead), the
/// original file is kept instead.
pub fn optimize(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    groups: &[PageGroup],
    output: &str,
) -> Result<Vec<String>, AppError> {
    if groups.is_empty() {
        return Err(empty_pages());
    }
    for g in groups {
        require_input(&g.path)?;
    }
    ensure_output_dir(output)?;

    let mut args: Vec<String> = vec![
        groups[0].path.clone(),
        "--linearize".into(),
        "--object-streams=generate".into(),
        "--pages".into(),
        ".".into(),
        groups[0].pages.clone(),
    ];
    for g in &groups[1..] {
        args.push(g.path.clone());
        args.push(g.pages.clone());
    }
    args.push("--".into());
    args.push(output.to_string());

    run_qpdf(app, handle, job_id, &args, "Optimizing PDF", None)?;

    // Size guard: when the result is just a rewrite of one whole file in its
    // natural order, never ship an output bigger than the input.
    if groups.len() == 1
        && spec_is_full_range(&groups[0].pages, qpdf::npages(app, &groups[0].path)?)
    {
        let in_size = std::fs::metadata(&groups[0].path).map(|m| m.len()).unwrap_or(0);
        let out_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(u64::MAX);
        if in_size > 0 && out_size >= in_size {
            std::fs::copy(&groups[0].path, output)
                .map_err(|e| AppError::io("Could not write the output file.", e))?;
            let _ = app.emit(
                "job:update",
                JobUpdate::new(job_id, "running", "Already optimal — kept the original file"),
            );
        }
    }
    Ok(vec![output.to_string()])
}

/// Split the combined document (ordered `picks` across files) into one or more
/// output files in `output_dir`, according to `mode`. For `Pages`, `picks` are
/// the already-selected pages and produce a single file.
pub fn split(
    app: &tauri::AppHandle,
    handle: &Arc<JobHandle>,
    job_id: &str,
    picks: &[PagePick],
    output_dir: &str,
    mode: &SplitMode,
) -> Result<Vec<String>, AppError> {
    if picks.is_empty() {
        return Err(empty_pages());
    }
    let mut seen = std::collections::HashSet::new();
    for p in picks {
        if seen.insert(p.path.as_str()) {
            require_input(&p.path)?;
        }
    }
    ensure_dir(output_dir)?;
    let stem = file_stem(&picks[0].path);
    let dir = Path::new(output_dir);

    match mode {
        SplitMode::EveryN { n } => {
            let chunk = (*n).max(1) as usize;
            let total_chunks = picks.len().div_ceil(chunk);
            let mut outputs: Vec<String> = Vec::new();
            for (idx, slice) in picks.chunks(chunk).enumerate() {
                if handle.is_cancelled() {
                    return Err(AppError::cancelled());
                }
                let part = idx + 1;
                let outfile = dir.join(format!("{stem}-part{part:03}.pdf"));
                let outfile_str = outfile.to_string_lossy().to_string();
                let groups = picks_to_groups(slice);
                let percent = (part as f32 / total_chunks as f32) * 100.0;
                let step = format!("Writing part {part} of {total_chunks}");
                assemble_groups(app, handle, job_id, &groups, &outfile_str, &step, Some(percent))?;
                outputs.push(outfile_str);
            }
            Ok(outputs)
        }
        SplitMode::Ranges { ranges } => {
            if ranges.is_empty() {
                return Err(AppError::new(
                    "NO_RANGES",
                    "No ranges given",
                    "Enter at least one page range, e.g. 1-5.",
                ));
            }
            let n = picks.len() as u32;
            let total = ranges.len();
            let mut outputs: Vec<String> = Vec::new();
            for (idx, r) in ranges.iter().enumerate() {
                if handle.is_cancelled() {
                    return Err(AppError::cancelled());
                }
                let lo = r.start.min(r.end).max(1);
                let hi = r.start.max(r.end).min(n);
                if lo > n {
                    return Err(AppError::new(
                        "INVALID_RANGE",
                        "Range out of bounds",
                        format!("Range {}-{} is outside the {n}-page document.", r.start, r.end),
                    ));
                }
                let slice = &picks[(lo as usize - 1)..=(hi as usize - 1)];
                let outfile = dir.join(format!("{stem}_p{lo}-{hi}.pdf"));
                let outfile_str = outfile.to_string_lossy().to_string();
                let groups = picks_to_groups(slice);
                let percent = ((idx + 1) as f32 / total as f32) * 100.0;
                let step = format!("Writing range {} of {total}", idx + 1);
                assemble_groups(app, handle, job_id, &groups, &outfile_str, &step, Some(percent))?;
                outputs.push(outfile_str);
            }
            Ok(outputs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_ranges, parse_pages, spec_is_full_range};

    #[test]
    fn full_range_specs_detected() {
        assert!(spec_is_full_range("1-z", 5));
        assert!(spec_is_full_range("1-5", 5));
        assert!(spec_is_full_range("1,2,3,4,5", 5));
    }

    #[test]
    fn partial_or_reordered_specs_rejected() {
        assert!(!spec_is_full_range("1-4", 5));
        assert!(!spec_is_full_range("2-5", 5));
        assert!(!spec_is_full_range("3,2,1", 3)); // reorder must never trigger the guard
    }

    #[test]
    fn parse_pages_sorts_and_dedupes() {
        assert_eq!(parse_pages("3,1,5-7,5").unwrap(), vec![1, 3, 5, 6, 7]);
        assert!(parse_pages("abc").is_err());
    }

    #[test]
    fn format_ranges_compacts_runs() {
        assert_eq!(format_ranges(&[1, 3, 4, 6]), "1,3-4,6");
        assert_eq!(format_ranges(&[2]), "2");
    }
}
