//! Resource bounds applied before lopdf parses source content.
//!
//! This is deliberately limited to object values and object streams. Raw xref
//! validation and bounded content-operator parsing are separate follow-up work.

mod headers;

use self::headers::Headers;
use crate::error::AppError;
use crate::pdf_engine::source_content_decode;
use lopdf::{Dictionary, Document, Object};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

const MAX_OBJECTS: usize = 2_000_000;
const MAX_OBJECT_NESTING: usize = 100;
const OBJECT_SCAN_FACTOR: usize = 2;
const OBJECT_SCAN_SLACK_BYTES: usize = 1 << 20;
const LENGTH_REF_STREAMS_MAX: usize = MAX_OBJECTS / 2;
const LENGTH_REF_CHAIN_MAX: usize = 32;
const OBJSTM_MAX_DECODED: usize = 32 << 20;
const OBJSTM_TOTAL_DECODED: usize = 256 << 20;
const OBJSTM_SCAN_BYTES: usize =
    OBJECT_SCAN_FACTOR * OBJSTM_TOTAL_DECODED + OBJECT_SCAN_SLACK_BYTES;

static LOAD_LOCK: Mutex<()> = Mutex::new(());
static OBJSTM_BUDGET: AtomicUsize = AtomicUsize::new(0);
static OBJSTM_SCAN_BUDGET: AtomicUsize = AtomicUsize::new(0);
static OBJSTM_REJECTED: AtomicBool = AtomicBool::new(false);
const REJECTED_OBJSTM: &[u8] = b"OffPdfRejectedObjStm";

pub(super) fn load_document(bytes: &[u8], path: &str) -> Result<Document, AppError> {
    check_objects(bytes)?;

    // lopdf's object-stream filter is a function pointer. Serialize loads so
    // the per-load atomic budgets cannot leak between concurrent classifiers.
    let _lock = LOAD_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    OBJSTM_BUDGET.store(OBJSTM_TOTAL_DECODED, Ordering::SeqCst);
    OBJSTM_SCAN_BUDGET.store(OBJSTM_SCAN_BYTES, Ordering::SeqCst);
    OBJSTM_REJECTED.store(false, Ordering::SeqCst);

    let document = lopdf::Reader {
        buffer: bytes,
        document: Document::new(),
    }
    .read(Some(object_stream_guard))
    .map_err(|error| AppError::invalid_pdf(path).with_details(format!("lopdf: {error}")))?;

    let rejected = OBJSTM_REJECTED.load(Ordering::SeqCst)
        || document
            .objects
            .values()
            .any(|object| matches!(object, Object::Stream(stream) if stream.dict.type_is(REJECTED_OBJSTM)));
    if rejected {
        return Err(file_too_complex(
            "an object stream is too large, too deeply nested, or cannot be decoded",
        ));
    }
    if document.objects.len() > MAX_OBJECTS {
        return Err(file_too_complex("too many objects"));
    }
    Ok(document)
}

fn object_stream_guard(id: (u32, u16), object: &mut Object) -> Option<((u32, u16), Object)> {
    let Object::Stream(stream) = object else {
        // lopdf uses this value for members of an object stream. At the top
        // level the return value is ignored.
        return Some((id, object.clone()));
    };

    if stream.dict.type_is(b"ObjStm") {
        let remaining = OBJSTM_BUDGET.load(Ordering::SeqCst);
        let cap = OBJSTM_MAX_DECODED.min(remaining);
        let declared = stream.dict.get(b"N").and_then(Object::as_i64).unwrap_or(0);
        let decoded = source_content_decode::decode_stream(stream, cap);
        let accepted = match decoded {
            Ok(data)
                if !(data.is_empty() && declared > 0)
                    && object_stream_members_ok(&stream.dict, &data, &OBJSTM_SCAN_BUDGET) =>
            {
                let debited = OBJSTM_BUDGET
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                        left.checked_sub(data.len())
                    })
                    .is_ok();
                if debited {
                    stream.dict.remove(b"Filter");
                    stream.dict.remove(b"DecodeParms");
                    stream.set_content(data);
                }
                debited
            }
            _ => false,
        };

        if !accepted {
            stream
                .dict
                .set("Type", Object::Name(REJECTED_OBJSTM.to_vec()));
            stream.content.clear();
            OBJSTM_REJECTED.store(true, Ordering::SeqCst);
        }
    }

    // Streams are top-level objects, where lopdf ignores the returned value.
    Some((id, Object::Null))
}

struct ScanBudget(usize);

impl ScanBudget {
    fn for_len(len: usize) -> Self {
        Self(
            len.saturating_mul(OBJECT_SCAN_FACTOR)
                .saturating_add(OBJECT_SCAN_SLACK_BYTES),
        )
    }

