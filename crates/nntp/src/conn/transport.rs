//! TCP/TLS transport, line framing, and connection compression.

use super::*;

const LINE_CAP: usize = 2 * 1024 * 1024;

pub(super) enum Inflate {
    None,
    Raw { dec: Decompress, pending: Vec<u8> },
    Gzip(GzipReader),
    ProbeGzip { pending: Vec<u8> },
}

pub(super) enum NetStream {
    Tcp(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    Disconnected,
}

impl AsyncRead for NetStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            NetStream::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            NetStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
            NetStream::Disconnected => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "connection is closed",
            ))),
        }
    }
}

impl AsyncWrite for NetStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            NetStream::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            NetStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
            NetStream::Disconnected => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "connection is closed",
            ))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            NetStream::Tcp(s) => Pin::new(s).poll_flush(cx),
            NetStream::Tls(s) => Pin::new(s).poll_flush(cx),
            NetStream::Disconnected => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "connection is closed",
            ))),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            NetStream::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            NetStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
            NetStream::Disconnected => Poll::Ready(Ok(())),
        }
    }
}

pub(super) struct Reader {
    pub(super) io: tokio::io::ReadHalf<NetStream>,
    pub(super) plain: Vec<u8>,
    pub(super) inflate: Inflate,
    pub(super) timeout: std::time::Duration,
    pub(super) traffic: Option<Arc<crate::Traffic>>,
    pub(super) connection: Option<Arc<crate::connection_stats::State>>,
}

pub(super) struct Writer {
    pub(super) io: tokio::io::WriteHalf<NetStream>,
    pub(super) deflate: Option<Compress>,
    pub(super) timeout: std::time::Duration,
    pub(super) traffic: Option<Arc<crate::Traffic>>,
    pub(super) connection: Option<Arc<crate::connection_stats::State>>,
}

impl Reader {
    pub(super) fn enable_raw(&mut self) -> Result<(), Error> {
        let leftover = std::mem::take(&mut self.plain);
        let mut dec = Decompress::new(false);
        let mut pending = Vec::new();
        let plain = if leftover.is_empty() {
            Vec::new()
        } else {
            pending.extend_from_slice(&leftover);
            inflate_pending(&mut dec, &mut pending)?
        };
        self.inflate = Inflate::Raw { dec, pending };
        self.plain = plain;
        Ok(())
    }

    pub(super) fn enable_gzip(&mut self) -> Result<(), Error> {
        let leftover = std::mem::take(&mut self.plain);
        self.inflate = Inflate::ProbeGzip { pending: leftover };
        self.probe_gzip()
    }

