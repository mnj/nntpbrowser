//! Transcripts shaped like legacy commercial reader responses, including the
//! binary zlib XZVER variant that a line-oriented fake server did not exercise.
use super::*;

const ROWS: &[u8] = b"1\tImage one\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<1@test>\t\t20\t1\r\n2\tImage two\tPoster\tTue, 10 Mar 2026 12:00:01 +0000\t<2@test>\t\t30\t1\r\n.\r\n";

#[tokio::test]
async fn stale_deflate_strategy_obeys_current_capabilities() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        overview_fmt(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "XZVER 1-2");
        write_all(&mut sock, b"500 unsupported\r\n").await;
        assert_eq!(read_line(&mut sock).await, "XFEATURE COMPRESS GZIP");
        write_all(&mut sock, b"500 unsupported\r\n").await;
        assert_eq!(read_line(&mut sock).await, "OVER 1-2");
        write_all(&mut sock, b"224 overview\r\n").await;
        write_all(&mut sock, ROWS).await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut rows = Vec::new();
    let strategy = conn
        .fetch_overview(1, 2, HeaderStrategy::CompressDeflate, |row| rows.push(row))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::Plain);
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn compress_503_falls_back_without_reconnecting_or_repeating_compress() {
    for hint in [HeaderStrategy::Unknown, HeaderStrategy::CompressDeflate] {
        let addr = serve(move |mut sock| async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "COMPRESS DEFLATE\r\n").await;
            overview_fmt(&mut sock).await;
            if hint == HeaderStrategy::Unknown {
                assert_eq!(read_line(&mut sock).await, "XZVER 1-2");
                write_all(&mut sock, b"500 unsupported\r\n").await;
            }
            assert_eq!(read_line(&mut sock).await, "COMPRESS DEFLATE");
            write_all(&mut sock, b"503 algorithm unavailable\r\n").await;
            assert_eq!(read_line(&mut sock).await, "XFEATURE COMPRESS GZIP");
            write_all(&mut sock, b"500 unsupported\r\n").await;
            assert_eq!(read_line(&mut sock).await, "OVER 1-2");
            write_all(&mut sock, b"224 overview\r\n").await;
            write_all(&mut sock, ROWS).await;
            assert_eq!(read_line(&mut sock).await, "DATE");
            write_all(&mut sock, b"111 20261010123456\r\n").await;
        })
        .await;
        let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
        let strategy = conn.fetch_overview(1, 2, hint, |_| {}).await.unwrap();
        assert_eq!(strategy, HeaderStrategy::Plain);
        assert_eq!(conn.date().await.unwrap(), "20261010123456");
    }
}

#[tokio::test]
async fn successful_reader_mode_refresh_discards_previous_capabilities() {
    for unavailable in [false, true] {
        let addr = serve(move |mut sock| async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "MODE-READER\r\nCOMPRESS DEFLATE\r\n").await;
            assert_eq!(read_line(&mut sock).await, "MODE READER");
            write_all(&mut sock, b"201 reader mode\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            if unavailable {
                write_all(&mut sock, b"500 capabilities unavailable\r\n").await;
            } else {
                write_all(&mut sock, b"101 caps\r\nREADER\r\nXOVER\r\n.\r\n").await;
            }
            overview_fmt(&mut sock).await;
            assert_eq!(
                read_line(&mut sock).await,
                if unavailable { "OVER 1-2" } else { "XOVER 1-2" }
            );
            write_all(&mut sock, b"224 overview\r\n").await;
            write_all(&mut sock, ROWS).await;
        })
        .await;
        let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
        assert!(!conn.capabilities().compress_deflate);
        assert!(!conn.capabilities().mode_reader);
        conn.fetch_overview(1, 2, HeaderStrategy::Plain, |_| {})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn oversized_pipeline_returns_promptly_and_keeps_the_session_usable() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        assert_eq!(read_line(&mut sock).await, "DATE");
        write_all(&mut sock, b"111 20261010123456\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let ids = [
        ArticleRef::Number(1),
        ArticleRef::MessageId("x".repeat(512)),
    ];
    assert!(matches!(
        tokio::time::timeout(Duration::from_millis(100), conn.body_pipeline(&ids))
            .await
            .unwrap(),
        Err(Error::Protocol { .. })
    ));
    assert_eq!(conn.date().await.unwrap(), "20261010123456");
}

