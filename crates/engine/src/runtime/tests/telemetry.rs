use super::*;

#[tokio::test]
async fn settings_server_test_uses_pool_and_reports_activity_while_streaming() {
    const RESPONSE: &[u8] = b"215 groups\r\nalt.test 10 1 y\r\n.\r\n";
    let addr = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "LIST ACTIVE *");
        write_all(&mut socket, &RESPONSE[..RESPONSE.len() - 3]).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        write_all(&mut socket, b".\r\n").await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let id = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    worker.test_server(id).await;
    let pool = worker.pools.get(&id).unwrap();
    assert_eq!(
        pool.received_bytes(),
        (RESPONSE.len() + b"201 ready\r\n".len() + b"101 caps\r\nOVER\r\n.\r\n".len()) as u64
    );
    assert_eq!(pool.active_connections(), 0);
    let mut active_seen = false;
    let mut pending_read_seen = false;
    let mut completed_seen = false;
    while let Ok(event) = events.try_recv() {
        match event {
            Event::ConnectionStats {
                server_id,
                connections,
                ..
            } if server_id == id => {
                assert_eq!(connections.len(), 1);
                if connections[0].read_pending && connections[0].receiving {
                    pending_read_seen = true;
                }
            }
            Event::NetworkStats {
                server_id,
                active,
                bytes_per_second,
                ..
            } if server_id == id => {
                if active == 1 && bytes_per_second > 0.0 {
                    active_seen = true;
                }
                assert!(bytes_per_second.is_finite());
            }
            Event::Notice(text) if text.contains("connected, 1 group") => completed_seen = true,
            _ => {}
        }
    }
    assert!(
        active_seen,
        "sampling must run during the header/list command"
    );
    assert!(completed_seen);
    assert!(
        pending_read_seen,
        "the held response must report its live reader state"
    );
}

#[tokio::test]
async fn fast_transfers_report_recent_activity_then_idle_open_sockets() {
    let addr = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <fast@test>");
        write_all(&mut socket, b"222 0 <fast@test>\r\nhello\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <tail@test>");
        write_all(&mut socket, b"222 0 <tail@test>\r\nlast bytes\r\n.\r\n").await;
        assert!(read_line(&mut socket).await.is_empty());
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    worker.ensure_pool(&worker.server(sid).unwrap()).await;
    let pool = worker.pools[&sid].clone();
    assert_eq!(
        pool.body(&nntp::ArticleRef::MessageId("fast@test".into()))
            .await
            .unwrap(),
        b"hello\r\n"
    );
    assert_eq!(pool.active_connections(), 0);
    let first = next_stats(&mut events).await;
    assert!(
        matches!(first, Event::NetworkStats { server_id, active: 1, open: 1, bytes_per_second, .. } if server_id == sid && bytes_per_second > 0.0)
    );
    let second = next_stats(&mut events).await;
    assert!(matches!(second, Event::NetworkStats {
        active: 0, open: 1, bytes_per_second, ..
    } if bytes_per_second > 0.0));
    let previous_traffic = pool.traffic();
    assert_eq!(
        pool.body(&nntp::ArticleRef::MessageId("tail@test".into()))
            .await
            .unwrap(),
        b"last bytes\r\n"
    );
    let final_traffic = pool.traffic();
    assert!(final_traffic.received > previous_traffic.received);
    assert!(final_traffic.sent > previous_traffic.sent);
    assert!(final_traffic.decoded_received > previous_traffic.decoded_received);
    drop(pool);
    worker.pools.clear();
    let closed_traffic = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            if matches!(event, Event::NetworkTraffic { closed: true, .. }) {
                return event;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(closed_traffic, Event::NetworkTraffic { traffic, closed:true, .. } if traffic == final_traffic)
    );
    let closed_connections = events.recv().await.unwrap();
    assert!(
        matches!(closed_connections, Event::ConnectionStats { connections, .. } if connections.is_empty())
    );
    let closed = next_stats(&mut events).await;
    assert!(matches!(
        closed,
        Event::NetworkStats {
            active: 0,
            open: 0,
            bytes_per_second: 0.0,
            ..
        }
    ));
}

async fn next_stats(events: &mut UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = events.recv().await.unwrap();
            if matches!(event, Event::NetworkStats { .. }) {
                return event;
            }
            assert!(matches!(
                event,
                Event::NetworkTraffic { closed: false, .. } | Event::ConnectionStats { .. }
            ));
        }
    })
    .await
    .unwrap()
}