    fn probe_gzip(&mut self) -> Result<(), Error> {
        let Inflate::ProbeGzip { pending } = &mut self.inflate else {
            return Ok(());
        };
        if pending.len() < 2 {
            return Ok(());
        }
        let bytes = std::mem::take(pending);
        if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut gzip = GzipReader::new();
            self.plain.extend_from_slice(&gzip.inflate(&bytes)?);
            self.inflate = Inflate::Gzip(gzip);
        } else {
            // Legacy XFEATURE implementations keep statuses plain and compress
            // individual response bodies as zlib members instead of gzip.
            self.plain.extend_from_slice(&bytes);
            self.inflate = Inflate::None;
        }
        Ok(())
    }

    pub(super) async fn fill(&mut self) -> Result<(), Error> {
        if self.plain.len() > LINE_CAP {
            return Err(Error::Protocol {
                code: 0,
                text: "response line too long".into(),
            });
        }
        let mut buf = [0u8; 8192];
        let _pending = self.connection.as_ref().map(|state| state.reading());
        let n = match tokio::time::timeout(self.timeout, self.io.read(&mut buf)).await {
            Ok(Ok(0)) => return Err(Error::Closed),
            Ok(Ok(n)) => n,
            Ok(Err(err)) => return Err(Error::Io(err)),
            Err(_) => return Err(Error::Timeout),
        };
        let chunk = &buf[..n];
        if let Some(traffic) = &self.traffic {
            traffic.receive(n);
        }
        match &mut self.inflate {
            Inflate::None => self.plain.extend_from_slice(chunk),
            Inflate::Raw { dec, pending } => {
                pending.extend_from_slice(chunk);
                let out = inflate_pending(dec, pending)?;
                self.plain.extend_from_slice(&out);
            }
            Inflate::Gzip(gzip) => {
                let out = gzip.inflate(chunk)?;
                self.plain.extend_from_slice(&out);
            }
            Inflate::ProbeGzip { pending } => pending.extend_from_slice(chunk),
        }
        self.probe_gzip()?;
        Ok(())
    }

    pub(super) async fn read_line(&mut self) -> Result<Vec<u8>, Error> {
        self.read_line_counted(true).await
    }

    async fn read_line_counted(&mut self, measure: bool) -> Result<Vec<u8>, Error> {
        loop {
            let before = self.plain.len();
            if let Some(line) = pop_line(&mut self.plain) {
                if measure {
                    self.measure_decoded(before - self.plain.len());
                }
                return Ok(line);
            }
            self.fill().await?;
        }
    }

    pub(super) fn measure_decoded(&self, bytes: usize) {
        if let Some(traffic) = &self.traffic {
            traffic.decode(bytes);
        }
    }

    pub(super) async fn read_status(&mut self) -> Result<(u16, String), Error> {
        let line = self.read_line().await?;
        parse_status_line(&line)
    }

    pub(super) async fn starts_zlib(&mut self) -> Result<bool, Error> {
        while self.plain.len() < 2 {
            self.fill().await?;
        }
        let cmf = self.plain[0];
        let flg = self.plain[1];
        Ok(cmf & 0x0f == 8
            && cmf >> 4 <= 7
            && (u16::from(cmf) * 256 + u16::from(flg)).is_multiple_of(31))
    }

    /// Some providers send XZVER as a raw zlib member, with the NNTP dot
    /// inside the compressed text. The member boundary, rather than CRLF,
    /// determines how much data to consume from the underlying connection.
    pub(super) async fn read_zlib_payload(&mut self) -> Result<Vec<u8>, Error> {
        let mut decoder = Decompress::new(true);
        let mut output = Vec::new();
        let mut buffer = [0u8; 16_384];
        loop {
            // Let the decoder drain any buffered output before awaiting more
            // network bytes: the complete member may already have arrived.
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let status = decoder
                .decompress(&self.plain, &mut buffer, flate2::FlushDecompress::None)
                .map_err(|err| Error::Compression(format!("compressed response zlib: {err}")))?;
            let consumed = (decoder.total_in() - before_in) as usize;
            let produced = (decoder.total_out() - before_out) as usize;
            self.plain.drain(..consumed);
            if output.len().saturating_add(produced) > 64 * 1024 * 1024 {
                return Err(Error::Compression(
                    "compressed response exceeds 64 MiB".into(),
                ));
            }
            output.extend_from_slice(&buffer[..produced]);
            self.measure_decoded(produced);
            if status == flate2::Status::StreamEnd {
                return Ok(output);
            }
            if consumed == 0 && produced == 0 {
                self.fill().await?;
            }
        }
    }

    pub(super) async fn read_response(&mut self) -> Result<(u16, String, Vec<Vec<u8>>), Error> {
        self.read_response_with_phase("response").await
    }

    pub(super) async fn read_response_with_phase(
        &mut self,
        phase: &'static str,
    ) -> Result<(u16, String, Vec<Vec<u8>>), Error> {
        let (code, text) = self
            .read_status()
            .await
            .map_err(|err| err.timeout_context(phase))?;
        trace_response(phase, code);
        if !is_multiline(code) {
            return Ok((code, text, Vec::new()));
        }
        Ok((code, text, self.read_data_lines(phase).await?))
    }

    pub(super) async fn read_data_lines(
        &mut self,
        phase: &'static str,
    ) -> Result<Vec<Vec<u8>>, Error> {
        self.read_data_lines_counted(phase, true).await
    }

    /// XZVER's compressed yEnc wrapper is not decoded overview data. The caller
    /// measures the resulting inflated payload instead, including on failure.
    pub(super) async fn read_encoded_data_lines(
        &mut self,
        phase: &'static str,
    ) -> Result<Vec<Vec<u8>>, Error> {
        self.read_data_lines_counted(phase, false).await
    }

    async fn read_data_lines_counted(
        &mut self,
        phase: &'static str,
        measure: bool,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let mut lines = Vec::new();
        loop {
            let line = self
                .read_line_counted(measure)
                .await
                .map_err(|err| err.timeout_context(phase))?;
            match unstuff(&line) {
                None => break,
                Some(data) => lines.push(data.to_vec()),
            }
        }
        Ok(lines)
    }

    pub(super) async fn read_bodies(
        &mut self,
        n: usize,
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let (code, text, lines) = self.read_response().await?;
            if code == 222 {
                out.push(Ok(join_lines(&lines)));
            } else {
                out.push(Err(Error::from_status(code, text)));
            }
        }
        Ok(out)
    }
}

