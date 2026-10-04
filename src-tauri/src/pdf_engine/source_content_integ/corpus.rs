//! Corpus contract tests (#32 fixtures, `fixtures/source-edit/`): CLASSIFY-API … CLASSIFY-BOUNDS.

use super::*;

// --- CLASSIFY-API -----------------------------------------------------------

#[test]
fn classify_api_lists_occurrences() {
    let path = fixture("text-tj.pdf");
    let hits = classify(&path, "CLASSIFY-API");
    assert!(
        !hits.is_empty(),
        "CLASSIFY-API: classify_source_content(text-tj.pdf) must list occurrences"
    );
}

// --- CLASSIFY-SOURCE-PATH ---------------------------------------------------

#[test]
fn classify_uses_corpus_source_path_not_page_pdf() {
    let path = fixture("text-tj.pdf");
    let rendered = path.to_string_lossy();
    assert!(
        rendered.contains("fixtures/source-edit") && rendered.ends_with("text-tj.pdf"),
        "CLASSIFY-SOURCE-PATH: must pass the corpus source path, not pagePdf / --empty --pages; got {}",
        path.display()
    );
    // Do not spawn qpdf --empty --pages and do not call page_pdf_b64.
    let hits = classify(&path, "CLASSIFY-SOURCE-PATH");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-SOURCE-PATH");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-SOURCE-PATH: text-tj.pdf page_index is 0 on the original source"
    );
}

// --- CLASSIFY-TRY-EDIT-TJ ---------------------------------------------------

#[test]
fn classify_text_tj_supported_at_origin() {
    let path = fixture("text-tj.pdf");
    let hits = classify(&path, "CLASSIFY-TRY-EDIT-TJ");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-TRY-EDIT-TJ");
    assert_supported_text_or_image(occ, "text", "CLASSIFY-TRY-EDIT-TJ");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-TRY-EDIT-TJ: page_index must be 0"
    );
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-TJ: origin x must be ~72; got {}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 720.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-TJ: origin y must be ~720; got {}",
        occ.rect.y
    );
    assert!(
        occ.rect.w > 0.0 && occ.rect.h > 0.0,
        "CLASSIFY-TRY-EDIT-TJ: w/h must be positive; got w={} h={}",
        occ.rect.w,
        occ.rect.h
    );
}

// --- CLASSIFY-TRY-EDIT-IMAGE ------------------------------------------------

