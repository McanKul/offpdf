//! Bounded parser for the cross-reference chain lopdf follows.

use super::{file_too_complex, MAX_OBJECTS};
use crate::error::AppError;
use crate::pdf_engine::source_content_decode::{self, DecodeError};
use std::borrow::Cow;
use std::collections::HashSet;

const MAX_XREF_CHAIN: usize = 64;
const XREF_TAIL_SEARCH_BYTES: usize = 2 << 10;
const XREF_STREAM_MAX_DECODED: usize = 64 << 20;
const XREF_STREAM_FIELD_WIDTH_MAX: i64 = 8;
const XREF_PREDICTOR_ROW_MAX: f64 = 1_024.0;
const DICT_TOKENS_MAX: usize = 100_000;
const DICT_NESTING_MAX: usize = 32;
const STRING_BYTES_MAX: usize = 1 << 20;
const STRING_PAREN_NESTING_MAX: usize = 100;

pub(super) fn check(bytes: &[u8]) -> Result<(), AppError> {
    let start = find_startxref(bytes).ok_or_else(|| invalid_xref("no startxref"))?;
    let mut queue = vec![start];
    let mut visited = HashSet::new();

    while let Some(offset) = queue.pop() {
        if !visited.insert(offset) {
            continue;
        }
        if visited.len() > MAX_XREF_CHAIN {
            return Err(file_too_complex("cross-reference chain too long"));
        }

        let at = skip_whitespace(bytes, offset);
        let dictionary = if starts_with(bytes, at, b"xref") {
            let trailer = table_trailer(bytes, at + 4).ok_or_else(|| invalid_xref("no trailer"))?;
            scan_dictionary(bytes, trailer + 7)?.0
        } else {
            check_xref_stream(bytes, at)?
        };

        if let Some(offset) = dictionary_uint(&dictionary, b"XRefStm") {
            queue.push(offset);
        }
        if dictionary_get(&dictionary, b"Prev").is_some() {
            let offset =
                dictionary_uint(&dictionary, b"Prev").ok_or_else(|| invalid_xref("bad /Prev"))?;
            queue.push(offset);
        }
    }
    Ok(())
}

fn find_startxref(bytes: &[u8]) -> Option<usize> {
    let tail_start = bytes.len().saturating_sub(XREF_TAIL_SEARCH_BYTES);
    let tail = bytes.get(tail_start..)?;
    let relative = tail.windows(9).rposition(|window| window == b"startxref")?;
    let (offset, _) = parse_unsigned(bytes, skip_whitespace(bytes, tail_start + relative + 9))?;
    usize::try_from(offset)
        .ok()
        .filter(|offset| *offset < bytes.len())
}

fn table_trailer(bytes: &[u8], from: usize) -> Option<usize> {
    let mut index = from;
    loop {
        match *bytes.get(index)? {
            byte if byte.is_ascii_digit()
                || byte == b'n'
                || byte == b'f'
                || is_whitespace(byte) =>
            {
                index += 1;
            }
            b'%' => {
                index += bytes
                    .get(index..)?
                    .iter()
                    .take_while(|byte| !matches!(byte, b'\r' | b'\n'))
                    .count();
            }
            _ => break,
        }
    }
    starts_with(bytes, index, b"trailer").then_some(index)
}

fn check_xref_stream(bytes: &[u8], at: usize) -> Result<Dictionary, AppError> {
    let invalid = |message: &str| invalid_xref(&format!("{message} at byte {at}"));
    let (_, index) = parse_unsigned(bytes, at).ok_or_else(|| invalid("no xref"))?;
    let (_, index) =
        parse_unsigned(bytes, skip_whitespace(bytes, index)).ok_or_else(|| invalid("no xref"))?;
    let index = skip_whitespace(bytes, index);
    if !starts_with(bytes, index, b"obj") {
        return Err(invalid("no xref"));
    }

    let (dictionary, end) = scan_dictionary(bytes, index + 3)?;
    if !matches!(dictionary_get(&dictionary, b"Type"), Some(Value::Name(name)) if name == b"XRef") {
        return Err(invalid("not an xref stream"));
    }
    let stream = skip_whitespace(bytes, end);
    if !starts_with(bytes, stream, b"stream") {
        return Err(invalid("xref stream without data"));
    }

    let mut data_start = stream + 6;
    if bytes.get(data_start) == Some(&b'\r') {
        data_start += 1;
    }
    if bytes.get(data_start) == Some(&b'\n') {
        data_start += 1;
    }
    let by_length = dictionary_uint(&dictionary, b"Length")
        .and_then(|length| data_start.checked_add(length))
        .filter(|end| starts_with(bytes, skip_whitespace(bytes, *end), b"endstream"));
    let data_end = match by_length {
        Some(end) => end,
        None => find_from(bytes, data_start, b"endstream")
            .ok_or_else(|| invalid("unterminated xref stream"))?,
    };
    let payload = bytes.get(data_start..data_end).unwrap_or_default();
    decode_xref_payload(&dictionary, payload)?;
    check_xref_fields(&dictionary, at)?;
    Ok(dictionary)
}

