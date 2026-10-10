use super::*;
use tokio::io::AsyncReadExt;

fn assert_cancelled(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>, gid: i64) {
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(events.iter().filter(|event| matches!(event, Event::HeaderFetchCancelled { group_id } if *group_id == gid)).count(), 1);
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::HeaderFetchFinished { .. } | Event::Notice(_))));
}

#[tokio::test]
async fn queued_fetch_then_immediate_cancel_respects_order_before_network_dispatch() {
    for defer_during_other_work in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut worker, mut events) = fixture_worker();
        let sid = worker
            .catalog
            .insert_server(&fixture_server(listener.local_addr().unwrap().port()))
            .unwrap();
        let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
        let (sender, receiver) = crossbeam_channel::unbounded();
        worker.cmd_rx = receiver;
        sender
            .send(Command::FetchHeaders {
                group_id: gid,
                limit: None,
            })
            .unwrap();
        sender.send(Command::CancelHeaderFetch(gid)).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            if defer_during_other_work {
                // Preview/download work polls controls and defers header commands.
                // The cancel must stay behind the fetch it was sent to stop.
                worker.poll_controls();
                while let Some(command) = worker.deferred.pop_front() {
                    worker.on_command(command).await;
                }
            } else {
                while let Ok(command) = worker.cmd_rx.try_recv() {
                    worker.on_command(command).await;
                }
            }
        })
        .await
        .expect("already queued cancellation should return immediately");
        assert_cancelled(&mut events, gid);
        assert!(worker
            .headers
            .stored_ranges(sid, "alt.test")
            .unwrap()
            .is_empty());
        assert!(worker.active_header_fetch.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "cancelled queued fetch must not open a socket"
        );
    }
}

#[tokio::test]
async fn cancellation_interrupts_connect_and_group_waits_and_discards_the_socket() {
    for hold_greeting in [true, false] {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started = Arc::new(std::sync::Mutex::new(Some(started_tx)));
        let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
        let closed = Arc::new(std::sync::Mutex::new(Some(closed_tx)));
        let addr = serve(move |mut sock| {
            let started = started.clone();
            let closed = closed.clone();
            async move {
                if !hold_greeting {
                    write_all(&mut sock, b"201 ready\r\n").await;
                    assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
                    write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
                    assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
                }
                started.lock().unwrap().take().unwrap().send(()).unwrap();
                let mut byte = [0];
                assert_eq!(sock.read(&mut byte).await.unwrap(), 0);
                closed.lock().unwrap().take().unwrap().send(()).unwrap();
            }
        })
        .await;
        let (mut worker, mut events) = fixture_worker();
        let sid = worker
            .catalog
            .insert_server(&fixture_server(addr.port()))
            .unwrap();
        let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
        let (sender, receiver) = crossbeam_channel::unbounded();
        worker.cmd_rx = receiver;
        let cancel = async {
            started_rx.await.unwrap();
            // An unrelated group's cancel must not terminate this fetch.
            sender.send(Command::CancelHeaderFetch(gid + 1)).unwrap();
            tokio::time::sleep(Duration::from_millis(75)).await;
            sender.send(Command::CancelHeaderFetch(gid)).unwrap();
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(worker.fetch_headers(gid, None), cancel);
            closed_rx.await.unwrap();
        })
        .await
        .expect("cancel should not wait for the configured network timeout");
        assert_cancelled(&mut events, gid);
        assert!(worker
            .headers
            .stored_ranges(sid, "alt.test")
            .unwrap()
            .is_empty());
        assert_eq!(worker.pools[&sid].open_connections(), 0);
        assert!(worker.active_header_fetch.is_none());
    }
}