#[test]
fn classify_image_unique_supported() {
    let path = fixture("image-unique.pdf");
    let hits = classify(&path, "CLASSIFY-TRY-EDIT-IMAGE");
    let occ = first_of_kind(&hits, "image", "CLASSIFY-TRY-EDIT-IMAGE");
    assert_supported_text_or_image(occ, "image", "CLASSIFY-TRY-EDIT-IMAGE");
    assert_eq!(
        occ.page_index, 0,
        "CLASSIFY-TRY-EDIT-IMAGE: page_index must be 0"
    );
    assert!(
        (occ.rect.x - 72.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: x must be ~72; got {}",
        occ.rect.x
    );
    assert!(
        (occ.rect.y - 400.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: y must be ~400; got {}",
        occ.rect.y
    );
    assert!(
        (occ.rect.w - 40.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: w must be ~40; got {}",
        occ.rect.w
    );
    assert!(
        (occ.rect.h - 40.0).abs() <= 1.0,
        "CLASSIFY-TRY-EDIT-IMAGE: h must be ~40; got {}",
        occ.rect.h
    );
}

// --- CLASSIFY-NO-TOUNICODE --------------------------------------------------

#[test]
fn classify_cid_no_tounicode_is_unsupported() {
    let path = fixture("text-cid-no-tounicode.pdf");
    let hits = classify(&path, "CLASSIFY-NO-TOUNICODE");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-NO-TOUNICODE");
    // Deliberate change (§C, A.10): the stand-in's CIDFontType0 has neither a ToUnicode nor a
    // program; FONT_NOT_EMBEDDED (priority 20) comes before NO_TOUNICODE (25). NO_TOUNICODE on its
    // own (an embedded font without ToUnicode) is CLS-H12b.
    assert_unsupported(occ, "text", "FONT_NOT_EMBEDDED", "CLASSIFY-NO-TOUNICODE");
}

// --- CLASSIFY-SHARED-IMAGE --------------------------------------------------

#[test]
fn classify_image_reused_shared_xobject() {
    let path = fixture("image-reused.pdf");
    let hits = classify(&path, "CLASSIFY-SHARED-IMAGE");
    let images: Vec<&SourceOccurrence> = hits.iter().filter(|o| kind_token(o) == "image").collect();
    assert_eq!(
        images.len(),
        2,
        "CLASSIFY-SHARED-IMAGE: image-reused.pdf must yield two image occurrences; got {:?}",
        hits.iter()
            .map(|o| (
                o.page_index,
                kind_token(o),
                capability_token(o),
                reason_code(o)
            ))
            .collect::<Vec<_>>()
    );
    for occ in images {
        assert_unsupported(occ, "image", "SHARED_XOBJECT", "CLASSIFY-SHARED-IMAGE");
    }
}

// --- CLASSIFY-STAND-INS -----------------------------------------------------

#[test]
fn classify_stand_ins_locked_reasons() {
    for (file, kind, reason) in STAND_INS {
        let path = fixture(file);
        let hits = classify(&path, "CLASSIFY-STAND-INS");
        let occ = first_of_kind(&hits, kind, "CLASSIFY-STAND-INS");
        assert_unsupported(occ, kind, reason, &format!("CLASSIFY-STAND-INS: {file}"));
        for other in &hits {
            assert_ne!(
                capability_token(other),
                "supported",
                "CLASSIFY-STAND-INS: {file} must never return supported"
            );
        }
    }
}

// --- CLASSIFY-NO-SILENT-OVERLAY ---------------------------------------------

#[test]
fn classify_stand_ins_never_supported_or_overlay_fallback() {
    let manifest = load_manifest();
    let stand_in_rows: Vec<&FixtureRow> = manifest
        .fixtures
        .iter()
        .filter(|r| r.intent == "unsupported-stand-in")
        .collect();
    assert!(
        !stand_in_rows.is_empty(),
        "CLASSIFY-NO-SILENT-OVERLAY: manifest must list unsupported-stand-in rows"
    );
    for row in stand_in_rows {
        let path = fixture(&row.path);
        let hits = classify(&path, "CLASSIFY-NO-SILENT-OVERLAY");
        for occ in &hits {
            assert_ne!(
                capability_token(occ),
                "supported",
                "CLASSIFY-NO-SILENT-OVERLAY: {} must not be supported",
                row.id
            );
            if let Some(code) = reason_code(occ) {
                assert!(
                    !looks_like_overlay_fallback(&code),
                    "CLASSIFY-NO-SILENT-OVERLAY: {} reason must not be an overlay fallback; got {code}",
                    row.id
                );
                assert!(
                    frozen_reasons().contains(&code),
                    "CLASSIFY-NO-SILENT-OVERLAY: {} reason {code} is not a frozen code",
                    row.id
                );
            }
            assert!(
                !looks_like_overlay_fallback(&capability_token(occ)),
                "CLASSIFY-NO-SILENT-OVERLAY: {} capability must not be an overlay fallback",
                row.id
            );
        }
    }
}

// --- CLASSIFY-NO-SAVE -------------------------------------------------------

#[test]
fn classify_does_not_write_source_or_dest() {
    let path = fixture("text-tj.pdf");
    let parent = path
        .parent()
        .expect("CLASSIFY-NO-SAVE: fixture has a parent dir");
    let before_names = dir_names(parent);
    let before_bytes = fs::read(&path).unwrap();
    let before_meta = fs::metadata(&path).unwrap();
    let before_mtime = before_meta.modified().ok();
    let before_len = before_meta.len();

    // Entry point must not write, whether it returns Ok or Err.
    let _ = classify_source_content(&path);

    let after_bytes = fs::read(&path).unwrap();
    assert_eq!(
        after_bytes, before_bytes,
        "CLASSIFY-NO-SAVE: source bytes of text-tj.pdf must be unchanged"
    );
    let after_meta = fs::metadata(&path).unwrap();
    assert_eq!(
        after_meta.len(),
        before_len,
        "CLASSIFY-NO-SAVE: source length must be unchanged"
    );
    if let (Some(before), Ok(after)) = (before_mtime, after_meta.modified()) {
        assert_eq!(
            after, before,
            "CLASSIFY-NO-SAVE: source mtime must be unchanged"
        );
    }
    let after_names = dir_names(parent);
    assert_eq!(
        after_names, before_names,
        "CLASSIFY-NO-SAVE: no dest sibling may be created next to the fixture; before={before_names:?} after={after_names:?}"
    );
}

// --- CLASSIFY-NO-EDITABLE-CLAIM ---------------------------------------------

#[test]
fn classify_try_edit_is_not_auto_supported() {
    let cid = classify(
        &fixture("text-cid-tounicode.pdf"),
        "CLASSIFY-NO-EDITABLE-CLAIM",
    );
    let cid_occ = first_of_kind(&cid, "text", "CLASSIFY-NO-EDITABLE-CLAIM");
    // Deliberate change (§C, D16): a Type0 font with a good ToUnicode is editable now, but this
    // one has no program, so glyph presence cannot be proven: FONT_NOT_EMBEDDED.
    assert_unsupported(
        cid_occ,
        "text",
        "FONT_NOT_EMBEDDED",
        "CLASSIFY-NO-EDITABLE-CLAIM: text-cid-tounicode.pdf is try-edit but must not be treated as supported",
    );
    assert_ne!(
        reason_code(cid_occ).as_deref(),
        Some("NO_TOUNICODE"),
        "CLASSIFY-NO-EDITABLE-CLAIM: text-cid-tounicode.pdf has a ToUnicode CMap; NO_TOUNICODE is the wrong reason"
    );

    let kerned = classify(&fixture("text-tj-kerned.pdf"), "CLASSIFY-NO-EDITABLE-CLAIM");
    let kerned_occ = first_of_kind(&kerned, "text", "CLASSIFY-NO-EDITABLE-CLAIM");
    assert_supported_text_or_image(
        kerned_occ,
        "text",
        "CLASSIFY-NO-EDITABLE-CLAIM: text-tj-kerned.pdf is the human pick for supported",
    );

    let manifest = load_manifest();
    let try_edit: Vec<&FixtureRow> = manifest
        .fixtures
        .iter()
        .filter(|r| r.intent == "try-edit")
        .collect();
    assert!(
        try_edit.iter().any(|r| r.id == "text-cid-tounicode"),
        "CLASSIFY-NO-EDITABLE-CLAIM: manifest still marks text-cid-tounicode as try-edit"
    );
    assert!(
        try_edit.iter().any(|r| r.id == "text-tj-kerned"),
        "CLASSIFY-NO-EDITABLE-CLAIM: manifest still marks text-tj-kerned as try-edit"
    );
}

// --- CLASSIFY-STALE ---------------------------------------------------------

#[test]
fn classify_mutated_copy_locator_is_stale() {
    let src = fixture("text-tj.pdf");
    let hits = classify(&src, "CLASSIFY-STALE");
    let occ = first_of_kind(&hits, "text", "CLASSIFY-STALE");
    let locator = occ.locator.clone();
    assert!(
        !locator.trim().is_empty(),
        "CLASSIFY-STALE: locator must be a non-empty opaque string"
    );

    match resolve_source_locator(&src, &locator) {
        Ok(_) => {}
        Err(err) => assert_ne!(
            err.code.as_str(),
            "STALE",
            "CLASSIFY-STALE: original source must not be STALE"
        ),
    }

    let scratch = Scratch::new("stale");
    let copy = scratch.file("copy.pdf");
    fs::copy(&src, &copy).unwrap();
    let mut bytes = fs::read(&copy).unwrap();
    flip_one_payload_byte(&mut bytes);
    fs::write(&copy, &bytes).unwrap();

    let err = resolve_source_locator(&copy, &locator)
        .expect_err("CLASSIFY-STALE: resolve_source_locator on a mutated copy must be Err, not Ok");
    assert_eq!(
        err.code, "STALE",
        "CLASSIFY-STALE: AppError.code must be STALE; got {} ({})",
        err.code, err.message
    );
}

// --- CLASSIFY-BOUNDS --------------------------------------------------------

#[test]
fn classify_missing_path_is_invalid_pdf() {
    let scratch = Scratch::new("missing");
    let missing = scratch.file("no-such.pdf");
    expect_err_code(
        classify_source_content(&missing),
        "INVALID_PDF",
        "CLASSIFY-BOUNDS: missing path",
    );
}

#[test]
fn classify_broken_tiny_pdf_is_app_error() {
    let scratch = Scratch::new("broken");
    let path = scratch.file("tiny.pdf");
    fs::write(&path, b"%PDF-1.4\n%% truncated").unwrap();
    expect_bounds_err(
        classify_source_content(&path),
        &["MALFORMED_CONTENT", "INVALID_PDF"],
        "CLASSIFY-BOUNDS: broken tiny PDF",
    );
}

#[test]
fn classify_geom_only_pages_are_empty() {
    for name in [
        "geom-crop-offset.pdf",
        "geom-user-unit.pdf",
        "geom-rotate-90.pdf",
    ] {
        let hits = classify(&fixture(name), "CLASSIFY-BOUNDS");
        assert!(
            hits.is_empty(),
            "CLASSIFY-BOUNDS: {name} is geom-only (re f) and must return an empty list, not a fake supported/GEOMETRY row; got {:?}",
            hits.iter()
                .map(|o| (kind_token(o), capability_token(o), reason_code(o)))
                .collect::<Vec<_>>()
        );
    }
    for name in GEOM_ONLY {
        let hits = classify(&fixture(name), "CLASSIFY-BOUNDS");
        assert!(
            hits.iter().all(|o| capability_token(o) != "supported"),
            "CLASSIFY-BOUNDS: {name} must not mark a geom-only page supported"
        );
    }
}

#[test]
fn classify_oversize_sparse_file_is_file_too_large() {
    // Sparse tempfile — do not commit a 400 MiB PDF.
    let scratch = Scratch::new("huge");
    let path = scratch.file("huge.pdf");
    let f = File::create(&path).unwrap();
    f.set_len(FILE_CAP_BYTES + 1).unwrap();
    drop(f);
    expect_err_code(
        classify_source_content(&path),
        "FILE_TOO_LARGE",
        "CLASSIFY-BOUNDS: set_len(400MiB+1)",
    );
}