fn decode_xref_payload(dictionary: &Dictionary, payload: &[u8]) -> Result<(), AppError> {
    let filters: Vec<&[u8]> = match dictionary_get(dictionary, b"Filter") {
        None => Vec::new(),
        Some(Value::Name(name)) => vec![name],
        Some(Value::Array(items)) if items.len() <= 4 => items
            .iter()
            .map(|item| match item {
                Value::Name(name) => Ok(name.as_slice()),
                _ => Err(file_too_complex("xref stream filter")),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(file_too_complex("xref stream filter")),
    };

    let mut data = Cow::Borrowed(payload);
    for filter in filters {
        let decoded = match filter {
            b"FlateDecode" | b"Fl" => {
                source_content_decode::inflate_capped(&data, XREF_STREAM_MAX_DECODED)
            }
            b"ASCIIHexDecode" | b"AHx" => {
                source_content_decode::ascii_hex_decode(&data, XREF_STREAM_MAX_DECODED)
            }
            b"ASCII85Decode" | b"A85" => {
                source_content_decode::ascii85_decode(&data, XREF_STREAM_MAX_DECODED)
            }
            _ => return Err(file_too_complex("xref stream filter")),
        };
        data = Cow::Owned(decoded.map_err(|error| match error {
            DecodeError::TooLarge => file_too_complex("xref stream too large"),
            other => invalid_xref(&format!("xref stream data: {other:?}")),
        })?);
    }
    if data.len() > XREF_STREAM_MAX_DECODED {
        return Err(file_too_complex("xref stream too large"));
    }
    Ok(())
}

fn check_xref_fields(dictionary: &Dictionary, at: usize) -> Result<(), AppError> {
    let invalid = |message: &str| invalid_xref(&format!("{message} at byte {at}"));
    let too_large = |message: &str| file_too_complex(&format!("{message} at byte {at}"));
    let integers = |value: Option<&Value>| -> Option<Vec<i64>> {
        match value? {
            Value::Array(items) => items
                .iter()
                .map(|item| match item {
                    Value::Number { value, integer } if *integer && value.abs() < 1e15 => {
                        Some(*value as i64)
                    }
                    _ => None,
                })
                .collect(),
            _ => None,
        }
    };

    let widths =
        integers(dictionary_get(dictionary, b"W")).ok_or_else(|| invalid("xref stream /W"))?;
    if widths.len() != 3
        || widths
            .iter()
            .any(|width| !(0..=XREF_STREAM_FIELD_WIDTH_MAX).contains(width))
        || widths.iter().sum::<i64>() == 0
    {
        return Err(invalid("xref stream /W"));
    }

    let index = match dictionary_get(dictionary, b"Index") {
        Some(value) => integers(Some(value)).ok_or_else(|| invalid("xref stream /Index"))?,
        None => vec![
            0,
            dictionary_uint(dictionary, b"Size").ok_or_else(|| invalid("xref stream /Size"))?
                as i64,
        ],
    };
    if index.len() % 2 != 0 || index.iter().any(|value| *value < 0) {
        return Err(invalid("xref stream /Index"));
    }
    let count = index
        .iter()
        .skip(1)
        .step_by(2)
        .try_fold(0i64, |sum, value| sum.checked_add(*value));
    if count.is_none_or(|count| count > MAX_OBJECTS as i64) {
        return Err(too_large("xref stream lists too many objects"));
    }
    if !predictor_row_ok(dictionary) {
        return Err(too_large("xref stream predictor row too large"));
    }
    Ok(())
}

fn predictor_row_ok(dictionary: &Dictionary) -> bool {
    let Some(Value::Dictionary(parameters)) = dictionary_get(dictionary, b"DecodeParms") else {
        return true;
    };
    let number = |key: &[u8], default: f64| match dictionary_get(parameters, key) {
        Some(Value::Number { value, .. }) => *value,
        _ => default,
    };
    if !(10.0..=15.0).contains(&number(b"Predictor", 1.0)) {
        return true;
    }
    let row = number(b"Columns", 1.0).max(1.0)
        * number(b"Colors", 1.0).max(1.0)
        * number(b"BitsPerComponent", 8.0).max(8.0)
        / 8.0;
    row.is_finite() && row <= XREF_PREDICTOR_ROW_MAX
}

type Dictionary = Vec<(Vec<u8>, Value)>;

#[derive(Debug)]
enum Value {
    Number { value: f64, integer: bool },
    Name(Vec<u8>),
    Array(Vec<Value>),
    Dictionary(Dictionary),
    Other,
}

fn dictionary_get<'a>(dictionary: &'a Dictionary, key: &[u8]) -> Option<&'a Value> {
    dictionary
        .iter()
        .rev()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
}