#[tokio::test]
async fn cancellation_keeps_completed_windows_discards_the_tail_and_resumes_missing_headers() {
    let round = Arc::new(AtomicUsize::new(0));
    let (blocked_tx, blocked_rx) = tokio::sync::oneshot::channel();
    let blocked = Arc::new(std::sync::Mutex::new(Some(blocked_tx)));
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let closed = Arc::new(std::sync::Mutex::new(Some(closed_tx)));
    let addr = serve(move |mut sock| {
        let round = round.fetch_add(1, Ordering::SeqCst);
        let blocked = blocked.clone();
        let closed = closed.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
            write_all(&mut sock, b"211 20010 1 20010 alt.test\r\n").await;
            assert_eq!(read_line(&mut sock).await, "LIST OVERVIEW.FMT");
            write_all(&mut sock, b"500 unsupported\r\n").await;
            if round == 0 {
                assert_eq!(read_line(&mut sock).await, "OVER 1-20000");
                write_overview(&mut sock, 1, 20000, true).await;
            }
            assert_eq!(read_line(&mut sock).await, "OVER 20001-20010");
            if round == 0 {
                write_overview(&mut sock, 20001, 20002, false).await;
                blocked.lock().unwrap().take().unwrap().send(()).unwrap();
                let mut byte = [0];
                assert_eq!(sock.read(&mut byte).await.unwrap(), 0);
                closed.lock().unwrap().take().unwrap().send(()).unwrap();
            } else {
                write_overview(&mut sock, 20001, 20010, true).await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.ensure_pool(&worker.server(sid).unwrap()).await;
    worker.pools[&sid]
        .set_header_strategy(nntp::HeaderStrategy::Plain)
        .await;
    let (sender, receiver) = crossbeam_channel::unbounded();
    worker.cmd_rx = receiver;
    let cancel = async {
        blocked_rx.await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        sender.send(Command::CancelHeaderFetch(gid)).unwrap();
        tokio::time::timeout(Duration::from_secs(1), closed_rx)
            .await
            .expect("cancelled overview socket must close promptly")
            .unwrap();
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(worker.fetch_headers(gid, None), cancel);
    })
    .await
    .unwrap();
    assert_cancelled(&mut events, gid);
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 20000);
    assert_eq!(worker.headers.watermark(sid, "alt.test").unwrap(), 20000);
    assert!(worker
        .headers
        .stored_ranges(sid, "alt.test")
        .unwrap()
        .iter()
        .all(|&(_, last)| last <= 20000));
    // This stale cancel was queued after completion; it cannot cancel a new run.
    sender.send(Command::CancelHeaderFetch(gid)).unwrap();
    sender
        .send(Command::FetchHeaders {
            group_id: gid,
            limit: None,
        })
        .unwrap();
    while let Ok(command) = worker.cmd_rx.try_recv() {
        worker.on_command(command).await;
    }
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 20010);
    assert_eq!(worker.headers.watermark(sid, "alt.test").unwrap(), 20010);
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(events.iter().any(|event| matches!(event, Event::HeaderFetchFinished { group_id, error:None } if *group_id == gid)));
    assert!(!events
        .iter()
        .any(|event| matches!(event, Event::HeaderFetchCancelled { .. })));
}

#[tokio::test]
async fn cancellation_interrupts_a_partial_binary_xzver_response() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let started = Arc::new(std::sync::Mutex::new(Some(started_tx)));
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let closed = Arc::new(std::sync::Mutex::new(Some(closed_tx)));
    let addr = serve(move |mut sock| {
        let started = started.clone();
        let closed = closed.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\nXZVER\r\n.\r\n").await;
            assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
            write_all(&mut sock, b"211 10 1 10 alt.test\r\n").await;
            assert_eq!(read_line(&mut sock).await, "LIST OVERVIEW.FMT");
            write_all(&mut sock, b"500 unsupported\r\n").await;
            assert_eq!(read_line(&mut sock).await, "XZVER 1-10");
            // A valid zlib prefix without its compressed payload or EOF.
            write_all(&mut sock, b"224 overview\r\n\x78\x01").await;
            started.lock().unwrap().take().unwrap().send(()).unwrap();
            let mut byte = [0];
            assert_eq!(sock.read(&mut byte).await.unwrap(), 0);
            closed.lock().unwrap().take().unwrap().send(()).unwrap();
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    let (sender, receiver) = crossbeam_channel::unbounded();
    worker.cmd_rx = receiver;
    let cancel = async {
        started_rx.await.unwrap();
        sender.send(Command::CancelHeaderFetch(gid)).unwrap();
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(worker.fetch_headers(gid, None), cancel);
        closed_rx.await.unwrap();
    })
    .await
    .expect("compressed overview cancellation must not wait for timeout/fallback");
    assert_cancelled(&mut events, gid);
    assert!(worker
        .headers
        .stored_ranges(sid, "alt.test")
        .unwrap()
        .is_empty());
    assert_eq!(worker.pools[&sid].open_connections(), 0);
}

async fn write_overview(sock: &mut tokio::net::TcpStream, first: u64, last: u64, finished: bool) {
    let mut response = String::from("224 overview\r\n");
    for article in first..=last {
        response.push_str(&format!("{article}\tarticle{article}\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<{article}@test>\t\t10\t1\r\n"));
    }
    if finished {
        response.push_str(".\r\n");
    }
    write_all(sock, response.as_bytes()).await;
}
