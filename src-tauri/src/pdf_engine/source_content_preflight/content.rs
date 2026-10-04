//! Bounded lexical preflight for page and Form content streams.
//!
//! lopdf still produces the operation objects used by the classifier, but only
//! after this scanner has bounded every recursive value and removed verified
//! inline-image payloads from the bytes lopdf receives.

use crate::error::AppError;
use crate::pdf_engine::source_content_decode::{self, DecodeError};
const MAX_OPERATIONS: usize = 50_000;
const MAX_OPERAND_NODES: usize = 300_000;
const MAX_OPERANDS_PER_OPERATOR: usize = 65_536;
const MAX_CONTAINER_ITEMS: usize = 65_536;
const MAX_NESTING: usize = 32;
const MAX_STRING_BYTES: usize = 1 << 20;
const MAX_LITERAL_NESTING: usize = 100;
const MAX_INLINE_BYTES: usize = 16 << 20;
const MAX_INLINE_ENTRIES: usize = 256;

#[derive(Debug)]
pub(crate) struct PreparedContent<'a> {
    original: &'a [u8],
    sanitized: Option<Vec<u8>>,
    pub(crate) inline_images: usize,
}

impl PreparedContent<'_> {
    pub(crate) fn bytes(&self) -> &[u8] {
        self.sanitized.as_deref().unwrap_or(self.original)
    }
}

pub(super) fn prepare(bytes: &[u8]) -> Result<PreparedContent<'_>, AppError> {
    let mut scanner = Scanner::new(bytes);
    let mut pending_operands = 0usize;
    let mut inline_spans = Vec::new();

    loop {
        scanner.skip_space();
        if scanner.position >= bytes.len() {
            break;
        }
        if scanner.scan_value(0)?.is_some() {
            pending_operands = pending_operands.saturating_add(1);
            if pending_operands > MAX_OPERANDS_PER_OPERATOR {
                return Err(content_too_complex("too many operands for one operator"));
            }
            continue;
        }

        let operator = scanner.regular_word()?;
        if operator == b"BI" {
            if pending_operands != 0 {
                return Err(malformed_content("Operands appear before an inline image."));
            }
            scanner.state.add_operations(3)?; // BI, ID, EI
            inline_spans.push(scanner.inline_image()?);
        } else {
            if matches!(operator, b"ID" | b"EI") {
                return Err(malformed_content(
                    "An inline-image operator appears outside an inline image.",
                ));
            }
            scanner.state.add_operations(1)?;
            pending_operands = 0;
        }
    }

    if pending_operands != 0 {
        return Err(malformed_content(
            "A content stream ends with operands but no operator.",
        ));
    }

    let sanitized = if inline_spans.is_empty() {
        None
    } else {
        let mut output = Vec::new();
        output
            .try_reserve_exact(bytes.len())
            .map_err(|_| content_too_complex("inline-image sanitization allocation"))?;
        let mut copied = 0usize;
        for span in &inline_spans {
            if span.data_start < copied || span.ei_end > bytes.len() {
                return Err(malformed_content("Inline-image ranges overlap."));
            }
            output.extend_from_slice(bytes.get(copied..span.data_start).unwrap_or_default());
            output.extend_from_slice(b"EI");
            copied = span.ei_end;
        }
        output.extend_from_slice(bytes.get(copied..).unwrap_or_default());
        Some(output)
    };

    Ok(PreparedContent {
        original: bytes,
        sanitized,
        inline_images: inline_spans.len(),
    })
}

struct InlineSpan {
    data_start: usize,
    ei_end: usize,
}

#[derive(Default)]
struct ScanState {
    nodes: usize,
    operations: usize,
}

impl ScanState {
    fn add_node(&mut self) -> Result<(), AppError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_OPERAND_NODES {
            return Err(content_too_complex("too many operand nodes"));
        }
        Ok(())
    }

    fn add_operations(&mut self, count: usize) -> Result<(), AppError> {
        self.operations = self.operations.saturating_add(count);
        if self.operations > MAX_OPERATIONS {
            return Err(content_too_complex("too many operations"));
        }
        Ok(())
    }
}

struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
    state: ScanState,
}

impl<'a> Scanner<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            state: ScanState::default(),
        }
    }

    fn skip_space(&mut self) {
        loop {
            match self.bytes.get(self.position) {
                Some(byte) if is_whitespace(*byte) => self.position += 1,
                Some(b'%') => {
                    self.position += self.bytes.get(self.position..).map_or(0, |rest| {
                        rest.iter()
                            .take_while(|byte| !matches!(byte, b'\r' | b'\n'))
                            .count()
                    });
                }
                _ => break,
            }
        }
    }

    fn scan_value(&mut self, depth: usize) -> Result<Option<Value>, AppError> {
        self.skip_space();
        let start = self.position;
        let Some(byte) = self.bytes.get(start).copied() else {
            return Ok(None);
        };

        let value = match byte {
            b'/' => {
                self.state.add_node()?;
                Value::Name(self.name()?)
            }
            b'(' => {
                self.state.add_node()?;
                self.literal_string()?;
                Value::Other
            }
            b'<' if self.bytes.get(start + 1) == Some(&b'<') => {
                self.state.add_node()?;
                self.dictionary(depth)?;
                Value::Other
            }
            b'<' => {
                self.state.add_node()?;
                self.hex_string()?;
                Value::Other
            }
            b'[' => {
                self.state.add_node()?;
                Value::Array(self.array(depth)?)
            }
            _ if token_at(self.bytes, start, b"true") => {
                self.state.add_node()?;
                self.position += 4;
                Value::Bool(true)
            }
            _ if token_at(self.bytes, start, b"false") => {
                self.state.add_node()?;
                self.position += 5;
                Value::Bool(false)
            }
            _ if token_at(self.bytes, start, b"null") => {
                self.state.add_node()?;
                self.position += 4;
                Value::Other
            }
            _ if byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.') => {
                self.state.add_node()?;
                let token = self.regular_word()?;
                let number = parse_number(token)
                    .filter(|number| number.abs() <= 1e9)
                    .ok_or_else(|| {
                        malformed_content("A content number is invalid or too large.")
                    })?;
                Value::Number {
                    value: number,
                    integer: !token.contains(&b'.'),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn array(&mut self, depth: usize) -> Result<Vec<Value>, AppError> {
        if depth >= MAX_NESTING {
            return Err(content_too_complex("array or dictionary nesting"));
        }
        self.position += 1;
        let mut items = Vec::new();
        loop {
            self.skip_space();
            if self.bytes.get(self.position) == Some(&b']') {
                self.position += 1;
                return Ok(items);
            }
            if items.len() >= MAX_CONTAINER_ITEMS {
                return Err(content_too_complex("too many array items"));
            }
            let value = self
                .scan_value(depth + 1)?
                .ok_or_else(|| malformed_content("An array contains a non-value token."))?;
            items.push(value);
        }
    }

    fn dictionary(&mut self, depth: usize) -> Result<(), AppError> {
        if depth >= MAX_NESTING {
            return Err(content_too_complex("array or dictionary nesting"));
        }
        self.position += 2;
        let mut entries = 0usize;
        loop {
            self.skip_space();
            if self
                .bytes
                .get(self.position..)
                .is_some_and(|rest| rest.starts_with(b">>"))
            {
                self.position += 2;
                return Ok(());
            }
            if entries >= MAX_CONTAINER_ITEMS {
                return Err(content_too_complex("too many dictionary entries"));
            }
            if self.bytes.get(self.position) != Some(&b'/') {
                return Err(malformed_content("A dictionary key is not a name."));
            }
            self.state.add_node()?;
            self.name()?;
            self.scan_value(depth + 1)?
                .ok_or_else(|| malformed_content("A dictionary value is missing."))?;
            entries += 1;
        }
    }

    fn name(&mut self) -> Result<Vec<u8>, AppError> {
        self.position += 1;
        let mut output = Vec::new();
        while let Some(byte) = self
            .bytes
            .get(self.position)
            .copied()
            .filter(|byte| is_regular(*byte))
        {
            let escaped = (
                self.bytes
                    .get(self.position + 1)
                    .and_then(|byte| hex_digit(*byte)),
                self.bytes
                    .get(self.position + 2)
                    .and_then(|byte| hex_digit(*byte)),
            );
            match (byte, escaped) {
                (b'#', (Some(high), Some(low))) => {
                    output.push((high << 4) | low);
                    self.position += 3;
                }
                _ => {
                    output.push(byte);
                    self.position += 1;
                }
            }
            if output.len() > MAX_STRING_BYTES {
                return Err(content_too_complex("name length"));
            }
        }
        Ok(output)
    }

    fn literal_string(&mut self) -> Result<(), AppError> {
        let start = self.position;
        self.position += 1;
        let mut depth = 1usize;
        let mut decoded = 0usize;
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(malformed_content("A literal string is unterminated."));
            };
            self.position += 1;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.bytes.get(self.position).copied() else {
                        return Err(malformed_content("A literal string is unterminated."));
                    };
                    self.position += 1;
                    match escaped {
                        b'0'..=b'7' => {
                            for _ in 0..2 {
                                if matches!(self.bytes.get(self.position), Some(b'0'..=b'7')) {
                                    self.position += 1;
                                } else {
                                    break;
                                }
                            }
                            decoded += 1;
                        }
                        b'\r' => {
                            if self.bytes.get(self.position) == Some(&b'\n') {
                                self.position += 1;
                            }
                        }
                        b'\n' => {}
                        _ => decoded += 1,
                    }
                }
                b'(' => {
                    depth += 1;
                    if depth > MAX_LITERAL_NESTING {
                        return Err(content_too_complex("literal string nesting"));
                    }
                    decoded += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                    decoded += 1;
                }
                b'\r' => {
                    if self.bytes.get(self.position) == Some(&b'\n') {
                        self.position += 1;
                    }
                    decoded += 1;
                }
                _ => decoded += 1,
            }
            if decoded > MAX_STRING_BYTES {
                return Err(content_too_complex("literal string length"));
            }
            if self.position <= start {
                return Err(malformed_content("A literal string could not be scanned."));
            }
        }
    }

    fn hex_string(&mut self) -> Result<(), AppError> {
        self.position += 1;
        let mut digits = 0usize;
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(malformed_content("A hex string is unterminated."));
            };
            self.position += 1;
            if byte == b'>' {
                return Ok(());
            }
            if is_whitespace(byte) {
                continue;
            }
            if hex_digit(byte).is_none() {
                return Err(malformed_content("A hex string contains a non-hex byte."));
            }
            digits += 1;
            if digits.div_ceil(2) > MAX_STRING_BYTES {
                return Err(content_too_complex("hex string length"));
            }
        }
    }

    fn regular_word(&mut self) -> Result<&'a [u8], AppError> {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| is_regular(*byte))
        {
            self.position += 1;
        }
        if self.position == start {
            return Err(malformed_content(
                "A content token starts with a stray delimiter.",
            ));
        }
        Ok(self.bytes.get(start..self.position).unwrap_or_default())
    }

    fn inline_image(&mut self) -> Result<InlineSpan, AppError> {
        let mut dictionary = Vec::new();
        loop {
            self.skip_space();
            if token_at(self.bytes, self.position, b"ID") {
                self.position += 2;
                break;
            }
            if dictionary.len() >= MAX_INLINE_ENTRIES {
                return Err(content_too_complex("inline-image dictionary"));
            }
            if self.bytes.get(self.position) != Some(&b'/') {
                return Err(malformed_content(
                    "An inline-image dictionary key is not a name.",
                ));
            }
            self.state.add_node()?;
            let key = self.name()?;
            let value = self
                .scan_value(1)?
                .ok_or_else(|| malformed_content("An inline-image value is missing."))?;
            dictionary.push((key, value));
        }

        let separator = self
            .bytes
            .get(self.position)
            .copied()
            .filter(|byte| is_whitespace(*byte))
            .ok_or_else(|| malformed_content("Inline-image ID is not followed by whitespace."))?;
        let one = self.position + 1;
        let mut found = prove_inline_end(self.bytes, &dictionary, one)?;
        if found.is_none() && separator == b'\r' && self.bytes.get(one) == Some(&b'\n') {
            found = prove_inline_end(self.bytes, &dictionary, one + 1)?;
        }
        let (data_start, ei) = found.ok_or_else(|| {
            malformed_content("An inline-image boundary could not be verified safely.")
        })?;
        self.position = ei + 2;
        Ok(InlineSpan {
            data_start,
            ei_end: self.position,
        })
    }
}

