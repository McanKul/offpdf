//! Every budget and tolerance of Edit text (SPEC §B.2). Exceeding a budget: per page → page
//! `PAGE_TOO_COMPLEX`; at file level → `FILE_TOO_LARGE`/`FILE_TOO_COMPLEX`; inside the gate →
//! `EDIT_VERIFY_FAILED` "too large to verify". Never a skipped check.

pub const FILE_CAP_BYTES: u64 = 400 * 1024 * 1024;
pub const MAX_OBJECTS: usize = 2_000_000;
pub const MAX_XREF_CHAIN: usize = 64;
pub const XREF_STREAM_MAX_DECODED: usize = 64 << 20;
pub const OBJSTM_MAX_DECODED: usize = 32 << 20;
pub const OBJSTM_TOTAL_DECODED: usize = 256 << 20;
pub const MAX_OBJECT_NESTING: usize = 100; // preflight: every value lopdf can parse (objects, ObjStm members)
pub const STREAM_MAX_DECODED: usize = 32 << 20; // any single stream
pub const PAGE_CONTENT_MAX_DECODED: usize = 48 << 20;
pub const PAGE_DECODE_BUDGET: usize = 96 << 20; // content + forms + fonts + cmaps for one page walk
pub const PAGE_PARTS_MAX: usize = 256;
pub const PAGE_OPS_MAX: usize = 250_000; // counted while lexing
pub const TOKEN_NESTING_MAX: usize = 32; // arrays/dicts in content
pub const ARRAY_ITEMS_MAX: usize = 65_536;
pub const STRING_BYTES_MAX: usize = 1 << 20;
pub const LITERAL_PAREN_NESTING_MAX: usize = 100;
pub const INLINE_IMAGE_MAX_BYTES: usize = 16 << 20;
pub const Q_DEPTH_MAX: usize = 64;
pub const MARKED_DEPTH_MAX: usize = 64;
pub const FORM_DEPTH_MAX: usize = 8;
pub const FORM_PAINTS_PER_PAGE_MAX: usize = 4_096;
pub const GLYPHS_PER_PAGE_MAX: usize = 400_000;
pub const RUNS_PER_PAGE_MAX: usize = 20_000;
pub const RUN_MEMBERS_MAX: usize = 256;
pub const FONTS_PER_PAGE_MAX: usize = 256;
pub const FONT_PROGRAM_MAX_DECODED: usize = 16 << 20;
pub const TOUNICODE_MAX_DECODED: usize = 2 << 20;
pub const CMAP_MAPPINGS_MAX: usize = 131_072;
pub const CIDTOGID_MAX_BYTES: usize = 131_072;
pub const W_ENTRIES_MAX: usize = 65_536;
pub const STRUCT_CHAIN_MAX: usize = 16;
pub const NUMBER_TREE_NODES_MAX: usize = 100_000;
pub const EDIT_TEXT_CHARS_MAX: usize = 1_000;
pub const EDITS_PER_PAGE_MAX: usize = 200;
pub const EDITS_PER_SAVE_MAX: usize = 500;
pub const DRIFT_TOLERANCE_PT: f64 = 0.01;
pub const STATE_EPSILON: f64 = 1e-9;
pub const COLOR_EPSILON: f64 = 1e-6;
pub const AXIS_EPSILON_REL: f64 = 1e-4;
pub const SHEAR_MAX: f64 = 0.5;
pub const JOIN_GAP_EM: f64 = 0.3;
pub const JOIN_BASELINE_TOL_PT: f64 = 0.01;
pub const SYNTH_SPACE_EM: f64 = 0.2;
pub const DEFAULT_KERN_SPACE: f64 = -250.0;
pub const PER_GLYPH_MIN_RUNS: usize = 12;
pub const PER_GLYPH_SHARE: f64 = 0.8;
pub const DUPLICATE_OVERLAP_SHARE: f64 = 0.5;
pub const CLIP_CONTAIN_TOL_PT: f64 = 0.5;
pub const OVERLAP_WARN_TOL_PT: f64 = 0.5;
pub const NEGLIGIBLE_KERN: f64 = 0.0005;
pub const NUMBER_DECIMALS: usize = 4;
pub const NUMBER_ABS_MAX: f64 = 1e9;
pub const F32_DRIFT_GUARD_PT: f64 = 0.002;
pub const STYLE_EPSILON: f64 = 0.001;
pub const SIZE_MIN_PT: f64 = 4.0;
pub const SIZE_MAX_PT: f64 = 144.0;
pub const LETTER_SPACING_MIN_PT: f64 = -2.0;
pub const LETTER_SPACING_MAX_PT: f64 = 10.0;
pub const PDFTOTEXT_OUTPUT_MAX: usize = 16 << 20;
pub const WORD_BBOX_TOL_PT: f64 = 0.05;
pub const EDIT_BAND_PAD_PT: f64 = 2.0;
pub const RENDER_DPI_MAX: u32 = 96;
pub const RENDER_PIXELS_MAX: u64 = 25_000_000;
pub const RENDER_CHANNEL_TOL: u8 = 24;
pub const RENDER_OUTSIDE_PIXELS_MAX: u64 = 8;
pub const RENDER_MASK_PAD_PX: i64 = 4;
/// Pixels A5 lets change in the boxes of glyphs no edit changes, outside the edited glyphs' own
/// boxes, and pixels of a neighbour's ink it lets go under the masks (review-verify HIGH-A): an
/// honest edit changes none there (0 in every test and probe), and a narrow follower is smaller
/// than `RENDER_OUTSIDE_PIXELS_MAX` (a 12 pt period whitened changes 5 pixels at 96 DPI).
pub const RENDER_KEPT_PIXELS_MAX: u64 = 2;
/// Pixels the edited glyphs' own boxes grow by where A5 checks the glyphs no edit changes:
/// Poppler's anti-aliasing reaches one pixel past a glyph's outward-rounded box (every honest
/// change in the overlap sweeps of review-verify HIGH-A lay exactly 1 pixel out).
pub const RENDER_OWN_PAD_PX: i64 = 1;
pub const GATE_DECODED_TOTAL: u64 = 4 << 30; // decode budget of one gate run (A2 decodes only qpdf-compressed streams; A3/A4/B1–B3)
pub const PREVIEW_PDF_MAX_BYTES: u64 = 48 << 20;
/// A cached page model over this size leaves the cache for its preview, which frees it once the
/// edit is planned (the page's next visit builds it again): held through the preview, the source
/// model, the extracted page's model and Phase A's walks stacked past §H R20's 256 MiB per file
/// (review-final MEDIUM-3: 295 MiB for an 84 MiB model; released, about 2.5 × the model).
pub const PREVIEW_SHARED_MODEL_MAX: usize = 64 << 20;
pub const UPDATE_JSON_MAX_BYTES: usize = 128 << 20;
pub const SUBPROCESS_TIMEOUT_SECS: u64 = 120;
pub const CACHE_SNAPSHOTS_MAX: usize = 2;
pub const CACHE_SNAPSHOT_BYTES_MAX: u64 = 256 << 20; // larger files are re-read per call
pub const CACHE_PAGE_MODELS_MAX: usize = 32;
// revision 2
pub const VERIFY_CAP_MARGIN_BYTES: u64 = 256 << 20; // read_verification_snapshot cap = 2 × Σ inputs + margin
pub const QPDF_JSON_MAX_BYTES: usize = 64 << 20; // stdout cap of `qpdf --json=2 --json-key=pages`
pub const CHECK_MEMO_MAX: usize = 16; // fingerprints whose qpdf --check result is memoised
pub const HEAD_TAIL_HASH_BYTES: usize = 64 << 10; // stat_matches also hashes the first and last 64 KiB
pub const PAGE_TREE_DEPTH_MAX: usize = 64; // own page-tree walk for /Kids occurrence counts
pub const GRAPH_DIRECT_DEPTH_MAX: usize = 100; // canonical graph serialisation (= preflight nesting)
pub const STRUCT_ORDER_NODES_MAX: usize = 100_000; // structure-tree DFS for reading order
pub const XY_CUT_DEPTH_MAX: usize = 32;
pub const XY_CUT_ROW_GAP_EM: f64 = 0.5; // horizontal cut: empty band ≥ 0.5 × median effective size
pub const XY_CUT_COL_GAP_EM: f64 = 1.0; // vertical cut: empty gutter ≥ 1 × median effective size
pub const COLUMN_GAP_EM: f64 = 1.0; // a kept TJ gap ≥ 1 em absorbs the width change (B.12 step 9)
pub const WRAPPER_MATRIX_EPSILON: f64 = 1e-6; // Phase B: linear part of (wrapper cm × Form /Matrix) vs identity
pub const WRAPPER_TRANSLATION_TOL_PT: f64 = 0.001; // Phase B: its translation vs 0