impl Writer {
    pub(super) fn enable_deflate(&mut self) {
        self.deflate = Some(Compress::new(Compression::new(3), false));
    }

    pub(super) async fn write_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        crate::codec::validate_command(data)?;
        self.write_wire_bytes(data).await
    }

    /// RFC 4643 gives SASL exchanges their own mechanism-dependent line limits.
    pub(super) async fn write_sasl_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.len() > 32 * 1024 {
            return Err(Error::Auth);
        }
        self.write_wire_bytes(data).await
    }

    async fn write_wire_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        let owned;
        let bytes = if let Some(comp) = self.deflate.as_mut() {
            owned = compress_sync(comp, data)?;
            owned.as_slice()
        } else {
            data
        };
        write_and_flush_counted(
            &mut self.io,
            bytes,
            self.traffic.as_deref(),
            self.connection.as_ref(),
            self.timeout,
        )
        .await
    }
}

async fn write_and_flush_counted<W: AsyncWrite + Unpin>(
    io: &mut W,
    bytes: &[u8],
    traffic: Option<&crate::Traffic>,
    connection: Option<&Arc<crate::connection_stats::State>>,
    timeout: std::time::Duration,
) -> Result<(), Error> {
    let _pending = connection.map(|state| state.writing());
    match tokio::time::timeout(timeout, write_counted(io, bytes, traffic)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => return Err(Error::Io(err)),
        Err(_) => return Err(Error::Timeout),
    }
    match tokio::time::timeout(timeout, io.flush()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(Error::Io(err)),
        Err(_) => Err(Error::Timeout),
    }
}

// Count each successful write immediately, so later I/O failure, timeout or
// task cancellation cannot discard traffic already accepted by the transport.
async fn write_counted<W: AsyncWrite + Unpin>(
    io: &mut W,
    mut bytes: &[u8],
    traffic: Option<&crate::Traffic>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = io.write(bytes).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write command",
            ));
        }
        if let Some(traffic) = traffic {
            traffic.send(n);
        }
        bytes = &bytes[n..];
    }
    Ok(())
}

fn is_multiline(code: u16) -> bool {
    matches!(
        code,
        100 | 101 | 215 | 220 | 221 | 222 | 224 | 225 | 230 | 231
    )
}

pub(super) async fn write_all_cmds(writer: &mut Writer, cmds: &[Vec<u8>]) -> Result<(), Error> {
    // Reject the batch before writing anything; partial submission would leave
    // responses queued on a connection whose caller received a local error.
    for cmd in cmds {
        crate::codec::validate_command(cmd)?;
    }
    for cmd in cmds {
        writer.write_bytes(cmd).await?;
    }
    Ok(())
}

