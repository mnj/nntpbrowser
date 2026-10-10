use super::media::add_media_post;
use super::*;
use std::sync::Arc;

async fn settle(worker: &mut Worker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !worker.preview_pending.is_empty() {
        worker.drive_previews().await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "preview stayed pending"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_count(counter: &AtomicUsize, count: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while counter.load(Ordering::SeqCst) < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "peer never reached {count}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn viewport_change_aborts_offscreen_backlog_frees_connection_and_keeps_cached_save() {
    let old_started = Arc::new(AtomicUsize::new(0));
    let old_closed = Arc::new(AtomicUsize::new(0));
    let new_started = Arc::new(AtomicUsize::new(0));
    let (old, closed, visible) = (old_started.clone(), old_closed.clone(), new_started.clone());
    let addr = serve(move |mut socket| {
        let (old, closed, visible) = (old.clone(), closed.clone(), visible.clone());
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut socket).await;
                if command.is_empty() {
                    break;
                }
                if command == "BODY <old0.png-1@test>" {
                    old.fetch_add(1, Ordering::SeqCst);
                    // This socket must be closed on cancellation, not reused.
                    assert!(read_line(&mut socket).await.is_empty());
                    closed.fetch_add(1, Ordering::SeqCst);
                    break;
                }
                let filename = if command == "BODY <cached.png-1@test>" {
                    "cached.png"
                } else {
                    assert_eq!(command, "BODY <visible.png-1@test>");
                    visible.fetch_add(1, Ordering::SeqCst);
                    "visible.png"
                };
                let (encoded, _) = yenc_simd::encode_article(b"abcdef", filename, 1, 1, 0, 6);
                write_multiline(&mut socket, &encoded).await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1;
    let sid = worker.catalog.insert_server(&server).unwrap();
    let cached = add_media_post(&mut worker, sid, "Cached", &[("cached.png", 6, 1)]);
    worker.preview_media(cached, 1024, 0);
    settle(&mut worker).await;
    let cached_bytes = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::PreviewMediaReady { post_id, bytes, .. } if post_id == cached => Some(bytes),
            _ => None,
        })
        .unwrap();
    let mut offscreen = Vec::new();
    for index in 0..6 {
        let id = add_media_post(
            &mut worker,
            sid,
            &format!("Old{index}"),
            &[(&format!("old{index}.png"), 6, 1)],
        );
        offscreen.push(id);
        worker.preview_media(id, 1024, 0);
    }
    worker.drive_previews().await;
    wait_count(&old_started, 1).await;
    assert_eq!(worker.preview_active.len(), 1);
    assert_eq!(worker.preview_queue.len(), 5);
    let visible = add_media_post(&mut worker, sid, "Visible", &[("visible.png", 6, 1)]);
    worker
        .on_command(Command::RetainPreviewMedia {
            post_ids: vec![visible],
            generation: 0,
        })
        .await;
    assert!(worker.preview_pending.is_empty());
    assert!(worker.preview_queue.is_empty());
    worker.preview_media(visible, 1024, 0);
    settle(&mut worker).await;
    wait_count(&old_closed, 1).await;
    assert_eq!(new_started.load(Ordering::SeqCst), 1);
    let mut ready = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            Event::PreviewMediaReady { post_id, bytes, .. } => {
                ready.push(post_id);
                assert_eq!(&*bytes, b"abcdef");
            }
            Event::PreviewMediaFailed { error, .. } => {
                panic!("visibility cancellation emitted failure: {error}")
            }
            _ => {}
        }
    }
    assert_eq!(ready, [visible]);
    assert!(offscreen
        .iter()
        .all(|id| !worker.preview_pending.contains(&(*id, 0))));
    assert!(!worker.downloads.exists());
    worker.save_preview_media(cached, 0);
    let saved = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::MediaSaved { post_id, path } if post_id == cached => Some(path),
            _ => None,
        })
        .unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), &*cached_bytes);
}

#[tokio::test]
async fn completed_cancelled_task_cannot_clear_a_replacement_pending_for_the_same_post() {
    let hits = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let (count, go) = (hits.clone(), release.clone());
    let addr = serve(move |mut socket| {
        let (count, go) = (count.clone(), go.clone());
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut socket).await;
                if command.is_empty() {
                    break;
                }
                assert_eq!(command, "BODY <photo.png-1@test>");
                let waiting = go.notified();
                tokio::pin!(waiting);
                waiting.as_mut().enable();
                if count.fetch_add(1, Ordering::SeqCst) > 0 {
                    waiting.await;
                }
                let (encoded, _) = yenc_simd::encode_article(b"abcdef", "photo.png", 1, 1, 0, 6);
                write_multiline(&mut socket, &encoded).await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let id = add_media_post(&mut worker, sid, "Photo", &[("photo.png", 6, 1)]);
    worker.preview_media(id, 1024, 0);
    worker.drive_previews().await;
    wait_count(&hits, 1).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while worker.pools[&sid].active_connections() > 0 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // The first task completed but its result has not yet been collected.
    worker.retain_preview_media(&[], 0);
    worker.retain_preview_media(&[id], 0);
    worker.preview_media(id, 1024, 0);
    worker.drive_previews().await;
    wait_count(&hits, 2).await;
    assert!(worker.preview_pending.contains(&(id, 0)));
    assert_eq!(worker.preview_active.len(), 1);
    assert!(
        !std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(
            event,
            Event::PreviewMediaReady { .. } | Event::PreviewMediaFailed { .. }
        ))
    );
    release.notify_waiters();
    settle(&mut worker).await;
    assert_eq!(
        std::iter::from_fn(|| events.try_recv().ok())
            .filter(|event| matches!(event,Event::PreviewMediaReady { post_id,.. } if *post_id==id))
            .count(),
        1
    );
}

