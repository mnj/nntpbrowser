//! Raw DEFLATE (RFC 8054), zlib, gzip, and xz for overview payloads.

#[cfg(test)]
use std::io::Write;
use std::io::{Cursor, Read};

#[cfg(test)]
use flate2::Compression;
use flate2::{Decompress, FlushDecompress, Status};

use crate::error::Error;

#[derive(Clone, Copy)]
pub(crate) enum Wrap {
    Raw,
    Zlib,
    Gzip,
}

pub(crate) fn inflate_all(data: &[u8], wrap: Wrap) -> Result<Vec<u8>, Error> {
    if matches!(wrap, Wrap::Gzip) {
        let mut dec = flate2::read::GzDecoder::new(data);
        let mut out = Vec::new();
        dec.read_to_end(&mut out)
            .map_err(|err| Error::Compression(err.to_string()))?;
        return Ok(out);
    }
    let mut dec = match wrap {
        Wrap::Raw => Decompress::new(false),
        Wrap::Zlib => Decompress::new(true),
        Wrap::Gzip => unreachable!(),
    };
    let mut out = Vec::new();
    let mut input = data;
    let mut buf = [0u8; 16 * 1024];
    loop {
        let before_in = dec.total_in();
        let before_out = dec.total_out();
        let status = dec
            .decompress(input, &mut buf, FlushDecompress::Finish)
            .map_err(|err| Error::Compression(err.to_string()))?;
        let consumed = (dec.total_in() - before_in) as usize;
        let produced = (dec.total_out() - before_out) as usize;
        out.extend_from_slice(&buf[..produced]);
        input = &input[consumed.min(input.len())..];
        if status == Status::StreamEnd {
            break;
        }
        if consumed == 0 && produced == 0 {
            return Err(Error::Compression("inflate made no progress".into()));
        }
    }
    Ok(out)
}

pub(crate) fn inflate_xz(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut input = Cursor::new(data);
    let mut output = Vec::new();
    lzma_rs::xz_decompress(&mut input, &mut output)
        .map_err(|err| Error::Compression(err.to_string()))?;
    Ok(output)
}

pub(crate) fn looks_like_zlib(data: &[u8]) -> bool {
    if data.len() < 2 || data[0] != 0x78 {
        return false;
    }
    (u16::from(data[0]) * 256 + u16::from(data[1])) % 31 == 0
}

pub(crate) fn inflate_auto(data: &[u8]) -> Result<Vec<u8>, Error> {
    if data.starts_with(&[0x1f, 0x8b]) {
        inflate_all(data, Wrap::Gzip)
    } else if data.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        inflate_xz(data)
    } else if looks_like_zlib(data) {
        inflate_all(data, Wrap::Zlib)
    } else {
        inflate_all(data, Wrap::Raw)
    }
}

#[cfg(test)]
pub(crate) fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), Compression::new(6));
    enc.write_all(data)
        .map_err(|err| Error::Compression(err.to_string()))?;
    enc.finish()
        .map_err(|err| Error::Compression(err.to_string()))
}

/// Used by tests that build an xz fixture.
#[cfg(test)]
pub(crate) fn xz_compress(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut input = Cursor::new(data);
    let mut output = Vec::new();
    lzma_rs::xz_compress(&mut input, &mut output)
        .map_err(|err| Error::Compression(err.to_string()))?;
    Ok(output)
}
