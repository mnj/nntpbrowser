//! Decode yEnc and complete uuencoded article bodies before writing file parts.

use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use thiserror::Error;
use yenc_simd::decode_yenc;

mod names;
mod uuencode;
pub(crate) use names::same_attachment_name;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("uuencode: {0}")]
    Uuencode(String),
    #[error("decoded article exceeds {0} bytes")]
    TooLarge(usize),
    #[error("yenc: {0}")]
    Yenc(String),
    #[error("decoded part is empty")]
    Empty,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub filename: Option<String>,
    /// Zero-based offset in the assembled file.
    pub begin: u64,
    pub file_size: Option<u64>,
    pub data: Vec<u8>,
    pub crc32: u32,
}

/// Decode one NNTP article body. `part_begin` from `yenc-simd` is already zero-based.
pub fn decode_body(article: &[u8]) -> Result<Part, DecodeError> {
    decode_body_limited(article, usize::MAX)
}

/// Bound decoded bytes. UU decoding checks the bound before growing its buffer.
/// yEnc callers should also bound the encoded BODY before calling this helper.
pub fn decode_body_limited(article: &[u8], max_bytes: usize) -> Result<Part, DecodeError> {
    if !has_ybegin(article) {
        if let Some(decoded) = uuencode::decode(article, max_bytes)? {
            return Ok(decoded);
        }
    }
    let decoded = decode_yenc(article).map_err(|err| DecodeError::Yenc(err.to_string()))?;
    if decoded.data.len() > max_bytes {
        return Err(DecodeError::TooLarge(max_bytes));
    }
    if decoded.data.is_empty() {
        return Err(DecodeError::Empty);
    }
    Ok(Part {
        filename: decoded.filename,
        begin: decoded.part_begin.unwrap_or(0),
        file_size: decoded.file_size,
        data: decoded.data,
        crc32: decoded.crc32,
    })
}

/// Decode ordered uuencoded article fragments. Only line-boundary splits are
/// supported; the first fragment carries begin and the final fragment its trailer.
pub fn decode_uuencode_parts_limited(
    articles: &[&[u8]],
    max_bytes: usize,
) -> Result<Part, DecodeError> {
    uuencode::decode_parts(articles, max_bytes)
}

/// Strict recognition for staging offset-free classic UU continuation articles.
pub fn is_uuencode_fragment(article: &[u8]) -> bool {
    !has_ybegin(article) && uuencode::is_fragment(article)
}

/// Assemble classic UU fragments in part order with bounded memory. Writes only
/// after the complete framing and filename are validated; failures remove temp data.
pub fn write_uuencode_files_limited(
    articles: &[std::path::PathBuf],
    path: &Path,
    expected_name: &str,
    max_bytes: u64,
) -> Result<u64, DecodeError> {
    uuencode::write_files(articles, path, expected_name, max_bytes)
}

/// Write `part.data` at `part.begin`. Grows the file to `file_size` when that is known
/// and larger than the current length. Parts may arrive out of order.
pub fn write_part(path: &Path, part: &Part) -> Result<(), DecodeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    if let Some(size) = part.file_size {
        let len = file.metadata()?.len();
        if size > len {
            file.set_len(size)?;
        }
    }
    file.seek(SeekFrom::Start(part.begin))?;
    file.write_all(&part.data)?;
    Ok(())
}

fn has_ybegin(article: &[u8]) -> bool {
    article
        .split(|byte| *byte == b'\n')
        .any(|line| line.starts_with(b"=ybegin "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yenc_simd::encode_article;

    #[test]
    fn round_trip_single_part() {
        let raw = b"hello yenc payload";
        let (encoded, _crc) = encode_article(raw, "note.txt", 1, 1, 0, raw.len() as u64);
        let part = decode_body(&encoded).unwrap();
        assert_eq!(part.data, raw);
        assert_eq!(part.filename.as_deref(), Some("note.txt"));
        assert_eq!(part.begin, 0);
    }

    #[test]
    fn writes_parts_out_of_order() {
        let raw = b"abcdefghij";
        let (a, _) = encode_article(&raw[..4], "blob.bin", 1, 2, 0, raw.len() as u64);
        let (b, _) = encode_article(&raw[4..], "blob.bin", 2, 2, 4, raw.len() as u64);
        let dir = std::env::temp_dir().join(format!("nntpbrowser-yenc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blob.bin");
        let second = decode_body(&b).unwrap();
        let first = decode_body(&a).unwrap();
        write_part(&path, &second).unwrap();
        write_part(&path, &first).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), raw);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classic_uuencode_is_decoded() {
        let article = b"begin 644 file.bin\n#0V%T\n \nend\n";
        let part = decode_body(article).unwrap();
        assert_eq!(part.data, b"Cat");
        assert_eq!(part.filename.as_deref(), Some("file.bin"));
        assert_eq!(part.file_size, Some(3));
    }
}
