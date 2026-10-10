use super::media::add_media_post;
use super::*;
use std::sync::Arc;

async fn settle(worker: &mut Worker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        worker.drive_previews().await;
        if worker.preview_pending.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "preview did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn assert_ram_only(worker: &Worker) {
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
    assert!(!worker.downloads.exists());
    for name in ["media-cache", "article-bodies", "thumbs"] {
        assert!(
            !worker.data_dir.join(name).exists(),
            "preview created {name}"
        );
    }
}

#[tokio::test]
async fn multipart_gallery_preview_pipelines_in_ram_and_only_explicit_save_writes() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let addr = serve(move |mut sock| {
        let count = count.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            // Require both requests before sending either response: real pipelining.
            assert_eq!(read_line(&mut sock).await, "BODY <image.jpg-1@test>");
            assert_eq!(read_line(&mut sock).await, "BODY <image.jpg-2@test>");
            count.fetch_add(2, Ordering::SeqCst);
            for (part, begin, data) in [(1, 0, b"abc".as_slice()), (2, 3, b"def".as_slice())] {
                let (encoded, _) = yenc_simd::encode_article(data, "image.jpg", part, 2, begin, 6);
                write_multiline(&mut sock, &encoded).await;
            }
            assert!(read_line(&mut sock).await.is_empty());
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1; // Two parts pipeline on the one configured socket.
    let sid = worker.catalog.insert_server(&server).unwrap();
    let pid = add_media_post(
        &mut worker,
        sid,
        "Picture",
        &[("image.jpg", 6, 2), ("packed.zip", 2, 1)],
    );
    worker.preview_media(pid, 1024, 0);
    worker.preview_media(pid, 1024, 0);
    settle(&mut worker).await;
    assert_ram_only(&worker);
    let mut cached = None;
    let mut progress = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            Event::PreviewMediaReady {
                post_id,
                generation: 0,
                name,
                bytes,
            } => {
                assert_eq!(post_id, pid);
                assert_eq!(name, "image.jpg");
                assert_eq!(&*bytes, b"abcdef");
                cached = Some(bytes);
            }
            Event::PreviewMediaProgress {
                received, total, ..
            } => progress.push((received, total)),
            Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    assert_eq!(progress.last(), Some(&(6, 6)));
    let cached = cached.unwrap();
    worker.set_preview_context(1);
    worker.preview_media(pid, 1024, 1);
    settle(&mut worker).await;
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::PreviewMediaReady { generation: 1, .. }
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_ram_only(&worker);
    worker.save_preview_media(pid, 1);
    let saved = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::MediaSaved { path, .. } => Some(path),
            _ => None,
        })
        .unwrap();
    assert_eq!(std::fs::read(&saved).unwrap(), &*cached);
    let jobs = worker.catalog.list_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, "done");
    assert_eq!(Path::new(&jobs[0].dest_path), saved);
    assert_eq!(jobs[0].bytes_done, 6);
    assert!(worker
        .catalog
        .segments_for_job(jobs[0].id)
        .unwrap()
        .is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn ram_preview_rejects_false_size_gaps_overlap_and_crc_without_disk_side_effects() {
    for (case, begin, declared) in [
        ("Huge", 2, 100_000_000),
        ("Gap", 4, 6),
        ("Overlap", 1, 6),
        ("Crc", 3, 6),
    ] {
        let addr = serve(move |mut sock| async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "BODY <clip.mp4-1@test>");
            assert_eq!(read_line(&mut sock).await, "BODY <clip.mp4-2@test>");
            let (first, _) = yenc_simd::encode_article(b"abc", "clip.mp4", 1, 2, 0, declared);
            write_multiline(&mut sock, &first).await;
            let (mut second, _) =
                yenc_simd::encode_article(b"def", "clip.mp4", 2, 2, begin, declared);
            if case == "Crc" {
                // Damage encoded payload, retaining the original checksum.
                let start = second
                    .windows(2)
                    .position(|bytes| bytes == b"\r\n")
                    .unwrap()
                    + 2;
                let start = start
                    + second[start..]
                        .windows(2)
                        .position(|bytes| bytes == b"\r\n")
                        .unwrap()
                    + 2;
                second[start] ^= 1;
            }
            write_multiline(&mut sock, &second).await;
        })
        .await;
        let (mut worker, mut events) = fixture_worker();
        let sid = worker
            .catalog
            .insert_server(&fixture_server(addr.port()))
            .unwrap();
        let pid = add_media_post(&mut worker, sid, case, &[("clip.mp4", 6, 2)]);
        worker.preview_media(pid, 1024, 0);
        settle(&mut worker).await;
        assert_ram_only(&worker);
        let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert!(
            results
                .iter()
                .any(|event| matches!(event, Event::PreviewMediaFailed { .. })),
            "case {case}"
        );
        assert!(
            !results
                .iter()
                .any(|event| matches!(event, Event::PreviewMediaReady { .. })),
            "case {case}"
        );
    }
}

