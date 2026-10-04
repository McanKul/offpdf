//! Byte-offset content tokenizer (SPEC §B.6). Iterative, bounded, full consumption: every op
//! carries the exact byte span it was read from, trailing operands or stray delimiters are
//! errors (never a silently truncated prefix), and inline-image payload ends are proven
//! (§A.4) or marked unproven for every op that follows.
//!
//! Operand nodes are counted as they are read (`LexLimits::operand_nodes_max`), and every value
//! is kept at exactly its size (strings, names, arrays, dictionaries and an op's operand list
//! hold no spare room), so a lexed page costs `nodes × size_of::<Operand>()` plus the bytes it
//! copied, which the walker's page-model budget can bound before it lexes.

use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::TextReason;
use std::sync::atomic::{AtomicBool, Ordering};

mod arity;
mod inline;
mod scan;
mod values;

pub use arity::check_arity;
pub(crate) use inline::InlineEnds;
use values::Builder;

/// `[start, end)` in the buffer that was lexed.
pub type Span = std::ops::Range<usize>;

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Number {
        value: f64,
        span: Span,
    }, // [+-]?(\d+\.?\d*|\.\d+); |x| ≤ 1e9 in content
    Name {
        bytes: Vec<u8>,
        span: Span,
    }, // #xx decoded
    Str {
        bytes: Vec<u8>,
        hex: bool,
        span: Span,
    }, // escapes decoded; CR/CRLF → LF
    Array {
        items: Vec<Operand>,
        span: Span,
    },
    Dict {
        entries: Vec<(Vec<u8>, Operand)>,
        span: Span,
    }, // only as operands of BDC/DP and inside BI
    Bool {
        value: bool,
        span: Span,
    },
    Null {
        span: Span,
    },
}

impl Operand {
    pub fn span(&self) -> &Span {
        match self {
            Operand::Number { span, .. }
            | Operand::Name { span, .. }
            | Operand::Str { span, .. }
            | Operand::Array { span, .. }
            | Operand::Dict { span, .. }
            | Operand::Bool { span, .. }
            | Operand::Null { span } => span,
        }
    }
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Operand::Number { value, .. } => Some(*value),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Operand::Name { bytes, .. } => Some(bytes),
            _ => None,
        }
    }
    pub fn as_str_bytes(&self) -> Option<&[u8]> {
        match self {
            Operand::Str { bytes, .. } => Some(bytes),
            _ => None,
        }
    }
}

#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)] // variants mirror the operator spelling; `b`/`B`, `f`/`F` … would collide in CamelCase
pub enum Operator {
    b, B, bStar, BStar, BDC, BI, BMC, BT, BX, c, cm, CS, cs, d, d0, d1, Do, DP, EI, EMC, ET, EX,
    f, F, fStar, G, g, gs, h, i, ID, j, J, K, k, l, m, M, MP, n, q, Q, re, RG, rg, ri, s, S, SC, sc,
    SCN, scn, sh, TStar, Tc, Td, TD, Tf, Tj, TJ, TL, Tm, Tr, Ts, Tw, Tz, v, w, W, WStar, y,
    Quote, DoubleQuote,
    Unknown, // only legal inside BX … EX
}

use Operator as O;

#[rustfmt::skip]
const OPERATORS: [(Operator, &str); 73] = [
    (O::b, "b"), (O::B, "B"), (O::bStar, "b*"), (O::BStar, "B*"), (O::BDC, "BDC"), (O::BI, "BI"),
    (O::BMC, "BMC"), (O::BT, "BT"), (O::BX, "BX"), (O::c, "c"), (O::cm, "cm"), (O::CS, "CS"),
    (O::cs, "cs"), (O::d, "d"), (O::d0, "d0"), (O::d1, "d1"), (O::Do, "Do"), (O::DP, "DP"),
    (O::EI, "EI"), (O::EMC, "EMC"), (O::ET, "ET"), (O::EX, "EX"), (O::f, "f"), (O::F, "F"),
    (O::fStar, "f*"), (O::G, "G"), (O::g, "g"), (O::gs, "gs"), (O::h, "h"), (O::i, "i"),
    (O::ID, "ID"), (O::j, "j"), (O::J, "J"), (O::K, "K"), (O::k, "k"), (O::l, "l"), (O::m, "m"),
    (O::M, "M"), (O::MP, "MP"), (O::n, "n"), (O::q, "q"), (O::Q, "Q"), (O::re, "re"),
    (O::RG, "RG"), (O::rg, "rg"), (O::ri, "ri"), (O::s, "s"), (O::S, "S"), (O::SC, "SC"),
    (O::sc, "sc"), (O::SCN, "SCN"), (O::scn, "scn"), (O::sh, "sh"), (O::TStar, "T*"),
    (O::Tc, "Tc"), (O::Td, "Td"), (O::TD, "TD"), (O::Tf, "Tf"), (O::Tj, "Tj"), (O::TJ, "TJ"),
    (O::TL, "TL"), (O::Tm, "Tm"), (O::Tr, "Tr"), (O::Ts, "Ts"), (O::Tw, "Tw"), (O::Tz, "Tz"),
    (O::v, "v"), (O::w, "w"), (O::W, "W"), (O::WStar, "W*"), (O::y, "y"), (O::Quote, "'"),
    (O::DoubleQuote, "\""),
];

