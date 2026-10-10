use super::*;
use crate::{Pool, TrafficSnapshot};

const GREETING: &[u8] = b"201 ready\r\n";
const CAPS: &[u8] = b"101 caps\r\nREADER\r\nOVER\r\nCOMPRESS DEFLATE\r\n.\r\n";
const FMT: &[u8] = b"500 positional fields\r\n";

#[derive(Default)]
struct Transcript(Mutex<TrafficSnapshot>);

impl Transcript {
    async fn send(&self, sock: &mut TcpStream, wire: &[u8], decoded: usize) {
        {
            let mut totals = self.0.lock().unwrap();
            totals.received += wire.len() as u64;
            totals.decoded_received += decoded as u64;
        }
        write_all(sock, wire).await;
    }

    async fn plain(&self, sock: &mut TcpStream, wire: &[u8]) {
        self.send(sock, wire, wire.len()).await;
    }

    async fn command(&self, sock: &mut TcpStream, command: &str) {
        assert_eq!(read_line(sock).await, command);
        self.0.lock().unwrap().sent += (command.len() + 2) as u64;
    }

    async fn setup(&self, sock: &mut TcpStream) {
        self.plain(sock, GREETING).await;
        self.command(sock, "CAPABILITIES").await;
        self.plain(sock, CAPS).await;
    }

    async fn format(&self, sock: &mut TcpStream) {
        self.command(sock, "LIST OVERVIEW.FMT").await;
        self.plain(sock, FMT).await;
    }

    fn snapshot(&self) -> TrafficSnapshot {
        *self.0.lock().unwrap()
    }
}

fn overview() -> Vec<u8> {
    format!(
        "1\t{}\tPoster\tDate\t<one@test>\t\t12\t1\r\n.\r\n",
        "sample ".repeat(1000)
    )
    .into_bytes()
}

#[tokio::test]
async fn plain_pipeline_counts_setup_stuffing_and_keeps_totals_after_pool_drop() {
    let transcript = Arc::new(Transcript::default());
    let expected = transcript.clone();
    let addr = serve(move |mut sock| {
        let transcript = transcript.clone();
        async move {
            transcript.setup(&mut sock).await;
            transcript.command(&mut sock, "BODY <one@test>").await;
            transcript.command(&mut sock, "BODY <two@test>").await;
            transcript
                .plain(&mut sock, b"222 one\r\n..stuffed\r\n.\r\n430 missing\r\n")
                .await;
        }
    })
    .await;
    let pool = Pool::new(endpoint(1, addr, 0, false));
    let totals = pool.traffic_counter();
    let result = pool
        .body_many_limited(
            &[
                ArticleRef::MessageId("one@test".into()),
                ArticleRef::MessageId("two@test".into()),
            ],
            &[1024, 1024],
        )
        .await
        .unwrap();
    assert_eq!(result[0].as_ref().unwrap(), b".stuffed\r\n");
    assert!(matches!(result[1], Err(Error::Missing)));
    assert_eq!(pool.traffic(), expected.snapshot());
    assert_eq!(pool.received_bytes(), expected.snapshot().received);
    drop(pool);
    assert_eq!(totals.snapshot(), expected.snapshot());
}

#[tokio::test]
async fn raw_deflate_counts_compressed_command_writes_and_consumed_decoded_responses() {
    let transcript = Arc::new(Transcript::default());
    let expected = transcript.clone();
    let addr = serve(move |mut sock| {
        let transcript = transcript.clone();
        async move {
            transcript.setup(&mut sock).await;
            transcript.format(&mut sock).await;
            transcript.command(&mut sock, "COMPRESS DEFLATE").await;
            transcript
                .plain(&mut sock, b"206 compression active\r\n")
                .await;
            let mut decoder = Decompress::new(false);
            let mut encoder = Compress::new(Compression::new(3), false);
            let mut pending = Vec::new();
            let mut plain = Vec::new();
            for (command, response) in [
                (
                    "OVER 1-1",
                    [b"224 overview\r\n".as_slice(), &overview()].concat(),
                ),
                (
                    "LIST ACTIVE *",
                    b"215 active\r\nalt.test 1 1 y\r\n.\r\n".to_vec(),
                ),
            ] {
                loop {
                    if let Some(line) = pop_line(&mut plain) {
                        assert_eq!(line, command.as_bytes());
                        break;
                    }
                    let mut raw = [0; 1024];
                    let n = sock.read(&mut raw).await.unwrap();
                    assert!(n > 0);
                    transcript.0.lock().unwrap().sent += n as u64;
                    pending.extend_from_slice(&raw[..n]);
                    plain.extend(inflate_pending(&mut decoder, &mut pending).unwrap());
                }
                let wire = compress_sync(&mut encoder, &response).unwrap();
                transcript.send(&mut sock, &wire, response.len()).await;
            }
        }
    })
    .await;
    let pool = Pool::new(endpoint(1, addr, 0, false));
    pool.set_header_strategy(HeaderStrategy::CompressDeflate)
        .await;
    let mut rows = 0;
    pool.fetch_overview(1, 1, |_| rows += 1).await.unwrap();
    assert_eq!(rows, 1);
    assert_eq!(pool.list_active("*").await.unwrap().len(), 1);
    assert_eq!(pool.traffic(), expected.snapshot());
    assert!(pool.traffic().decoded_received > pool.traffic().received);
}

