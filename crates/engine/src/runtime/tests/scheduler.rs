use super::*;
use std::sync::Arc;

#[tokio::test]
async fn configured_connections_and_pipeline_depth_are_used_with_serialized_part_writes() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let (connections, max_connections, seen, ready) = (
        active.clone(),
        peak.clone(),
        requests.clone(),
        barrier.clone(),
    );
    let addr = serve(move |mut sock| {
        let (connections, max_connections, seen, ready) = (
            connections.clone(),
            max_connections.clone(),
            seen.clone(),
            ready.clone(),
        );
        async move {
            let count = connections.fetch_add(1, Ordering::SeqCst) + 1;
            max_connections.fetch_max(count, Ordering::SeqCst);
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\nPIPELINING\r\n.\r\n").await;
            let mut parts = Vec::new();
            // No response until two BODY commands arrive on each of three
            // independent sockets. A serial or non-pipelined worker deadlocks.
            for _ in 0..2 {
                let command = read_line(&mut sock).await;
                let part = command
                    .strip_prefix("BODY <part")
                    .and_then(|text| text.strip_suffix("@test>"))
                    .unwrap()
                    .parse::<u32>()
                    .unwrap();
                parts.push(part);
                seen.fetch_add(1, Ordering::SeqCst);
            }
            tokio::time::timeout(Duration::from_secs(3), ready.wait())
                .await
                .expect("three concurrent pipeline connections");
            for part in parts {
                let data = [b'0' + part as u8; 2];
                let (encoded, _) = yenc_simd::encode_article(
                    &data,
                    "parts.bin",
                    part,
                    6,
                    (part as u64 - 1) * 2,
                    12,
                );
                write_multiline(&mut sock, &encoded).await;
            }
            connections.fetch_sub(1, Ordering::SeqCst);
        }
    })
    .await;
    let (mut worker, _events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 3;
    server.pipeline_depth = 2;
    worker.catalog.insert_server(&server).unwrap();
    let dest = worker.downloads.join("parallel");
    let parts = (1..=6)
        .map(|part| NewSegment {
            filename: "parts.bin".into(),
            part,
            message_id: format!("part{part}@test"),
            begin: (part - 1) * 2,
            end: part * 2,
            article: None,
        })
        .collect::<Vec<_>>();
    let id = worker
        .catalog
        .create_job(None, "Parallel", &dest, None, &parts)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        worker.drive_one().await;
        let job = worker
            .catalog
            .list_jobs()
            .unwrap()
            .into_iter()
            .find(|job| job.id == id)
            .unwrap();
        if job.state == "done" {
            assert_eq!(job.bytes_done, 12);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "download did not finish: {job:?}"
        );
    }
    assert_eq!(requests.load(Ordering::SeqCst), 6);
    assert_eq!(peak.load(Ordering::SeqCst), 3);
    assert_eq!(
        std::fs::read(dest.join("parts.bin")).unwrap(),
        b"112233445566"
    );
}

#[tokio::test]
async fn pause_and_resume_do_not_apply_results_from_the_previous_transfer_generation() {
    let hits = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(tokio::sync::Notify::new());
    let (count, ready) = (hits.clone(), release.clone());
    let addr = serve(move |mut sock| {
        let (count, ready) = (count.clone(), ready.clone());
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "BODY <control@test>");
            let attempt = count.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                ready.notified().await;
            }
            let data = if attempt == 0 { b"old" } else { b"new" };
            let (encoded, _) = yenc_simd::encode_article(data, "control.bin", 1, 1, 0, 3);
            write_multiline(&mut sock, &encoded).await;
        }
    })
    .await;
    let (mut worker, _events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1;
    server.pipeline_depth = 1;
    worker.catalog.insert_server(&server).unwrap();
    let dest = worker.downloads.join("control");
    let id = worker
        .catalog
        .create_job(
            None,
            "Control",
            &dest,
            None,
            &[NewSegment {
                filename: "control.bin".into(),
                part: 1,
                message_id: "control@test".into(),
                begin: 0,
                end: 3,
                article: None,
            }],
        )
        .unwrap();
    worker.drive_one().await;
    worker.pause(id);
    worker.resume(id);
    release.notify_one();
    worker.drive_one().await;
    assert!(!dest.join("control.bin").exists());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while worker.catalog.list_jobs().unwrap()[0].state != "done" {
        worker.drive_one().await;
        assert!(std::time::Instant::now() < deadline);
    }
    assert_eq!(std::fs::read(dest.join("control.bin")).unwrap(), b"new");
    assert_eq!(worker.catalog.list_jobs().unwrap()[0].bytes_done, 3);
}

#[tokio::test]
async fn cancellation_discards_already_issued_transfer_without_writing_or_restarting() {
    let release = Arc::new(tokio::sync::Notify::new());
    let ready = release.clone();
    let addr = serve(move |mut sock| {
        let ready = ready.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "BODY <cancel@test>");
            ready.notified().await;
            let (encoded, _) = yenc_simd::encode_article(b"abc", "cancel.bin", 1, 1, 0, 3);
            write_multiline(&mut sock, &encoded).await;
        }
    })
    .await;
    let (mut worker, _events) = fixture_worker();
    worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let dest = worker.downloads.join("cancel");
    let id = worker
        .catalog
        .create_job(
            None,
            "Cancel",
            &dest,
            None,
            &[NewSegment {
                filename: "cancel.bin".into(),
                part: 1,
                message_id: "cancel@test".into(),
                begin: 0,
                end: 3,
                article: None,
            }],
        )
        .unwrap();
    worker.drive_one().await;
    worker.cancel(id);
    release.notify_one();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !worker.transfers.is_empty() {
        worker.drive_one().await;
        assert!(std::time::Instant::now() < deadline);
    }
    assert!(!dest.join("cancel.bin").exists());
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    assert_eq!(job.state, "cancelled");
    assert_eq!(job.bytes_done, 0);
    assert!(!worker.drive_one().await);
}