#[derive(Debug)]
enum Value {
    Number { value: f64, integer: bool },
    Name(Vec<u8>),
    Array(Vec<Value>),
    Bool(bool),
    Other,
}

fn prove_inline_end(
    bytes: &[u8],
    dictionary: &[(Vec<u8>, Value)],
    start: usize,
) -> Result<Option<(usize, usize)>, AppError> {
    let verify = |end: usize| ei_after(bytes, end).map(|ei| (start, ei));
    if let Some(length) = dictionary_uint(dictionary, &[b"L", b"Length"]) {
        if length > MAX_INLINE_BYTES {
            return Err(content_too_complex("inline-image size"));
        }
        if let Some(found) = start.checked_add(length).and_then(verify) {
            return Ok(Some(found));
        }
    }

    let filters = match image_filters(dictionary) {
        Some(filters) => filters,
        None => return Ok(None),
    };
    if filters.is_empty() {
        if let Some(size) = unfiltered_size(dictionary) {
            if size > MAX_INLINE_BYTES as u64 {
                return Err(content_too_complex("inline-image size"));
            }
            return Ok(start.checked_add(size as usize).and_then(verify));
        }
        return Ok(None);
    }

    let payload = bytes.get(start..).unwrap_or_default();
    match filters.first().map(Vec::as_slice) {
        Some(b"Fl" | b"FlateDecode") if filters.len() == 1 => {
            match source_content_decode::inflate_end(payload, MAX_INLINE_BYTES) {
                Ok(consumed) => Ok(start.checked_add(consumed).and_then(verify)),
                Err(DecodeError::TooLarge) => Err(content_too_complex("inline-image size")),
                Err(_) => Ok(None),
            }
        }
        Some(b"AHx" | b"ASCIIHexDecode") => {
            Ok(find_terminator(payload, b">", start).and_then(&verify))
        }
        Some(b"A85" | b"ASCII85Decode") => {
            Ok(find_terminator(payload, b"~>", start).and_then(&verify))
        }
        _ => Ok(None),
    }
}

