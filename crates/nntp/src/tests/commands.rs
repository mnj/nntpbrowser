use super::*;

#[tokio::test]
async fn later_window_reconnect_failure_keeps_the_first_completed_body() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let addr = serve(move |mut sock| {
        let count = count.clone();
        async move {
            if count.fetch_add(1, Ordering::SeqCst) > 0 {
                write_all(&mut sock, b"400 temporarily unavailable\r\n").await;
                return;
            }
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "BODY <one@test>");
            write_all(&mut sock, b"222 body\r\none\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "BODY <two@test>");
            write_all(&mut sock, b"400 closing\r\n").await;
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.pipeline_depth = 1;
    ep.max_connections = 1;
    let pool = crate::Pool::new(ep);
    let ids = ["one", "two", "three"].map(|id| ArticleRef::MessageId(format!("{id}@test")));
    let replies = pool
        .body_many_limited(&ids, &[100, 100, 100])
        .await
        .unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0].as_ref().unwrap(), b"one\r\n");
    assert!(replies[1].is_err());
    assert!(replies[2].is_err());
}

#[tokio::test]
async fn bounded_pipeline_keeps_completed_replies_and_closes_oversized_window() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let addr = serve(move |mut sock| {
        let count = count.clone();
        async move {
            let attempt = count.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "PIPELINING\r\n").await;
            if attempt == 0 {
                for id in ["one", "two", "three"] {
                    assert_eq!(read_line(&mut sock).await, format!("BODY <{id}@test>"));
                }
                write_all(
                    &mut sock,
                    b"222 body\r\na\r\n.\r\n222 body\r\n123456789\r\n",
                )
                .await;
                tokio::time::sleep(Duration::from_millis(200)).await;
            } else {
                assert_eq!(read_line(&mut sock).await, "BODY <again@test>");
                write_all(&mut sock, b"222 body\r\nok\r\n.\r\n").await;
            }
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.pipeline_depth = 3;
    ep.max_connections = 1;
    let pool = crate::Pool::new(ep);
    let ids = ["one", "two", "three"].map(|id| ArticleRef::MessageId(format!("{id}@test")));
    let replies = pool.body_many_limited(&ids, &[16, 8, 16]).await.unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0].as_ref().unwrap(), b"a\r\n");
    assert!(matches!(replies[1], Err(Error::BodyTooLarge(8))));
    assert!(matches!(replies[2], Err(Error::Closed)));
    assert_eq!(
        pool.body_limited(&ArticleRef::MessageId("again@test".into()), 4)
            .await
            .unwrap(),
        b"ok\r\n"
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn headers_and_article_bodies_share_the_configured_connection_ceiling() {
    let requests = Arc::new(AtomicUsize::new(0));
    let groups = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(tokio::sync::Notify::new());
    let (count, group_count, release) = (requests.clone(), groups.clone(), ready.clone());
    let addr = serve(move |mut sock| {
        let (count, group_count, release) = (count.clone(), group_count.clone(), release.clone());
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() {
                    break;
                }
                if command.starts_with("BODY ") {
                    count.fetch_add(1, Ordering::SeqCst);
                    release.notified().await;
                    write_all(&mut sock, b"222 body\r\nok\r\n.\r\n").await;
                } else {
                    assert_eq!(command, "GROUP alt.test");
                    group_count.fetch_add(1, Ordering::SeqCst);
                    write_all(&mut sock, b"211 1 1 1 alt.test\r\n").await;
                }
            }
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.max_connections = 2;
    let pool = Arc::new(crate::Pool::new(ep));
    let first = tokio::spawn({
        let pool = pool.clone();
        async move { pool.body(&ArticleRef::Number(1)).await }
    });
    let second = tokio::spawn({
        let pool = pool.clone();
        async move { pool.body(&ArticleRef::Number(2)).await }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while requests.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let header = tokio::spawn({
        let pool = pool.clone();
        async move { pool.group("alt.test").await }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(groups.load(Ordering::SeqCst), 0); // Both permits are used by BODY.
    ready.notify_waiters();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(header.await.unwrap().unwrap().high, 1);
    assert_eq!(groups.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bounded_body_rejects_oversize_and_discards_partial_connection() {
    let count = Arc::new(AtomicUsize::new(0));
    let attempts = count.clone();
    let addr = serve(move |mut sock| {
        let attempts = attempts.clone();
        async move {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "BODY <part@test>");
            if attempt == 0 {
                // No terminator follows. The bounded reader must fail now.
                write_all(&mut sock, b"222 body\r\n123456789\r\n").await;
                tokio::time::sleep(Duration::from_millis(300)).await;
            } else {
                write_all(&mut sock, b"222 body\r\nok\r\n.\r\n").await;
            }
        }
    })
    .await;
    let pool = crate::Pool::new(endpoint(1, addr, 1, false));
    let id = ArticleRef::MessageId("part@test".into());
    assert!(matches!(
        pool.body_limited(&id, 8).await,
        Err(Error::BodyTooLarge(8))
    ));
    assert_eq!(pool.body_limited(&id, 4).await.unwrap(), b"ok\r\n");
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn bad_password_is_not_in_the_error() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        let user = read_line(&mut sock).await;
        assert!(user.starts_with("AUTHINFO USER "), "{user}");
        write_all(&mut sock, b"381 password required\r\n").await;
        let pass = read_line(&mut sock).await;
        assert!(pass.starts_with("AUTHINFO PASS "), "{pass}");
        write_all(&mut sock, b"481 Authentication failed\r\n").await;
    })
    .await;
    let mut cfg = plain_cfg(addr);
    cfg.username = Some("reader".into());
    cfg.password = Some("s3cret-pass".into());
    let err = match Connection::connect(&cfg).await {
        Ok(_) => panic!("expected authentication to fail"),
        Err(err) => err,
    };
    let shown = err.to_string();
    assert!(!shown.contains("s3cret-pass"), "{shown}");
    assert!(matches!(err, Error::Auth), "{err:?}");
}

#[tokio::test]
async fn group_and_list_active() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        let group = read_line(&mut sock).await;
        assert_eq!(group, "GROUP alt.test");
        write_all(&mut sock, b"211 10 1 100 alt.test\r\n").await;
        let list = read_line(&mut sock).await;
        assert_eq!(list, "LIST ACTIVE alt.*");
        write_all(
            &mut sock,
            b"215 list\r\nalt.test 100 1 y\r\nalt.binaries.foo 5 1 n\r\n.\r\n",
        )
        .await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let status = conn.group("alt.test").await.unwrap();
    assert_eq!(status.count, 10);
    assert_eq!(status.low, 1);
    assert_eq!(status.high, 100);
    assert_eq!(status.name, "alt.test");
    let groups = conn.list_active("alt.*").await.unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].name, "alt.test");
    assert_eq!(groups[0].high, 100);
    assert_eq!(groups[0].low, 1);
    assert_eq!(groups[0].status, 'y');
    assert_eq!(groups[1].high, 5);
    assert_eq!(groups[1].status, 'n');
}

#[tokio::test]
async fn over_dot_stuffing() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        overview_fmt(&mut sock).await;
        let over = read_line(&mut sock).await;
        assert_eq!(over, "OVER 1-2");
        // The second overview line is sent already dot-stuffed (`2\t...` on the wire
        // begins with an extra `.`) so the client must unstuff before parsing.
        write_all(
            &mut sock,
            b"224 overview\r\n\
1\t.subject\tAnn\tTue, 10 Mar 2026 12:00:00 +0000\t<m1@example>\t\t10\t1\r\n\
.2\tdot stuffed\tBob\tTue, 10 Mar 2026 12:00:01 +0000\t<m2@example>\t<m1@example>\t20\t2\r\n\
.\r\n",
        )
        .await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut lines = Vec::new();
    let strategy = conn
        .fetch_overview(1, 2, HeaderStrategy::Plain, |line| lines.push(line))
        .await
        .unwrap();
    assert_eq!(strategy, HeaderStrategy::Plain);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].article, 1);
    assert_eq!(lines[0].subject, ".subject");
    assert_eq!(lines[0].message_id, "<m1@example>");
    assert_eq!(lines[1].article, 2);
    assert_eq!(lines[1].subject, "dot stuffed");
    assert_eq!(lines[1].references, "<m1@example>");
    assert_eq!(lines[1].bytes, 20);
    assert_eq!(lines[1].lines, 2);
}

