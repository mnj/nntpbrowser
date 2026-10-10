//! Historical uuencode and POSIX begin-base64 framing. Decode one complete file;
//! split articles without a complete block need ordered assembly by the caller.
//! Format reference: https://man.openbsd.org/uuencode.5

use super::{DecodeError, Part};
use base64::Engine;
mod files;
pub(super) use files::write_files;

fn invalid(message: &str) -> DecodeError {
    DecodeError::Uuencode(message.into())
}

pub(super) fn decode(article: &[u8], cap: usize) -> Result<Option<Part>, DecodeError> {
    decode_lines(article_lines(article), cap)
}

fn article_lines(article: &[u8]) -> impl Iterator<Item = &[u8]> {
    article.split_inclusive(|byte| *byte == b'\n').map(|line| {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        line.strip_suffix(b"\r").unwrap_or(line)
    })
}

pub(super) fn decode_parts(articles: &[&[u8]], cap: usize) -> Result<Part, DecodeError> {
    decode_lines(
        articles.iter().flat_map(|article| article_lines(article)),
        cap,
    )?
    .ok_or_else(|| invalid("ordered fragments have no begin header"))
}

fn decode_lines<'a>(
    mut lines: impl Iterator<Item = &'a [u8]>,
    cap: usize,
) -> Result<Option<Part>, DecodeError> {
    let header =
        lines.find(|line| line.starts_with(b"begin ") || line.starts_with(b"begin-base64 "));
    let Some(header) = header else {
        return Ok(None);
    };
    let base64 = header.starts_with(b"begin-base64 ");
    let rest = &header[if base64 { 13 } else { 6 }..];
    let separator = rest
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or_else(|| invalid("header has no filename"))?;
    let mode = &rest[..separator];
    if !matches!(mode.len(), 3 | 4) || !mode.iter().all(|byte| (b'0'..=b'7').contains(byte)) {
        return Err(invalid("invalid octal mode"));
    }
    let name = std::str::from_utf8(&rest[separator + 1..])
        .map_err(|_| invalid("filename is not UTF-8"))?;
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(invalid("invalid filename"));
    }
    let mut bytes = Vec::new();
    if base64 {
        decode_base64(&mut lines, &mut bytes, cap)?;
    } else {
        decode_classic(&mut lines, &mut bytes, cap)?;
    }
    if lines.any(|line| line.starts_with(b"begin ") || line.starts_with(b"begin-base64 ")) {
        return Err(invalid(
            "multiple complete attachments cannot form one multipart file",
        ));
    }
    if bytes.is_empty() {
        return Err(DecodeError::Empty);
    }
    Ok(Some(Part {
        filename: Some(name.into()),
        begin: 0,
        file_size: Some(bytes.len() as u64),
        crc32: crc32fast::hash(&bytes),
        data: bytes,
    }))
}

/// Continuation fragments lack filenames and offsets. Recognize only complete
/// encoded lines, then require a full ordered stream before writing any file.
pub(super) fn is_fragment(article: &[u8]) -> bool {
    if is_classic_fragment(article) {
        return true;
    }
    let mut data = false;
    for line in article_lines(article) {
        if line == b"====" {
            return true;
        }
        if line.is_empty()
            || !line.len().is_multiple_of(4)
            || !line
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        {
            return false;
        }
        data = true;
    }
    data
}

fn is_classic_fragment(article: &[u8]) -> bool {
    if article_lines(article)
        .any(|line| line.starts_with(b"begin ") || line.starts_with(b"begin-base64 "))
    {
        return true;
    }
    let mut encoded = false;
    for line in article_lines(article) {
        if line == b"end" {
            return true;
        }
        let Some((&count, data)) = line.split_first() else {
            return false;
        };
        let Ok(count) = value(count) else {
            return false;
        };
        let required = (count as usize).div_ceil(3) * 4;
        if data.len() < required
            || data[..required].iter().any(|byte| value(*byte).is_err())
            || !data[required..].iter().all(u8::is_ascii_whitespace)
        {
            return false;
        }
        encoded = true;
    }
    encoded
}

