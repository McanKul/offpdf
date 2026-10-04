//! T5 end-to-end tests (SPEC §E.7, qpdf + Poppler required): edits are made the way the editor
//! makes them — `service::open_source` and `service::inspect_page` give the run ids — and saved
//! through the real Edit PDF export (`edit_overlay::export_edit_pdf_with_check_exe`, real qpdf
//! runner). Every successful save is checked on the **published** file: `qpdf --check` exit 0,
//! Poppler extracts the new text and not the old one, unedited pages keep their decoded content
//! (as page parts or, after an overlay, as qpdf's wrapper Form), no sibling `.offpdf-*.pdf.tmp`,
//! the work folder removed, and every source byte-identical to before.
//!
//! `fonts.rs` E2E-01…06, `save.rs` E2E-07…09, 17, 18, 22, `overlay.rs` E2E-10a…f, 11 and VO-01's
//! end-to-end half, `refusals.rs` E2E-12…16, 19…21 and GATE-25, `preview.rs` PREV-01…07.

mod fonts;
mod join;
mod memory;
mod overlay;
pub(crate) mod preview;
pub(crate) mod refusals;
pub(crate) mod save;

use crate::error::AppError;
use crate::models::PageGroup;
use crate::pdf_engine::edit_forms::FormValue;
use crate::pdf_engine::edit_overlay::{export_edit_pdf_with_check_exe, EditDocumentIn};
use crate::pdf_engine::qpdf;
use crate::pdf_engine::text_edit::cache::TextEditCache;
use crate::pdf_engine::text_edit::content::{page_content, qpdf_join};
use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget};
use crate::pdf_engine::text_edit::dto::{PageTextDto, TextRunDto, TextSourceDto};
use crate::pdf_engine::text_edit::engines::{Engines, RunOpts};
use crate::pdf_engine::text_edit::poppler::pdftotext_words;
use crate::pdf_engine::text_edit::service;
use crate::pdf_engine::text_edit::snapshot::{fnv1a_u64, read_verification_snapshot};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use lopdf::{Document, Object, ObjectId};
use serde_json::{json, Value};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const VERIFY_CAP: u64 = 4 << 30;

/// Extra inputs of one save.
#[derive(Default)]
pub(crate) struct SaveOpts<'a> {
    pub cancel: Option<&'a AtomicBool>,
    pub form_values: Vec<FormValue>,
    pub flatten_form: bool,
    pub flatten_annotations: bool,
    /// Called with each qpdf argv the pipeline runs, before it runs.
    pub on_run: Option<&'a dyn Fn(&[String])>,
    /// Destination (default: a fresh `out-N/saved.pdf`).
    pub dest: Option<PathBuf>,
}

/// A save's published file (the sources were checked unchanged, nothing was left behind).
pub(crate) struct Saved {
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

pub(crate) struct E2e {
    pub scratch: Scratch,
    pub engines: Engines,
    pub cache: TextEditCache,
    seq: Cell<usize>,
}

/// The scratch folder (with `temp/textedit/`) is deleted right after this: every background
/// `qpdf --check` the harness started must have read its input by then, or a check that lost its
/// input would decide what a later test of the same bytes sees (review-T5 M1, the e2e_05/cmd_07
/// collision).
impl Drop for E2e {
    fn drop(&mut self) {
        self.cache.wait_checks();
    }
}

pub(crate) fn font_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/fonts/NotoSans-Regular.ttf")
}

impl E2e {
    /// `None` (after `skip:`) when an engine is missing; fails with `OFFPDF_REQUIRE_ENGINES=1`.
    pub(crate) fn new(test: &str) -> Option<E2e> {
        let engines = engines_or_skip(test)?;
        Some(E2e {
            scratch: Scratch::new(test),
            engines,
            cache: TextEditCache::default(),
            seq: Cell::new(0),
        })
    }

    fn next(&self) -> usize {
        let n = self.seq.get() + 1;
        self.seq.set(n);
        n
    }

    pub(crate) fn temp_root(&self) -> PathBuf {
        self.scratch.path("temp")
    }

