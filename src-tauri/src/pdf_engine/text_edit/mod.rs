//! Edit text (v0.4): in-place editing of existing PDF text, fail closed.
//!
//! A change is a byte splice of show-text operators inside the page's own content stream,
//! written by qpdf (`--update-from-json`) and verified before anything is published. Nothing
//! here covers old text with new text, flattens or rasterises a page, re-serialises a page
//! with lopdf, or overwrites the input.
//!
//! Policy for every file in this module tree:
//! - Fail closed: every refusal carries a SCREAMING_SNAKE reason code (`reasons.rs`); no
//!   silent fallback, no skipped edit, no partial list returned as `Ok`.
//! - Bounded: one capped read per operation (`snapshot.rs`), every decompression capped while
//!   inflating (`decode.rs`), every parser with an operation and nesting budget (`limits.rs`).
//! - No panics on data derived from a PDF: no `unwrap`/`expect`/`panic!`/unchecked indexing in
//!   production code (denied below for clippy), checked arithmetic for offsets.
//! - Forbidden lopdf APIs (enforced by `tests_guard::no_forbidden_apis`, which skips comments
//!   and string literals): `Content::decode`, `Content::encode`, `string_to_bytes`,
//!   `replace_text`, `encode_text`, `decompressed_content`, `get_plain_content`,
//!   `get_page_content`, `Document::load`, `load_mem`, `.decompress()`, `IncrementalDocument`,
//!   `.save(`, `save_to(`. lopdf only reads, and only from the bytes of one snapshot.
//! - Test seams exist only under `#[cfg(test)]`; no environment variable or setting can weaken
//!   a check (`OFFPDF_REQUIRE_ENGINES` only turns a test skip into a test failure).
//!
//! Callers use full paths (`text_edit::snapshot::read_snapshot`); there is no `pub use`.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

// T1 engine core
pub(crate) mod content;
pub(crate) mod decode;
pub(crate) mod engines;
pub(crate) mod lexer;
pub(crate) mod limits;
pub(crate) mod reasons;
pub(crate) mod snapshot;

// T2 fonts
pub(crate) mod fonts;

// T3 walker, runs, reading order
pub(crate) mod context;
pub(crate) mod geometry;
pub(crate) mod order;
pub(crate) mod runs;
pub(crate) mod state;
pub(crate) mod structure;
pub(crate) mod walker;

// T4 rewrite, verify, apply, gate, preview
pub(crate) mod apply;
pub(crate) mod encode;
pub(crate) mod fit;
pub(crate) mod gate;
pub(crate) mod graph;
pub(crate) mod poppler;
pub(crate) mod preview;
pub(crate) mod rewrite;
pub(crate) mod verify;

// T5 integration
pub(crate) mod cache;
pub(crate) mod dto;
pub(crate) mod export;
pub(crate) mod service;

// Tests and test helpers
#[cfg(test)]
pub(crate) mod bench;
#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
pub(crate) mod tests_dto;
#[cfg(test)]
pub(crate) mod tests_e2e;
#[cfg(test)]
pub(crate) mod tests_gate;
#[cfg(test)]
pub(crate) mod tests_guard;
#[cfg(test)]
pub(crate) mod tests_independent;
#[cfg(test)]
pub(crate) mod tests_io;
#[cfg(test)]
pub(crate) mod tests_plan;
#[cfg(test)]
pub(crate) mod tests_walk;
