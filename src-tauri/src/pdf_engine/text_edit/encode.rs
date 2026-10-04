//! Bytes of a replacement (SPEC §B.12): numbers written with at most four decimals and read back
//! (every later computation uses the value a viewer will parse), uppercase hex strings of codes,
//! and the replacement grammar (§A.4) that every replacement is checked against twice — before any
//! IO (`rewrite`) and on the re-walk (`verify`).

use crate::pdf_engine::text_edit::fonts::Code;
use crate::pdf_engine::text_edit::lexer::{
    is_delimiter, is_whitespace, lex_content, LexLimits, Operator,
};
use crate::pdf_engine::text_edit::limits::{NUMBER_ABS_MAX, NUMBER_DECIMALS};
use crate::pdf_engine::text_edit::reasons::{EditProblem, EditProblemCode};

/// The operators a replacement may contain (§A.4): `Tf Tc Tw T* TJ` plus the fill operators a
/// verbatim restore or the new colour can need (`g rg k cs sc scn`).
pub const REPLACEMENT_OPERATORS: [Operator; 11] = [
    Operator::Tf,
    Operator::Tc,
    Operator::Tw,
    Operator::TStar,
    Operator::TJ,
    Operator::g,
    Operator::rg,
    Operator::k,
    Operator::cs,
    Operator::sc,
    Operator::scn,
];

/// A number as written in a replacement: ≤ `NUMBER_DECIMALS` decimals, trailing zeros and dot
/// stripped, `-0` → `0`, never an exponent, `|v| ≤ NUMBER_ABS_MAX`.
pub fn fmt_num(v: f64) -> Result<String, EditProblem> {
    if !v.is_finite() || v.abs() > NUMBER_ABS_MAX {
        return Err(EditProblem::new(
            EditProblemCode::EditVerifyFailed,
            Some(format!("number out of range: {v}")),
        ));
    }
    let mut s = format!("{v:.prec$}", prec = NUMBER_DECIMALS);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" || s.is_empty() {
        s = "0".to_string();
    }
    Ok(s)
}

/// `fmt_num` and the value a reader parses back from it.
pub fn num(v: f64) -> Result<(String, f64), EditProblem> {
    let s = fmt_num(v)?;
    let parsed = s.parse::<f64>().map_err(|_| {
        EditProblem::new(
            EditProblemCode::EditVerifyFailed,
            Some(format!("number does not read back: {s}")),
        )
    })?;
    Ok((s, parsed))
}

/// `<HEX>` of `codes` (uppercase, each code big-endian in `code.len` bytes); `<>` when empty.
pub fn hex_codes(codes: &[Code]) -> String {
    let mut out = String::with_capacity(2 + codes.len() * 4);
    out.push('<');
    for code in codes {
        let len = usize::from(code.len).min(4);
        let bytes = code.value.to_be_bytes();
        for b in bytes.get(4 - len..).unwrap_or_default() {
            out.push_str(&format!("{b:02X}"));
        }
    }
    out.push('>');
    out
}

/// Whether a replacement written right after `prev` must start with a space so that its first
/// token cannot fuse with the previous one (`prev` is the byte before the splice, `None` at the
/// start of a part, where the previous part's last byte is unknown to a reader that concatenates
/// parts).
pub fn needs_leading_space(prev: Option<u8>, replacement: &[u8]) -> bool {
    let regular = |c: u8| !is_whitespace(c) && !is_delimiter(c);
    match (prev, replacement.first()) {
        (_, None) => false,
        (None, Some(c)) => regular(*c),
        (Some(p), Some(c)) => regular(p) && regular(*c),
    }
}

/// The replacement grammar (§A.4): the bytes lex completely, every operator is one of
/// `REPLACEMENT_OPERATORS`, and there are exactly `expected_tj` `TJ` operators.
pub fn check_replacement_grammar(bytes: &[u8], expected_tj: usize) -> Result<(), &'static str> {
    let ops =
        lex_content(bytes, &LexLimits::page(), None).map_err(|_| "replacement does not lex")?;
    let mut tj = 0usize;
    for op in &ops {
        if !REPLACEMENT_OPERATORS.contains(&op.operator) || op.in_compat {
            return Err(op.operator.as_str());
        }
        if op.operator == Operator::TJ {
            tj = tj.saturating_add(1);
        }
    }
    if tj != expected_tj {
        return Err("TJ count");
    }
    Ok(())
}