fn extend(output: &mut Vec<u8>, data: &[u8], cap: usize) -> Result<(), DecodeError> {
    if data.len() > cap.saturating_sub(output.len()) {
        return Err(DecodeError::TooLarge(cap));
    }
    output.extend_from_slice(data);
    Ok(())
}

fn value(byte: u8) -> Result<u8, DecodeError> {
    match byte {
        b' '..=b'`' => Ok((byte - b' ') & 63),
        _ => Err(invalid("invalid encoded character")),
    }
}

fn decode_classic<'a>(
    mut lines: impl Iterator<Item = &'a [u8]>,
    output: &mut Vec<u8>,
    cap: usize,
) -> Result<(), DecodeError> {
    for line in lines.by_ref() {
        let Some((&count, encoded)) = line.split_first() else {
            return Err(invalid("empty encoded line"));
        };
        let count = value(count)? as usize;
        if count == 0 {
            if !encoded.iter().all(u8::is_ascii_whitespace) {
                return Err(invalid("invalid zero-length line"));
            }
            if lines.next() != Some(b"end".as_slice()) {
                return Err(invalid(
                    "missing end trailer; multipart fragments require ordered assembly",
                ));
            }
            return Ok(());
        }
        // A line's decoded count is at most 63 bytes. Check the global bound
        // before decoding or extending rather than allocating from wire metadata.
        if count > cap.saturating_sub(output.len()) {
            return Err(DecodeError::TooLarge(cap));
        }
        let mut decoded = [0u8; 63];
        decode_classic_line(encoded, count, &mut decoded)?;
        extend(output, &decoded[..count], cap)?;
    }
    Err(invalid(
        "missing zero-length line and end trailer; multipart fragments require ordered assembly",
    ))
}

fn decode_classic_line(
    encoded: &[u8],
    count: usize,
    out: &mut [u8; 63],
) -> Result<(), DecodeError> {
    let required = count.div_ceil(3) * 4;
    if encoded.len() < required {
        return Err(invalid("truncated encoded line"));
    }
    if !encoded[required..].iter().all(u8::is_ascii_whitespace) {
        return Err(invalid("extra encoded characters"));
    }
    let mut offset = 0;
    for quartet in encoded[..required].as_chunks::<4>().0 {
        let a = value(quartet[0])?;
        let b = value(quartet[1])?;
        let c = value(quartet[2])?;
        let d = value(quartet[3])?;
        let decoded = [a << 2 | b >> 4, b << 4 | c >> 2, c << 6 | d];
        let take = (count - offset).min(3);
        out[offset..offset + take].copy_from_slice(&decoded[..take]);
        offset += take;
    }
    Ok(())
}

fn decode_base64<'a>(
    lines: impl Iterator<Item = &'a [u8]>,
    output: &mut Vec<u8>,
    cap: usize,
) -> Result<(), DecodeError> {
    let mut quartet = Vec::with_capacity(4);
    let mut padded = false;
    for line in lines {
        if line == b"====" {
            if !quartet.is_empty() {
                return Err(invalid("incomplete base64 quartet"));
            }
            return Ok(());
        }
        for &byte in line {
            // POSIX uudecode ignores characters outside the Base64 alphabet.
            if !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')) {
                continue;
            }
            if padded {
                return Err(invalid("data follows base64 padding"));
            }
            quartet.push(byte);
            if quartet.len() == 4 {
                let mut decoded = [0u8; 3];
                let count = base64::engine::general_purpose::STANDARD
                    .decode_slice(&quartet, &mut decoded)
                    .map_err(|_| invalid("invalid base64 quartet"))?;
                padded = quartet.contains(&b'=');
                extend(output, &decoded[..count], cap)?;
                quartet.clear();
            }
        }
    }
    Err(invalid("missing base64 ==== trailer"))
}

#[cfg(test)]
mod tests;