#[tokio::test]
async fn compressed_overview_drains_large_buffered_output_before_waiting_for_network() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "XZVER\r\n").await;
        overview_fmt(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "XZVER 1-5000");
        let mut rows = Vec::new();
        for _ in 0..5000 {
            rows.extend_from_slice(&ROWS[..ROWS.len() - 3]);
        }
        rows.extend_from_slice(b".\r\n");
        let compressed = zlib_compress(&rows).unwrap();
        assert!(compressed.len() < 8192);
        let mut response = b"224 compressed overview\r\n".to_vec();
        response.extend_from_slice(&compressed);
        write_all(&mut sock, &response).await;
        // The peer waits for our next command; no bytes follow the member.
        assert_eq!(read_line(&mut sock).await, "GROUP alt.other");
        write_all(&mut sock, b"211 1 4 4 alt.other\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut count = 0;
    conn.fetch_overview(1, 5000, HeaderStrategy::Xzver, |_| count += 1)
        .await
        .unwrap();
    assert_eq!(count, 10000);
    assert_eq!(conn.group("alt.other").await.unwrap().high, 4);
}

#[tokio::test]
async fn binary_zlib_xzver_ends_at_member_boundary_without_a_wire_dot() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "XZVER\r\nXOVER\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 0 1 2 alt.test\r\n").await;
        overview_fmt(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "XZVER 1-2");
        let compressed = zlib_compress(ROWS).unwrap();
        write_all(&mut sock, b"224 compressed overview follows\r\n").await;
        write_all(&mut sock, &compressed[..1]).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        write_all(&mut sock, &compressed[1..]).await;
        // No outer dot or newline: both belong to the decompressed text.
        assert_eq!(read_line(&mut sock).await, "GROUP alt.other");
        write_all(&mut sock, b"211 1 4 4 alt.other\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    conn.group("alt.test").await.unwrap();
    let mut rows = Vec::new();
    let strategy = conn
        .fetch_overview(1, 2, HeaderStrategy::Unknown, |line| rows.push(line))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::Xzver);
    assert_eq!(
        rows.iter().map(|row| row.article).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(conn.group("alt.other").await.unwrap().high, 4);
}

#[tokio::test]
async fn authenticated_capabilities_select_xover_without_sending_over() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(
            &mut sock,
            b"101 caps\r\nVERSION 2\r\nAUTHINFO USER\r\n.\r\n",
        )
        .await;
        assert_eq!(read_line(&mut sock).await, "AUTHINFO USER reader");
        write_all(&mut sock, b"381 password\r\n").await;
        assert_eq!(read_line(&mut sock).await, "AUTHINFO PASS secret");
        write_all(&mut sock, b"281 accepted\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(
            &mut sock,
            b"101 caps\r\nVERSION 2\r\nMODE-READER\r\nREADER\r\nXOVER\r\nXZVER\r\n.\r\n",
        )
        .await;
        assert_eq!(read_line(&mut sock).await, "MODE READER");
        write_all(&mut sock, b"201 reader\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nREADER\r\nXOVER\r\nXZVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 0 1 2 alt.test\r\n").await;
        overview_fmt(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "XOVER 1-2");
        write_all(&mut sock, b"224 overview\r\n").await;
        write_all(&mut sock, ROWS).await;
    })
    .await;
    let mut cfg = plain_cfg(addr);
    cfg.username = Some("reader".into());
    cfg.password = Some("secret".into());
    let mut conn = Connection::connect(&cfg).await.unwrap();
    assert!(conn.capabilities().over);
    conn.group("alt.test").await.unwrap();
    let mut articles = Vec::new();
    conn.fetch_overview(1, 2, HeaderStrategy::Plain, |row| {
        articles.push(row.article)
    })
    .await
    .unwrap();
    assert_eq!(articles, [1, 2]);
}