#[test]
fn viewport_controls_are_applied_at_header_batch_poll_boundaries() {
    let (mut worker, _events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
    let first = add_media_post(&mut worker, sid, "First", &[("first.png", 6, 1)]);
    let second = add_media_post(&mut worker, sid, "Second", &[("second.png", 6, 1)]);
    worker.preview_media(first, 1024, 0);
    worker.preview_media(second, 1024, 0);
    let (commands, rx) = crossbeam_channel::unbounded();
    worker.cmd_rx = rx;
    commands
        .send(Command::RetainPreviewMedia {
            post_ids: vec![second],
            generation: 0,
        })
        .unwrap();
    worker.poll_controls();
    assert_eq!(worker.preview_pending, HashSet::from([(second, 0)]));
    assert_eq!(worker.preview_queue.len(), 1);
    assert!(worker.deferred.is_empty());
    commands.send(Command::PreviewContext(1)).unwrap();
    worker.poll_controls();
    assert_eq!(worker.preview_context, 1);
    assert!(worker.preview_pending.is_empty());
    assert!(worker.preview_queue.is_empty());
    assert!(worker.deferred.is_empty());
}

#[tokio::test]
async fn queued_intents_skip_obsolete_headers_and_reject_deferred_offscreen_requests() {
    let (mut worker, mut events) = fixture_worker();
    let (commands, rx) = crossbeam_channel::unbounded();
    worker.cmd_rx = rx;
    // Invalid IDs make any attempted preparation observable as a failure. Both
    // intents are cheap to enqueue; the first is removed before any lookup.
    for command in [
        Command::RetainPreviewMedia {
            post_ids: vec![101],
            generation: 0,
        },
        Command::PreviewMedia {
            post_id: 101,
            max_bytes: 1024,
            generation: 0,
        },
        Command::RetainPreviewMedia {
            post_ids: vec![102],
            generation: 0,
        },
        Command::PreviewMedia {
            post_id: 102,
            max_bytes: 1024,
            generation: 0,
        },
    ] {
        commands.send(command).unwrap();
    }
    worker.poll_controls();
    // A previously deferred command arriving after a viewport change is rejected.
    worker.preview_media(101, 1024, 0);
    assert_eq!(worker.preview_pending, HashSet::from([(102, 0)]));
    assert_eq!(worker.preview_queue.len(), 1);
    assert!(worker.deferred.is_empty());
    assert!(events.try_recv().is_err());
    worker.drive_previews().await;
    let failures = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| {
            if let Event::PreviewMediaFailed { post_id, .. } = event {
                Some(post_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(failures, vec![102]);
    // A new context supports callers which do not use explicit viewport retention.
    worker.set_preview_context(1);
    worker.preview_media(101, 1024, 1);
    assert_eq!(worker.preview_pending, HashSet::from([(101, 1)]));
}

#[tokio::test]
async fn visible_preview_starts_between_header_windows_using_the_same_pool() {
    let body_started = Arc::new(AtomicUsize::new(0));
    let body = body_started.clone();
    let addr = serve(move |mut socket| {
        let body = body.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\nOVER\r\n.\r\n").await;
            loop {
                let command = read_line(&mut socket).await;
                match command.as_str() {
                    "" => break,
                    "GROUP alt.test" => write_all(&mut socket, b"211 2 1 20003 alt.test\r\n").await,
                    "LIST OVERVIEW.FMT" => write_all(&mut socket, b"500 unsupported\r\n").await,
                    "OVER 2-20001" => write_all(&mut socket, b"423 absent\r\n").await,
                    "OVER 20002-20003" => {
                        // Sync cannot finish unless dispatch at the preceding window
                        // boundary gave the viewport a slot in the shared pool.
                        wait_count(&body, 1).await;
                        write_all(&mut socket, b"423 absent\r\n").await;
                    }
                    "BODY <visible.png-1@test>" => {
                        body.fetch_add(1, Ordering::SeqCst);
                        let (encoded, _) =
                            yenc_simd::encode_article(b"abcdef", "visible.png", 1, 1, 0, 6);
                        write_multiline(&mut socket, &encoded).await;
                    }
                    other => panic!("unexpected command: {other}"),
                }
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 2;
    let sid = worker.catalog.insert_server(&server).unwrap();
    let id = add_media_post(&mut worker, sid, "Visible", &[("visible.png", 6, 1)]);
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.ensure_pool(&worker.server(sid).unwrap()).await;
    let original_pool = worker.pools[&sid].clone();
    original_pool
        .set_header_strategy(nntp::HeaderStrategy::Plain)
        .await;
    let (commands, rx) = crossbeam_channel::unbounded();
    worker.cmd_rx = rx;
    commands
        .send(Command::RetainPreviewMedia {
            post_ids: vec![id],
            generation: 0,
        })
        .unwrap();
    commands
        .send(Command::PreviewMedia {
            post_id: id,
            max_bytes: 1024,
            generation: 0,
        })
        .unwrap();
    worker.fetch_headers(gid, None).await;
    assert_eq!(body_started.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&original_pool, &worker.pools[&sid]));
    settle(&mut worker).await;
    assert!(std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event, Event::PreviewMediaReady { post_id, .. } if post_id == id)));
}
