use super::*;

fn overview_text(pad: usize) -> String {
    format!(
        "1\tName{pad}\tFoo\tTue, 10 Mar 2026 12:00:00 +0000\t<m1@example>\t\t100\t2\n2\t.hidden subject\tBar\tTue, 10 Mar 2026 12:00:01 +0000\t<m2@example>\t\t50\t1\n",
        pad = " ".repeat(pad)
    )
}

fn yenc_with_dot_line(compress: fn(&[u8]) -> Result<Vec<u8>, Error>) -> Vec<Vec<u8>> {
    for pad in 0..80 {
        let text = overview_text(pad);
        let compressed = compress(text.as_bytes()).unwrap();
        for width in 1..=96 {
            let lines = yenc::encode_width(&compressed, width);
            let stuffed = lines
                .iter()
                .any(|line| !line.starts_with(b"=y") && line.first() == Some(&b'.'));
            if stuffed {
                assert!(
                    lines[0].windows(7).any(|w| w == b"size=-1"),
                    "encoder must emit size=-1"
                );
                return lines;
            }
        }
    }
    panic!("could not force a yEnc line that NNTP must dot-stuff");
}

async fn xzver_roundtrip(compress: fn(&[u8]) -> Result<Vec<u8>, Error>) {
    let lines = yenc_with_dot_line(compress);
    let addr = serve(move |mut sock| {
        let lines = lines.clone();
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            overview_fmt(&mut sock).await;
            let cmd = read_line(&mut sock).await;
            assert!(cmd.starts_with("XZVER "), "{cmd}");
            write_multiline(&mut sock, "224 overview", &lines).await;
        }
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut got = Vec::new();
    let strategy = conn
        .fetch_overview(1, 2, HeaderStrategy::Unknown, |line| got.push(line))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::Xzver);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].article, 1);
    assert!(got[0].subject.starts_with("Name"), "{}", got[0].subject);
    assert_eq!(got[0].from, "Foo");
    assert_eq!(got[0].message_id, "<m1@example>");
    assert_eq!(got[1].article, 2);
    assert_eq!(got[1].subject, ".hidden subject");
    assert_eq!(got[1].bytes, 50);
}

#[tokio::test]
async fn xzver_zlib_yenc_with_dot_stuffed_line() {
    xzver_roundtrip(zlib_compress).await;
}

#[tokio::test]
async fn xzver_xz_payload() {
    xzver_roundtrip(xz_compress).await;
}

struct SessionLog {
    pre: Vec<String>,
    post: Vec<String>,
    raw_after: Vec<u8>,
}

