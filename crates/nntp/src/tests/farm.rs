use super::*;

#[tokio::test]
async fn pool_metrics_count_parallel_connections_and_bytes_before_bodies_finish() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let release = gate.clone();
    let addr = serve(move |mut sock| {
        let gate = gate.clone();
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert!(read_line(&mut sock).await.starts_with("BODY "));
            write_all(&mut sock, b"222 0 <id>\r\nhello\r\n").await;
            gate.acquire().await.unwrap().forget();
            write_all(&mut sock, b".\r\n").await;
        }
    })
    .await;
    let pool = Arc::new(crate::Pool::new(endpoint(1, addr, 0, false)));
    let mut tasks = tokio::task::JoinSet::new();
    for id in ["a@test", "b@test"] {
        let pool = pool.clone();
        tasks.spawn(async move { pool.body(&ArticleRef::MessageId(id.into())).await.unwrap() });
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while pool.active_connections() != 2 || pool.received_bytes() < setup_bytes() * 2 + 36 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(pool.open_connections(), 2);
    assert_eq!(pool.take_active_peak(), 2);
    release.add_permits(2);
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap(), b"hello\r\n");
    }
    assert_eq!(pool.active_connections(), 0);
    assert_eq!(pool.open_connections(), 2, "idle sockets stay open");
    assert_eq!(pool.take_active_peak(), 0);
    assert_eq!(pool.received_bytes(), setup_bytes() * 2 + 44);
    assert_ne!(
        pool.metrics_id(),
        crate::Pool::new(endpoint(1, addr, 0, false)).metrics_id()
    );
}

fn setup_bytes() -> u64 {
    (b"201 reader ready\r\n".len()
        + b"101 Capabilities\r\nVERSION 2\r\nREADER\r\nOVER\r\n.\r\n".len()) as u64
}

#[tokio::test]
async fn pool_activity_retains_short_requests_and_tracks_aborted_socket_lifetimes() {
    let wait = Arc::new(tokio::sync::Notify::new());
    let blocked = wait.clone();
    let addr = serve(move |mut sock| {
        let blocked = blocked.clone();
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert!(read_line(&mut sock).await.starts_with("BODY "));
            write_multiline(&mut sock, "222 0 <id>", &[b"fast".to_vec()]).await;
            assert!(read_line(&mut sock).await.starts_with("BODY "));
            blocked.notify_one();
            assert!(read_line(&mut sock).await.is_empty());
        }
    })
    .await;
    let pool = Arc::new(crate::Pool::new(endpoint(1, addr, 0, false)));
    assert_eq!(
        pool.body(&ArticleRef::MessageId("fast@test".into()))
            .await
            .unwrap(),
        b"fast\r\n"
    );
    assert_eq!(pool.active_connections(), 0);
    assert_eq!(pool.open_connections(), 1);
    assert_eq!(
        pool.take_active_peak(),
        1,
        "a finished request survives sampling"
    );
    assert_eq!(pool.take_active_peak(), 0, "the next idle sample resets");
    let mut tasks = tokio::task::JoinSet::new();
    let task_pool = pool.clone();
    tasks.spawn(async move {
        task_pool
            .body(&ArticleRef::MessageId("blocked@test".into()))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), wait.notified())
        .await
        .unwrap();
    assert_eq!(pool.active_connections(), 1);
    assert_eq!(pool.open_connections(), 1);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    assert_eq!(pool.active_connections(), 0);
    assert_eq!(
        pool.open_connections(),
        0,
        "interrupted sockets are discarded"
    );
    assert_eq!(pool.take_active_peak(), 1);
}

async fn article_server(
    missing: bool,
    payload: &'static [u8],
    hits: Arc<AtomicUsize>,
) -> SocketAddr {
    serve(move |mut sock| {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            let cmd = read_line(&mut sock).await;
            assert!(cmd.starts_with("BODY "), "{cmd}");
            if missing {
                write_all(&mut sock, b"430 No such article\r\n").await;
            } else {
                write_multiline(&mut sock, "222 0 <id>", &[payload.to_vec()]).await;
            }
        }
    })
    .await
}

#[tokio::test]
async fn farm_skips_fill_until_non_fill_misses() {
    let hits_a = Arc::new(AtomicUsize::new(0));
    let hits_b = Arc::new(AtomicUsize::new(0));
    let hits_c = Arc::new(AtomicUsize::new(0));
    let addr_a = article_server(true, b"from-a", hits_a.clone()).await;
    let addr_b = article_server(false, b"from-b", hits_b.clone()).await;
    let addr_c = article_server(false, b"from-c", hits_c.clone()).await;
    let farm = Farm::new(vec![
        endpoint(3, addr_c, 0, true),
        endpoint(2, addr_b, 2, false),
        endpoint(1, addr_a, 1, false),
    ]);
    let (id, body) = farm
        .body(&ArticleRef::MessageId("seg@example".into()))
        .await
        .unwrap();
    assert_eq!(id, 2);
    assert_eq!(body, b"from-b\r\n");
    assert!(hits_a.load(Ordering::SeqCst) >= 1);
    assert!(hits_b.load(Ordering::SeqCst) >= 1);
    assert_eq!(hits_c.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn farm_uses_fill_after_every_non_fill_misses() {
    let hits_c = Arc::new(AtomicUsize::new(0));
    let addr_a = article_server(true, b"from-a", Arc::new(AtomicUsize::new(0))).await;
    let addr_b = article_server(true, b"from-b", Arc::new(AtomicUsize::new(0))).await;
    let addr_c = article_server(false, b"from-c", hits_c.clone()).await;
    let farm = Farm::new(vec![
        endpoint(1, addr_a, 1, false),
        endpoint(2, addr_b, 2, false),
        endpoint(3, addr_c, 9, true),
    ]);
    let (id, body) = farm
        .body(&ArticleRef::MessageId("seg@example".into()))
        .await
        .unwrap();
    assert_eq!(id, 3);
    assert_eq!(body, b"from-c\r\n");
    assert_eq!(hits_c.load(Ordering::SeqCst), 1);
}
