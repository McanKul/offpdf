//! T1 IO tests (SPEC §E.3): LEX (`tests_io/lex.rs`), DEC (here), SNAP (+ the preflight
//! hardening in `tests_io/preflight.rs` and `tests_io/xref.rs`), CON and ENG.
//! Test IDs appear in test names and assertion messages.

mod con;
mod eng;
pub(crate) mod lex;
mod preflight;
mod snap;
mod xref;

use crate::pdf_engine::text_edit::decode::{
    ascii85_decode, ascii_hex_decode, decode_stream, inflate_capped, inflate_end, DecodeBudget,
    DecodeError,
};
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::testkit::pdf::{zlib, zlib_zero_bomb};
use crate::pdf_engine::text_edit::testkit::thread_peak;
use lopdf::{Dictionary, Object, Stream};

fn stream(filter: Option<Object>, parms: Option<Object>, content: Vec<u8>) -> Stream {
    let mut d = Dictionary::new();
    if let Some(f) = filter {
        d.set("Filter", f);
    }
    if let Some(p) = parms {
        d.set("DecodeParms", p);
    }
    Stream::new(d, content)
}

fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
}

fn decode(s: &Stream) -> Result<Vec<u8>, DecodeError> {
    decode_stream(
        s,
        limits::STREAM_MAX_DECODED,
        &mut DecodeBudget::new(usize::MAX),
    )
}

fn text(n: usize) -> Vec<u8> {
    let line = b"BT /F1 12 Tf (Hello World) Tj ET\n";
    (0..n).map(|i| line[i % line.len()]).collect()
}

#[test]
fn dec01_flate_round_trip() {
    let plain = text(200_000);
    assert_eq!(
        inflate_capped(&zlib(&plain), 1 << 20).unwrap(),
        plain,
        "DEC-01"
    );
    assert_eq!(
        decode(&stream(Some(name("FlateDecode")), None, zlib(&plain))).unwrap(),
        plain,
        "DEC-01 via stream"
    );
    assert_eq!(
        decode(&stream(Some(name("Fl")), None, zlib(b"q Q"))).unwrap(),
        b"q Q",
        "DEC-01 abbreviation"
    );
    let z = zlib(b"abc");
    assert_eq!(
        inflate_end(&z, 100).unwrap(),
        z.len() - 4,
        "DEC-01 consumed input excludes the Adler-32"
    );
    assert_eq!(
        inflate_capped(&zlib(&plain), plain.len()).unwrap().len(),
        plain.len(),
        "cap is inclusive"
    );
    assert_eq!(
        inflate_capped(&zlib(&plain), plain.len() - 1),
        Err(DecodeError::TooLarge)
    );
}

#[test]
fn dec02_truncated_flate_is_corrupt() {
    let z = zlib(&text(100_000));
    let cut = &z[..z.len() * 55 / 100];
    assert_eq!(
        inflate_capped(cut, 1 << 20),
        Err(DecodeError::Corrupt("truncated flate")),
        "DEC-02"
    );
    assert_eq!(
        inflate_capped(&z[..1], 100),
        Err(DecodeError::Corrupt("truncated flate"))
    );
}

#[test]
fn dec03_bomb_is_too_large_with_bounded_peak() {
    let bomb = zlib_zero_bomb(1024); // ≈1 MiB in, 1 GiB out
    assert!(bomb.len() < 2 << 20);
    let cap = 1 << 20;
    let (result, peak) = thread_peak(|| inflate_capped(&bomb, cap));
    assert_eq!(result, Err(DecodeError::TooLarge), "DEC-03");
    assert!(
        peak <= cap + (64 << 10),
        "DEC-03 peak {peak} ≤ cap + 64 KiB"
    );
    let big_cap = 16 << 20;
    let (result, peak) = thread_peak(|| inflate_capped(&bomb, big_cap));
    assert_eq!(result, Err(DecodeError::TooLarge));
    assert!(peak <= big_cap + (64 << 10), "DEC-03 peak {peak}");
    let z = zlib(&text(4 << 20));
    let (ok, peak) = thread_peak(|| inflate_capped(&z, 8 << 20));
    let n = ok.unwrap().len();
    assert!(
        peak <= n + (256 << 10),
        "DEC-03 a successful decode holds the output once: {peak}"
    );
}

#[test]
fn dec04_bad_adler_after_stream_end_is_ok() {
    let mut z = zlib(b"BT (x) Tj ET");
    let n = z.len();
    z[n - 1] ^= 0xFF;
    z.extend_from_slice(b"\r\n trailing garbage");
    assert_eq!(inflate_capped(&z, 100).unwrap(), b"BT (x) Tj ET", "DEC-04");
}