fn dictionary_uint(dictionary: &Dictionary, key: &[u8]) -> Option<usize> {
    match dictionary_get(dictionary, key)? {
        Value::Number {
            value,
            integer: true,
        } if *value >= 0.0 && *value < 1e15 => Some(*value as usize),
        _ => None,
    }
}

#[derive(Debug)]
enum Token {
    Value(Value),
    Word(bool),
    DictionaryOpen,
    DictionaryClose,
    ArrayOpen,
    ArrayClose,
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_space();
        let start = self.position;
        let Some(byte) = self.bytes.get(start).copied() else {
            return Ok(None);
        };
        let token = match byte {
            b'(' => {
                self.literal_string()?;
                Token::Value(Value::Other)
            }
            b'<' if self.bytes.get(start + 1) == Some(&b'<') => {
                self.position += 2;
                Token::DictionaryOpen
            }
            b'<' => {
                self.hex_string()?;
                Token::Value(Value::Other)
            }
            b'>' if self.bytes.get(start + 1) == Some(&b'>') => {
                self.position += 2;
                Token::DictionaryClose
            }
            b'>' | b')' | b'{' | b'}' => {
                return Err(ParseError::Malformed(start, "stray delimiter"));
            }
            b'[' => {
                self.position += 1;
                Token::ArrayOpen
            }
            b']' => {
                self.position += 1;
                Token::ArrayClose
            }
            b'/' => Token::Value(Value::Name(self.name()?)),
            _ => self.regular()?,
        };
        Ok(Some(token))
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

    fn regular(&mut self) -> Result<Token, ParseError> {
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| is_regular(*byte))
        {
            self.position += 1;
        }
        let text = self.bytes.get(start..self.position).unwrap_or_default();
        if text.is_empty() {
            return Err(ParseError::Malformed(start, "empty token"));
        }
        if text == b"true" || text == b"false" || text == b"null" {
            return Ok(Token::Value(Value::Other));
        }
        let numeric = text
            .first()
            .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.'));
        if numeric {
            let value = parse_number(text).ok_or(ParseError::Malformed(start, "bad number"))?;
            return Ok(Token::Value(Value::Number {
                value,
                integer: !text.contains(&b'.'),
            }));
        }
        Ok(Token::Word(text == b"R"))
    }

    fn name(&mut self) -> Result<Vec<u8>, ParseError> {
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
            if output.len() > STRING_BYTES_MAX {
                return Err(ParseError::TooComplex("xref dictionary name"));
            }
        }
        Ok(output)
    }

    fn literal_string(&mut self) -> Result<(), ParseError> {
        let start = self.position;
        self.position += 1;
        let mut depth = 1usize;
        let mut output_bytes = 0usize;
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(ParseError::Malformed(start, "unterminated string"));
            };
            self.position += 1;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.bytes.get(self.position).copied() else {
                        return Err(ParseError::Malformed(start, "unterminated string"));
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
                            output_bytes += 1;
                        }
                        b'\r' => {
                            if self.bytes.get(self.position) == Some(&b'\n') {
                                self.position += 1;
                            }
                        }
                        b'\n' => {}
                        _ => output_bytes += 1,
                    }
                }
                b'(' => {
                    depth += 1;
                    if depth > STRING_PAREN_NESTING_MAX {
                        return Err(ParseError::TooComplex("xref dictionary string nesting"));
                    }
                    output_bytes += 1;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    output_bytes += 1;
                }
                b'\r' => {
                    if self.bytes.get(self.position) == Some(&b'\n') {
                        self.position += 1;
                    }
                    output_bytes += 1;
                }
                _ => output_bytes += 1,
            }
            if output_bytes > STRING_BYTES_MAX {
                return Err(ParseError::TooComplex("xref dictionary string"));
            }
        }
        Ok(())
    }

    fn hex_string(&mut self) -> Result<(), ParseError> {
        let start = self.position;
        self.position += 1;
        let mut digits = 0usize;
        loop {
            let Some(byte) = self.bytes.get(self.position).copied() else {
                return Err(ParseError::Malformed(start, "unterminated hex string"));
            };
            self.position += 1;
            if byte == b'>' {
                return Ok(());
            }
            if is_whitespace(byte) {
                continue;
            }
            if hex_digit(byte).is_none() {
                return Err(ParseError::Malformed(start, "bad hex string"));
            }
            digits += 1;
            if digits.div_ceil(2) > STRING_BYTES_MAX {
                return Err(ParseError::TooComplex("xref dictionary string"));
            }
        }
    }
}