impl Operator {
    pub fn from_token(t: &[u8]) -> Operator {
        OPERATORS
            .iter()
            .find(|(_, s)| s.as_bytes() == t)
            .map(|(o, _)| *o)
            .unwrap_or(O::Unknown)
    }
    pub fn as_str(self) -> &'static str {
        OPERATORS
            .iter()
            .find(|(o, _)| *o == self)
            .map(|(_, s)| *s)
            .unwrap_or("?")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineProof {
    LengthKey,
    UnfilteredSize,
    FlateEnd,
    AsciiEnd,
    Heuristic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineImage {
    pub dict: Vec<(Vec<u8>, Operand)>,
    pub data: Span,
    pub proof: InlineProof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Op {
    pub operator: Operator,
    pub operands: Vec<Operand>,
    pub span: Span, // first operand start (after whitespace/comments) or operator start .. operator end; BI: .. end of "EI"
    pub op_span: Span, // the operator token itself
    pub in_compat: bool, // inside BX … EX
    pub after_unproven_inline_image: bool,
    pub inline_image: Option<InlineImage>,
}

pub struct LexLimits {
    pub ops_max: usize,
    pub nesting_max: usize,
    pub array_items_max: usize,
    pub string_bytes_max: usize,
    pub paren_nesting_max: usize,
    pub inline_image_max: usize,
    /// Operand nodes (numbers, names, strings, booleans, nulls, arrays, dictionaries — nested
    /// ones, dictionary keys and inline-image values included) one lex may build, counted as each
    /// is read: past it the lex is `TooComplex { what: OPERAND_NODES }` before the next node is
    /// kept (48 MiB of `1 1 1 …` would otherwise build ~24 M nodes, ≈ 1.2 GiB, review T3 r3
    /// MEDIUM-3). The walker lowers it to what the page may still hold.
    pub operand_nodes_max: usize,
}

/// `LexError::TooComplex` detail of the operand-node cap.
pub const OPERAND_NODES: &str = "operand nodes";

impl LexLimits {
    pub fn page() -> Self {
        LexLimits {
            ops_max: limits::PAGE_OPS_MAX,
            nesting_max: limits::TOKEN_NESTING_MAX,
            array_items_max: limits::ARRAY_ITEMS_MAX,
            string_bytes_max: limits::STRING_BYTES_MAX,
            paren_nesting_max: limits::LITERAL_PAREN_NESTING_MAX,
            inline_image_max: limits::INLINE_IMAGE_MAX_BYTES,
            operand_nodes_max: limits::OPERAND_NODES_MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexError {
    Malformed {
        at: usize,
        what: &'static str,
    },
    TooComplex {
        what: &'static str,
    },
    /// The caller's cancel flag was set (checked every `LEX_CANCEL_EVERY_OPS` ops).
    Cancelled,
}

impl LexError {
    /// MALFORMED_CONTENT | PAGE_TOO_COMPLEX (a cancelled lex is never shown as a reason).
    pub fn page_reason(&self) -> TextReason {
        match self {
            LexError::Malformed { .. } => TextReason::MalformedContent,
            LexError::TooComplex { .. } | LexError::Cancelled => TextReason::PageTooComplex,
        }
    }
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LexError::Malformed { at, what } => write!(f, "malformed content at byte {at}: {what}"),
            LexError::TooComplex { what } => write!(f, "content too complex: {what}"),
            LexError::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    Object,
    CMap,
    Type1Clear,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Operand(Operand),
    Keyword { bytes: Vec<u8>, span: Span },
    DictOpen(usize),
    DictClose(usize),
    ArrayOpen(usize),
    ArrayClose(usize),
}

fn malformed<T>(at: usize, what: &'static str) -> Result<T, LexError> {
    Err(LexError::Malformed { at, what })
}

pub(crate) fn is_whitespace(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

pub(crate) fn is_delimiter(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(c: u8) -> bool {
    !is_whitespace(c) && !is_delimiter(c)
}

/// Validates `[+-]?(\d+\.?\d*|\.\d+)` and parses it from the exact digits.
fn parse_number(t: &[u8]) -> Option<f64> {
    let (neg, body) = match t.first() {
        Some(b'+') => (false, t.get(1..)?),
        Some(b'-') => (true, t.get(1..)?),
        _ => (false, t),
    };
    let dot = body.iter().position(|c| *c == b'.');
    let (int, frac) = match dot {
        Some(p) => (body.get(..p)?, body.get(p + 1..)?),
        None => (body, &b""[..]),
    };
    if (int.is_empty() && frac.is_empty()) || !int.iter().chain(frac).all(u8::is_ascii_digit) {
        return None;
    }
    let text = format!(
        "{}{}.{}",
        if neg { "-" } else { "" },
        if int.is_empty() {
            "0"
        } else {
            std::str::from_utf8(int).ok()?
        },
        if frac.is_empty() {
            "0"
        } else {
            std::str::from_utf8(frac).ok()?
        }
    );
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[derive(Debug)]
enum Raw {
    Operand(Operand),
    Word(Span),
    DictOpen(usize),
    DictClose(usize),
    ArrayOpen(usize),
    ArrayClose(usize),
    BraceOpen(usize),
    BraceClose(usize),
}

impl Raw {
    fn start(&self) -> usize {
        match self {
            Raw::Operand(o) => o.span().start,
            Raw::Word(s) => s.start,
            Raw::DictOpen(p) | Raw::DictClose(p) | Raw::ArrayOpen(p) | Raw::ArrayClose(p) => *p,
            Raw::BraceOpen(p) | Raw::BraceClose(p) => *p,
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    content: bool, // content stream: strict numbers ≤ 1e9, braces illegal
    lenient: bool, // CMap/Type1: invalid numbers become words, braces are words
    string_max: usize,
    paren_max: usize,
    nodes: usize, // operand nodes read so far (`LexLimits::operand_nodes_max`)
    nodes_max: usize,
    /// The last forward search for each inline-image ASCII terminator (`>`, `~>`): where it
    /// started and the first hit at or after it (`inline::find_terminator`).
    terminators: [Option<(usize, Option<usize>)>; 2],
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8], content: bool, lenient: bool, lim: &LexLimits) -> Self {
        Reader {
            b,
            pos: 0,
            content,
            lenient,
            string_max: lim.string_bytes_max,
            paren_max: lim.paren_nesting_max,
            nodes: 0,
            nodes_max: lim.operand_nodes_max,
            terminators: [None; 2],
        }
    }

    /// Counts the node `raw` makes (a value, or an array or dictionary it opens).
    fn count_node(&mut self, raw: &Raw) -> Result<(), LexError> {
        if matches!(raw, Raw::Operand(_) | Raw::ArrayOpen(_) | Raw::DictOpen(_)) {
            self.nodes = self.nodes.saturating_add(1);
            if self.nodes > self.nodes_max {
                return Err(LexError::TooComplex {
                    what: OPERAND_NODES,
                });
            }
        }
        Ok(())
    }

    fn at(&self, i: usize) -> Option<u8> {
        self.b.get(i).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.at(self.pos) {
            if is_whitespace(c) {
                self.pos += 1;
            } else if c == b'%' {
                while let Some(c) = self.at(self.pos) {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn next(&mut self) -> Result<Option<Raw>, LexError> {
        self.skip_ws();
        let start = self.pos;
        let Some(c) = self.at(start) else {
            return Ok(None);
        };
        let raw = match c {
            b'(' => Raw::Operand(self.literal_string()?),
            b'<' if self.at(start + 1) == Some(b'<') => {
                self.pos += 2;
                Raw::DictOpen(start)
            }
            b'<' => Raw::Operand(self.hex_string()?),
            b'>' if self.at(start + 1) == Some(b'>') => {
                self.pos += 2;
                Raw::DictClose(start)
            }
            b'>' => return malformed(start, "stray >"),
            b')' => return malformed(start, "stray )"),
            b'[' | b']' | b'{' | b'}' => {
                self.pos += 1;
                match c {
                    b'[' => Raw::ArrayOpen(start),
                    b']' => Raw::ArrayClose(start),
                    _ if self.content => return malformed(start, "brace in content"),
                    b'{' => Raw::BraceOpen(start),
                    _ => Raw::BraceClose(start),
                }
            }
            b'/' => Raw::Operand(self.name()?),
            _ => self.regular()?,
        };
        self.count_node(&raw)?;
        Ok(Some(raw))
    }

    fn regular(&mut self) -> Result<Raw, LexError> {
        let start = self.pos;
        while self.at(self.pos).is_some_and(is_regular) {
            self.pos += 1;
        }
        let span = start..self.pos;
        let tok = self.b.get(span.clone()).unwrap_or_default();
        let span_c = span.clone();
        let op = |value| Raw::Operand(value);
        match tok {
            b"true" => return Ok(op(Operand::Bool { value: true, span })),
            b"false" => return Ok(op(Operand::Bool { value: false, span })),
            b"null" => return Ok(op(Operand::Null { span })),
            _ => {}
        }
        let numeric = tok
            .first()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.'));
        if !numeric {
            return Ok(Raw::Word(span));
        }
        match parse_number(tok) {
            Some(value) if !self.content || value.abs() <= limits::NUMBER_ABS_MAX => {
                Ok(op(Operand::Number { value, span }))
            }
            Some(_) => malformed(start, "number too large"),
            None if self.lenient => Ok(Raw::Word(span_c)),
            None => malformed(start, "bad number"),
        }
    }

    fn name(&mut self) -> Result<Operand, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut bytes = Vec::new();
        while let Some(c) = self.at(self.pos).filter(|c| is_regular(*c)) {
            let hex = (
                self.at(self.pos + 1).and_then(hex_digit),
                self.at(self.pos + 2).and_then(hex_digit),
            );
            match (c, hex) {
                (b'#', (Some(h), Some(l))) => {
                    bytes.push((h << 4) | l);
                    self.pos += 3;
                }
                _ => {
                    bytes.push(c);
                    self.pos += 1;
                }
            }
            if bytes.len() > self.string_max {
                return Err(LexError::TooComplex {
                    what: "name length",
                });
            }
        }
        bytes.shrink_to_fit();
        Ok(Operand::Name {
            bytes,
            span: start..self.pos,
        })
    }

    fn literal_string(&mut self) -> Result<Operand, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut depth = 1usize;
        let mut bytes = Vec::new();
        loop {
            let Some(c) = self.at(self.pos) else {
                return malformed(start, "unterminated string");
            };
            self.pos += 1;
            match c {
                b'\\' => {
                    let Some(e) = self.at(self.pos) else {
                        return malformed(start, "unterminated string");
                    };
                    self.pos += 1;
                    match e {
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'b' => bytes.push(8),
                        b'f' => bytes.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.at(self.pos) {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            bytes.push((v & 0xFF) as u8);
                        }
                        b'\r' => {
                            if self.at(self.pos) == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => bytes.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    if depth > self.paren_max {
                        return Err(LexError::TooComplex {
                            what: "string nesting",
                        });
                    }
                    bytes.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    bytes.push(c);
                }
                b'\r' => {
                    if self.at(self.pos) == Some(b'\n') {
                        self.pos += 1;
                    }
                    bytes.push(b'\n');
                }
                other => bytes.push(other),
            }
            if bytes.len() > self.string_max {
                return Err(LexError::TooComplex {
                    what: "string length",
                });
            }
        }
        bytes.shrink_to_fit();
        Ok(Operand::Str {
            bytes,
            hex: false,
            span: start..self.pos,
        })
    }

    fn hex_string(&mut self) -> Result<Operand, LexError> {
        let start = self.pos;
        self.pos += 1;
        let mut bytes = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(c) = self.at(self.pos) else {
                return malformed(start, "unterminated hex string");
            };
            self.pos += 1;
            if c == b'>' {
                break;
            }
            if is_whitespace(c) {
                continue;
            }
            let Some(v) = hex_digit(c) else {
                return malformed(start, "bad hex string");
            };
            match high.take() {
                None => high = Some(v),
                Some(h) => bytes.push((h << 4) | v),
            }
            if bytes.len() > self.string_max {
                return Err(LexError::TooComplex {
                    what: "string length",
                });
            }
        }
        if let Some(h) = high {
            bytes.push(h << 4);
        }
        bytes.shrink_to_fit();
        Ok(Operand::Str {
            bytes,
            hex: true,
            span: start..self.pos,
        })
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Lexes a content stream into ops with exact byte spans.
pub fn lex_content(
    bytes: &[u8],
    limits: &LexLimits,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<Op>, LexError> {
    lex_inner(bytes, limits, cancel, false)
}

fn lex_inner(
    bytes: &[u8],
    lim: &LexLimits,
    cancel: Option<&AtomicBool>,
    tail_check: bool,
) -> Result<Vec<Op>, LexError> {
    let mut r = Reader::new(bytes, true, false, lim);
    let mut builder = Builder::new(lim);
    let mut ops: Vec<Op> = Vec::new();
    let mut operands: Vec<Operand> = Vec::new();
    let mut op_start: Option<usize> = None;
    let mut compat = 0usize;
    let mut after_unproven = false;
    loop {
        let raw = match r.next() {
            Ok(Some(raw)) => raw,
            Ok(None) => break,
            // a heuristic tail window may end inside a string: the ops before it count
            Err(LexError::Malformed {
                what: "unterminated string" | "unterminated hex string",
                ..
            }) if tail_check => return Ok(ops),
            Err(e) => return Err(e),
        };
        let start = raw.start();
        let first = *op_start.get_or_insert(start);
        let span = match raw {
            Raw::Word(span) if builder.frames.is_empty() => span,
            other => {
                if let Some(v) = builder.feed(other)? {
                    if operands.len() >= lim.array_items_max {
                        return Err(LexError::TooComplex { what: "operands" });
                    }
                    operands.push(v);
                }
                continue;
            }
        };
        let operator = Operator::from_token(bytes.get(span.clone()).unwrap_or_default());
        let in_compat = compat > 0;
        let mut op = Op {
            operator,
            // Exactly as many slots as operands (the scratch vector keeps its room): a one-operand
            // op kept for the page's lifetime would otherwise hold room for four.
            operands: operands.drain(..).collect(),
            span: first..span.end,
            op_span: span.clone(),
            in_compat,
            after_unproven_inline_image: after_unproven,
            inline_image: None,
        };
        op_start = None;
        match operator {
            O::BI => {
                if tail_check {
                    return Ok(ops);
                }
                if !op.operands.is_empty() {
                    return malformed(first, "operands before BI");
                }
                let image = inline::inline_image(&mut r, lim, span.end)?;
                op.span = span.start..r.pos;
                if image.proof == InlineProof::Heuristic {
                    after_unproven = true;
                }
                op.inline_image = Some(image);
            }
            O::ID | O::EI => return malformed(span.start, "inline image operator outside BI"),
            O::Unknown if !in_compat => return malformed(span.start, "unknown operator"),
            O::Unknown => {}
            O::BX => {
                check_arity(&op)?;
                compat = compat
                    .checked_add(1)
                    .ok_or(LexError::TooComplex { what: "BX nesting" })?;
                if compat > lim.nesting_max {
                    return Err(LexError::TooComplex { what: "BX nesting" });
                }
            }
            O::EX => {
                check_arity(&op)?;
                compat = compat.saturating_sub(1);
            }
            _ => check_arity(&op)?,
        }
        if ops.len() >= lim.ops_max {
            return Err(LexError::TooComplex { what: "operations" });
        }
        ops.push(op);
        if ops.len() % limits::LEX_CANCEL_EVERY_OPS == 0
            && cancel.is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err(LexError::Cancelled);
        }
    }
    if !tail_check && (!builder.frames.is_empty() || !operands.is_empty()) {
        return malformed(bytes.len(), "trailing operands");
    }
    Ok(ops)
}

/// Flat tokens of a PostScript-like buffer (objects, CMaps, Type1 clear text); no arity.
/// `R` and every other keyword come back as `Keyword`; brackets must balance.
pub fn scan_tokens(
    bytes: &[u8],
    mode: ScanMode,
    max_tokens: usize,
) -> Result<Vec<Token>, LexError> {
    scan::tokens(bytes, mode, max_tokens)
}

/// Tokens of the one dictionary starting at `start` (after whitespace/comments) through its
/// matching `>>`, and the offset just after it. Used by the snapshot preflight.
pub fn scan_dict_at(
    bytes: &[u8],
    start: usize,
    max_tokens: usize,
) -> Result<(Vec<Token>, usize), LexError> {
    scan::dict_at(bytes, start, max_tokens)
}