#[test]
fn dec05_flate_over_plain_bytes_is_corrupt() {
    assert_eq!(
        inflate_capped(b"BT (x) Tj ET", 100),
        Err(DecodeError::Corrupt("bad zlib header")),
        "DEC-05"
    );
    assert_eq!(
        inflate_capped(&[0x78, 0x9C, 0xFF, 0xFF, 0xFF], 100),
        Err(DecodeError::Corrupt("flate data error"))
    );
    assert_eq!(
        inflate_capped(&[0x78, 0xBB, 0, 0, 0, 0], 100),
        Err(DecodeError::Corrupt("bad zlib header")),
        "FDICT"
    );
}

#[test]
fn dec06_empty_input_is_empty_output() {
    assert_eq!(inflate_capped(b"", 0).unwrap(), b"", "DEC-06");
    assert_eq!(
        decode(&stream(Some(name("FlateDecode")), None, Vec::new())).unwrap(),
        b""
    );
    assert_eq!(
        decode(&stream(None, None, b"q Q".to_vec())).unwrap(),
        b"q Q",
        "no filter"
    );
    assert_eq!(
        decode(&stream(
            Some(Object::Array(vec![])),
            Some(Object::Null),
            b"q".to_vec()
        ))
        .unwrap(),
        b"q"
    );
}

#[test]
fn dec07_filter_chains() {
    let plain = b"BT /F1 12 Tf (chain) Tj ET".to_vec();
    let hex: Vec<u8> = zlib(&plain)
        .iter()
        .flat_map(|b| format!("{b:02X} ").into_bytes())
        .chain(*b">")
        .collect();
    let chain = stream(
        Some(Object::Array(vec![name("AHx"), name("Fl")])),
        None,
        hex.clone(),
    );
    assert_eq!(decode(&chain).unwrap(), plain, "DEC-07 [/AHx /Fl]");
    let long = stream(Some(Object::Array(vec![name("AHx"); 5])), None, hex);
    assert!(
        matches!(decode(&long), Err(DecodeError::UnsupportedFilter(_))),
        "DEC-07 more than 4 filters"
    );
    let mut budget = DecodeBudget::new(10);
    let s = stream(None, None, text(11));
    assert_eq!(
        decode_stream(&s, 100, &mut budget),
        Err(DecodeError::TooLarge),
        "DEC-07 budget is a cap"
    );
    assert_eq!(budget.remaining(), 10);
    assert!(budget.take(4).is_ok());
    assert_eq!(budget.take(7), Err(DecodeError::TooLarge));
    assert_eq!(budget.remaining(), 6);
}

#[test]
fn dec08_ascii_decoder_edges() {
    assert_eq!(
        ascii_hex_decode(b"48 65\n6C6c6F>", 100).unwrap(),
        b"Hello",
        "DEC-08 AHx"
    );
    assert_eq!(
        ascii_hex_decode(b"414", 100).unwrap(),
        [0x41, 0x40],
        "DEC-08 odd digit, missing EOD"
    );
    assert_eq!(
        ascii_hex_decode(b"41 >junk", 100).unwrap(),
        b"A",
        "DEC-08 data after EOD ignored"
    );
    assert_eq!(
        ascii_hex_decode(b"4X", 100),
        Err(DecodeError::Corrupt("bad hex digit"))
    );
    assert_eq!(ascii_hex_decode(b"414243", 2), Err(DecodeError::TooLarge));
    assert_eq!(
        ascii85_decode(b"87cURD]i,\"Ebo80~>", 100).unwrap(),
        b"Hello World!",
        "DEC-08 A85"
    );
    assert_eq!(
        ascii85_decode(b"<~87cURD]i,\"Ebo80~>", 100).unwrap(),
        b"Hello World!",
        "DEC-08 <~ prefix"
    );
    assert_eq!(
        ascii85_decode(b"z 8\n7cU~>", 100).unwrap(),
        [0, 0, 0, 0, b'H', b'e', b'l'],
        "DEC-08 z + partial"
    );
    assert_eq!(
        ascii85_decode(b"87cURD]i~", 100),
        Err(DecodeError::Corrupt("bad ASCII85 end"))
    );
    assert_eq!(
        ascii85_decode(b"8~>", 100),
        Err(DecodeError::Corrupt("bad ASCII85 final group"))
    );
    assert_eq!(
        ascii85_decode(b"s8W-\"~>", 100),
        Err(DecodeError::Corrupt("ASCII85 group overflow"))
    );
    assert_eq!(
        ascii85_decode(b"87czU~>", 100),
        Err(DecodeError::Corrupt("bad ASCII85 character")),
        "z mid-group"
    );
    assert_eq!(ascii85_decode(b"zz", 7), Err(DecodeError::TooLarge));
}