#[tokio::test]
async fn xzver_unsupported_then_compress_deflate() {
    let log = Arc::new(Mutex::new(SessionLog {
        pre: Vec::new(),
        post: Vec::new(),
        raw_after: Vec::new(),
    }));
    let log_bg = log.clone();
    let addr = serve(move |mut sock| {
        let log = log_bg.clone();
        async move {
            greet(&mut sock).await;
            let cmd = read_line(&mut sock).await;
            log.lock().unwrap().pre.push(cmd);
            write_all(
                &mut sock,
                b"101 Capabilities\r\nVERSION 2\r\nREADER\r\nOVER\r\nMODE-READER\r\nCOMPRESS DEFLATE\r\n.\r\n",
            )
            .await;
            for _ in 0..2 {
                let cmd = read_line(&mut sock).await;
                log.lock().unwrap().pre.push(cmd.clone());
                if cmd.starts_with("AUTHINFO USER ") {
                    write_all(&mut sock, b"381 password required\r\n").await;
                } else if cmd.starts_with("AUTHINFO PASS ") {
                    write_all(&mut sock, b"281 Authentication accepted\r\n").await;
                }
            }
            let refreshed = read_line(&mut sock).await;
            assert_eq!(refreshed, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\nMODE-READER\r\nCOMPRESS DEFLATE\r\n.\r\n").await;
            let mode = read_line(&mut sock).await;
            log.lock().unwrap().pre.push(mode);
            write_all(&mut sock, b"200 reader mode\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\nCOMPRESS DEFLATE\r\n.\r\n").await;

            loop {
                let cmd = read_line(&mut sock).await;
                log.lock().unwrap().pre.push(cmd.clone());
                if cmd.starts_with("LIST OVERVIEW.FMT") {
                    write_all(
                        &mut sock,
                        b"215 fields\r\nSubject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
                    )
                    .await;
                } else if cmd.starts_with("XZVER") {
                    write_all(&mut sock, b"500 Unknown command\r\n").await;
                } else if cmd == "COMPRESS DEFLATE" {
                    write_all(&mut sock, b"206 Compression active\r\n").await;
                    break;
                } else {
                    panic!("unexpected plaintext command: {cmd}");
                }
            }

            let mut peer = Peer::new();
            let mut raw = Vec::new();
            loop {
                let cmd = peer.read_line(&mut sock, &mut raw).await;
                log.lock().unwrap().post.push(cmd.clone());
                if cmd.starts_with("OVER ") {
                    let body = b"224 overview\r\n1\tHello\tPoster\tDate\t<m@x>\t\t12\t1\r\n.\r\n";
                    peer.write(&mut sock, body).await;
                } else if cmd.starts_with("GROUP ") {
                    peer.write(&mut sock, b"211 1 1 1 alt.binaries.test\r\n").await;
                    break;
                } else if cmd == "QUIT" {
                    break;
                } else {
                    panic!("unexpected compressed command: {cmd}");
                }
            }
            log.lock().unwrap().raw_after = raw;
        }
    })
    .await;

    let mut cfg = plain_cfg(addr);
    cfg.username = Some("reader".into());
    cfg.password = Some("s3cret-pass".into());
    let mut conn = Connection::connect(&cfg).await.unwrap();
    let mut lines = Vec::new();
    let strategy = conn
        .fetch_overview(1, 1, HeaderStrategy::Unknown, |line| lines.push(line))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::CompressDeflate);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].subject, "Hello");
    assert_eq!(lines[0].message_id, "<m@x>");
    let status = conn.group("alt.binaries.test").await.unwrap();
    assert_eq!(status.high, 1);

    let saved = log.lock().unwrap();
    let pass_at = saved
        .pre
        .iter()
        .position(|line| line.starts_with("AUTHINFO PASS "))
        .expect("auth");
    let compress_at = saved
        .pre
        .iter()
        .position(|line| line == "COMPRESS DEFLATE")
        .expect("compress");
    assert!(pass_at < compress_at);
    assert!(!saved.pre.iter().any(|line| line.starts_with("GROUP ")));
    assert!(saved.post.iter().any(|line| line.starts_with("GROUP ")));
    assert!(saved.post.iter().any(|line| line.starts_with("OVER ")));
    assert!(!saved.post.iter().any(|line| line.contains("AUTHINFO")));
    assert!(!saved.raw_after.windows(6).any(|w| w == b"GROUP "));
    assert!(!saved.raw_after.windows(8).any(|w| w == b"AUTHINFO"));
    assert!(!saved.raw_after.windows(5).any(|w| w == b"Hello"));
}

struct Peer {
    dec: Decompress,
    pending: Vec<u8>,
    plain: Vec<u8>,
    comp: Compress,
}

impl Peer {
    fn new() -> Self {
        Self {
            dec: Decompress::new(false),
            pending: Vec::new(),
            plain: Vec::new(),
            comp: Compress::new(Compression::new(3), false),
        }
    }

    async fn read_line(&mut self, sock: &mut TcpStream, raw: &mut Vec<u8>) -> String {
        loop {
            if let Some(line) = pop_line(&mut self.plain) {
                return String::from_utf8_lossy(&line).into_owned();
            }
            let mut buf = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf))
                .await
                .expect("inflate read timeout")
                .expect("read");
            if n == 0 {
                panic!("eof in compressed stream");
            }
            raw.extend_from_slice(&buf[..n]);
            self.pending.extend_from_slice(&buf[..n]);
            let out = inflate_pending(&mut self.dec, &mut self.pending).unwrap();
            self.plain.extend(out);
        }
    }

    async fn write(&mut self, sock: &mut TcpStream, data: &[u8]) {
        let wire = compress_sync(&mut self.comp, data).unwrap();
        write_all(sock, &wire).await;
    }
}

#[tokio::test]
async fn xfeature_gzip_then_over() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        overview_fmt(&mut sock).await;
        let xzver = read_line(&mut sock).await;
        assert!(xzver.starts_with("XZVER "), "{xzver}");
        write_all(&mut sock, b"500 Unknown command\r\n").await;
        let feature = read_line(&mut sock).await;
        assert_eq!(feature, "XFEATURE COMPRESS GZIP");
        write_all(&mut sock, b"290 feature enabled\r\n").await;
        let over = read_line(&mut sock).await;
        assert_eq!(over, "OVER 1-1");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), Compression::new(3));
        use std::io::Write;
        enc.write_all(b"224 overview\r\n1\tHello\tPoster\tDate\t<m@x>\t\t12\t1\r\n.\r\n")
            .unwrap();
        enc.flush().unwrap();
        let wire = std::mem::take(enc.get_mut());
        assert!(wire.starts_with(&[0x1f, 0x8b]), "response is a gzip stream");
        assert!(!wire.starts_with(b"224 "));
        write_all(&mut sock, &wire).await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut lines = Vec::new();
    let strategy = conn
        .fetch_overview(1, 1, HeaderStrategy::Unknown, |line| lines.push(line))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::XfeatureGzip);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].subject, "Hello");
    assert_eq!(lines[0].bytes, 12);
}
