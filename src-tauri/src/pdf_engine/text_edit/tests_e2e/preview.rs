//! PREV-01…07: the preview command path (`service::preview_edits`) — a real qpdf-written,
//! Phase-A-proven one-page PDF — and its agreement with Save.

use super::save::pages_doc;
use super::{page_parts, E2e};
use crate::pdf_engine::text_edit::dto::{TextEditIn, TextPreviewDto};
use crate::pdf_engine::text_edit::reasons::Face;
use crate::pdf_engine::text_edit::reasons::{EditProblemCode, TextWarningCode};
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::service;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, DocBuilder, PageSpec, HELVETICA,
};
use std::path::{Path, PathBuf};

pub(crate) fn decode_b64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("bad base64 byte {c}"),
        }
    };
    let mut out = Vec::new();
    for chunk in s.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|c| **c == b'=').count();
        let n = chunk
            .iter()
            .take(4 - pad)
            .fold(0u32, |acc, c| (acc << 6) | val(*c))
            << (6 * pad as u32);
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    out
}

pub(crate) fn edit_in(run_id: &str, old: &str, new: &str, style: SourceTextStyleIn) -> TextEditIn {
    TextEditIn {
        run_id: run_id.to_string(),
        original_text: old.to_string(),
        text: new.to_string(),
        style,
    }
}

impl E2e {
    /// `service::preview_edits` of `(old, new, style)` edits on page `page`.
    pub(crate) fn preview(
        &self,
        path: &Path,
        page: u32,
        edits: &[(&str, &str, SourceTextStyleIn)],
    ) -> TextPreviewDto {
        let (fp, dto) = self.page(path, page);
        let ins: Vec<TextEditIn> = edits
            .iter()
            .map(|(old, new, style)| {
                let run = dto
                    .runs
                    .iter()
                    .find(|r| r.text == *old)
                    .unwrap_or_else(|| panic!("no run {old:?}"));
                edit_in(&run.id, old, new, style.clone())
            })
            .collect();
        service::preview_edits(
            &self.cache,
            &self.engines,
            &self.temp_root(),
            &path.to_string_lossy(),
            &fp,
            page,
            &ins,
        )
        .unwrap_or_else(|e| panic!("preview: {e} {:?}", e.details))
    }

    /// Writes a preview's page PDF next to the scratch files.
    pub(crate) fn preview_file(&self, preview: &TextPreviewDto, name: &str) -> PathBuf {
        let b64 = preview.page_pdf.as_deref().expect("preview pdf");
        self.file(name, &decode_b64(b64))
    }
}

#[test]
fn prev_01_ok_verdict_and_a_page_pdf() {
    let Some(t) = E2e::new("prev_01") else {
        return;
    };
    let src = t.file("p1.pdf", &fx::word());
    let p = t.preview(
        &src,
        0,
        &[("Invoice 2026", "Invoice 2027", SourceTextStyleIn::default())],
    );
    assert!(p.verdicts.iter().all(|v| v.ok), "PREV-01 {:?}", p.verdicts);
    assert!(p.page_problem.is_none());
    let v = &p.verdicts[0];
    assert!(
        v.caret_offsets.as_ref().is_some_and(|c| c.len() == 13),
        "{v:?}"
    );
    assert!(v.new_rect.is_some());
    let page = t.preview_file(&p, "p1-preview.pdf");
    assert!(t.words(&page, 0).contains("Invoice2027"));
}

#[test]
fn prev_02_preview_parts_equal_save_parts() {
    let Some(t) = E2e::new("prev_02") else {
        return;
    };
    let src = t.file("p2.pdf", &pages_doc(&["Preview line", "Second page"]));
    let p = t.preview(
        &src,
        0,
        &[(
            "Preview line",
            "Previewed line",
            SourceTextStyleIn::default(),
        )],
    );
    let page = t.preview_file(&p, "p2-preview.pdf");
    let obj = t.edit(&src, 0, 0, "Preview line", "Previewed line");
    let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("PREV-02 save");
    assert_eq!(page_parts(&page, 0), page_parts(&saved.path, 0), "PREV-02");
}

