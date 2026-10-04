# Source-edit fixture corpus

Synthetic PDFs for spike #11 / issue #32. Licensed like OffPDF (MIT). No
third-party, customer, or photographed documents. Rasters are 2×2 (or
smaller) DeviceRGB/Gray; CID files use a tiny synthetic Type0, not a copy
of Noto.

These files describe **existing page structures**. They are **not** claimed
editable. `manifest.json` `intent` is only `try-edit` (later attempt) or
`unsupported-stand-in` (explicit negative). Do not add an `editable` key.

Regenerate: `write_corpus_fixture(id, dest)` in
`src-tauri/src/pdf_engine/source_edit_fixtures.rs` is the source of truth.
After lopdf `save` it runs `qpdf in out` when qpdf is available so
`qpdf --check` passes. Committed `*.pdf` files must be that writer’s output
so uncompressed stream dumps match.

Manifest intents never claim editability; the engine decides and Edit text
re-verifies every change at save. Text-edit fixtures are generated in temp by
`pdf_engine/text_edit/testkit`.

Classifier expectations (v0.4, `source_content_integ.rs`): `text-cid-tounicode`
now reports `FONT_NOT_EMBEDDED` instead of `AMBIGUOUS_UNICODE` (its Type0 font
has no embedded program, so glyph presence cannot be proven).
`text-cid-no-tounicode` reports `FONT_NOT_EMBEDDED` too, which comes before
`NO_TOUNICODE` in the reason order. The PDFs and `manifest.json` are unchanged;
both intents stay `try-edit`.
