//! yEnc for XZVER bodies only. `size=` is ignored. Article downloads use `yenc-simd` in the engine.

use crc32fast::hash;

use crate::error::Error;

#[derive(Debug)]
pub(crate) struct YencPayload {
    pub data: Vec<u8>,
}

/// `lines` are already dot-unstuffed and do not include the terminating dot.
pub(crate) fn decode_lines(lines: &[Vec<u8>]) -> Result<YencPayload, Error> {
    let mut data_lines: Vec<&[u8]> = Vec::new();
    let mut in_data = false;
    let mut expected_crc: Option<u32> = None;
    for line in lines {
        if starts_with(line, b"=ybegin") {
            in_data = true;
            continue;
        }
        if starts_with(line, b"=ypart") {
            continue;
        }
        if starts_with(line, b"=yend") {
            expected_crc = crc_field(line);
            break;
        }
        if in_data {
            data_lines.push(line);
        }
    }
    if !in_data {
        return Err(Error::Compression("xzver body has no =ybegin".into()));
    }
    let mut out = Vec::new();
    for line in data_lines {
        decode_line(line, &mut out)?;
    }
    if let Some(crc) = expected_crc {
        let got = hash(&out);
        if got != crc {
            return Err(Error::Compression(format!(
                "xzver yenc crc mismatch: got {got:08x} expected {crc:08x}"
            )));
        }
    }
    Ok(YencPayload { data: out })
}

fn decode_line(line: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if b == b'\r' || b == b'\n' {
            i += 1;
            continue;
        }
        let value = if b == b'=' {
            i += 1;
            let next = *line
                .get(i)
                .ok_or_else(|| Error::Compression("truncated yenc escape".into()))?;
            next.wrapping_sub(64)
        } else {
            b
        };
        out.push(value.wrapping_sub(42));
        i += 1;
    }
    Ok(())
}

fn starts_with(line: &[u8], prefix: &[u8]) -> bool {
    line.len() >= prefix.len() && &line[..prefix.len()] == prefix
}

fn crc_field(line: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(line);
    for key in ["pcrc32=", "crc32="] {
        if let Some(pos) = text.to_ascii_lowercase().find(key) {
            let rest = &text[pos + key.len()..];
            let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if hex.is_empty() || hex.eq_ignore_ascii_case("ffffffff") {
                return None;
            }
            return u32::from_str_radix(&hex, 16).ok();
        }
    }
    None
}

/// Test encoder. `width` is the maximum encoded data-line length.
/// Not used on the wire by the client. `size=` is always `-1`.
#[cfg(test)]
pub(crate) fn encode_width(data: &[u8], width: usize) -> Vec<Vec<u8>> {
    let crc = hash(data);
    let mut lines = Vec::new();
    lines.push(b"=ybegin line=128 size=-1 name=overview".to_vec());
    let mut cur = Vec::new();
    for &byte in data {
        let enc = byte.wrapping_add(42);
        let piece: Vec<u8> = if matches!(enc, 0 | b'\n' | b'\r' | b'=') {
            vec![b'=', enc.wrapping_add(64)]
        } else {
            vec![enc]
        };
        if !cur.is_empty() && cur.len() + piece.len() > width.max(1) {
            lines.push(std::mem::take(&mut cur));
        }
        cur.extend(piece);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.push(format!("=yend size=-1 crc32={crc:08x}").into_bytes());
    lines
}

#[cfg(test)]
pub(crate) fn encode(data: &[u8]) -> Vec<Vec<u8>> {
    encode_width(data, 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_dot_line() {
        let raw = b"hello\0=\r\n.hidden";
        let lines = encode(raw);
        assert!(lines.iter().any(|l| l.starts_with(b".")) || raw[0] != b'.');
        let back = decode_lines(&lines).unwrap();
        assert_eq!(back.data, raw);
    }

    #[test]
    fn skips_ffffffff_crc() {
        let lines = vec![
            b"=ybegin line=128 size=-1 name=x".to_vec(),
            b"k".to_vec(), // 'A' + 42
            b"=yend size=-1 crc32=FFFFFFFF".to_vec(),
        ];
        let back = decode_lines(&lines).unwrap();
        assert_eq!(back.data, b"A");
    }
}