fn find_terminator(payload: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    let search_len = payload
        .len()
        .min(MAX_INLINE_BYTES.saturating_add(needle.len()));
    let relative = payload
        .get(..search_len)?
        .windows(needle.len())
        .position(|window| window == needle)?;
    start.checked_add(relative)?.checked_add(needle.len())
}

fn ei_after(bytes: &[u8], mut position: usize) -> Option<usize> {
    while bytes.get(position).is_some_and(|byte| is_whitespace(*byte)) {
        position += 1;
    }
    if !bytes
        .get(position..)
        .is_some_and(|rest| rest.starts_with(b"EI"))
    {
        return None;
    }
    bytes
        .get(position + 2)
        .is_none_or(|byte| is_whitespace(*byte) || is_delimiter(*byte))
        .then_some(position)
}

fn dictionary_get<'a>(dictionary: &'a [(Vec<u8>, Value)], keys: &[&[u8]]) -> Option<&'a Value> {
    dictionary
        .iter()
        .rev()
        .find(|(key, _)| keys.contains(&key.as_slice()))
        .map(|(_, value)| value)
}

fn dictionary_uint(dictionary: &[(Vec<u8>, Value)], keys: &[&[u8]]) -> Option<usize> {
    match dictionary_get(dictionary, keys)? {
        Value::Number {
            value,
            integer: true,
        } if *value >= 0.0 => Some(*value as usize),
        _ => None,
    }
}