#[tokio::test]
async fn gzip_and_binary_zlib_count_inflated_payloads_once() {
    for gzip in [true, false] {
        let transcript = Arc::new(Transcript::default());
        let expected = transcript.clone();
        let addr = serve(move |mut sock| {
            let transcript = transcript.clone();
            async move {
                transcript.setup(&mut sock).await;
                transcript.format(&mut sock).await;
                transcript
                    .command(&mut sock, "XFEATURE COMPRESS GZIP")
                    .await;
                transcript
                    .plain(&mut sock, b"290 compression active\r\n")
                    .await;
                transcript.command(&mut sock, "OVER 1-1").await;
                let rows = overview();
                if gzip {
                    use std::io::Write;
                    let response = [b"224 overview\r\n".as_slice(), &rows].concat();
                    let mut encoder =
                        flate2::write::GzEncoder::new(Vec::new(), Compression::fast());
                    encoder.write_all(&response).unwrap();
                    let wire = encoder.finish().unwrap();
                    transcript.send(&mut sock, &wire, response.len()).await;
                } else {
                    transcript.plain(&mut sock, b"224 overview\r\n").await;
                    transcript
                        .send(&mut sock, &zlib_compress(&rows).unwrap(), rows.len())
                        .await;
                }
            }
        })
        .await;
        let pool = Pool::new(endpoint(1, addr, 0, false));
        pool.set_header_strategy(HeaderStrategy::XfeatureGzip).await;
        pool.fetch_overview(1, 1, |_| {}).await.unwrap();
        assert_eq!(pool.traffic(), expected.snapshot(), "gzip={gzip}");
        assert!(pool.traffic().decoded_received > pool.traffic().received);
    }
}

#[tokio::test]
async fn xzver_binary_and_yenc_wrappers_are_replaced_by_inflated_bytes() {
    for encoding in [0, 1, 2] {
        let transcript = Arc::new(Transcript::default());
        let expected = transcript.clone();
        let addr = serve(move |mut sock| {
            let transcript = transcript.clone();
            async move {
                transcript.setup(&mut sock).await;
                transcript.format(&mut sock).await;
                transcript.command(&mut sock, "XZVER 1-1").await;
                transcript
                    .plain(&mut sock, b"224 compressed overview\r\n")
                    .await;
                let rows = overview();
                let compressed = if encoding == 2 {
                    xz_compress(&rows).unwrap()
                } else {
                    zlib_compress(&rows).unwrap()
                };
                let mut wire = Vec::new();
                if encoding == 0 {
                    wire = compressed;
                } else {
                    for line in yenc::encode(&compressed) {
                        wire.extend(stuff(&line));
                    }
                    wire.extend_from_slice(b".\r\n");
                }
                transcript.send(&mut sock, &wire, rows.len()).await;
            }
        })
        .await;
        let pool = Pool::new(endpoint(1, addr, 0, false));
        pool.set_header_strategy(HeaderStrategy::Xzver).await;
        pool.fetch_overview(1, 1, |_| {}).await.unwrap();
        assert_eq!(pool.traffic(), expected.snapshot(), "encoding={encoding}");
    }
}

#[tokio::test]
async fn failed_authentication_counts_all_retries_without_storing_credentials() {
    let transcript = Arc::new(Transcript::default());
    let expected = transcript.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let addr = serve(move |mut sock| {
        let transcript = transcript.clone();
        let attempts = attempts.clone();
        async move {
            attempts.fetch_add(1, Ordering::SeqCst);
            transcript.setup(&mut sock).await;
            transcript.command(&mut sock, "AUTHINFO USER reader").await;
            transcript
                .plain(&mut sock, b"381 password required\r\n")
                .await;
            transcript
                .command(&mut sock, "AUTHINFO PASS test-password")
                .await;
            transcript
                .plain(&mut sock, b"481 authentication failed\r\n")
                .await;
        }
    })
    .await;
    let mut server = endpoint(1, addr, 0, false);
    server.config.username = Some("reader".into());
    server.config.password = Some("test-password".into());
    let pool = Pool::new(server);
    assert!(matches!(pool.list_active("*").await, Err(Error::Auth)));
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    assert_eq!(pool.traffic(), expected.snapshot());
    assert!(pool.traffic().sent > 0);
    assert_eq!(pool.open_connections(), 0);
    assert!(
        pool.connection_stats().is_empty(),
        "failed authentication attempts do not leave live entries"
    );
}

#[tokio::test]
async fn cancelled_partial_reply_keeps_wire_bytes_but_does_not_count_unconsumed_read_ahead() {
    let transcript = Arc::new(Transcript::default());
    let expected = transcript.clone();
    let addr = serve(move |mut sock| {
        let transcript = transcript.clone();
        async move {
            transcript.setup(&mut sock).await;
            transcript.command(&mut sock, "BODY <blocked@test>").await;
            transcript
                .send(
                    &mut sock,
                    b"222 body\r\ncomplete\r\nunfinished",
                    b"222 body\r\ncomplete\r\n".len(),
                )
                .await;
            let mut byte = [0; 1];
            assert_eq!(
                sock.read(&mut byte).await.unwrap(),
                0,
                "cancelled socket must close"
            );
        }
    })
    .await;
    let pool = Arc::new(Pool::new(endpoint(1, addr, 0, false)));
    let request = pool.clone();
    let task = tokio::spawn(async move {
        request
            .body(&ArticleRef::MessageId("blocked@test".into()))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let target = expected.snapshot();
            if target.received > (GREETING.len() + CAPS.len()) as u64 && pool.traffic() == target {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(pool.traffic(), expected.snapshot());
    assert!(pool.traffic().received > pool.traffic().decoded_received);
    assert_eq!(pool.open_connections(), 0);
}