#[test]
fn dec09_unsupported_filters_and_parameters() {
    let unsupported = |s: Stream| matches!(decode(&s), Err(DecodeError::UnsupportedFilter(_)));
    for f in [
        "LZWDecode",
        "RunLengthDecode",
        "RL",
        "DCTDecode",
        "JPXDecode",
        "JBIG2Decode",
        "CCITTFaxDecode",
        "Crypt",
        "Bogus",
    ] {
        assert!(
            unsupported(stream(Some(name(f)), None, b"x".to_vec())),
            "DEC-09 {f}"
        );
    }
    let mut parms = Dictionary::new();
    parms.set("Predictor", 12);
    parms.set("Columns", 5);
    assert!(
        unsupported(stream(
            Some(name("FlateDecode")),
            Some(Object::Dictionary(parms.clone())),
            zlib(b"x")
        )),
        "DEC-09 predictor"
    );
    let arr = Object::Array(vec![Object::Null, Object::Dictionary(parms)]);
    assert!(unsupported(stream(
        Some(Object::Array(vec![name("AHx"), name("Fl")])),
        Some(arr),
        b"x".to_vec()
    )));
    let mut one = Dictionary::new();
    one.set("Predictor", 1);
    assert!(decode(&stream(
        Some(name("FlateDecode")),
        Some(Object::Dictionary(one)),
        zlib(b"ok")
    ))
    .is_ok());
    let mut ext = stream(None, None, b"x".to_vec());
    ext.dict.set(
        "F",
        Object::String(b"file.bin".to_vec(), lopdf::StringFormat::Literal),
    );
    assert!(unsupported(ext), "DEC-09 /F external file");
    assert!(
        unsupported(stream(Some(Object::Integer(3)), None, b"x".to_vec())),
        "malformed /Filter"
    );
    assert_eq!(
        DecodeError::UnsupportedFilter("x".into()).page_reason(),
        TextReason::UnsupportedFilter
    );
    assert_eq!(
        DecodeError::Corrupt("x").page_reason(),
        TextReason::MalformedContent
    );
    assert_eq!(
        DecodeError::TooLarge.page_reason(),
        TextReason::PageTooComplex
    );
}