#[tokio::test]
async fn speculative_xzver_timeout_reconnects_group_and_falls_back_to_plain() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let addr = serve(move |mut sock| {
        let attempts = attempts.clone();
        async move {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nXOVER\r\nXZVER\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
            write_all(&mut sock, b"211 2 1 2 alt.test\r\n").await;
            overview_fmt(&mut sock).await;
            if attempt == 0 {
                assert_eq!(read_line(&mut sock).await, "XZVER 1-2");
                write_all(&mut sock, b"224 incomplete compressed response\r\n").await;
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else {
                assert_eq!(read_line(&mut sock).await, "XOVER 1-2");
                write_all(&mut sock, b"224 overview\r\n").await;
                write_all(&mut sock, ROWS).await;
            }
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.config.timeout = Duration::from_millis(100);
    let pool = crate::Pool::new(ep);
    pool.group("alt.test").await.unwrap();
    let mut rows = Vec::new();
    let strategy = pool
        .fetch_overview(1, 2, |row| rows.push(row.article))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::Plain);
    assert_eq!(rows, [1, 2]);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn legacy_server_without_capabilities_uses_xover_after_over_rejection() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"500 unknown command\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 2 1 2 alt.test\r\n").await;
        assert_eq!(read_line(&mut sock).await, "LIST OVERVIEW.FMT");
        write_all(&mut sock, b"500 unknown command\r\n").await;
        assert_eq!(read_line(&mut sock).await, "OVER 1-2");
        write_all(&mut sock, b"500 unknown command\r\n").await;
        assert_eq!(read_line(&mut sock).await, "XOVER 1-2");
        write_all(&mut sock, b"224 overview\r\n").await;
        write_all(&mut sock, ROWS).await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    conn.group("alt.test").await.unwrap();
    let mut rows = Vec::new();
    conn.fetch_overview(1, 2, HeaderStrategy::Plain, |row| rows.push(row.article))
        .await
        .unwrap();
    assert_eq!(rows, [1, 2]);
}

#[tokio::test]
async fn xfeature_accepts_plain_status_and_independent_zlib_bodies_without_dots() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(
            &mut sock,
            b"101 caps\r\nXOVER\r\nXFEATURE-COMPRESS\r\n.\r\n",
        )
        .await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 2 1 2 alt.test\r\n").await;
        overview_fmt(&mut sock).await;
        assert_eq!(read_line(&mut sock).await, "XFEATURE COMPRESS GZIP");
        write_all(&mut sock, b"290 compression enabled\r\n").await;
        for _ in 0..2 {
            assert_eq!(read_line(&mut sock).await, "XOVER 1-2");
            let compressed = zlib_compress(&ROWS[..ROWS.len() - 3]).unwrap();
            let mut response = b"224 compressed overview\r\n".to_vec();
            response.extend_from_slice(&compressed);
            write_all(&mut sock, &response).await;
        }
        assert_eq!(read_line(&mut sock).await, "LIST ACTIVE *");
        let mut response = b"215 active\r\n".to_vec();
        response.extend_from_slice(&zlib_compress(b"alt.test 2 1 y\r\n").unwrap());
        write_all(&mut sock, &response).await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    conn.group("alt.test").await.unwrap();
    for _ in 0..2 {
        let mut rows = Vec::new();
        assert_eq!(
            conn.fetch_overview(1, 2, HeaderStrategy::XfeatureGzip, |row| rows
                .push(row.article))
                .await
                .unwrap(),
            HeaderStrategy::XfeatureGzip
        );
        assert_eq!(rows, [1, 2]);
    }
    let groups = conn.list_active("*").await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "alt.test");
}
