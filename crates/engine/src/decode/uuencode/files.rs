//! Streaming reconstruction for explicit downloads, without buffering a whole file.

use super::*;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

pub(in crate::decode) fn write_files(
    parts: &[PathBuf],
    output: &Path,
    expected_name: &str,
    cap: u64,
) -> Result<u64, DecodeError> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pending = output.with_file_name(format!(".uu-decoding-{}-{nonce}", std::process::id()));
    let result = (|| {
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        let mut begun = false;
        let mut zero = false;
        let mut ended = false;
        let mut written = 0u64;
        let mut base64 = false;
        let mut quartet = Vec::with_capacity(4);
        let mut padded = false;
        let mut line = Vec::new();
        for path in parts {
            let mut input = BufReader::new(File::open(path)?);
            loop {
                line.clear();
                // An NNTP UU data line is tiny; cap malformed/comment lines too.
                let size = input
                    .by_ref()
                    .take(1_048_577)
                    .read_until(b'\n', &mut line)?;
                if size == 0 {
                    break;
                }
                if size > 1_048_576 {
                    return Err(invalid("line exceeds 1 MiB"));
                }
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.starts_with(b"begin ") || line.starts_with(b"begin-base64 ") {
                    if begun {
                        return Err(invalid("multiple begin headers in multipart file"));
                    }
                    // Share mode/name validation with the normal decoder, using
                    // one known data byte solely to validate the header.
                    let mut probe = line.to_vec();
                    base64 = line.starts_with(b"begin-base64 ");
                    if base64 {
                        probe.extend_from_slice(b"\nQQ==\n====\n");
                    } else {
                        probe.extend_from_slice(b"\n!00``\n`\nend\n");
                    }
                    let name = decode(&probe, 1)?
                        .ok_or_else(|| invalid("invalid header"))?
                        .filename;
                    if !name
                        .as_deref()
                        .is_some_and(|name| super::super::same_attachment_name(name, expected_name))
                    {
                        return Err(invalid(
                            "attachment filename does not match downloaded file",
                        ));
                    }
                    begun = true;
                    continue;
                }
                if !begun || ended {
                    continue;
                }
                if base64 {
                    if line == b"====" {
                        if !quartet.is_empty() {
                            return Err(invalid("incomplete base64 quartet"));
                        }
                        ended = true;
                        continue;
                    }
                    for &byte in line {
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
                            quartet.clear();
                            written = written
                                .checked_add(count as u64)
                                .filter(|size| *size <= cap)
                                .ok_or(DecodeError::TooLarge(
                                    usize::try_from(cap).unwrap_or(usize::MAX),
                                ))?;
                            target.write_all(&decoded[..count])?;
                        }
                    }
                    continue;
                }
                if zero {
                    if line != b"end" {
                        return Err(invalid("missing end trailer"));
                    }
                    ended = true;
                    continue;
                }
                let (&count, encoded) = line
                    .split_first()
                    .ok_or_else(|| invalid("empty encoded line"))?;
                let count = value(count)? as usize;
                let mut decoded = [0u8; 63];
                decode_classic_line(encoded, count, &mut decoded)?;
                if count == 0 {
                    zero = true;
                    continue;
                }
                written = written
                    .checked_add(count as u64)
                    .filter(|size| *size <= cap)
                    .ok_or(DecodeError::TooLarge(
                        usize::try_from(cap).unwrap_or(usize::MAX),
                    ))?;
                target.write_all(&decoded[..count])?;
            }
        }
        if !begun || !ended {
            return Err(invalid("incomplete multipart begin/zero/end framing"));
        }
        if written == 0 {
            return Err(DecodeError::Empty);
        }
        target.flush()?;
        drop(target);
        std::fs::rename(&pending, output)?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}