#[test]
fn limits_are_consistent() {
    use limits::*;
    assert!(
        STREAM_MAX_DECODED <= PAGE_CONTENT_MAX_DECODED
            && PAGE_CONTENT_MAX_DECODED <= PAGE_DECODE_BUDGET
    );
    assert!(
        OBJSTM_MAX_DECODED <= OBJSTM_TOTAL_DECODED
            && XREF_STREAM_MAX_DECODED as u64 <= FILE_CAP_BYTES
    );
    assert!(MAX_OBJECT_NESTING == GRAPH_DIRECT_DEPTH_MAX && TOKEN_NESTING_MAX < MAX_OBJECT_NESTING);
    assert!(
        Q_DEPTH_MAX == 64
            && MARKED_DEPTH_MAX == 64
            && FORM_DEPTH_MAX == 8
            && PAGE_TREE_DEPTH_MAX == 64
    );
    assert!(EDITS_PER_PAGE_MAX < EDITS_PER_SAVE_MAX && EDIT_TEXT_CHARS_MAX == 1_000);
    assert!(RUN_MEMBERS_MAX <= RUNS_PER_PAGE_MAX && RUNS_PER_PAGE_MAX <= GLYPHS_PER_PAGE_MAX);
    assert!(
        FORM_PAINTS_PER_PAGE_MAX > 0
            && FONTS_PER_PAGE_MAX > 0
            && PAGE_PARTS_MAX == 256
            && PAGE_OPS_MAX == 250_000
    );
    assert!(
        FONT_PROGRAM_MAX_DECODED <= STREAM_MAX_DECODED
            && TOUNICODE_MAX_DECODED < FONT_PROGRAM_MAX_DECODED
    );
    assert!(
        CMAP_MAPPINGS_MAX == CIDTOGID_MAX_BYTES
            && W_ENTRIES_MAX == ARRAY_ITEMS_MAX
            && STRUCT_CHAIN_MAX == 16
    );
    assert!(NUMBER_TREE_NODES_MAX == STRUCT_ORDER_NODES_MAX && XY_CUT_DEPTH_MAX == 32);
    assert!(STRING_BYTES_MAX < INLINE_IMAGE_MAX_BYTES && LITERAL_PAREN_NESTING_MAX == 100);
    // Preflight budgets (review-T1 fix pass): a predictor row fits the widest legal xref row,
    // a /Length chain stays far below the ~200 objects that overflowed a debug-build rayon
    // stack in the probe, and the object scans read at least every byte once.
    assert!(
        XREF_PREDICTOR_ROW_MAX >= 3 * XREF_STREAM_FIELD_WIDTH_MAX as usize
            && LENGTH_REF_CHAIN_MAX * 4 <= 200
            && LENGTH_REF_STREAMS_MAX <= MAX_OBJECTS
            && OBJECT_SCAN_FACTOR >= 1
            && INLINE_HEURISTIC_CANDIDATES_MAX > 0
    );
    assert!(
        DRIFT_TOLERANCE_PT == 0.01
            && JOIN_BASELINE_TOL_PT == DRIFT_TOLERANCE_PT
            && STATE_EPSILON < COLOR_EPSILON
    );
    assert!(
        AXIS_EPSILON_REL == 1e-4 && SHEAR_MAX == 0.5 && JOIN_GAP_EM == 0.3 && SYNTH_SPACE_EM == 0.2
    );
    assert!(DEFAULT_KERN_SPACE == -250.0 && PER_GLYPH_MIN_RUNS == 12 && PER_GLYPH_SHARE == 0.8);
    assert!(
        DUPLICATE_OVERLAP_SHARE == 0.5
            && CLIP_CONTAIN_TOL_PT == OVERLAP_WARN_TOL_PT
            && NEGLIGIBLE_KERN > 0.0
    );
    assert!(
        NUMBER_DECIMALS == 4 && NUMBER_ABS_MAX == 1e9 && F32_DRIFT_GUARD_PT < DRIFT_TOLERANCE_PT
    );
    assert!(
        STYLE_EPSILON == 0.001
            && SIZE_MIN_PT < SIZE_MAX_PT
            && LETTER_SPACING_MIN_PT < LETTER_SPACING_MAX_PT
    );
    assert!(PDFTOTEXT_OUTPUT_MAX <= QPDF_JSON_MAX_BYTES && WORD_BBOX_TOL_PT < EDIT_BAND_PAD_PT);
    assert!(
        RENDER_DPI_MAX == 96
            && RENDER_PIXELS_MAX > 0
            && RENDER_CHANNEL_TOL == 24
            && RENDER_OUTSIDE_PIXELS_MAX == 8
    );
    assert!(
        RENDER_MASK_PAD_PX == 4 && GATE_DECODED_TOTAL > FILE_CAP_BYTES && PREVIEW_PDF_MAX_BYTES > 0
    );
    assert!(UPDATE_JSON_MAX_BYTES > STREAM_MAX_DECODED && SUBPROCESS_TIMEOUT_SECS == 120);
    assert!(
        CACHE_SNAPSHOTS_MAX == 2
            && CACHE_SNAPSHOT_BYTES_MAX < FILE_CAP_BYTES
            && CACHE_PAGE_MODELS_MAX == 32
    );
    assert!(
        VERIFY_CAP_MARGIN_BYTES == 256 << 20
            && CHECK_MEMO_MAX == 16
            && HEAD_TAIL_HASH_BYTES == 64 << 10
    );
    assert!(XY_CUT_ROW_GAP_EM < XY_CUT_COL_GAP_EM && COLUMN_GAP_EM == 1.0);
    assert!(
        WRAPPER_MATRIX_EPSILON < WRAPPER_TRANSLATION_TOL_PT
            && MAX_OBJECTS == 2_000_000
            && MAX_XREF_CHAIN == 64
    );
    assert!(
        XREF_TAIL_SEARCH_BYTES == 2048
            && PREFLIGHT_DICT_TOKENS_MAX > 0
            && XREF_STREAM_FIELD_WIDTH_MAX == 8
    );
    assert!(TOOL_STDERR_MAX > 0 && TOOL_POLL_MS == 20 && LEX_CANCEL_EVERY_OPS == 4_096);
    assert!(INLINE_HEURISTIC_WINDOW == 64 && INFLATE_STEP_BYTES == 64 << 10);
    assert_eq!(file_cap(), FILE_CAP_BYTES);
    set_file_cap_override(Some(10));
    assert_eq!(file_cap(), 10);
    set_file_cap_override(None);
    assert_eq!(file_cap(), FILE_CAP_BYTES);
}