    fn debit(&mut self, amount: usize) -> Result<(), ScanError> {
        self.0 = self.0.checked_sub(amount).ok_or(ScanError::Budget)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanError {
    TooDeep,
    Budget,
}

impl ScanError {
    fn into_app_error(self) -> AppError {
        match self {
            Self::TooDeep => file_too_complex(&format!(
                "objects nested more than {MAX_OBJECT_NESTING} levels deep"
            )),
            Self::Budget => file_too_complex("object scan budget exceeded"),
        }
    }
}

struct LengthRef {
    run: Range<usize>,
    target: u32,
}

fn check_objects(bytes: &[u8]) -> Result<(), AppError> {
    let mut budget = ScanBudget::for_len(bytes.len());
    let mut length_refs = Vec::new();
    let mut object_count = 0usize;

    for (run, value_at) in Headers::new(bytes) {
        object_count = object_count.saturating_add(1);
        if object_count > MAX_OBJECTS {
            return Err(file_too_complex("too many object headers"));
        }

        let target = scan_value(bytes, value_at, &mut budget).map_err(ScanError::into_app_error)?;
        if let Some(target) = target {
            if length_refs.len() >= LENGTH_REF_STREAMS_MAX {
                return Err(file_too_complex(
                    "too many streams with a /Length reference",
                ));
            }
            length_refs.push(LengthRef { run, target });
        }
    }

    let longest = longest_length_chain(bytes, &length_refs);
    if longest > LENGTH_REF_CHAIN_MAX {
        return Err(file_too_complex(&format!(
            "stream /Length references may chain through {longest} objects"
        )));
    }
    Ok(())
}

fn object_stream_members_ok(dictionary: &Dictionary, data: &[u8], shared: &AtomicUsize) -> bool {
    let first = dictionary
        .get(b"First")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|value| usize::try_from(value).ok());
    let Some(first) = first else {
        return true;
    };
    let header = data
        .get(..first)
        .and_then(|value| std::str::from_utf8(value).ok());
    let (Some(header), true) = (
        header,
        dictionary.get(b"N").and_then(Object::as_i64).is_ok(),
    ) else {
        return true;
    };

    let numbers: Vec<Option<u32>> = header
        .split_whitespace()
        .map(|number| number.parse::<u32>().ok())
        .collect();
    let granted = ScanBudget::for_len(data.len())
        .0
        .min(shared.load(Ordering::SeqCst));
    let mut budget = ScanBudget(granted);
    let members_ok = numbers.chunks_exact(2).all(|pair| {
        let (Some(Some(_)), Some(Some(offset))) = (pair.first(), pair.get(1)) else {
            return true;
        };
        let offset = usize::try_from(*offset)
            .ok()
            .and_then(|offset| first.checked_add(offset));
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

fn skip_space(bytes: &[u8], mut index: usize) -> usize {
    loop {
        match bytes.get(index) {
            Some(byte) if is_whitespace(*byte) => index += 1,
            Some(b'%') => {
                index += bytes.get(index..).map_or(0, |rest| {
                    rest.iter()
                        .take_while(|byte| !matches!(byte, b'\r' | b'\n'))
                        .count()
                });
            }
            _ => return index,
        }
    }
}

const VALUE_WORD_BYTES: &[u8] = b"0123456789+-.Rtruefalsn";

fn regular_end(bytes: &[u8], from: usize) -> usize {
    from + bytes.get(from..).map_or(0, |rest| {
        rest.iter()
            .take_while(|byte| !is_whitespace(**byte) && !is_delimiter(**byte))
            .count()
    })
}

fn skip_literal(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 0usize;
    while let Some(&byte) = bytes.get(index) {
        index += 1;
        match byte {
            b'\\' => index += 1,
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
    index.min(bytes.len())
}

fn scan_value(bytes: &[u8], at: usize, budget: &mut ScanBudget) -> Result<Option<u32>, ScanError> {
    let mut index = at;
    let mut depth = 0usize;
    let mut top_dictionary = false;
    let mut length_state = LengthState::Idle;
    let mut length_ref = None;

    let closed_dictionary = loop {
        index = skip_space(bytes, index);
        let Some(&byte) = bytes.get(index) else {
            break false;
        };
        let pair = bytes.get(index + 1) == Some(&byte);

        if byte == b'[' || (byte == b'<' && pair) {
            if depth == 0 {
                top_dictionary = byte == b'<';
            }
            depth += 1;
            if depth > MAX_OBJECT_NESTING {
                return Err(ScanError::TooDeep);
            }
            length_state = LengthState::Idle;
            index += if byte == b'[' { 1 } else { 2 };
            continue;
        }
        if byte == b']' || (byte == b'>' && pair) {
            let Some(next_depth) = depth.checked_sub(1) else {
                break false;
            };
            depth = next_depth;
            index += if byte == b']' { 1 } else { 2 };
            if depth == 0 {
                break top_dictionary;
            }
            continue;
        }

        let end = match byte {
            b'(' => skip_literal(bytes, index),
            b'<' => bytes
                .get(index..)
                .and_then(|rest| rest.iter().position(|candidate| *candidate == b'>'))
                .map_or(bytes.len(), |position| index + position + 1),
            b'/' => regular_end(bytes, index + 1),
            _ if is_delimiter(byte) => break false,
            _ => {
                let end = regular_end(bytes, index);
                let word = bytes.get(index..end).unwrap_or_default();
                if !word
                    .iter()
                    .all(|candidate| VALUE_WORD_BYTES.contains(candidate))
                {
                    break false;
                }
                end
            }
        };

        let token = bytes.get(index..end).unwrap_or_default();
        if depth == 1 && top_dictionary {
            length_state = length_state.step(token, &mut length_ref);
        }
        index = end;
        if depth == 0 {
            break false;
        }
    };

    let end = if closed_dictionary {
        skip_space(bytes, index)
    } else {
        index
    };
    budget.debit(end.saturating_sub(at))?;
    let stream = closed_dictionary
        && bytes
            .get(end..)
            .is_some_and(|rest| rest.starts_with(b"stream"));
    Ok(length_ref.filter(|_| stream))
}

#[derive(Clone, Copy)]
enum LengthState {
    Idle,
    Key,
    Id(u32),
    Generation(u32),
}

impl LengthState {
    fn step(self, token: &[u8], found: &mut Option<u32>) -> Self {
        if let Some(name) = token.strip_prefix(b"/") {
            return if name_is(name, b"Length") {
                Self::Key
            } else {
                Self::Idle
            };
        }

        let digits = token
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let number = token.get(..digits).unwrap_or_default();
        let rest = token.get(digits..).unwrap_or_default();
        match self {
            Self::Key if rest.is_empty() => parse_u32(number).map_or(Self::Idle, Self::Id),
            Self::Id(id) if digits > 0 && rest.is_empty() => Self::Generation(id),
            Self::Id(id) if digits > 0 && rest.starts_with(b"R") => {
                *found = Some(id);
                Self::Idle
            }
            Self::Generation(id) if token.starts_with(b"R") => {
                *found = Some(id);
                Self::Idle
            }
            _ => Self::Idle,
        }
    }
}

fn parse_u32(digits: &[u8]) -> Option<u32> {
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn name_is(raw: &[u8], wanted: &[u8]) -> bool {
    let hex = |byte: u8| (byte as char).to_digit(16);
    let mut expected = wanted.iter();
    let mut index = 0usize;
    while let Some(&byte) = raw.get(index) {
        let decoded = if byte == b'#' {
            match (
                raw.get(index + 1).and_then(|byte| hex(*byte)),
                raw.get(index + 2).and_then(|byte| hex(*byte)),
            ) {
                (Some(high), Some(low)) => {
                    index += 3;
                    high * 16 + low
                }
                _ => break,
            }
        } else {
            index += 1;
            u32::from(byte)
        };
        if expected.next().map(|byte| u32::from(*byte)) != Some(decoded) {
            return false;
        }
    }
    expected.next().is_none()
}

fn suffix_ids(run: &[u8]) -> Vec<u32> {
    let mut ids: Vec<u32> = (1..=run.len().min(10))
        .filter_map(|length| parse_u32(run.get(run.len() - length..)?))
        .collect();
    ids.dedup();
    ids
}

fn longest_length_chain(bytes: &[u8], references: &[LengthRef]) -> usize {
    if references.is_empty() {
        return 0;
    }

    let mut nodes: Vec<u32> = references
        .iter()
        .map(|reference| reference.target)
        .collect();
    nodes.sort_unstable();
    nodes.dedup();
    let node = |id: u32| nodes.binary_search(&id).ok();
    let mut edges = Vec::new();
    for reference in references {
        let Some(to) = node(reference.target) else {
            continue;
        };
        let run = bytes.get(reference.run.clone()).unwrap_or_default();
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

fn longest_scc_path(node_count: usize, edges: &[(usize, usize)]) -> usize {
    const UNSEEN: usize = usize::MAX;
    let outgoing = |vertex: usize| {
        let low = edges.partition_point(|edge| edge.0 < vertex);
        let high = edges.partition_point(|edge| edge.0 <= vertex);
        edges.get(low..high).unwrap_or_default()
    };

    let mut index = vec![UNSEEN; node_count];
    let mut low = vec![0usize; node_count];
    let mut on_stack = vec![false; node_count];
    let mut component = vec![UNSEEN; node_count];
    let mut best = Vec::new();
    let mut stack = Vec::new();
    let mut counter = 0usize;

    for root in 0..node_count {
        if index.get(root) != Some(&UNSEEN) {
            continue;
        }
        let mut calls = vec![(root, 0usize)];
        visit(
            root,
            &mut counter,
            &mut index,
            &mut low,
            &mut on_stack,
            &mut stack,
        );

        while let Some((vertex, position)) = calls.last().copied() {
            if let Some(&(_, next)) = outgoing(vertex).get(position) {
                if let Some(frame) = calls.last_mut() {
                    frame.1 += 1;
                }
                if index.get(next) == Some(&UNSEEN) {
                    visit(
                        next,
                        &mut counter,
                        &mut index,
                        &mut low,
                        &mut on_stack,
                        &mut stack,
                    );
                    calls.push((next, 0));
                } else if on_stack.get(next) == Some(&true) {
                    let next_index = index.get(next).copied().unwrap_or(UNSEEN);
                    if let Some(vertex_low) = low.get_mut(vertex) {
                        *vertex_low = (*vertex_low).min(next_index);
                    }
                }
                continue;
            }

            calls.pop();
            let vertex_low = low.get(vertex).copied().unwrap_or(0);
            if let Some(&(parent, _)) = calls.last() {
                if let Some(parent_low) = low.get_mut(parent) {
                    *parent_low = (*parent_low).min(vertex_low);
                }
            }
            if Some(&vertex_low) != index.get(vertex) {
                continue;
            }

            let component_id = best.len();
            let mut members = Vec::new();
            while let Some(member) = stack.pop() {
                if let Some(value) = on_stack.get_mut(member) {
                    *value = false;
                }
                if let Some(value) = component.get_mut(member) {
                    *value = component_id;
                }
                members.push(member);
                if member == vertex {
                    break;
                }
            }
            let successors = members
                .iter()
                .flat_map(|member| outgoing(*member).iter())
                .filter_map(|&(_, next)| {
                    component
                        .get(next)
                        .copied()
                        .filter(|id| *id != component_id && *id != UNSEEN)
                })
                .filter_map(|id| best.get(id).copied())
                .max()
                .unwrap_or(0);
            best.push(members.len().saturating_add(successors));
        }
    }
    best.into_iter().max().unwrap_or(0)
}

fn visit(
    vertex: usize,
    counter: &mut usize,
    index: &mut [usize],
    low: &mut [usize],
    on_stack: &mut [bool],
    stack: &mut Vec<usize>,
) {
    if let (Some(index_value), Some(low_value), Some(stack_value)) = (
        index.get_mut(vertex),
        low.get_mut(vertex),
        on_stack.get_mut(vertex),
    ) {
        *index_value = *counter;
        *low_value = *counter;
        *stack_value = true;
    }
    *counter += 1;
    stack.push(vertex);
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn file_too_complex(detail: &str) -> AppError {
    AppError::new(
        "FILE_TOO_COMPLEX",
        "This PDF is too complex to check",
        "Some of its internal data is too large or too deeply nested to check safely.",
    )
    .with_details(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_object(levels: usize) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n1 0 obj\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'[', levels));
        bytes.extend(std::iter::repeat_n(b']', levels));
        bytes.extend_from_slice(b"\nendobj\n");
        bytes
    }

    #[test]
    fn accepts_object_nesting_at_limit() {
        check_objects(&nested_object(MAX_OBJECT_NESTING)).unwrap();
    }

    #[test]
    fn rejects_object_nesting_over_limit() {
        let error = check_objects(&nested_object(MAX_OBJECT_NESTING + 1)).unwrap_err();
        assert_eq!(error.code, "FILE_TOO_COMPLEX");
        assert!(error.details.unwrap_or_default().contains("nested"));
    }

    #[test]
    fn rejects_long_stream_length_reference_chain() {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        for id in 1..=LENGTH_REF_CHAIN_MAX + 2 {
            bytes.extend_from_slice(
                format!(
                    "{id} 0 obj\n<< /Length {} 0 R >>\nstream\nx\nendstream\nendobj\n",
                    id + 1
                )
                .as_bytes(),
            );
        }
        let error = check_objects(&bytes).unwrap_err();
        assert_eq!(error.code, "FILE_TOO_COMPLEX");
        assert!(error.details.unwrap_or_default().contains("/Length"));
    }

    #[test]
    fn checks_object_stream_member_nesting() {
        let mut dictionary = Dictionary::new();
        dictionary.set("First", Object::Integer(4));
        dictionary.set("N", Object::Integer(1));
        let mut data = b"7 0 ".to_vec();
        data.extend(std::iter::repeat_n(b'[', MAX_OBJECT_NESTING + 1));
        data.extend(std::iter::repeat_n(b']', MAX_OBJECT_NESTING + 1));
        let shared = AtomicUsize::new(OBJSTM_SCAN_BYTES);
        assert!(!object_stream_members_ok(&dictionary, &data, &shared));
    }
}