// T1 implementation budgets (not in the §B.2 list; each bounds one T1 parser or reader)
pub const XREF_TAIL_SEARCH_BYTES: usize = 2 << 10; // `startxref` must sit in the last 2 KiB
pub const PREFLIGHT_DICT_TOKENS_MAX: usize = 100_000; // trailer / xref-stream dictionary tokens
pub const XREF_STREAM_FIELD_WIDTH_MAX: i64 = 8; // bytes per `/W` field of an xref stream
pub const TOOL_STDERR_MAX: usize = 1 << 20; // stderr kept from one subprocess (the rest is drained)
pub const TOOL_POLL_MS: u64 = 20; // subprocess wait loop period
pub const LEX_CANCEL_EVERY_OPS: usize = 4_096; // lexer checks the cancel flag this often
pub const INLINE_HEURISTIC_WINDOW: usize = 64; // bytes after a candidate `EI` that must lex cleanly
pub const INFLATE_STEP_BYTES: usize = 64 << 10; // output step of the capped inflater
pub const INLINE_HEURISTIC_CANDIDATES_MAX: usize = 256; // `EI` candidates tried per unproven inline image
pub const XREF_PREDICTOR_ROW_MAX: usize = 1 << 10; // PNG-predictor row of an xref stream (bytes)
pub const OBJECT_SCAN_FACTOR: usize = 2; // object value scans read ≤ 2 × the buffer …
pub const OBJECT_SCAN_SLACK_BYTES: usize = 1 << 20; // … + 1 MiB in total
pub const LENGTH_REF_STREAMS_MAX: usize = MAX_OBJECTS / 2; // streams whose /Length is a reference
pub const LENGTH_REF_CHAIN_MAX: usize = 32; // objects lopdf parses recursively for one stream's /Length
pub const OBJSTM_SCAN_BYTES: usize =
    OBJECT_SCAN_FACTOR * OBJSTM_TOTAL_DECODED + OBJECT_SCAN_SLACK_BYTES; // member scans of all object streams of one load

