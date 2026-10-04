//! Content-part boundaries: ONE rule for the walker and the save gate (review T5 H1 and probe
//! R7; review-final MEDIUM-2, LOW-1, LOW-2, LOW-4). The walker refuses a page whose boundary is
//! not neutral `MALFORMED_CONTENT` (a line commented out in every viewer must never be offered as
//! editable); the save gate (`gate::join`) lets qpdf's overlay join stand in for a page's parts
//! only when every boundary is neutral. Sharing the rule keeps a page the editor offers from
//! failing every overlay save.
//!
//! Viewers do not join a page's `/Contents` parts alike: pdf.js reads them as one stream (a
//! comment, a string, a name, a number or an operator runs on into the next part), Poppler ends a
//! token at a part's end but lets comments and strings run on, MuPDF and PDFium put a space
//! between parts, qpdf a newline after a part that does not end with one, and the buffer the
//! model walks has a newline at every boundary. A boundary is neutral when inserting whitespace
//! there changes nothing in any of these readings (ISO 32000-1 §7.8.2: a division between streams
//! "may occur only at the boundaries between lexical tokens"):
//! - it lies between tokens: not inside a string, a hex string, a `<<`/`>>` pair, an inline image
//!   (from `BI` through `EI`), or a BX … EX section (pdf.js reads unknown operators there with its
//!   partial command names, so any boundary inside one is refused);
//! - a comment open at the end of a part is neutral only when the next non-empty part starts with
//!   CR or LF (every reader ends the comment there);
//! - where two regular bytes meet, the token before is a number the next byte cannot continue
//!   (not a digit, `.`, `+`, `-`, `e` or `E`: pdf.js stops a number at any other byte), or a known
//!   operator that no 1–3-byte prefix of the next token extends into another command pdf.js knows
//!   (its operators and its partial names `BM`, `BD`, `fa`, `fal`, `fals`, `nu`, `nul`, …):
//!   `ET`|`BT`, `0`|`cm` are neutral, `s`|`h`, `B`|`M…`, `12`|`3`, `/F`|`1` are not;
//! - an inline image must end more than `INLINE_HEURISTIC_WINDOW` bytes before its part's end,
//!   whatever proved its end (review-verify MEDIUM-A): pdf.js 4.10 picks an image's `EI` by
//!   lexing the 15 bytes after it, so the next part's bytes, shifted by qpdf's `\n`, could make
//!   it pick another `EI` before and after the join (a stamp-only save would hide or reveal text
//!   in pdf.js).
//!
//! The scan is a streaming state machine over the bytes (O(n), no op cap, nothing allocated per
//! token): only the lexical state at each part's end and the token next to the boundary matter,
//! so a page of any number of ops is judged without lexing it. Inline images are skipped with the
//! lexer's own end proofs (`lexer::InlineEnds`); an image that does not end in its part, or ends
//! within that window of its end, makes the next boundary not neutral.

use super::PageContent;
use crate::pdf_engine::text_edit::lexer::{is_delimiter, is_whitespace, InlineEnds, Operator};

/// The longest content operator (`BDC`, `SCN`, …): how far a known operator could run on.
const OPERATOR_BYTES_MAX: usize = 3;

/// pdf.js's partial command names (`EvaluatorPreprocessor.opMap`, pdfjs-dist 4.10): its lexer
/// keeps reading a command while the bytes read so far are a known name, these included.
const PDFJS_PARTIALS: [&[u8]; 10] = [
    b"BM", b"BD", b"true", b"fa", b"fal", b"fals", b"false", b"nu", b"nul", b"null",
];

/// Checks every boundary between two non-empty parts of `content`; `Err` carries the joined
/// offset (the end of the part) of the first boundary viewers read differently.
pub(crate) fn check_part_joins(content: &PageContent) -> Result<(), usize> {
    let parts: Vec<&[u8]> = (0..content.parts.len())
        .map(|i| content.part_bytes(i))
        .collect();
    match first_unsafe_boundary(&parts) {
        None => Ok(()),
        Some(i) => Err(content
            .parts
            .get(i)
            .map_or(0, |p| p.start.saturating_add(p.len))),
    }
}

/// The index of the first part whose end is a boundary that is not neutral (see the module doc),
/// or `None`. Empty parts are skipped (a comment runs on over them).
pub(crate) fn first_unsafe_boundary(parts: &[&[u8]]) -> Option<usize> {
    let mut scan = Scan {
        state: State::Between,
        compat: 0,
    };
    let mut parts = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.is_empty())
        .peekable();
    while let Some((i, part)) = parts.next() {
        scan.read(part);
        if let Some((_, next)) = parts.peek() {
            if !scan.boundary(part, next) {
                return Some(i);
            }
        }
    }
    None
}

fn regular(c: u8) -> bool {
    !is_whitespace(c) && !is_delimiter(c)
}

/// The lexical state of the stream read so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Between,
    /// A regular token from `start` in the current part (a name when it starts with `/`).
    Token {
        start: usize,
        name: bool,
    },
    Comment,
    /// A literal string, `depth` parentheses deep, a backslash pending when `escape`.
    Str {
        depth: usize,
        escape: bool,
    },
    Hex,
    /// After a `<` that opens a dictionary or a hex string.
    Lt,
    /// After a `>` outside a hex string (the first of `>>`).
    Gt,
    /// Inside an inline image that does not end in its part, or ends too near its end for every
    /// reader to agree where (the scan cannot follow it).
    Lost,
}

struct Scan {
    state: State,
    /// BX … EX nesting.
    compat: usize,
}

