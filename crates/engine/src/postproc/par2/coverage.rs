//! Conservative exponent coverage for avoiding redundant volume requests.
//! This reads packet structure, not recovery payload hashes. Successful repair
//! and final payload verification remain the authority for download completion.
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

const HEADER: usize = 64;
const MAX_EXPONENTS: u64 = 65_536;
const MAX_PACKETS: usize = 1_000_000;

/// True only when structurally complete, matching-set RecoverySlice packets
/// cover every hinted exponent. Any error/unknown/excessive range returns false
/// so the caller keeps the candidate eligible rather than trusting its name.
pub(crate) fn volume_covers_range(
    path: &Path,
    pinned_set: &str,
    slice_size: u64,
    range: (u64, u64),
) -> bool {
    volume_matches_range(path, pinned_set, slice_size, range, true)
}

/// True only when a structurally complete file has at least one matching-set
/// RecoverySlice exponent in the requested range. Filename hints are ignored.
pub(crate) fn volume_overlaps_range(
    path: &Path,
    pinned_set: &str,
    slice_size: u64,
    range: (u64, u64),
) -> bool {
    volume_matches_range(path, pinned_set, slice_size, range, false)
}

fn volume_matches_range(
    path: &Path,
    pinned_set: &str,
    slice_size: u64,
    range: (u64, u64),
    require_all: bool,
) -> bool {
    let Some(set_id) = decode_set_id(pinned_set) else {
        return false;
    };
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    metadata.is_file()
        && covers(
            &mut file,
            metadata.len(),
            &set_id,
            slice_size,
            range,
            require_all,
        )
        .unwrap_or(false)
}

fn decode_set_id(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 {
        return None;
    }
    let digit = |byte| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let mut id = [0; 16];
    for (index, bytes) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        id[index] = digit(bytes[0])? * 16 + digit(bytes[1])?;
    }
    Some(id)
}

fn covers<R: Read + Seek>(
    reader: &mut R,
    length: u64,
    set_id: &[u8; 16],
    slice_size: u64,
    (first, last): (u64, u64),
    require_all: bool,
) -> io::Result<bool> {
    let Some(count) = last
        .checked_sub(first)
        .filter(|&count| count > 0 && count <= MAX_EXPONENTS)
    else {
        return Ok(false);
    };
    let Some(slice_packet) = slice_size.checked_add(68).filter(|_| slice_size > 0) else {
        return Ok(false);
    };
    if last > u64::from(u32::MAX) + 1 {
        return Ok(false);
    }
    let mut present = vec![false; count as usize];
    let mut found = 0;
    let mut position = 0;
    let mut header = [0; HEADER];
    for _ in 0..MAX_PACKETS {
        if position == length {
            return Ok(if require_all {
                found == count
            } else {
                found > 0
            });
        }
        if length.saturating_sub(position) < HEADER as u64 {
            return Ok(false);
        }
        reader.seek(SeekFrom::Start(position))?;
        reader.read_exact(&mut header)?;
        if &header[..8] != b"PAR2\0PKT" {
            return Ok(false);
        }
        let packet_len = u64::from_le_bytes(header[8..16].try_into().unwrap());
        if packet_len < HEADER as u64 || packet_len % 4 != 0 || packet_len > length - position {
            return Ok(false);
        }
        if &header[32..48] == set_id && &header[48..64] == b"PAR 2.0\0RecvSlic" {
            if packet_len != slice_packet {
                return Ok(false);
            }
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes)?;
            let exponent = u64::from(u32::from_le_bytes(bytes));
            if exponent >= first && exponent < last {
                let seen = &mut present[(exponent - first) as usize];
                if !*seen {
                    *seen = true;
                    found += 1;
                }
            }
        }
        // Skip slice bytes without ever buffering them. Validate the entire
        // file, including trailing packets, before treating coverage as known.
        position += packet_len;
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
