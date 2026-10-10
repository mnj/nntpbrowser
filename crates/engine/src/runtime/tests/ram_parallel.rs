use super::media::add_media_post;
use super::*;
use std::sync::Arc;
mod multipart;

async fn settle(worker: &mut Worker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !worker.preview_pending.is_empty() {
        worker.drive_previews().await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "parallel preview did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn fifty_visible_previews_use_fifty_configured_connections_and_queue_the_fifty_first() {
    let barrier = Arc::new(tokio::sync::Barrier::new(50));
    let requests = Arc::new(AtomicUsize::new(0));
    let busy = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let observed = (
        barrier.clone(),
        requests.clone(),
        busy.clone(),
        peak.clone(),
    );
    let addr = serve(move |mut socket| {
        let (barrier, requests, busy, peak) = observed.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut socket).await;
                if command.is_empty() {
                    break;
                }
                let name = command
                    .strip_prefix("BODY <")
                    .unwrap()
                    .strip_suffix("-1@test>")
                    .unwrap();
                let index = requests.fetch_add(1, Ordering::SeqCst);
                let active = busy.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(active, Ordering::SeqCst);
                if index < 50 {
                    tokio::time::timeout(Duration::from_secs(10), barrier.wait())
                        .await
                        .expect("all50socketsmustissueBODYbeforeanyreply");
                }
                let (body, _) = yenc_simd::encode_article(b"abcdef", name, 1, 1, 0, 6);
                write_multiline(&mut socket, &body).await;
                busy.fetch_sub(1, Ordering::SeqCst);
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut config = fixture_server(addr.port());
    config.max_connections = 50;
    let sid = worker.catalog.insert_server(&config).unwrap();
    for index in 0..51 {
        let id = add_media_post(
            &mut worker,
            sid,
            &format!("Visible{index}"),
            &[(&format!("image{index}.jpg"), 6, 1)],
        );
        worker.preview_media(id, 1024, 0);
    }
    worker.drive_previews().await;
    assert_eq!(worker.preview_active.len(), 50);
    assert_eq!(worker.preview_queue.len(), 1);
    settle(&mut worker).await;
    assert_eq!(requests.load(Ordering::SeqCst), 51);
    assert_eq!(peak.load(Ordering::SeqCst), 50);
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|event| matches!(event, Event::PreviewMediaReady { .. }))
            .count(),
        51
    );
    assert!(!results
        .iter()
        .any(|event| matches!(event, Event::PreviewMediaFailed { .. })));
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
    assert!(!worker.downloads.exists());
}

#[tokio::test]
async fn one_uneven_multipart_preview_spreads_fifty_sockets_and_pipelines_extra_part() {
    let barrier = Arc::new(tokio::sync::Barrier::new(50));
    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = (barrier.clone(), connections.clone(), requests.clone());
    let addr = serve(move |mut socket| {
        let (barrier, connections, requests) = observed.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            let command = read_line(&mut socket).await;
            let first = command
                .strip_prefix("BODY <many.jpg-")
                .unwrap()
                .strip_suffix("@test>")
                .unwrap()
                .parse::<u32>()
                .unwrap();
            let mut parts = vec![first];
            requests.fetch_add(1, Ordering::SeqCst);
            if first == 1 {
                assert_eq!(read_line(&mut socket).await, "BODY <many.jpg-2@test>");
                requests.fetch_add(1, Ordering::SeqCst);
                parts.push(2);
            }
            connections.fetch_add(1, Ordering::SeqCst);
            tokio::time::timeout(Duration::from_secs(10), barrier.wait())
                .await
                .expect("all50sockets+FIFOextraBODYbeforeanyreply");
            // Later windows can reply first; offset validation must still yield
            // the exact final sequence rather than completion order.
            if first == 1 {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            for part in parts {
                let (body, _) = yenc_simd::encode_article(
                    &[part as u8],
                    "many.jpg",
                    part,
                    51,
                    part as u64 - 1,
                    51,
                );
                write_multiline(&mut socket, &body).await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut config = fixture_server(addr.port());
    config.max_connections = 50;
    config.pipeline_depth = 8;
    let sid = worker.catalog.insert_server(&config).unwrap();
    let id = add_media_post(&mut worker, sid, "Many parts", &[("many.jpg", 51, 51)]);
    worker.preview_media(id, 1024, 0);
    settle(&mut worker).await;
    assert_eq!(connections.load(Ordering::SeqCst), 50);
    assert_eq!(requests.load(Ordering::SeqCst), 51);
    let bytes = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::PreviewMediaReady { bytes, .. } => Some(bytes),
            Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
            _ => None,
        })
        .unwrap();
    assert_eq!(&*bytes, &(1..=51).collect::<Vec<u8>>());
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
    assert!(!worker.downloads.exists());
}
