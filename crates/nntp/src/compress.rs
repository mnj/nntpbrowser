#[cfg(test)]
use flate2::Compression;
use flate2::{Compress, Decompress, FlushCompress, FlushDecompress, Status};

use crate::error::Error;

/// Raw DEFLATE (no zlib header), one context per direction for the life of the socket.
/// Client commands are sync-flushed so the peer can read a full command without Finish.
#[cfg(test)]
pub(crate) struct RawDeflate {
    comp: Compress,
    decomp: Decompress,
    pending_in: Vec<u8>,
}

#[cfg(test)]
impl RawDeflate {
    pub(crate) fn new() -> Self {
        Self {
            comp: Compress::new(Compression::new(3), false),
            decomp: Decompress::new(false),
            pending_in: Vec::new(),
        }
    }

    pub(crate) fn compress_sync(&mut self, input: &[u8]) -> Result<Vec<u8>, Error> {
        compress_sync(&mut self.comp, input)
    }

    pub(crate) fn inflate(&mut self, input: &[u8]) -> Result<Vec<u8>, Error> {
        self.pending_in.extend_from_slice(input);
        inflate_pending(&mut self.decomp, &mut self.pending_in)
    }
}

/// Continuous gzip member on the read side (XFEATURE COMPRESS GZIP).
/// `flate2` rust backend has no `Decompress::new_gzip`; `GzDecoder` parses the
/// header and then raw-inflates, which matches a sync-flushed gzip stream.
pub(crate) struct GzipReader {
    dec: flate2::write::GzDecoder<Vec<u8>>,
}

impl GzipReader {
    pub(crate) fn new() -> Self {
        Self {
            dec: flate2::write::GzDecoder::new(Vec::new()),
        }
    }

    pub(crate) fn inflate(&mut self, mut input: &[u8]) -> Result<Vec<u8>, Error> {
        use std::io::Write;
        while !input.is_empty() {
            let n = self
                .dec
                .write(input)
                .map_err(|e| Error::Compression(format!("gzip: {e}")))?;
            if n == 0 {
                return Err(Error::Compression("gzip decoder made no progress".into()));
            }
            input = &input[n..];
        }
        self.dec
            .flush()
            .map_err(|e| Error::Compression(format!("gzip flush: {e}")))?;
        Ok(std::mem::take(self.dec.get_mut()))
    }
}

pub(crate) fn compress_sync(comp: &mut Compress, mut input: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > 10_000 {
            return Err(Error::Compression(
                "deflate sync flush did not finish".into(),
            ));
        }
        let before_in = comp.total_in();
        let before_out = comp.total_out();
        let status = comp
            .compress(input, &mut buf, FlushCompress::Sync)
            .map_err(|e| Error::Compression(e.to_string()))?;
        let consumed = (comp.total_in() - before_in) as usize;
        let produced = (comp.total_out() - before_out) as usize;
        if consumed > input.len() {
            return Err(Error::Compression("deflate consumed past input".into()));
        }
        out.extend_from_slice(&buf[..produced]);
        input = &input[consumed..];
        match status {
            Status::StreamEnd => break,
            Status::Ok | Status::BufError => {
                let input_done = input.is_empty();
                let buffer_full = produced == buf.len();
                if input_done && !buffer_full {
                    break;
                }
                if input_done && produced == 0 && consumed == 0 {
                    break;
                }
            }
        }
    }
    Ok(out)
}

pub(crate) fn inflate_pending(
    decomp: &mut Decompress,
    pending: &mut Vec<u8>,
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16384];
    let mut guard = 0u32;
    loop {
        if pending.is_empty() {
            break;
        }
        guard += 1;
        if guard > 10_000 {
            return Err(Error::Compression("deflate inflate did not finish".into()));
        }
        let before_in = decomp.total_in();
        let before_out = decomp.total_out();
        let status = decomp
            .decompress(pending, &mut buf, FlushDecompress::Sync)
            .map_err(|e| Error::Compression(e.to_string()))?;
        let consumed = (decomp.total_in() - before_in) as usize;
        let produced = (decomp.total_out() - before_out) as usize;
        if consumed > pending.len() {
            return Err(Error::Compression("inflate consumed past input".into()));
        }
        pending.drain(..consumed);
        out.extend_from_slice(&buf[..produced]);
        if status == Status::StreamEnd {
            break;
        }
        if consumed == 0 && produced == 0 {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn raw_deflate_sync_flush_roundtrip_in_chunks() {
        let mut codec = RawDeflate::new();
        let mut peer_dec = Decompress::new(false);
        let mut peer_pending = Vec::new();
        let mut got = Vec::new();
        for cmd in [b"GROUP alt.test\r\n".as_slice(), b"OVER 1-2\r\n".as_slice()] {
            let wire = codec.compress_sync(cmd).unwrap();
            assert!(!wire.is_empty());
            assert!(!wire.windows(cmd.len()).any(|w| w == cmd));
            for chunk in wire.chunks(1) {
                peer_pending.extend_from_slice(chunk);
                got.extend(inflate_pending(&mut peer_dec, &mut peer_pending).unwrap());
            }
        }
        assert_eq!(got, b"GROUP alt.test\r\nOVER 1-2\r\n");

        let back = codec
            .inflate(&{
                let mut server = Compress::new(Compression::new(3), false);
                compress_sync(&mut server, b"211 1 1 1 alt.test\r\n").unwrap()
            })
            .unwrap();
        assert_eq!(back, b"211 1 1 1 alt.test\r\n");
    }

    #[test]
    fn gzip_reader_sees_sync_flushed_member() {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), Compression::new(3));
        let mut reader = GzipReader::new();
        let mut got = Vec::new();
        for part in [
            b"224 overview\r\n".as_slice(),
            b"1\tsub\r\n.\r\n".as_slice(),
        ] {
            enc.write_all(part).unwrap();
            enc.flush().unwrap();
            let wire = std::mem::take(enc.get_mut());
            for chunk in wire.chunks(3) {
                got.extend(reader.inflate(chunk).unwrap());
            }
        }
        assert_eq!(got, b"224 overview\r\n1\tsub\r\n.\r\n");
    }
}