#[test]
fn prev_03_a_failed_verdict_has_no_page_pdf() {
    let Some(t) = E2e::new("prev_03") else {
        return;
    };
    let src = t.file("p3.pdf", &fx::word());
    let p = t.preview(
        &src,
        0,
        &[(
            "Invoice 2026",
            "Invoice 2026Y",
            SourceTextStyleIn::default(),
        )],
    );
    assert!(p.page_pdf.is_none(), "PREV-03 keeps the last good page");
    assert_eq!(p.verdicts[0].code, Some(EditProblemCode::GlyphMissing));
    assert_eq!(p.verdicts[0].chars, vec!["Y".to_string()]);
    assert!(!p.verdicts[0].ok && p.verdicts[0].caret_offsets.is_none());
}

/// Two incompressible 25 MB images on a page: every one-page copy is over 48 MiB.
fn heavy_page() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let side = 2_900usize;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut noise = |n: usize| -> Vec<u8> {
        let mut v = Vec::with_capacity(n + 8);
        while v.len() < n {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            v.extend_from_slice(&state.to_le_bytes());
        }
        v.truncate(n);
        v
    };
    let dict = format!(
        "/Type /XObject /Subtype /Image /Width {side} /Height {side} /ColorSpace /DeviceRGB /BitsPerComponent 8"
    );
    let a = d.b.add_stream(&dict, &noise(side * side * 3));
    let b = d.b.add_stream(&dict, &noise(side * side * 3));
    d.page(PageSpec::new(
        b"q 100 0 0 100 400 600 cm /Im0 Do Q q 100 0 0 100 400 450 cm /Im1 Do Q \
          BT /F1 12 Tf 72 700 Td (Heavy page) Tj ET",
        &format!("/Font << /F1 {f} 0 R >> /XObject << /Im0 {a} 0 R /Im1 {b} 0 R >>"),
    ));
    d.build()
}

#[test]
fn prev_04_an_oversize_preview_is_unavailable_not_an_error() {
    let Some(t) = E2e::new("prev_04") else {
        return;
    };
    let src = t.file("heavy.pdf", &heavy_page());
    let p = t.preview(
        &src,
        0,
        &[("Heavy page", "Heavy pages", SourceTextStyleIn::default())],
    );
    assert!(p.verdicts.iter().all(|v| v.ok), "PREV-04 {:?}", p.verdicts);
    assert!(p.page_pdf.is_none(), "PREV-04 page pdf over 48 MiB");
    assert!(
        p.warnings
            .iter()
            .any(|w| w.code == TextWarningCode::PreviewUnavailable),
        "PREV-04 {:?}",
        p.warnings
    );
}

