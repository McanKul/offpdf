//! Object preflight (SPEC §B.4, R3): bounds lopdf 0.34's two recursions before it parses.
//!
//! - Nesting. lopdf's `nom_parser` recurses once per `[`/`<<` level. It starts parsing at every
//!   xref offset (`N G obj`, then one value) and at every object-stream member offset. An xref
//!   offset may point anywhere, also inside a string, a comment or stream data, so the value after
//!   **every** `N G obj` header in the file is scanned, wherever it sits (`headers.rs` finds them
//!   in linear time), and every object-stream member is scanned after `objstm_guard` decodes it. A scan counts depth until the value ends or
//!   until the first byte lopdf's grammar cannot consume (lopdf's descent stops there too); it is
//!   lenient wherever lopdf is strict, so the depth it sees is never below lopdf's.
//! - `/Length` chains. While parsing a stream whose `/Length` is a reference, lopdf parses the
//!   referenced object, which recurses again if that is a stream with a `/Length` reference. The
//!   chain is bounded by `LENGTH_REF_CHAIN_MAX` over a graph that over-approximates which object
//!   an id can resolve to: an xref offset may point inside a header's first number, so a header
//!   stands for every numeric suffix of it.

use super::headers::Headers;
use super::preflight::find_from;
use crate::error::AppError;
use crate::pdf_engine::text_edit::lexer;
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons;
use lopdf::{Dictionary, Object};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bytes the value scans over one buffer may read in total. In a well-formed file every value
/// is read once; scans that overlap (headers inside strings) are what this bounds.
struct ScanBudget(usize);

impl ScanBudget {
    fn for_len(len: usize) -> Self {
        ScanBudget(
            len.saturating_mul(limits::OBJECT_SCAN_FACTOR)
                .saturating_add(limits::OBJECT_SCAN_SLACK_BYTES),
        )
    }

    fn debit(&mut self, n: usize) -> Result<(), ScanError> {
        self.0 = self.0.checked_sub(n).ok_or(ScanError::Budget)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanError {
    TooDeep,
    Budget,
}

impl ScanError {
    fn to_app_error(self) -> AppError {
        reasons::file_too_complex(&match self {
            ScanError::TooDeep => format!(
                "objects nested more than {} levels deep",
                limits::MAX_OBJECT_NESTING
            ),
            ScanError::Budget => "object scan budget exceeded".to_string(),
        })
    }
}

/// A stream (`N G obj << … /Length T G R … >> stream`) whose header's first number spans `run`.
struct LengthRef {
    run: Range<usize>,
    target: u32,
}

/// Scans every `N G obj` header in `b` (nesting of its value) and bounds `/Length` chains.
pub(super) fn check_objects(b: &[u8]) -> Result<(), AppError> {
    let mut budget = ScanBudget::for_len(b.len());
    let mut refs: Vec<LengthRef> = Vec::new();
    for (run, value_at) in Headers::new(b) {
        let target = scan_value(b, value_at, &mut budget).map_err(ScanError::to_app_error)?;
        if let Some(target) = target {
            if refs.len() >= limits::LENGTH_REF_STREAMS_MAX {
                return Err(reasons::file_too_complex(
                    "too many streams with a /Length reference",
                ));
            }
            refs.push(LengthRef { run, target });
        }
    }
    let longest = longest_length_chain(b, &refs);
    if longest > limits::LENGTH_REF_CHAIN_MAX {
        return Err(reasons::file_too_complex(&format!(
            "stream /Length references may chain through {longest} objects"
        )));
    }
    Ok(())
}

/// lopdf's `ObjectStream::new` over the decoded data of an object stream: the members it would
/// parse (same header parsing) nest at most `MAX_OBJECT_NESTING` levels. `false` → reject it.
/// The scans of all object streams of one load draw on `shared` (review-T1 round 2, MEDIUM-1):
/// each stream may read up to 2 × its data + 1 MiB, but never more than is left, and what it
/// read is debited afterwards. Streams scanned concurrently on rayon workers can overdraw it by
/// at most one such grant each.
pub(super) fn objstm_members_ok(dict: &Dictionary, data: &[u8], shared: &AtomicUsize) -> bool {
    let first = dict.get(b"First").and_then(Object::as_i64).ok();
    let Some(first) = first.and_then(|f| usize::try_from(f).ok()) else {
        return true; // lopdf drops the stream without parsing a member
    };
    let header = data.get(..first).and_then(|h| std::str::from_utf8(h).ok());
    let (Some(header), true) = (header, dict.get(b"N").and_then(Object::as_i64).is_ok()) else {
        return true;
    };
    let numbers: Vec<Option<u32>> = header
        .split_whitespace()
        .map(|n| n.parse::<u32>().ok())
        .collect();
    let granted = ScanBudget::for_len(data.len())
        .0
        .min(shared.load(Ordering::SeqCst));
    let mut budget = ScanBudget(granted);
    let members_ok = numbers.chunks_exact(2).all(|pair| {
        let (Some(Some(_)), Some(Some(off))) = (pair.first(), pair.get(1)) else {
            return true;
        };
        let offset = usize::try_from(*off)
            .ok()
            .and_then(|o| first.checked_add(o));
        match offset {
            Some(offset) if offset < data.len() => scan_value(data, offset, &mut budget).is_ok(),
            _ => true,
        }
    });
    let used = granted.saturating_sub(budget.0);
    let debited = shared
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(used)
        })
        .is_ok();
    members_ok && debited
}