#[tokio::test]
async fn body_by_message_id() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        let cmd = read_line(&mut sock).await;
        assert_eq!(cmd, "BODY <part@example>");
        write_multiline(
            &mut sock,
            "222 0 <part@example>",
            &[
                b"=ybegin line=128 size=5 name=a.bin".to_vec(),
                b"hello".to_vec(),
                b".hidden".to_vec(),
            ],
        )
        .await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let body = conn
        .body(&ArticleRef::MessageId("part@example".into()))
        .await
        .unwrap();
    assert_eq!(
        body,
        b"=ybegin line=128 size=5 name=a.bin\r\nhello\r\n.hidden\r\n"
    );
}

#[tokio::test]
async fn stat_and_missing() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        let stat = read_line(&mut sock).await;
        assert_eq!(stat, "STAT 5");
        write_all(&mut sock, b"223 5 <id@example>\r\n").await;
        let missing = read_line(&mut sock).await;
        assert_eq!(missing, "STAT <missing@example>");
        write_all(&mut sock, b"430 No such article\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let n = conn.stat(&ArticleRef::Number(5)).await.unwrap();
    assert_eq!(n, 5);
    let err = conn
        .stat(&ArticleRef::MessageId("missing@example".into()))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Missing));
}

#[tokio::test]
async fn pipeline_matches_send_order_when_server_waits() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_bg = seen.clone();
    let addr = serve(move |mut sock| {
        let seen = seen_bg.clone();
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            let mut cmds = Vec::new();
            for _ in 0..3 {
                cmds.push(read_line(&mut sock).await);
            }
            *seen.lock().unwrap() = cmds.clone();
            write_multiline(&mut sock, "222 0 <a@x>", &[b"alpha".to_vec()]).await;
            write_all(&mut sock, b"430 No such article\r\n").await;
            write_multiline(&mut sock, "222 0 <c@x>", &[b"gamma".to_vec()]).await;
        }
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let ids = [
        ArticleRef::MessageId("a@x".into()),
        ArticleRef::MessageId("b@x".into()),
        ArticleRef::MessageId("c@x".into()),
    ];
    let got = conn.body_pipeline(&ids).await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            "BODY <a@x>".to_string(),
            "BODY <b@x>".to_string(),
            "BODY <c@x>".to_string(),
        ]
    );
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].as_ref().unwrap(), b"alpha\r\n");
    assert!(matches!(got[1].as_ref().unwrap_err(), Error::Missing));
    assert_eq!(got[2].as_ref().unwrap(), b"gamma\r\n");
}

#[tokio::test]
async fn article_refs_with_line_breaks_never_reach_the_wire() {
    let addr = serve(|mut sock| async move {
        greet(&mut sock).await;
        capabilities(&mut sock, "").await;
        // No BODY, HEAD, STAT, or partial pipeline should have been written.
        assert_eq!(read_line(&mut sock).await, "DATE");
        write_all(&mut sock, b"111 20261010120000\r\n").await;
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let id = ArticleRef::MessageId("<bad@test>\r\nQUIT".into());
    assert!(conn.body(&id).await.is_err());
    assert!(conn.head(&id).await.is_err());
    assert!(conn.stat(&id).await.is_err());
    assert!(conn
        .body_pipeline(&[ArticleRef::Number(1), id])
        .await
        .is_err());
    assert_eq!(conn.date().await.unwrap(), "20261010120000");
}
