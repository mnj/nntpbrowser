use super::*;

#[tokio::test]
async fn active_list_streams_before_terminator_and_skips_junk() {
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let addr = serve(move |mut sock| {
        let release_rx = release_rx.clone();
        async move {
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "LIST ACTIVE *");
            write_all(
                &mut sock,
                b"215 active\r\ninvalid junk\r\nalt.first 9 4 y\r\n",
            )
            .await;
            // The client must deliver a callback before the response finishes.
            release_rx.lock().await.take().unwrap().await.unwrap();
            write_all(&mut sock, b"..hidden 3 1 m\r\n.\r\n").await;
        }
    })
    .await;
    let mut conn = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let mut release = Some(release_tx);
    let mut groups = Vec::new();
    conn.list_active_each("*", |group| {
        if let Some(release) = release.take() {
            release.send(()).unwrap();
        }
        groups.push(group);
    })
    .await
    .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(
        (&*groups[0].name, groups[0].low, groups[0].high),
        ("alt.first", 4, 9)
    );
    assert_eq!(groups[1].name, ".hidden");
    assert_eq!(groups[1].status, 'm');
}

#[tokio::test]
async fn active_pool_does_not_retry_partial_callbacks() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let addr = serve(move |mut sock| {
        let hits = hits.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "LIST ACTIVE *");
            write_all(&mut sock, b"215 active\r\nalt.partial 8 1 y\r\n").await;
        }
    })
    .await;
    let pool = crate::Pool::new(endpoint(1, addr, 1, false));
    let mut groups = Vec::new();
    assert!(matches!(
        pool.list_active_each("*", |group| groups.push(group)).await,
        Err(Error::Closed)
    ));
    assert_eq!(groups.len(), 1);
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn active_pool_retries_before_first_callback() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let addr = serve(move |mut sock| {
        let hits = hits.clone();
        async move {
            let attempt = hits.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "LIST ACTIVE *");
            if attempt != 0 {
                write_all(&mut sock, b"215 active\r\nalt.retry 8 1 y\r\n.\r\n").await;
            }
        }
    })
    .await;
    let pool = crate::Pool::new(endpoint(1, addr, 1, false));
    let groups = pool.list_active("*").await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(observed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn overview_retry_restores_selected_group() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let addr = serve(move |mut sock| {
        let hits = hits.clone();
        async move {
            let attempt = hits.fetch_add(1, Ordering::SeqCst);
            greet(&mut sock).await;
            capabilities(&mut sock, "").await;
            assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
            write_all(&mut sock, b"211 1 1 1 alt.test\r\n").await;
            overview_fmt(&mut sock).await;
            assert_eq!(read_line(&mut sock).await, "OVER 1-1");
            if attempt != 0 {
                write_all(
                    &mut sock,
                    b"224 overview\r\n1\tSubject\tPoster\tdate\t<1@x>\t\t10\t1\r\n.\r\n",
                )
                .await;
            }
        }
    })
    .await;
    let pool = crate::Pool::new(endpoint(1, addr, 1, false));
    pool.set_header_strategy(HeaderStrategy::Plain).await;
    pool.group("alt.test").await.unwrap();
    let mut articles = Vec::new();
    pool.fetch_overview(1, 1, |line| articles.push(line.article))
        .await
        .unwrap();
    assert_eq!(articles, [1]);
    assert_eq!(observed.load(Ordering::SeqCst), 2);
}