// T3 budget pass (not in the §B.2 list; see DEVIATIONS "[T3-budget]")
/// Bytes one page model build (`walk_page` + `build_runs`) may hold at once, and the model and its
/// #33 Classify pass together: every allocation that grows with the page (the font models it keeps
/// included) is charged before it is made, and the page is `PAGE_TOO_COMPLEX` "page model size"
/// past it (`walker/budget.rs`). A cached model stays within §H R20's 256 MiB per file; real pages
/// hold a few MiB. Measured (fix pass 2026-10-03, debug build): 100,000 one-glyph shows under one
/// state hold 86 MiB and classify in 22 MiB more; ~131,000 is the most (the records vector's
/// doubling at 131,072 needs room for both buffers); 3,000 lines × 33 glyphs with a `Tm` per glyph
/// hold 89 MiB, 112 MiB with a colour change per word.
pub const PAGE_MODEL_BYTES_MAX: usize = 160 << 20;
/// Bytes of font models the unused fonts of a page's resources (siblings for faces and joins)
/// may add to its model (fix pass 2026-10-03, review T3-budget HIGH-1): a font that does not fit
/// is left out, never the page refused. Drawn fonts are charged to the page budget in full.
pub const PAGE_UNUSED_FONT_BYTES_MAX: usize = 32 << 20;
/// Lexed operand nodes alive at once in one walk (the page's ops, plus a descended Form's while it
/// runs); also `LexLimits::operand_nodes_max`, so the lexer stops before it builds more. Six per
/// op at `PAGE_OPS_MAX` (`c` has six), so only long number arrays reach it.
pub const OPERAND_NODES_MAX: usize = 6 * PAGE_OPS_MAX;
/// Longest font resource name a sibling group may hold (ISO 32000 Annex C name limit). A longer
/// name types only its own runs: each run's surface lists every sibling's name.
pub const SIBLING_NAME_BYTES_MAX: usize = 127;
/// Longest unmodelled ExtGState key (Annex C again); longer is `PAGE_TOO_COMPLEX` "ExtGState
/// keys". Every `gs` compares the keys it sets with the ones in force.
pub const EXTGSTATE_KEY_BYTES_MAX: usize = 127;

/// The source-file cap: `FILE_CAP_BYTES` in production; tests may override it per thread
/// (`set_file_cap_override`, used by E2E-21 to exercise the verification cap).
pub fn file_cap() -> u64 {
    #[cfg(test)]
    if let Some(cap) = FILE_CAP_OVERRIDE.with(|c| c.get()) {
        return cap;
    }
    FILE_CAP_BYTES
}

/// `PAGE_MODEL_BYTES_MAX` in production; tests may override it per thread
/// (`set_model_bytes_override`) to reach the budget with small pages.
pub fn model_bytes_max() -> usize {
    #[cfg(test)]
    if let Some(bytes) = MODEL_BYTES_OVERRIDE.with(|c| c.get()) {
        return bytes;
    }
    PAGE_MODEL_BYTES_MAX
}

/// Bytes all object-stream member scans of one load may read: `OBJSTM_SCAN_BYTES` in
/// production; tests may override it per thread (`set_objstm_scan_override`; `guarded_load` reads
/// it on the calling thread before lopdf's workers start).
pub fn objstm_scan_budget() -> usize {
    #[cfg(test)]
    if let Some(bytes) = OBJSTM_SCAN_OVERRIDE.with(|c| c.get()) {
        return bytes;
    }
    OBJSTM_SCAN_BYTES
}

#[cfg(test)]
thread_local! {
    static FILE_CAP_OVERRIDE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static OBJSTM_SCAN_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static MODEL_BYTES_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Test seam: override `model_bytes_max()` on the current thread (`None` restores the constant).
#[cfg(test)]
pub fn set_model_bytes_override(bytes: Option<usize>) {
    MODEL_BYTES_OVERRIDE.with(|c| c.set(bytes));
}

/// Test seam: override `file_cap()` on the current thread (`None` restores the constant).
#[cfg(test)]
pub fn set_file_cap_override(cap: Option<u64>) {
    FILE_CAP_OVERRIDE.with(|c| c.set(cap));
}

/// Test seam: override `objstm_scan_budget()` on the current thread (`None` restores the constant).
#[cfg(test)]
pub fn set_objstm_scan_override(bytes: Option<usize>) {
    OBJSTM_SCAN_OVERRIDE.with(|c| c.set(bytes));
}
