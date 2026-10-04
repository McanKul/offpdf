//! Generic token scanner (objects, CMaps, Type1 clear text) used by the snapshot preflight and
//! the font parsers: flat tokens, balanced brackets, no arity.

use super::{malformed, LexError, LexLimits, Raw, Reader, ScanMode, Token};

/// Content limits without the operand-node cap (`max_tokens` bounds a scan).
fn scan_limits() -> LexLimits {
    LexLimits {
        operand_nodes_max: usize::MAX,
        ..LexLimits::page()
    }
}

pub(super) fn tokens(
    bytes: &[u8],
    mode: ScanMode,
    max_tokens: usize,
) -> Result<Vec<Token>, LexError> {
    let lim = scan_limits();
    let mut r = Reader::new(bytes, false, mode != ScanMode::Object, &lim);
    let mut out = Vec::new();
    let mut depth: Vec<bool> = Vec::new(); // true = dict
    while let Some(raw) = r.next()? {
        if out.len() >= max_tokens {
            return Err(LexError::TooComplex { what: "tokens" });
        }
        out.push(flat_token(raw, &mut depth, &r, &lim)?);
    }
    if !depth.is_empty() {
        return malformed(bytes.len(), "unbalanced brackets");
    }
    Ok(out)
}

pub(super) fn dict_at(
    bytes: &[u8],
    start: usize,
    max_tokens: usize,
) -> Result<(Vec<Token>, usize), LexError> {
    let lim = scan_limits();
    let mut r = Reader::new(bytes, false, false, &lim);
    r.pos = start;
    let mut out = Vec::new();
    let mut depth: Vec<bool> = Vec::new();
    loop {
        let raw = r.next()?.ok_or(LexError::Malformed {
            at: r.pos,
            what: "unterminated dictionary",
        })?;
        if out.is_empty() && !matches!(raw, Raw::DictOpen(_)) {
            return malformed(raw.start(), "dictionary expected");
        }
        if out.len() >= max_tokens {
            return Err(LexError::TooComplex { what: "tokens" });
        }
        out.push(flat_token(raw, &mut depth, &r, &lim)?);
        if depth.is_empty() {
            return Ok((out, r.pos));
        }
    }
}

fn flat_token(
    raw: Raw,
    depth: &mut Vec<bool>,
    r: &Reader<'_>,
    lim: &LexLimits,
) -> Result<Token, LexError> {
    let mut push = |dict: bool| {
        if depth.len() >= lim.nesting_max {
            return Err(LexError::TooComplex { what: "nesting" });
        }
        depth.push(dict);
        Ok(())
    };
    Ok(match raw {
        Raw::Operand(o) => Token::Operand(o),
        Raw::Word(span) => Token::Keyword {
            bytes: r.b.get(span.clone()).unwrap_or_default().to_vec(),
            span,
        },
        Raw::DictOpen(p) => {
            push(true)?;
            Token::DictOpen(p)
        }
        Raw::ArrayOpen(p) => {
            push(false)?;
            Token::ArrayOpen(p)
        }
        Raw::DictClose(p) => match depth.pop() {
            Some(true) => Token::DictClose(p),
            _ => return malformed(p, "stray >>"),
        },
        Raw::ArrayClose(p) => match depth.pop() {
            Some(false) => Token::ArrayClose(p),
            _ => return malformed(p, "stray ]"),
        },
        Raw::BraceOpen(p) | Raw::BraceClose(p) if r.lenient => Token::Keyword {
            bytes: r.b.get(p..p + 1).unwrap_or_default().to_vec(),
            span: p..p + 1,
        },
        Raw::BraceOpen(p) | Raw::BraceClose(p) => return malformed(p, "brace outside a procedure"),
    })
}
