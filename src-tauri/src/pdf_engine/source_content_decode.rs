//! Bounded decoding for the read-only source classifier.
//!
//! Filtered streams never fall back to their encoded bytes. Every supported
//! decoder checks its output limit while producing data.

use flate2::{Decompress, FlushDecompress, Status};
use lopdf::{Dictionary, Object, Stream};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DecodeError {
    UnsupportedFilter(String),
    Corrupt,
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Flate,
    AsciiHex,
    Ascii85,
}

pub(super) fn decode_stream(stream: &Stream, cap: usize) -> Result<Vec<u8>, DecodeError> {
    let filters = filters_of(&stream.dict)?;
    let mut data: Cow<'_, [u8]> = Cow::Borrowed(&stream.content);
    for filter in filters {
        let decoded = match filter {
            Filter::Flate => inflate_capped(&data, cap)?,
            Filter::AsciiHex => ascii_hex_decode(&data, cap)?,
            Filter::Ascii85 => ascii85_decode(&data, cap)?,
        };
        data = Cow::Owned(decoded);
    }
    if data.len() > cap {
        return Err(DecodeError::TooLarge);
    }
    Ok(data.into_owned())
}

fn filters_of(dict: &Dictionary) -> Result<Vec<Filter>, DecodeError> {
    for key in [&b"F"[..], b"FFilter", b"FDecodeParms"] {
        if dict.has(key) {
            return Err(DecodeError::UnsupportedFilter(
                "external file filter".into(),
            ));
        }
    }
    let names: Vec<&[u8]> = match dict.get(b"Filter").ok() {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Name(name)) => vec![name],
        Some(Object::Array(items)) if items.len() <= 4 => items
            .iter()
            .map(|item| match item {
                Object::Name(name) => Ok(name.as_slice()),
                _ => Err(DecodeError::UnsupportedFilter("malformed /Filter".into())),
            })
            .collect::<Result<_, _>>()?,
        Some(Object::Array(_)) => {
            return Err(DecodeError::UnsupportedFilter(
                "more than four chained filters".into(),
            ))
        }
        Some(_) => return Err(DecodeError::UnsupportedFilter("malformed /Filter".into())),
    };
    check_decode_parms(dict.get(b"DecodeParms").ok())?;
    names.into_iter().map(filter_from_name).collect()
}