fn image_filters(dictionary: &[(Vec<u8>, Value)]) -> Option<Vec<Vec<u8>>> {
    match dictionary_get(dictionary, &[b"F", b"Filter"]) {
        None => Some(Vec::new()),
        Some(Value::Name(name)) => Some(vec![name.clone()]),
        Some(Value::Array(items)) if items.len() <= 4 => items
            .iter()
            .map(|item| match item {
                Value::Name(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn unfiltered_size(dictionary: &[(Vec<u8>, Value)]) -> Option<u64> {
    let width = dictionary_uint(dictionary, &[b"W", b"Width"])? as u64;
    let height = dictionary_uint(dictionary, &[b"H", b"Height"])? as u64;
    let image_mask = matches!(
        dictionary_get(dictionary, &[b"IM", b"ImageMask"]),
        Some(Value::Bool(true))
    );
    let (bits, components) = if image_mask {
        (1u64, 1u64)
    } else {
        let bits = dictionary_uint(dictionary, &[b"BPC", b"BitsPerComponent"])? as u64;
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
            return None;
        }
        let components = match dictionary_get(dictionary, &[b"CS", b"ColorSpace"])? {
            Value::Name(name) => match name.as_slice() {
                b"G" | b"DeviceGray" | b"I" | b"Indexed" => 1,
                b"RGB" | b"DeviceRGB" => 3,
                b"CMYK" | b"DeviceCMYK" => 4,
                _ => return None,
            },
            Value::Array(items) => match items.first() {
                Some(Value::Name(name)) if matches!(name.as_slice(), b"I" | b"Indexed") => 1,
                _ => return None,
            },
            _ => return None,
        };
        (bits, components)
    };
    let row_bits = width.checked_mul(bits)?.checked_mul(components)?;
    row_bits
        .checked_add(7)
        .map(|value| value / 8)?
        .checked_mul(height)
}

fn parse_number(token: &[u8]) -> Option<f64> {
    let (negative, body) = match token.first() {
        Some(b'+') => (false, token.get(1..)?),
        Some(b'-') => (true, token.get(1..)?),
        _ => (false, token),
    };
    let dot = body.iter().position(|byte| *byte == b'.');
    let (integer, fraction) = match dot {
        Some(position) => (body.get(..position)?, body.get(position + 1..)?),
        None => (body, &b""[..]),
    };
    if (integer.is_empty() && fraction.is_empty())
        || !integer.iter().chain(fraction).all(u8::is_ascii_digit)
    {
        return None;
    }
    let text = format!(
        "{}{}.{}",
        if negative { "-" } else { "" },
        if integer.is_empty() {
            "0"
        } else {
            std::str::from_utf8(integer).ok()?
        },
        if fraction.is_empty() {
            "0"
        } else {
            std::str::from_utf8(fraction).ok()?
        }
    );
    text.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn token_at(bytes: &[u8], at: usize, token: &[u8]) -> bool {
    let Some(end) = at.checked_add(token.len()) else {
        return false;
    };
    bytes.get(at..end) == Some(token)
        && bytes
            .get(end)
            .is_none_or(|byte| is_whitespace(*byte) || is_delimiter(*byte))
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

fn is_regular(byte: u8) -> bool {
    !is_whitespace(byte) && !is_delimiter(byte)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn malformed_content(detail: &str) -> AppError {
    AppError::new(
        "MALFORMED_CONTENT",
        "Part of this PDF can't be read",
        "OffPDF couldn't read this PDF's content stream safely.",
    )
    .with_suggestion("Run it through Repair PDF, then try again.")
    .with_details(detail)
}

fn content_too_complex(detail: &str) -> AppError {
    malformed_content(&format!("Content is too complex: {detail}."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn flate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn leaves_plain_content_borrowed_and_decodable() {
        let bytes = b"q 1 0 0 1 72 400 cm BT /F1 12 Tf (Hello) Tj ET Q";
        let prepared = prepare(bytes).unwrap();
        assert!(prepared.sanitized.is_none());
        assert_eq!(prepared.inline_images, 0);
        assert!(lopdf::content::Content::decode(prepared.bytes()).is_ok());
    }

    #[test]
    fn rejects_container_and_literal_nesting_over_limits() {
        let arrays = format!(
            "{}{} q",
            "[".repeat(MAX_NESTING + 1),
            "]".repeat(MAX_NESTING + 1)
        );
        assert_eq!(
            prepare(arrays.as_bytes()).unwrap_err().code,
            "MALFORMED_CONTENT"
        );

        let string = format!(
            "({}{}) Tj",
            "(".repeat(MAX_LITERAL_NESTING),
            ")".repeat(MAX_LITERAL_NESTING)
        );
        assert_eq!(
            prepare(string.as_bytes()).unwrap_err().code,
            "MALFORMED_CONTENT"
        );
    }

    #[test]
    fn rejects_operation_and_operand_budgets() {
        let operations = "q ".repeat(MAX_OPERATIONS + 1);
        assert!(prepare(operations.as_bytes())
            .unwrap_err()
            .details
            .unwrap_or_default()
            .contains("operations"));

        let operands = format!("{}q", "1 ".repeat(MAX_OPERANDS_PER_OPERATOR + 1));
        assert!(prepare(operands.as_bytes())
            .unwrap_err()
            .details
            .unwrap_or_default()
            .contains("operands"));
    }

    #[test]
    fn sanitizes_unfiltered_and_ascii_inline_images() {
        for bytes in [
            &b"q BI /W 1 /H 1 /CS /G /BPC 8 ID x EI Q"[..],
            &b"q BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID 78> EI Q"[..],
            &b"q BI /W 1 /H 1 /CS /G /BPC 8 /F /A85 ID GQ~> EI Q"[..],
        ] {
            let prepared = prepare(bytes).unwrap();
            assert_eq!(prepared.inline_images, 1);
            assert!(!prepared.bytes().windows(2).any(|window| window == b"78"));
            assert!(lopdf::content::Content::decode(prepared.bytes()).is_ok());
        }
    }

    #[test]
    fn sanitizes_multiple_inline_images_without_matching_text_or_comments() {
        let bytes = concat!(
            "% BI /F /DCT ID ignored EI\n",
            "(BI ID EI) Tj ",
            "BI /W 1 /H 1 /CS /G /BPC 8 ID x EI ",
            "BI /W 1 /H 1 /CS /G /BPC 8 ID y EI"
        )
        .as_bytes();
        let prepared = prepare(bytes).unwrap();
        assert_eq!(prepared.inline_images, 2);
        assert!(lopdf::content::Content::decode(prepared.bytes()).is_ok());
    }

    #[test]
    fn flate_proof_ignores_fake_ei_inside_payload() {
        let payload = flate(b"pixels EI still pixels");
        let mut bytes = b"q BI /W 1 /H 1 /CS /G /BPC 8 /F /Fl ID ".to_vec();
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(b" EI Q");
        let prepared = prepare(&bytes).unwrap();
        assert_eq!(prepared.inline_images, 1);
        assert!(lopdf::content::Content::decode(prepared.bytes()).is_ok());
    }

    #[test]
    fn rejects_unproven_or_unterminated_inline_images() {
        for bytes in [
            &b"BI /F /DCT ID fake EI Q"[..],
            &b"BI /W 1 /H 1 /CS /G /BPC 8 ID x"[..],
        ] {
            assert_eq!(prepare(bytes).unwrap_err().code, "MALFORMED_CONTENT");
        }
    }

    #[test]
    fn rejects_malformed_tail_after_inline_image() {
        let bytes = b"BI /W 1 /H 1 /BPC 8 /CS /G ID x EI (\\";
        assert_eq!(prepare(bytes).unwrap_err().code, "MALFORMED_CONTENT");
    }
}