struct Frame {
    dictionary: bool,
    values: Vec<Value>,
}

fn scan_dictionary(bytes: &[u8], start: usize) -> Result<(Dictionary, usize), AppError> {
    let mut reader = Reader {
        bytes,
        position: start,
    };
    if !matches!(
        reader.next().map_err(parse_error)?,
        Some(Token::DictionaryOpen)
    ) {
        return Err(invalid_xref(&format!(
            "dictionary expected at byte {start}"
        )));
    }
    let mut token_count = 1usize;
    let mut frames = vec![Frame {
        dictionary: true,
        values: Vec::new(),
    }];

    loop {
        let token = reader
            .next()
            .map_err(parse_error)?
            .ok_or_else(|| invalid_xref("unterminated dictionary"))?;
        token_count = token_count.saturating_add(1);
        if token_count > DICT_TOKENS_MAX {
            return Err(file_too_complex("xref dictionary tokens"));
        }

        match token {
            Token::DictionaryOpen | Token::ArrayOpen => {
                if frames.len() >= DICT_NESTING_MAX {
                    return Err(file_too_complex("xref dictionary nesting"));
                }
                frames.push(Frame {
                    dictionary: matches!(token, Token::DictionaryOpen),
                    values: Vec::new(),
                });
            }
            Token::DictionaryClose | Token::ArrayClose => {
                let expect_dictionary = matches!(token, Token::DictionaryClose);
                let frame = frames
                    .pop()
                    .ok_or_else(|| invalid_xref("stray dictionary delimiter"))?;
                if frame.dictionary != expect_dictionary {
                    return Err(invalid_xref("mismatched dictionary delimiter"));
                }
                let value = finish_frame(frame)?;
                if let Some(parent) = frames.last_mut() {
                    parent.values.push(value);
                } else {
                    return match value {
                        Value::Dictionary(dictionary) => Ok((dictionary, reader.position)),
                        _ => Err(invalid_xref("dictionary expected")),
                    };
                }
            }
            Token::Value(value) => frames
                .last_mut()
                .ok_or_else(|| invalid_xref("value outside dictionary"))?
                .values
                .push(value),
            Token::Word(true) => {
                let values = &mut frames
                    .last_mut()
                    .ok_or_else(|| invalid_xref("reference outside dictionary"))?
                    .values;
                let generation = values.pop();
                let object = values.pop();
                if !matches!(generation, Some(Value::Number { integer: true, .. }))
                    || !matches!(object, Some(Value::Number { integer: true, .. }))
                {
                    return Err(invalid_xref("bad object reference"));
                }
                values.push(Value::Other);
            }
            Token::Word(false) => frames
                .last_mut()
                .ok_or_else(|| invalid_xref("keyword outside dictionary"))?
                .values
                .push(Value::Other),
        }
    }
}

fn finish_frame(frame: Frame) -> Result<Value, AppError> {
    if !frame.dictionary {
        return Ok(Value::Array(frame.values));
    }
    let mut entries = Vec::with_capacity(frame.values.len() / 2);
    let mut values = frame.values.into_iter();
    while let Some(key) = values.next() {
        let Value::Name(key) = key else {
            return Err(invalid_xref("xref dictionary key is not a name"));
        };
        let value = values
            .next()
            .ok_or_else(|| invalid_xref("xref dictionary value is missing"))?;
        entries.push((key, value));
    }
    Ok(Value::Dictionary(entries))
}