fn filter_from_name(name: &[u8]) -> Result<Filter, DecodeError> {
    match name {
        b"FlateDecode" | b"Fl" => Ok(Filter::Flate),
        b"ASCIIHexDecode" | b"AHx" => Ok(Filter::AsciiHex),
        b"ASCII85Decode" | b"A85" => Ok(Filter::Ascii85),
        other => Err(DecodeError::UnsupportedFilter(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}

fn check_decode_parms(parms: Option<&Object>) -> Result<(), DecodeError> {
    let check = |dict: &Dictionary| match dict.get(b"Predictor").ok() {
        None | Some(Object::Integer(1)) => Ok(()),
        Some(_) => Err(DecodeError::UnsupportedFilter("predictor".into())),
    };
    match parms {
        None | Some(Object::Null) => Ok(()),
        Some(Object::Dictionary(dict)) => check(dict),
        Some(Object::Array(items)) => items.iter().try_for_each(|item| match item {
            Object::Null => Ok(()),
            Object::Dictionary(dict) => check(dict),
            _ => Err(DecodeError::UnsupportedFilter(
                "malformed /DecodeParms".into(),
            )),
        }),
        Some(_) => Err(DecodeError::UnsupportedFilter(
            "malformed /DecodeParms".into(),
        )),
    }
}

pub(super) fn inflate_capped(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    let mut decoder = Decompress::new(true);
    let mut output = Vec::new();
    let mut step = vec![0u8; 64 * 1024];
    loop {
        let input_before =
            usize::try_from(decoder.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let output_before =
            usize::try_from(decoder.total_out()).map_err(|_| DecodeError::TooLarge)?;
        let input = data.get(input_before..).ok_or(DecodeError::Corrupt)?;
        let status = decoder
            .decompress(input, &mut step, FlushDecompress::None)
            .map_err(|_| DecodeError::Corrupt)?;
        let input_after = usize::try_from(decoder.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let output_after =
            usize::try_from(decoder.total_out()).map_err(|_| DecodeError::TooLarge)?;
        let produced = output_after
            .checked_sub(output_before)
            .ok_or(DecodeError::Corrupt)?;
        if output.len().saturating_add(produced) > cap {
            return Err(DecodeError::TooLarge);
        }
        output.extend_from_slice(step.get(..produced).ok_or(DecodeError::Corrupt)?);
        match status {
            Status::StreamEnd => return Ok(output),
            Status::Ok | Status::BufError if produced == 0 && input_after == input_before => {
                return Err(DecodeError::Corrupt)
            }
            Status::Ok | Status::BufError => {}
        }
    }
}

/// Returns the encoded byte length of one complete zlib stream while bounding
/// its decoded output. Trailing bytes are intentionally left for the caller.
pub(super) fn inflate_end(data: &[u8], cap: usize) -> Result<usize, DecodeError> {
    let mut decoder = Decompress::new(true);
    let mut step = vec![0u8; 64 * 1024];
    loop {
        let input_before =
            usize::try_from(decoder.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let output_before =
            usize::try_from(decoder.total_out()).map_err(|_| DecodeError::TooLarge)?;
        let input = data.get(input_before..).ok_or(DecodeError::Corrupt)?;
        let status = decoder
            .decompress(input, &mut step, FlushDecompress::None)
            .map_err(|_| DecodeError::Corrupt)?;
        let input_after = usize::try_from(decoder.total_in()).map_err(|_| DecodeError::TooLarge)?;
        let output_after =
            usize::try_from(decoder.total_out()).map_err(|_| DecodeError::TooLarge)?;
        if output_after > cap {
            return Err(DecodeError::TooLarge);
        }
        match status {
            Status::StreamEnd => return Ok(input_after),
            Status::Ok | Status::BufError
                if input_after == input_before && output_after == output_before =>
            {
                return Err(DecodeError::Corrupt)
            }
            Status::Ok | Status::BufError => {}
        }
    }
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn ascii_hex_decode(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    let mut output = Vec::new();
    let mut high = None;
    for &byte in data {
        if byte == b'>' {
            break;
        }
        if is_pdf_whitespace(byte) {
            continue;
        }
        let value = hex_value(byte).ok_or(DecodeError::Corrupt)?;
        match high.take() {
            None => high = Some(value),
            Some(first) => push_capped(&mut output, (first << 4) | value, cap)?,
        }
    }
    if let Some(first) = high {
        push_capped(&mut output, first << 4, cap)?;
    }
    Ok(output)
}

pub(super) fn ascii85_decode(data: &[u8], cap: usize) -> Result<Vec<u8>, DecodeError> {
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    let mut output = Vec::new();
    let mut group = [0u8; 5];
    let mut count = 0usize;
    let mut bytes = body.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        match byte {
            byte if is_pdf_whitespace(byte) => {}
            b'~' if bytes.next() == Some(b'>') => break,
            b'z' if count == 0 => extend_capped(&mut output, &[0, 0, 0, 0], cap)?,
            b'!'..=b'u' => {
                group[count] = byte - b'!';
                count += 1;
                if count == 5 {
                    extend_capped(&mut output, &ascii85_group(&group)?, cap)?;
                    count = 0;
                }
            }
            _ => return Err(DecodeError::Corrupt),
        }
    }
    match count {
        0 => {}
        1 => return Err(DecodeError::Corrupt),
        count => {
            for slot in group.iter_mut().skip(count) {
                *slot = 84;
            }
            let decoded = ascii85_group(&group)?;
            extend_capped(&mut output, &decoded[..count - 1], cap)?;
        }
    }
    Ok(output)
}

fn ascii85_group(group: &[u8; 5]) -> Result<[u8; 4], DecodeError> {
    let value = group
        .iter()
        .fold(0u64, |acc, digit| acc * 85 + u64::from(*digit));
    Ok(u32::try_from(value)
        .map_err(|_| DecodeError::Corrupt)?
        .to_be_bytes())
}

fn push_capped(output: &mut Vec<u8>, byte: u8, cap: usize) -> Result<(), DecodeError> {
    if output.len() >= cap {
        return Err(DecodeError::TooLarge);
    }
    output.push(byte);
    Ok(())
}

fn extend_capped(output: &mut Vec<u8>, bytes: &[u8], cap: usize) -> Result<(), DecodeError> {
    if output.len().saturating_add(bytes.len()) > cap {
        return Err(DecodeError::TooLarge);
    }
    output.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn flate(data: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn stream(filter: Object, data: Vec<u8>) -> Stream {
        let mut dict = Dictionary::new();
        dict.set("Filter", filter);
        Stream::new(dict, data)
    }

    #[test]
    fn flate_is_strict_and_capped() {
        let encoded = flate(b"hello");
        let valid = stream(Object::Name(b"FlateDecode".to_vec()), encoded.clone());
        assert_eq!(decode_stream(&valid, 5).unwrap(), b"hello");
        assert_eq!(decode_stream(&valid, 4), Err(DecodeError::TooLarge));
        let mut with_trailing = encoded.clone();
        with_trailing.extend_from_slice(b" trailing");
        assert_eq!(inflate_end(&with_trailing, 5).unwrap(), encoded.len());
        assert_eq!(inflate_end(&with_trailing, 4), Err(DecodeError::TooLarge));
        assert_eq!(
            inflate_end(&encoded[..encoded.len() - 1], 5),
            Err(DecodeError::Corrupt)
        );

        let corrupt = stream(
            Object::Name(b"FlateDecode".to_vec()),
            b"not zlib data".to_vec(),
        );
        assert_eq!(decode_stream(&corrupt, 1024), Err(DecodeError::Corrupt));

        let mut truncated_data = flate(b"hello");
        truncated_data.truncate(truncated_data.len().saturating_sub(2));
        let truncated = stream(Object::Name(b"FlateDecode".to_vec()), truncated_data);
        assert_eq!(decode_stream(&truncated, 1024), Err(DecodeError::Corrupt));
    }

    #[test]
    fn common_ascii_filter_chain_is_bounded() {
        let chained = stream(
            Object::Array(vec![
                Object::Name(b"ASCIIHexDecode".to_vec()),
                Object::Name(b"FlateDecode".to_vec()),
            ]),
            flate(b"hello")
                .into_iter()
                .flat_map(|byte| format!("{byte:02x}").into_bytes())
                .chain([b'>'])
                .collect(),
        );
        assert_eq!(decode_stream(&chained, 1024).unwrap(), b"hello");
        assert_eq!(decode_stream(&chained, 4), Err(DecodeError::TooLarge));
    }

    #[test]
    fn unsupported_filters_fail_closed() {
        let lzw = stream(Object::Name(b"LZWDecode".to_vec()), vec![0; 8]);
        assert!(matches!(
            decode_stream(&lzw, 1024),
            Err(DecodeError::UnsupportedFilter(name)) if name == "LZWDecode"
        ));
    }
}