    pub(crate) fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        self.scratch.write(name, bytes)
    }

    pub(crate) fn try_open(&self, path: &Path) -> Result<TextSourceDto, AppError> {
        service::open_source(
            &self.cache,
            &self.engines,
            &self.temp_root(),
            &path.to_string_lossy(),
        )
    }

    pub(crate) fn open(&self, path: &Path) -> TextSourceDto {
        self.try_open(path)
            .unwrap_or_else(|e| panic!("open {}: {e} {:?}", path.display(), e.details))
    }

    pub(crate) fn try_inspect(
        &self,
        path: &Path,
        fp: &str,
        page: u32,
    ) -> Result<PageTextDto, AppError> {
        service::inspect_page(
            &self.cache,
            &self.engines,
            &self.temp_root(),
            &path.to_string_lossy(),
            fp,
            page,
        )
    }

    /// Opens `path` and inspects `page`.
    pub(crate) fn page(&self, path: &Path, page: u32) -> (String, PageTextDto) {
        let fp = self.open(path).fingerprint;
        let text = self
            .try_inspect(path, &fp, page)
            .unwrap_or_else(|e| panic!("inspect: {e} {:?}", e.details));
        (fp, text)
    }

    /// The run whose text is `text` on `page`, with the fingerprint.
    pub(crate) fn run(&self, path: &Path, page: u32, text: &str) -> (String, TextRunDto) {
        let (fp, dto) = self.page(path, page);
        let run = dto
            .runs
            .iter()
            .find(|r| r.text == text)
            .cloned()
            .unwrap_or_else(|| {
                let all: Vec<_> = dto.runs.iter().map(|r| (&r.text, r.reason)).collect();
                panic!("no run {text:?} on page {page}: {all:?}")
            });
        (fp, run)
    }

    /// The `sourceText` export object (the frontend's `toExportDocument` shape) changing the run
    /// `old` on source page `source_page` (dest page `dest`) to `new` with `style`.
    pub(crate) fn edit_styled(
        &self,
        path: &Path,
        dest: u32,
        source_page: u32,
        old: &str,
        new: &str,
        style: Value,
    ) -> Value {
        let (fp, run) = self.run(path, source_page, old);
        assert!(
            run.editable,
            "run {old:?} is not editable: {:?}",
            run.reason
        );
        json!({
            "id": format!("st-{}", self.next()),
            "kind": "sourceText",
            "pageIndex": dest,
            "rect": { "x": run.rect.x, "y": run.rect.y, "w": run.rect.w, "h": run.rect.h },
            "locked": true,
            "runId": run.id,
            "sourceFingerprint": fp,
            "sourcePageIndex": source_page,
            "originalText": run.text,
            "text": new,
            "style": style,
        })
    }

    /// `edit`, or `None` when the page offers no editable run `old`.
    pub(crate) fn try_edit(
        &self,
        path: &Path,
        dest: u32,
        source_page: u32,
        old: &str,
        new: &str,
    ) -> Option<Value> {
        let (_, dto) = self.page(path, source_page);
        dto.runs
            .iter()
            .any(|r| r.text == old && r.editable)
            .then(|| self.edit(path, dest, source_page, old, new))
    }

    pub(crate) fn edit(
        &self,
        path: &Path,
        dest: u32,
        source_page: u32,
        old: &str,
        new: &str,
    ) -> Value {
        self.edit_styled(path, dest, source_page, old, new, json!({}))
    }

    pub(crate) fn save(
        &self,
        groups: &[(&Path, &str)],
        objects: Vec<Value>,
    ) -> Result<Saved, AppError> {
        self.save_with(groups, objects, SaveOpts::default())
    }

    /// Saves through the real export (the qpdf runner of `edit_pdf_overlays`), then checks what
    /// every save must leave behind: unchanged sources, no sibling temp file, no work folder, no
    /// cache folder created by Save.
    pub(crate) fn save_with(
        &self,
        groups: &[(&Path, &str)],
        objects: Vec<Value>,
        opts: SaveOpts<'_>,
    ) -> Result<Saved, AppError> {
        let n = self.next();
        let doc: EditDocumentIn =
            serde_json::from_value(json!({ "version": 1, "objects": objects }))
                .unwrap_or_else(|e| panic!("export document: {e}"));
        let page_groups: Vec<PageGroup> = groups
            .iter()
            .map(|(p, pages)| PageGroup {
                path: p.to_string_lossy().into_owned(),
                pages: pages.to_string(),
            })
            .collect();
        let before: Vec<u64> = groups.iter().map(|(p, _)| file_hash(p)).collect();
        let dest = opts
            .dest
            .clone()
            .unwrap_or_else(|| self.scratch.path(&format!("out-{n}")).join("saved.pdf"));
        let out_dir = dest.parent().map(Path::to_path_buf).expect("dest folder");
        std::fs::create_dir_all(&out_dir).expect("out dir");
        let dest_before = dest.is_file().then(|| file_hash(&dest));
        let work = self.scratch.path(&format!("work-{n}"));
        std::fs::create_dir_all(&work).expect("work dir");
        let textedit_before = self.temp_root().join("textedit").exists();
        let qpdf = qpdf::resolve_qpdf_standalone();
        let on_run = opts.on_run;
        let result = export_edit_pdf_with_check_exe(
            &page_groups,
            &dest.to_string_lossy(),
            &doc,
            &font_path(),
            &work,
            &format!("e2e-{n}"),
            opts.cancel,
            &qpdf,
            None,
            None,
            &[],
            &opts.form_values,
            opts.flatten_form,
            opts.flatten_annotations,
            |args| {
                if let Some(f) = on_run {
                    f(args);
                }
                run_qpdf(&qpdf, args)
            },
        );
        // `edit_pdf_overlays` removes the job's work folder whatever happened.
        let _ = std::fs::remove_dir_all(&work);
        for ((p, _), h) in groups.iter().zip(&before) {
            assert_eq!(file_hash(p), *h, "source {} changed", p.display());
        }
        let leftovers: Vec<String> = std::fs::read_dir(&out_dir)
            .expect("out dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "sibling temp files left: {leftovers:?}"
        );
        assert!(!work.exists(), "work folder left");
        assert_eq!(
            self.temp_root().join("textedit").exists(),
            textedit_before,
            "Save must not use the editor's cache"
        );
        match result {
            Ok((_, warnings)) => {
                assert!(dest.is_file(), "published file missing");
                Ok(Saved {
                    path: dest,
                    warnings,
                })
            }
            Err(e) => {
                assert_eq!(
                    dest.is_file().then(|| file_hash(&dest)),
                    dest_before,
                    "a failed save changed {}",
                    dest.display()
                );
                Err(e)
            }
        }
    }

    /// `qpdf --check` of the published file exits 0.
    pub(crate) fn assert_checks_clean(&self, pdf: &Path) {
        let out = std::process::Command::new(&self.engines.qpdf)
            .arg("--check")
            .arg(pdf)
            .output()
            .expect("qpdf --check");
        assert_eq!(
            out.status.code(),
            Some(0),
            "qpdf --check {}: {}{}",
            pdf.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Poppler's words of page `page` (0-based), joined without whitespace.
    pub(crate) fn words(&self, pdf: &Path, page: u32) -> String {
        pdftotext_words(&self.engines, pdf, page + 1, &RunOpts::default())
            .unwrap_or_else(|e| panic!("pdftotext {}: {e}", pdf.display()))
            .iter()
            .map(|w| w.text.as_str())
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    /// The edited line reads `new` in the published file and `old` occurs fewer times than on
    /// the source page (when it is not part of `new`).
    pub(crate) fn assert_changed(
        &self,
        src: &Path,
        src_page: u32,
        out: &Path,
        dest: u32,
        old: &str,
        new: &str,
    ) {
        let compact = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        let (old, new) = (compact(old), compact(new));
        let after = self.words(out, dest);
        if !new.is_empty() {
            assert!(
                after.contains(&new),
                "page {}: {new:?} not in {after:?}",
                dest + 1
            );
        }
        if !old.is_empty() && !new.contains(&old) {
            let before = self.words(src, src_page);
            assert!(
                after.matches(&old).count() < before.matches(&old).count(),
                "page {}: old text {old:?} still extracted: {after:?}",
                dest + 1
            );
        }
    }

    /// Published page `dest` holds source page `src_page`'s decoded content unchanged: as its
    /// parts, or as the data of the overlay wrapper Form qpdf made of them.
    pub(crate) fn assert_unedited(&self, src: &Path, src_page: u32, out: &Path, dest: u32) {
        let parts = page_parts(src, src_page);
        let plain = parts.concat();
        let joined = qpdf_join(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>());
        let held = page_holdings(out, dest);
        assert!(
            held.iter().any(|h| *h == plain || *h == joined),
            "page {} of {} does not hold page {} of {} unchanged",
            dest + 1,
            out.display(),
            src_page + 1,
            src.display()
        );
    }

    /// `assert_checks_clean` + `assert_changed` for each `(src, src_page, dest, old, new)`.
    pub(crate) fn assert_saved(&self, out: &Path, changes: &[(&Path, u32, u32, &str, &str)]) {
        self.assert_checks_clean(out);
        for (src, sp, dest, old, new) in changes {
            self.assert_changed(src, *sp, out, *dest, old, new);
        }
    }
}

pub(crate) fn file_hash(path: &Path) -> u64 {
    fnv1a_u64(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

pub(crate) fn run_qpdf(qpdf: &Path, args: &[String]) -> Result<(), AppError> {
    let out = std::process::Command::new(qpdf)
        .args(args)
        .output()
        .map_err(|e| AppError::io("qpdf failed to start", e))?;
    match out.status.code() {
        Some(0) | Some(3) => Ok(()),
        _ => Err(AppError::engine_failed(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )),
    }
}

/// Decoded content parts of page `page` (0-based) of `pdf`.
pub(crate) fn page_parts(pdf: &Path, page: u32) -> Vec<Vec<u8>> {
    let snap = read_verification_snapshot(pdf, VERIFY_CAP).unwrap_or_else(|e| panic!("{e}"));
    let id = *snap.pages.get(page as usize).expect("page");
    let mut budget = DecodeBudget::new(1 << 30);
    let content = page_content(&snap.doc, id, &mut budget)
        .unwrap_or_else(|r| panic!("page content: {}", r.as_str()));
    (0..content.parts.len())
        .map(|i| content.part_bytes(i).to_vec())
        .collect()
}

/// What page `page` of `pdf` draws: its parts concatenated, and the decoded data of every Form
/// XObject of its resources (qpdf's overlay wrapper holds the original content there).
pub(crate) fn page_holdings(pdf: &Path, page: u32) -> Vec<Vec<u8>> {
    let snap = read_verification_snapshot(pdf, VERIFY_CAP).unwrap_or_else(|e| panic!("{e}"));
    let id = *snap.pages.get(page as usize).expect("page");
    let mut out = vec![page_parts(pdf, page).concat()];
    out.extend(page_forms(&snap.doc, id));
    out
}

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(r) => doc.get_object(*r).ok(),
        other => Some(other),
    }
}

fn page_forms(doc: &Document, page: ObjectId) -> Vec<Vec<u8>> {
    let Ok((inline, inherited)) = doc.get_page_resources(page) else {
        return Vec::new();
    };
    let mut dicts: Vec<&lopdf::Dictionary> = inline.into_iter().collect();
    dicts.extend(
        inherited
            .iter()
            .filter_map(|id| doc.get_dictionary(*id).ok()),
    );
    let mut forms = Vec::new();
    for res in dicts {
        let Some(xobjects) = res
            .get(b"XObject")
            .ok()
            .and_then(|o| resolve(doc, o))
            .and_then(|o| o.as_dict().ok())
        else {
            continue;
        };
        for (_, v) in xobjects.iter() {
            let Some(Object::Stream(s)) = resolve(doc, v) else {
                continue;
            };
            if s.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form".as_slice()) {
                let mut budget = DecodeBudget::new(1 << 30);
                if let Ok(data) = decode_stream(s, 1 << 30, &mut budget) {
                    forms.push(data);
                }
            }
        }
    }
    forms
}