/// `text` in a content stream with 100 KB of other data on both sides (outside the first and
/// last 64 KiB that `stat_matches` hashes).
fn padded_doc(text: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let pad = vec![b'%'; 100_000];
    d.b.add_stream("", &pad);
    let content = format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    d.page(PageSpec::new(
        content.as_bytes(),
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.b.add_stream("", &pad);
    d.build()
}

#[test]
fn prev_05_preview_reads_the_snapshot_copy_not_the_users_file() {
    let Some(t) = E2e::new("prev_05") else {
        return;
    };
    let bytes = padded_doc("Middle text");
    let src = t.file("p5.pdf", &bytes);
    let (fp, dto) = t.page(&src, 0);
    let run = dto
        .runs
        .iter()
        .find(|r| r.text == "Middle text")
        .expect("run")
        .clone();
    // Same length, same mtime, same first/last 64 KiB: only the middle changes on disk.
    let modified = std::fs::metadata(&src)
        .and_then(|m| m.modified())
        .expect("mtime");
    let at = bytes
        .windows(13)
        .position(|w| w == b"(Middle text)")
        .expect("needle");
    assert!(
        at > 65_536 && bytes.len() - at > 65_536,
        "PREV-05 fixture layout"
    );
    let mut changed = bytes.clone();
    changed[at..at + 13].copy_from_slice(b"(Mutant text)");
    std::fs::write(&src, &changed).expect("rewrite");
    std::fs::File::options()
        .write(true)
        .open(&src)
        .and_then(|f| f.set_modified(modified))
        .expect("restore mtime");
    let p = service::preview_edits(
        &t.cache,
        &t.engines,
        &t.temp_root(),
        &src.to_string_lossy(),
        &fp,
        0,
        &[edit_in(
            &run.id,
            "Middle text",
            "Middle texts",
            SourceTextStyleIn::default(),
        )],
    )
    .expect("PREV-05 preview");
    assert!(p.verdicts[0].ok, "{:?}", p.verdicts);
    let page = t.preview_file(&p, "p5-preview.pdf");
    let words = t.words(&page, 0);
    assert!(
        words.contains("Middletexts") && !words.contains("Mutant"),
        "PREV-05 {words}"
    );
}

#[test]
fn prev_06_face_from_a_shared_inherited_resource_dictionary() {
    let Some(t) = E2e::new("prev_06") else {
        return;
    };
    let src = t.file("p6.pdf", &fx::shared_inherited_resources());
    let bold = SourceTextStyleIn {
        face: Some(Face::Bold),
        ..SourceTextStyleIn::default()
    };
    let p = t.preview(&src, 0, &[("Regular words", "Regular words", bold)]);
    assert!(p.verdicts.iter().all(|v| v.ok), "PREV-06 {:?}", p.verdicts);
    assert!(p.page_pdf.is_some(), "PREV-06 preview: {:?}", p.warnings);
    let obj = t.edit_styled(
        &src,
        0,
        0,
        "Regular words",
        "Regular words",
        serde_json::json!({ "face": "bold" }),
    );
    let saved = t.save(&[(&src, "1-z")], vec![obj]).expect("PREV-06 save");
    t.assert_checks_clean(&saved.path);
    let (_, run) = t.run(&saved.path, 0, "Regular words");
    assert_eq!(
        run.style.map(|s| s.face),
        Some(Face::Bold),
        "PREV-06 saved face"
    );
}

#[test]
fn prev_07_concurrent_inspects_and_previews_share_the_cache_safely() {
    let Some(t) = E2e::new("prev_07") else {
        return;
    };
    let src = t.file("p7.pdf", &pages_doc(&["Concurrent line"]));
    let barrier = std::sync::Barrier::new(8);
    let results: Vec<Vec<Vec<u8>>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (cache, engines, temp) = (t.cache.clone(), t.engines.clone(), t.temp_root());
                let (src, barrier) = (src.clone(), &barrier);
                let out = t.scratch.path(&format!("p7-{i}.pdf"));
                s.spawn(move || {
                    let path = src.to_string_lossy().into_owned();
                    barrier.wait();
                    let info = service::open_source(&cache, &engines, &temp, &path).expect("open");
                    let page =
                        service::inspect_page(&cache, &engines, &temp, &path, &info.fingerprint, 0)
                            .expect("inspect");
                    let run = &page.runs[0];
                    let p = service::preview_edits(
                        &cache,
                        &engines,
                        &temp,
                        &path,
                        &info.fingerprint,
                        0,
                        &[edit_in(
                            &run.id,
                            &run.text,
                            "Concurrent lines",
                            SourceTextStyleIn::default(),
                        )],
                    )
                    .expect("preview");
                    assert!(p.verdicts[0].ok, "{:?}", p.verdicts);
                    std::fs::write(&out, decode_b64(p.page_pdf.as_deref().expect("pdf")))
                        .expect("write");
                    page_parts(&out, 0)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    assert!(
        results.windows(2).all(|w| w[0] == w[1]),
        "PREV-07 previews differ"
    );
}