#[tokio::test]
async fn context_switch_aborts_inflight_preview_and_rejects_stale_requests() {
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let addr = serve(move |mut sock| {
        let count = count.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "BODY <photo.png-1@test>");
            count.fetch_add(1, Ordering::SeqCst);
            assert!(read_line(&mut sock).await.is_empty());
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = add_media_post(&mut worker, sid, "Cancelled", &[("photo.png", 6, 1)]);
    worker.preview_media(pid, 1024, 0);
    worker.drive_previews().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while hits.load(Ordering::SeqCst) == 0 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    worker.set_preview_context(1);
    worker.preview_media(pid, 1024, 0);
    tokio::task::yield_now().await;
    worker.drive_previews().await;
    assert!(worker.preview_pending.is_empty());
    assert!(worker.preview_queue.is_empty());
    assert!(!std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event, Event::PreviewMediaReady { .. })));
    assert_ram_only(&worker);
}

#[tokio::test]
async fn startup_cancels_active_legacy_media_jobs_and_preview_size_boundary_is_strict() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Legacy", &[("photo.png", 10, 1)]);
    worker.cache_media(pid, 1024);
    worker.stop_legacy_media_jobs();
    assert_eq!(worker.catalog.list_jobs().unwrap()[0].state, "cancelled");
    worker.preview_media(pid, 10, 0);
    worker.drive_previews().await;
    assert!(worker.preview_queue.is_empty());
    assert!(std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event, Event::PreviewMediaFailed { .. })));
}

#[tokio::test]
async fn ram_workers_follow_the_configured_connection_limit_and_queue_remaining_demand() {
    let hits = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = (hits.clone(), active.clone(), peak.clone(), release.clone());
    let addr = serve(move |mut sock| {
        let (hits, active, peak, release) = observed.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() {
                    break;
                }
                let index = hits.fetch_add(1, Ordering::SeqCst);
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Register before root releases waiters to avoid lost notifications.
                let waiting = release.notified();
                tokio::pin!(waiting);
                waiting.as_mut().enable();
                if index < 2 {
                    waiting.await;
                }
                let name = command
                    .strip_prefix("BODY <")
                    .unwrap()
                    .strip_suffix("-1@test>")
                    .unwrap();
                let (encoded, _) = yenc_simd::encode_article(b"abcdef", name, 1, 1, 0, 6);
                write_multiline(&mut sock, &encoded).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 2;
    server.pipeline_depth = 1;
    let sid = worker.catalog.insert_server(&server).unwrap();
    for index in 0..5 {
        let pid = add_media_post(
            &mut worker,
            sid,
            &format!("Picture{index}"),
            &[(&format!("photo{index}.png"), 6, 1)],
        );
        worker.preview_media(pid, 1024, 0);
    }
    worker.drive_previews().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while hits.load(Ordering::SeqCst) < 2 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(worker.preview_tasks.len(), 2);
    assert_eq!(worker.preview_queue.len(), 3);
    release.notify_waiters();
    settle(&mut worker).await;
    assert_eq!(hits.load(Ordering::SeqCst), 5);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event, Event::PreviewMediaReady { .. }))
            .count(),
        5
    );
    assert_ram_only(&worker);
}