#[derive(Debug)]
enum ParseError {
    Malformed(usize, &'static str),
    TooComplex(&'static str),
}

fn parse_error(error: ParseError) -> AppError {
    match error {
        ParseError::Malformed(at, what) => invalid_xref(&format!("{what} at byte {at}")),
        ParseError::TooComplex(what) => file_too_complex(what),
    }
}

fn parse_number(bytes: &[u8]) -> Option<f64> {
    let (negative, body) = match bytes.first() {
        Some(b'+') => (false, bytes.get(1..)?),
        Some(b'-') => (true, bytes.get(1..)?),
        _ => (false, bytes),
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
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

fn parse_unsigned(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let digits = bytes
        .get(at..)?
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || digits > 19 {
        return None;
    }
    let text = std::str::from_utf8(bytes.get(at..at + digits)?).ok()?;
    Some((text.parse().ok()?, at + digits))
}

fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut index = from;
    while let Some(relative) = haystack
        .get(index..)?
        .iter()
        .position(|byte| *byte == first)
    {
        let position = index + relative;
        if haystack.get(position..position.checked_add(needle.len())?) == Some(needle) {
            return Some(position);
        }
        index = position + 1;
    }
    None
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(|byte| is_whitespace(*byte)) {
        index += 1;
    }
    index
}

fn starts_with(bytes: &[u8], at: usize, expected: &[u8]) -> bool {
    bytes
        .get(at..)
        .is_some_and(|rest| rest.starts_with(expected))
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

fn invalid_xref(detail: &str) -> AppError {
    AppError::new(
        "INVALID_PDF",
        "The selected file is not a valid PDF",
        "OffPDF could not read this PDF's cross-reference data.",
    )
    .with_suggestion("Make sure the file is a real PDF and is not corrupted.")
    .with_details(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::fmt::Write as _;
    use std::io::Write as _;

    const OBJECTS: [&str; 3] = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    ];

    struct File {
        bytes: Vec<u8>,
        offsets: Vec<usize>,
    }

    impl File {
        fn new() -> Self {
            let mut file = Self {
                bytes: b"%PDF-1.7\n".to_vec(),
                offsets: Vec::new(),
            };
            for (index, body) in OBJECTS.iter().enumerate() {
                file.offsets.push(file.bytes.len());
                file.bytes
                    .extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
            }
            file
        }

        fn xref_stream(&mut self, id: u32, before: &str, after: &str) -> usize {
            let mut rows = vec![0, 0, 0, 0, 0, 0xff, 0xff];
            for offset in &self.offsets {
                rows.push(1);
                rows.extend_from_slice(&u32::try_from(*offset).unwrap().to_be_bytes());
                rows.extend_from_slice(&[0, 0]);
            }
            let at = self.bytes.len();
            self.bytes.extend_from_slice(
                format!(
                    "{id} 0 obj\n<< {before} /Type /XRef /Size 4 /W [1 4 2] {after} /Length {} >>\nstream\n",
                    rows.len()
                )
                .as_bytes(),
            );
            self.bytes.extend_from_slice(&rows);
            self.bytes.extend_from_slice(b"\nendstream\nendobj\n");
            at
        }

        fn table(&mut self, after: &str) -> usize {
            let at = self.bytes.len();
            let mut table = "xref\n0 4\n0000000000 65535 f \n".to_string();
            for offset in &self.offsets {
                let _ = writeln!(table, "{offset:010} 00000 n ");
            }
            table.push_str(after);
            self.bytes.extend_from_slice(table.as_bytes());
            at
        }

        fn finish(mut self, start: usize) -> Vec<u8> {
            self.bytes
                .extend_from_slice(format!("startxref\n{start}\n%%EOF\n").as_bytes());
            self.bytes
        }
    }

    fn main_stream(before: &str, after: &str) -> Vec<u8> {
        let mut file = File::new();
        let xref = file.xref_stream(9, before, &format!("/Root 1 0 R {after}"));
        file.finish(xref)
    }

    fn assert_error(bytes: &[u8], code: &str, detail: &str) {
        let error = check(bytes).unwrap_err();
        assert_eq!(error.code, code, "{:?}", error.details);
        assert!(error.details.unwrap_or_default().contains(detail));
    }

    #[test]
    fn accepts_classic_table_and_xref_stream() {
        let mut classic = File::new();
        let table = classic.table("% producer note\ntrailer\n<< /Size 4 /Root 1 0 R >>\n");
        let classic = classic.finish(table);
        check(&classic).unwrap();
        assert_eq!(
            lopdf::Document::load_mem(&classic)
                .unwrap()
                .get_pages()
                .len(),
            1
        );

        let stream = main_stream("", "");
        check(&stream).unwrap();
        assert_eq!(
            lopdf::Document::load_mem(&stream)
                .unwrap()
                .get_pages()
                .len(),
            1
        );
    }

    #[test]
    fn accepts_bounded_flate_xref_payload() {
        let mut file = File::new();
        let mut rows = vec![0, 0, 0, 0, 0, 0xff, 0xff];
        for offset in &file.offsets {
            rows.push(1);
            rows.extend_from_slice(&u32::try_from(*offset).unwrap().to_be_bytes());
            rows.extend_from_slice(&[0, 0]);
        }
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&rows).unwrap();
        let compressed = encoder.finish().unwrap();
        let at = file.bytes.len();
        file.bytes.extend_from_slice(
            format!(
                "9 0 obj\n<< /Type /XRef /Size 4 /Root 1 0 R /W [1 4 2] /Filter /FlateDecode /Length {} >>\nstream\n",
                compressed.len()
            )
            .as_bytes(),
        );
        file.bytes.extend_from_slice(&compressed);
        file.bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let bytes = file.finish(at);
        check(&bytes).unwrap();
        assert_eq!(
            lopdf::Document::load_mem(&bytes).unwrap().get_pages().len(),
            1
        );
    }

    #[test]
    fn duplicate_fields_use_the_last_value() {
        assert_error(
            &main_stream("", "/Size 3000000"),
            "FILE_TOO_COMPLEX",
            "too many objects",
        );
        check(&main_stream("/Size 3000000", "")).unwrap();
    }

    #[test]
    fn rejects_real_xref_array_fields() {
        assert_error(&main_stream("", "/Index [0 4.0]"), "INVALID_PDF", "/Index");
        assert_error(&main_stream("", "/W [1 4.0 2]"), "INVALID_PDF", "/W");
    }

    #[test]
    fn rejects_large_predictor_rows() {
        assert_error(
            &main_stream(
                "",
                "/DecodeParms << /Predictor 12 /Columns 2000 /Colors 2000 >>",
            ),
            "FILE_TOO_COMPLEX",
            "predictor row",
        );
    }

    #[test]
    fn sums_xref_index_counts_without_overflow() {
        let index = "0 999999999999999 ".repeat(10_000);
        assert_error(
            &main_stream("", &format!("/Index [{index}]")),
            "FILE_TOO_COMPLEX",
            "too many objects",
        );
    }

    #[test]
    fn follows_xrefstm_from_xref_streams() {
        let mut file = File::new();
        let secondary = file.xref_stream(
            10,
            "",
            "/DecodeParms << /Predictor 12 /Columns 2000 /Colors 2000 >>",
        );
        let main = file.xref_stream(11, "", &format!("/Root 1 0 R /XRefStm {secondary}"));
        assert_error(&file.finish(main), "FILE_TOO_COMPLEX", "predictor row");
    }

    #[test]
    fn rejects_overlong_xref_chains() {
        let mut file = File::new();
        let mut previous = None;
        for id in 10..10 + MAX_XREF_CHAIN as u32 + 1 {
            let suffix = previous.map_or_else(
                || "/Root 1 0 R".to_string(),
                |offset| format!("/Prev {offset}"),
            );
            previous = Some(file.xref_stream(id, "", &suffix));
        }
        assert_error(
            &file.finish(previous.unwrap()),
            "FILE_TOO_COMPLEX",
            "chain too long",
        );
    }

    #[test]
    fn ignores_trailer_text_inside_table_comments() {
        let mut file = File::new();
        let bad = file.xref_stream(
            10,
            "",
            "/DecodeParms << /Predictor 12 /Columns 2000 /Colors 2000 >>",
        );
        let table = file.table(&format!(
            "%trailer << /Size 4 /Root 1 0 R >>\ntrailer\n<< /Size 4 /Root 1 0 R /Prev {bad} >>\n"
        ));
        assert_error(&file.finish(table), "FILE_TOO_COMPLEX", "predictor row");
    }

    #[test]
    fn rejects_deep_xref_dictionaries_before_lopdf() {
        let mut file = File::new();
        let nesting = format!("{}{}", "[".repeat(40), "]".repeat(40));
        let table = file.table(&format!(
            "trailer\n<< /Size 4 /Root 1 0 R /Deep {nesting} >>\n"
        ));
        assert_error(&file.finish(table), "FILE_TOO_COMPLEX", "nesting");
    }
}