impl Scan {
    /// Reads `part`, the state carrying over from the previous part as in one stream.
    fn read(&mut self, part: &[u8]) {
        let mut images: Option<InlineEnds<'_>> = None;
        let mut i = 0usize;
        while let Some(&c) = part.get(i) {
            i = match self.state {
                State::Lost => return,
                State::Comment => {
                    if c == b'\r' || c == b'\n' {
                        self.state = State::Between;
                    }
                    i + 1
                }
                State::Str { depth, escape } => {
                    self.state = in_string(depth, escape, c);
                    i + 1
                }
                State::Hex => {
                    if c == b'>' {
                        self.state = State::Between;
                    }
                    i + 1
                }
                State::Lt if c == b'<' => {
                    self.state = State::Between;
                    i + 1
                }
                State::Lt => {
                    self.state = State::Hex;
                    i
                }
                State::Gt => {
                    self.state = State::Between;
                    if c == b'>' {
                        i + 1
                    } else {
                        i
                    }
                }
                State::Token { .. } if regular(c) => i + 1,
                State::Token { start, name } => {
                    self.state = State::Between;
                    if !self.token_done(part.get(start..i).unwrap_or_default(), name) {
                        i
                    } else {
                        let ends = images.get_or_insert_with(|| InlineEnds::new(part));
                        match ends.end(i).filter(|end| *end > i) {
                            Some(end) => end,
                            None => {
                                self.state = State::Lost;
                                return;
                            }
                        }
                    }
                }
                State::Between => {
                    self.state = token_start(c, i);
                    i + 1
                }
            };
        }
    }

    /// A token ended: BX/EX nesting, and whether it is `BI` (an inline image starts after it).
    fn token_done(&mut self, token: &[u8], name: bool) -> bool {
        if name {
            return false;
        }
        match token {
            b"BX" => self.compat = self.compat.saturating_add(1),
            b"EX" => self.compat = self.compat.saturating_sub(1),
            b"BI" => return true,
            _ => {}
        }
        false
    }

    /// Whether the boundary after `part` (just read) and before `next` (the next non-empty part)
    /// is neutral.
    fn boundary(&mut self, part: &[u8], next: &[u8]) -> bool {
        let first = next.first().copied();
        let neutral = match self.state {
            State::Between => true,
            State::Comment => matches!(first, Some(b'\r' | b'\n')),
            State::Token { start, name } => {
                let token = part.get(start..).unwrap_or_default();
                let ends = !first.is_some_and(regular) || (!name && ends_before(token, next));
                self.state = State::Between;
                let image = self.token_done(token, name);
                ends && !image
            }
            State::Str { .. } | State::Hex | State::Lt | State::Gt | State::Lost => false,
        };
        neutral && self.compat == 0
    }
}

fn token_start(c: u8, at: usize) -> State {
    match c {
        b'%' => State::Comment,
        b'(' => State::Str {
            depth: 1,
            escape: false,
        },
        b'<' => State::Lt,
        b'>' => State::Gt,
        b'/' => State::Token {
            start: at,
            name: true,
        },
        c if regular(c) => State::Token {
            start: at,
            name: false,
        },
        _ => State::Between,
    }
}

fn in_string(depth: usize, escape: bool, c: u8) -> State {
    let depth = match (escape, c) {
        (true, _) => depth,
        (false, b'\\') => {
            return State::Str {
                depth,
                escape: true,
            }
        }
        (false, b'(') => depth.saturating_add(1),
        (false, b')') if depth <= 1 => return State::Between,
        (false, b')') => depth - 1,
        _ => depth,
    };
    State::Str {
        depth,
        escape: false,
    }
}

/// `[+-]?(\d+\.?\d*|\.\d+)` (the lexer's number syntax).
fn is_number(t: &[u8]) -> bool {
    let body = match t.first() {
        Some(b'+' | b'-') => t.get(1..).unwrap_or_default(),
        _ => t,
    };
    let dots = body.iter().filter(|c| **c == b'.').count();
    let digits = body.iter().filter(|c| c.is_ascii_digit()).count();
    digits > 0 && dots <= 1 && digits + dots == body.len()
}

fn known_command(t: &[u8]) -> bool {
    Operator::from_token(t) != Operator::Unknown || PDFJS_PARTIALS.contains(&t)
}

/// Whether every reader ends the regular `token` at the end of its part when the next part
/// (`next`) starts with a regular byte (see the module doc).
fn ends_before(token: &[u8], next: &[u8]) -> bool {
    let Some(&c) = next.first() else {
        return true;
    };
    if is_number(token) {
        return !matches!(c, b'0'..=b'9' | b'.' | b'+' | b'-' | b'e' | b'E');
    }
    let operator = Operator::from_token(token);
    if operator == Operator::Unknown || matches!(token, b"BI" | b"ID" | b"EI") {
        return false;
    }
    let run_on = next
        .iter()
        .take(OPERATOR_BYTES_MAX)
        .take_while(|c| regular(**c))
        .count();
    let mut word = [0u8; 2 * OPERATOR_BYTES_MAX];
    let Some(head) = word.get_mut(..token.len()) else {
        return false;
    };
    head.copy_from_slice(token);
    (1..=run_on).all(|l| {
        let end = token.len() + l;
        match (word.get_mut(token.len()..end), next.get(..l)) {
            (Some(tail), Some(more)) => tail.copy_from_slice(more),
            _ => return false,
        }
        !word.get(..end).is_some_and(known_command)
    })
}

#[cfg(test)]
mod tests;