pub(super) fn tls_connector() -> Result<TlsConnector, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| Error::Tls(err.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PartialWriter {
        accepted: bool,
        fail: bool,
    }

    impl AsyncWrite for PartialWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            if !this.accepted {
                this.accepted = true;
                Poll::Ready(Ok(bytes.len().min(3)))
            } else if this.fail {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "test failure",
                )))
            } else {
                Poll::Pending
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn failed_or_cancelled_command_writes_keep_the_accepted_prefix() {
        for fail in [true, false] {
            let traffic = crate::Traffic::default();
            let mut writer = PartialWriter {
                accepted: false,
                fail,
            };
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(10),
                write_counted(&mut writer, b"DATE\r\n", Some(&traffic)),
            )
            .await;
            if fail {
                assert_eq!(
                    result.unwrap().unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
            } else {
                assert!(
                    result.is_err(),
                    "pending write must be cancelled by timeout"
                );
            }
            assert_eq!(traffic.snapshot().sent, 3);
            assert_eq!(traffic.snapshot().received, 0);
        }
    }

    #[tokio::test]
    async fn blocked_write_snapshot_keeps_prefix_and_clears_pending_flag_after_cancellation() {
        let aggregate = Arc::new(crate::Traffic::default());
        let registry = Arc::new(crate::connection_stats::Registry::default());
        let registration = registry.register(aggregate.clone());
        let state = registration.state.clone();
        let task = tokio::spawn(async move {
            let mut writer = PartialWriter {
                accepted: false,
                fail: false,
            };
            write_and_flush_counted(
                &mut writer,
                b"DATE\r\n",
                Some(&state.traffic),
                Some(&state),
                std::time::Duration::from_secs(10),
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !registry.snapshot()[0].write_pending || aggregate.snapshot().sent != 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let blocked = registry.snapshot()[0];
        assert!(!blocked.read_pending);
        assert_eq!(blocked.traffic.sent, 3);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!registry.snapshot()[0].write_pending);
        assert_eq!(aggregate.snapshot().sent, 3);
        drop(registration);
        assert!(registry.snapshot().is_empty());
    }

    #[tokio::test]
    async fn oversized_commands_never_reach_the_wire_or_compressor() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut peer, _) = listener.accept().await.unwrap();
        let (_reader, io) = tokio::io::split(NetStream::Tcp(client));
        let mut writer = Writer {
            io,
            deflate: None,
            timeout: std::time::Duration::from_secs(1),
            traffic: None,
            connection: None,
        };
        let mut oversized = b"BODY <".to_vec();
        oversized.extend(std::iter::repeat_n(b'a', 510));
        oversized.extend_from_slice(b">\r\n");
        let long_arguments = format!("BODY {}\r\n", "a".repeat(498));
        assert!(long_arguments.len() < 512);
        assert!(writer.write_bytes(long_arguments.as_bytes()).await.is_err());
        assert!(writer.write_bytes(&oversized).await.is_err());
        assert!(
            write_all_cmds(&mut writer, &[b"DATE\r\n".to_vec(), oversized.clone()])
                .await
                .is_err()
        );
        writer.enable_deflate();
        assert!(writer.write_bytes(long_arguments.as_bytes()).await.is_err());
        assert!(writer.write_bytes(&oversized).await.is_err());
        assert_eq!(writer.deflate.as_ref().unwrap().total_in(), 0);

        writer.deflate = None;
        writer.write_bytes(b"DATE\r\n").await.unwrap();
        let mut received = [0u8; 6];
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            peer.read_exact(&mut received),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&received, b"DATE\r\n");
        let mut extra = [0u8; 1];
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), peer.read(&mut extra))
                .await
                .is_err()
        );
    }
}
