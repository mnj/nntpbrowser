use super::*;

#[tokio::test]
async fn parallel_uu_windows_keep_original_order_when_first_part_needs_fill() {
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let ready = barrier.clone();
    let primary = serve(move |mut socket| {
        let ready = ready.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            let command = read_line(&mut socket).await;
            tokio::time::timeout(Duration::from_secs(10), ready.wait())
                .await
                .expect("threeparallelUUwindows");
            match command.as_str() {
                "BODY <uu.jpg-1@test>" => {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    write_all(&mut socket, b"430 missing\r\n").await;
                }
                "BODY <uu.jpg-2@test>" => write_multiline(&mut socket, b"#9&5F\r\n").await,
                "BODY <uu.jpg-3@test>" => write_multiline(&mut socket, b"`\r\nend\r\n").await,
                _ => panic!("{command}"),
            }
        }
    })
    .await;
    let fill = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <uu.jpg-1@test>");
        write_multiline(&mut socket, b"begin 644 UU.JPG\r\n#86)C\r\n").await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut config = fixture_server(primary.port());
    config.max_connections = 3;
    let sid = worker.catalog.insert_server(&config).unwrap();
    let mut config = fixture_server(fill.port());
    config.fill = true;
    worker.catalog.insert_server(&config).unwrap();
    let id = add_media_post(&mut worker, sid, "Parallel UU", &[("uu.jpg", 6, 3)]);
    worker.preview_media(id, 1024, 0);
    settle(&mut worker).await;
    let bytes = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::PreviewMediaReady { bytes, .. } => Some(bytes),
            Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
            _ => None,
        })
        .unwrap();
    assert_eq!(&*bytes, b"abcdef");
    assert!(!worker.downloads.exists());
}

#[tokio::test]
async fn cancelling_multipart_preview_closes_all_nested_window_sockets() {
    let started = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let observed = (started.clone(), closed.clone());
    let addr = serve(move |mut socket| {
        let (started, closed) = observed.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            let command = read_line(&mut socket).await;
            if command == "BODY <new.jpg-1@test>" {
                let (body, _) = yenc_simd::encode_article(b"abcdef", "new.jpg", 1, 1, 0, 6);
                write_multiline(&mut socket, &body).await;
            } else {
                assert!(command.starts_with("BODY <old.jpg-"));
                started.fetch_add(1, Ordering::SeqCst);
                assert!(
                    read_line(&mut socket).await.is_empty(),
                    "abortedBODYsocketmustclose"
                );
                closed.fetch_add(1, Ordering::SeqCst);
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut config = fixture_server(addr.port());
    config.max_connections = 4;
    let sid = worker.catalog.insert_server(&config).unwrap();
    let old = add_media_post(&mut worker, sid, "Old", &[("old.jpg", 6, 4)]);
    worker.preview_media(old, 1024, 0);
    worker.drive_previews().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while started.load(Ordering::SeqCst) < 4 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let new = add_media_post(&mut worker, sid, "New", &[("new.jpg", 6, 1)]);
    worker.retain_preview_media(&[new], 0);
    worker.preview_media(new, 1024, 0);
    settle(&mut worker).await;
    while closed.load(Ordering::SeqCst) < 4 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(results.iter().any(|event|matches!(event,Event::PreviewMediaReady{post_id,bytes,..}if *post_id==new && &**bytes==b"abcdef")));
    assert!(!results.iter().any(|event|matches!(event,Event::PreviewMediaReady{post_id,..}|Event::PreviewMediaFailed{post_id,..}if *post_id==old)));
    assert!(!worker.downloads.exists());
}

#[tokio::test]
async fn concurrent_preview_bodies_share_one_encoded_file_budget() {
    let addr = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        let command = read_line(&mut socket).await;
        let part = if command == "BODY <large.jpg-1@test>" {
            1
        } else {
            assert_eq!(command, "BODY <large.jpg-2@test>");
            2
        };
        let (encoded, _) =
            yenc_simd::encode_article(b"abc", "large.jpg", part, 2, (part as u64 - 1) * 3, 6);
        let mut body = vec![b'x'; 40_000];
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&encoded);
        write_multiline(&mut socket, &body).await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let id = add_media_post(&mut worker, sid, "Large encoded", &[("large.jpg", 6, 2)]);
    worker.preview_media(id, 1024, 0);
    settle(&mut worker).await;
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(results.iter().any(
        |event| matches!(event,Event::PreviewMediaFailed{error,..}if error.contains("body exceeds"))
    ));
    assert!(!results
        .iter()
        .any(|event| matches!(event, Event::PreviewMediaReady { .. })));
    assert!(!worker.downloads.exists());
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
}
