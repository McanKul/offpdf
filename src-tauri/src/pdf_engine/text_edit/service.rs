//! Command logic of Edit text (SPEC §B.20): open, inspect, preview and release, callable from
//! tests without Tauri. Paths in, JSON-ready DTOs out; every refusal is an `AppError` with a
//! reason code. Open/inspect/preview errors are shown as they are (only Save errors carry
//! "The original file was not changed.").

use crate::error::AppError;
use crate::pdf_engine::source_content::classify_source_page;
use crate::pdf_engine::text_edit::cache::TextEditCache;
use crate::pdf_engine::text_edit::dto::{self, PageTextDto, TextPreviewDto, TextSourceDto};
use crate::pdf_engine::text_edit::engines::{Engines, RunOpts, SourceCheck};
use crate::pdf_engine::text_edit::export::page_refusal;
use crate::pdf_engine::text_edit::preview::preview_page;
use crate::pdf_engine::text_edit::reasons;
use crate::pdf_engine::text_edit::rewrite::TextEditIn;
use std::path::Path;

/// Sources above this size open and save normally, only more slowly (SPEC §E.7 BENCH-02).
pub const LARGE_SOURCE_BYTES: u64 = 100 << 20;

/// Opens `path` for Edit text: one bounded read, policy refusals (`ENCRYPTED`, `SIGNED`,
/// `UNSUPPORTED_XFA`, …), the synchronous page-map agreement (`PDF_NEEDS_REPAIR`), and the
/// background `qpdf --check`, which this call does not wait for.
pub fn open_source(
    cache: &TextEditCache,
    engines: &Engines,
    temp_root: &Path,
    path: &str,
) -> Result<TextSourceDto, AppError> {
    let src = cache.open(Path::new(path), temp_root, engines)?;
    let snap = &src.ctx.snap;
    let mut warnings = Vec::new();
    if snap.fingerprint.len > LARGE_SOURCE_BYTES {
        warnings.push(
            "This PDF is larger than 100 MB, so checking and saving text changes takes longer."
                .to_string(),
        );
    }
    Ok(TextSourceDto {
        fingerprint: snap.fingerprint.to_string(),
        page_count: u32::try_from(snap.pages.len()).unwrap_or(u32::MAX),
        warnings,
    })
}

/// Lines of one page (0-based `page_index` in the source) and whether each can be changed. The
/// cached page model goes through the #33 classifier (`classify_source_page`), whose run
/// capabilities and page reason are what the DTO reports; `STALE` when the file changed.
pub fn inspect_page(
    cache: &TextEditCache,
    engines: &Engines,
    temp_root: &Path,
    path: &str,
    fingerprint: &str,
    page_index: u32,
) -> Result<PageTextDto, AppError> {
    let src = cache.get(Path::new(path), temp_root, fingerprint, engines)?;
    let model = cache.page(&src, page_index)?;
    let classified = classify_source_page(&src.ctx, &model, None);
    Ok(dto::page_text(&model, &classified))
}

/// Plans `edits`, writes them into a one-page copy with qpdf and runs Phase A on it (§B.17).
/// Waits for the source's background `qpdf --check` first: problems are `PDF_NEEDS_REPAIR`,
/// benign warnings are allowed in the patched copy. User problems are verdicts, not errors.
pub fn preview_edits(
    cache: &TextEditCache,
    engines: &Engines,
    temp_root: &Path,
    path: &str,
    fingerprint: &str,
    page_index: u32,
    edits: &[TextEditIn],
) -> Result<TextPreviewDto, AppError> {
    let src = cache.get(Path::new(path), temp_root, fingerprint, engines)?;
    // A large model is handed over, and freed by the preview once planned (review-final MEDIUM-3).
    let model = cache.page_to_release(&src, page_index)?;
    // Nothing on a refused page can be changed (inspect lists no runs there).
    if let Some(e) = page_refusal(&model, page_index.saturating_add(1)) {
        return Err(e);
    }
    // The preview writes into the source's folder: a release or an eviction meanwhile deletes
    // it when the preview ends, not under it (review-T5 M2).
    let _lease = cache.lease(&src.dir);
    #[cfg(test)]
    seams::during_preview();
    let benign = match cache.source_check(&src, engines)?.wait(None)? {
        SourceCheck::Clean => Vec::new(),
        SourceCheck::Benign(lines) => lines,
        SourceCheck::Problems(lines) => return Err(reasons::pdf_needs_repair(&lines)),
    };
    let result = preview_page(
        &src.ctx,
        model,
        edits,
        &src.dir,
        engines,
        &benign,
        &RunOpts::default(),
    )?;
    Ok(dto::preview_dto(&result))
}

/// Forgets the file and deletes its temporary copies.
pub fn release_source(cache: &TextEditCache, path: &str) {
    cache.release(Path::new(path));
}

/// Test seam: runs a callback once, inside the next preview on this thread, right after the
/// preview leased its folder (review-T5 M2: a release during a preview).
#[cfg(test)]
pub(crate) mod seams {
    use std::cell::RefCell;

    type Hook = Box<dyn FnOnce()>;

    thread_local! {
        static DURING_PREVIEW: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) fn set_during_preview(hook: Option<Hook>) {
        DURING_PREVIEW.with(|h| *h.borrow_mut() = hook);
    }

    pub(super) fn during_preview() {
        if let Some(hook) = DURING_PREVIEW.with(|h| h.borrow_mut().take()) {
            hook();
        }
    }
}
