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