/// lopdf's `space`: whitespace and `%` comments (lenient: a comment may end at EOF).
fn skip_space(b: &[u8], mut i: usize) -> usize {
    loop {
        match b.get(i) {
            Some(c) if lexer::is_whitespace(*c) => i += 1,
            Some(b'%') => {
                i += b.get(i..).map_or(0, |r| {
                    r.iter().take_while(|c| !matches!(c, b'\r' | b'\n')).count()
                });
            }
            _ => return i,
        }
    }
}

/// Bytes lopdf can consume as (parts of) `null`, `true`, `false`, numbers and the `R` of a
/// reference; a regular word with any other byte stops lopdf's parse.
const VALUE_WORD_BYTES: &[u8] = b"0123456789+-.Rtruefalsn";

fn regular_end(b: &[u8], from: usize) -> usize {
    from + b.get(from..).map_or(0, |r| {
        r.iter()
            .take_while(|c| !lexer::is_whitespace(**c) && !lexer::is_delimiter(**c))
            .count()
    })
}

/// End of the literal string opening at `i` (balanced parentheses, `\` escapes).
fn skip_literal(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while let Some(&c) = b.get(i) {
        i += 1;
        match c {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    i.min(b.len())
}

/// Scans the one value lopdf parses at `at` (after `obj`, or an object-stream member) and returns
/// the target id of its `/Length N G R` when the value is a stream dictionary.
fn scan_value(b: &[u8], at: usize, budget: &mut ScanBudget) -> Result<Option<u32>, ScanError> {
    let mut i = at;
    let mut depth = 0usize;
    let mut top_dict = false;
    let mut length = LengthState::Idle;
    let mut length_ref = None;
    let closed_dict = loop {
        i = skip_space(b, i);
        let Some(&c) = b.get(i) else { break false };
        let pair = b.get(i + 1) == Some(&c);
        if c == b'[' || (c == b'<' && pair) {
            if depth == 0 {
                top_dict = c == b'<';
            }
            depth += 1;
            if depth > limits::MAX_OBJECT_NESTING {
                return Err(ScanError::TooDeep);
            }
            length = LengthState::Idle;
            i += if c == b'[' { 1 } else { 2 };
            continue;
        }
        if c == b']' || (c == b'>' && pair) {
            let Some(d) = depth.checked_sub(1) else {
                break false;
            };
            depth = d;
            i += if c == b']' { 1 } else { 2 };
            if depth == 0 {
                break top_dict;
            }
            continue;
        }
        let end = match c {
            b'(' => skip_literal(b, i),
            b'<' => find_from(b, i, b">").map_or(b.len(), |p| p + 1),
            b'/' => regular_end(b, i + 1),
            _ if lexer::is_delimiter(c) => break false,
            _ => {
                let end = regular_end(b, i);
                let word = b.get(i..end).unwrap_or_default();
                if !word.iter().all(|t| VALUE_WORD_BYTES.contains(t)) {
                    break false;
                }
                end
            }
        };
        let token = b.get(i..end).unwrap_or_default();
        if depth == 1 && top_dict {
            length = length.step(token, &mut length_ref);
        }
        i = end;
        if depth == 0 {
            break false;
        }
    };
    // The `stream` probe after a closed dictionary reads too: overlapping headers that close at
    // one `>>` would otherwise each skip the same trailing space unbudgeted (round 2, HIGH-2).
    let end = if closed_dict { skip_space(b, i) } else { i };
    budget.debit(end.saturating_sub(at))?;
    let stream = closed_dict && b.get(end..).is_some_and(|rest| rest.starts_with(b"stream"));
    Ok(length_ref.filter(|_| stream))
}

/// Recognises `/Length N G R` among the direct entries of a dictionary (the last one counts, as
/// in lopdf's `Dictionary::set`; a later direct `/Length` only makes this over-approximate).
#[derive(Clone, Copy)]
enum LengthState {
    Idle,
    Key,
    Id(u32),
    Gen(u32),
}

impl LengthState {
    fn step(self, token: &[u8], found: &mut Option<u32>) -> LengthState {
        if let Some(name) = token.strip_prefix(b"/") {
            return if name_is(name, b"Length") {
                LengthState::Key
            } else {
                LengthState::Idle
            };
        }
        let digits = token.iter().take_while(|c| c.is_ascii_digit()).count();
        let (number, rest) = token.split_at_checked(digits).unwrap_or((token, &[]));
        match self {
            LengthState::Key if rest.is_empty() => {
                parse_u32(number).map_or(LengthState::Idle, LengthState::Id)
            }
            LengthState::Id(id) if digits > 0 && rest.is_empty() => LengthState::Gen(id),
            LengthState::Id(id) if digits > 0 && rest.starts_with(b"R") => {
                *found = Some(id);
                LengthState::Idle
            }
            LengthState::Gen(id) if token.starts_with(b"R") => {
                *found = Some(id);
                LengthState::Idle
            }
            _ => LengthState::Idle,
        }
    }
}

fn parse_u32(digits: &[u8]) -> Option<u32> {
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// A name's bytes (after `/`) decoded like lopdf (`#xx`; an invalid `#` ends the name) equal
/// `want`.
fn name_is(raw: &[u8], want: &[u8]) -> bool {
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = want.iter();
    let mut k = 0;
    while let Some(&c) = raw.get(k) {
        let byte = if c == b'#' {
            match (
                raw.get(k + 1).and_then(|c| hex(*c)),
                raw.get(k + 2).and_then(|c| hex(*c)),
            ) {
                (Some(h), Some(l)) => {
                    k += 3;
                    h * 16 + l
                }
                _ => break,
            }
        } else {
            k += 1;
            u32::from(c)
        };
        if out.next().map(|w| u32::from(*w)) != Some(byte) {
            return false;
        }
    }
    out.next().is_none()
}

/// Every id an xref offset inside `run` can make lopdf parse: each numeric suffix that fits u32.
fn suffix_ids(run: &[u8]) -> Vec<u32> {
    let mut ids: Vec<u32> = (1..=run.len().min(10))
        .filter_map(|n| parse_u32(run.get(run.len() - n..)?))
        .collect();
    ids.dedup();
    ids
}

/// Upper bound on the objects lopdf parses for one top-level stream while resolving `/Length`
/// references: the stream itself plus the longest path of ids, each strongly connected component
/// counted with its size (lopdf never resolves an id twice in one chain).
fn longest_length_chain(b: &[u8], refs: &[LengthRef]) -> usize {
    if refs.is_empty() {
        return 0;
    }
    let mut nodes: Vec<u32> = refs.iter().map(|r| r.target).collect();
    nodes.sort_unstable();
    nodes.dedup();
    let node = |id: u32| nodes.binary_search(&id).ok();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for r in refs {
        let Some(to) = node(r.target) else { continue };
        let run = b.get(r.run.clone()).unwrap_or_default();
        edges.extend(
            suffix_ids(run)
                .into_iter()
                .filter_map(node)
                .map(|from| (from, to)),
        );
    }
    edges.sort_unstable();
    edges.dedup();
    longest_scc_path(nodes.len(), &edges).saturating_add(1)
}

/// Tarjan's strongly connected components (iterative) over `n` nodes and sorted `edges`; returns
/// the largest sum of component sizes along any path of the condensation.
fn longest_scc_path(n: usize, edges: &[(usize, usize)]) -> usize {
    const UNSEEN: usize = usize::MAX;
    let out = |v: usize| {
        let lo = edges.partition_point(|e| e.0 < v);
        let hi = edges.partition_point(|e| e.0 <= v);
        edges.get(lo..hi).unwrap_or_default()
    };
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![UNSEEN; n];
    let mut best: Vec<usize> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0usize;
    for root in 0..n {
        if index.get(root) != Some(&UNSEEN) {
            continue;
        }
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        visit(
            root,
            &mut counter,
            &mut index,
            &mut low,
            &mut on_stack,
            &mut stack,
        );
        while let Some((v, pos)) = calls.last().copied() {
            if let Some(&(_, w)) = out(v).get(pos) {
                if let Some(top) = calls.last_mut() {
                    top.1 += 1;
                }
                if index.get(w) == Some(&UNSEEN) {
                    visit(
                        w,
                        &mut counter,
                        &mut index,
                        &mut low,
                        &mut on_stack,
                        &mut stack,
                    );
                    calls.push((w, 0));
                } else if on_stack.get(w) == Some(&true) {
                    let iw = index.get(w).copied().unwrap_or(UNSEEN);
                    if let Some(lv) = low.get_mut(v) {
                        *lv = (*lv).min(iw);
                    }
                }
                continue;
            }
            calls.pop();
            let lv = low.get(v).copied().unwrap_or(0);
            if let Some(&(u, _)) = calls.last() {
                if let Some(lu) = low.get_mut(u) {
                    *lu = (*lu).min(lv);
                }
            }
            if Some(&lv) != index.get(v) {
                continue;
            }
            let c = best.len();
            let mut members = Vec::new();
            while let Some(w) = stack.pop() {
                if let Some(s) = on_stack.get_mut(w) {
                    *s = false;
                }
                if let Some(cw) = comp.get_mut(w) {
                    *cw = c;
                }
                members.push(w);
                if w == v {
                    break;
                }
            }
            let successors = members
                .iter()
                .flat_map(|m| out(*m).iter())
                .filter_map(|&(_, w)| comp.get(w).copied().filter(|cw| *cw != c && *cw != UNSEEN))
                .filter_map(|cw| best.get(cw).copied())
                .max()
                .unwrap_or(0);
            best.push(members.len().saturating_add(successors));
        }
    }
    best.into_iter().max().unwrap_or(0)
}

fn visit(
    v: usize,
    counter: &mut usize,
    index: &mut [usize],
    low: &mut [usize],
    on_stack: &mut [bool],
    stack: &mut Vec<usize>,
) {
    if let (Some(i), Some(l), Some(s)) = (index.get_mut(v), low.get_mut(v), on_stack.get_mut(v)) {
        *i = *counter;
        *l = *counter;
        *s = true;
    }
    *counter += 1;
    stack.push(v);
}
