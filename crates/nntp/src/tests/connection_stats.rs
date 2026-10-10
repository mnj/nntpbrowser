use super::*;
use crate::{ConnectionSnapshot, Pool, TrafficSnapshot};

const CAPS: &[u8] = b"101 caps\r\nREADER\r\nOVER\r\n.\r\n";
const GREETING: &[u8] = b"201 ready\r\n";
const PREFIX: &[u8] = b"222 body\r\npart\r\n";

fn sum(connections: &[ConnectionSnapshot]) -> TrafficSnapshot {
    connections
        .iter()
        .fold(TrafficSnapshot::default(), |mut total, connection| {
            total.received += connection.traffic.received;
            total.sent += connection.traffic.sent;
            total.decoded_received += connection.traffic.decoded_received;
            total
        })
}

async fn wait_for(
    pool: &Pool,
    condition: impl Fn(&[ConnectionSnapshot]) -> bool,
) -> Vec<ConnectionSnapshot> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let connections = pool.connection_stats();
            if condition(&connections) {
                return connections;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("connection snapshot condition")
}

#[tokio::test]
async fn live_connections_include_setup_waits_individual_traffic_idle_reuse_and_cancellation() {
    let setup = Arc::new(tokio::sync::Semaphore::new(0));
    let finish = Arc::new(tokio::sync::Semaphore::new(0));
    let (setup_gate, finish_gate) = (setup.clone(), finish.clone());
    let addr = serve(move |mut sock| {
        let (setup, finish) = (setup.clone(), finish.clone());
        async move {
            setup.acquire().await.unwrap().forget();
            write_all(&mut sock, GREETING).await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, CAPS).await;
            loop {
                // Read directly here: cancellation may close the idle or active
                // socket instead of sending another command.
                let mut command = Vec::new();
                let mut byte = [0; 1];
                loop {
                    if sock.read(&mut byte).await.unwrap() == 0 {
                        return;
                    }
                    command.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                assert!(command.starts_with(b"BODY "));
                write_all(&mut sock, PREFIX).await;
                if !command.starts_with(b"BODY <two@test>") {
                    assert_eq!(
                        sock.read(&mut byte).await.unwrap(),
                        0,
                        "cancelled socket closes"
                    );
                    return;
                }
                tokio::select! {
                    permit = finish.acquire() => {
                        permit.unwrap().forget();
                        write_all(&mut sock, b".\r\n").await;
                    }
                    read = sock.read(&mut byte) => {
                        assert_eq!(read.unwrap(), 0, "cancelled socket closes");
                        return;
                    }
                }
            }
        }
    })
    .await;
    let pool = Arc::new(Pool::new(endpoint(1, addr, 0, false)));
    let totals = pool.traffic_counter();
    let mut tasks = Vec::new();
    for id in ["one@test", "two@test"] {
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            pool.body(&ArticleRef::MessageId(id.into())).await
        }));
    }
    let connecting = wait_for(&pool, |connections| {
        connections.len() == 2 && connections.iter().all(|connection| connection.read_pending)
    })
    .await;
    assert_eq!(
        connecting
            .iter()
            .map(|connection| connection.id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(connecting
        .iter()
        .all(|connection| !connection.authenticated && !connection.write_pending));
    assert_eq!(sum(&connecting), TrafficSnapshot::default());
    assert_eq!(
        pool.open_connections(),
        0,
        "connecting sockets aren't authenticated yet"
    );
    assert_eq!(pool.active_connections(), 2);
    setup_gate.add_permits(2);

    let received = (GREETING.len() + CAPS.len() + PREFIX.len()) as u64;
    let blocked = wait_for(&pool, |connections| {
        connections.len() == 2
            && connections.iter().all(|connection| {
                connection.authenticated
                    && connection.read_pending
                    && connection.traffic.decoded_received == received
            })
    })
    .await;
    assert!(blocked.iter().all(|connection| !connection.write_pending));
    assert_eq!(pool.open_connections(), 2);
    assert_eq!(
        sum(&blocked),
        pool.traffic(),
        "each live byte reaches the aggregate exactly once"
    );
    for connection in &blocked {
        assert_eq!(connection.traffic.received, received);
        assert_eq!(
            connection.traffic.sent,
            (b"CAPABILITIES\r\n".len() + b"BODY <one@test>\r\n".len()) as u64
        );
    }

    let cancelled = tasks.remove(0);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    let remaining = wait_for(&pool, |connections| connections.len() == 1).await;
    let remaining_id = remaining[0].id;
    assert_eq!(pool.open_connections(), 1);
    assert_eq!(pool.active_connections(), 1);
    assert_eq!(
        pool.traffic(),
        sum(&blocked),
        "removed sockets retain their cumulative traffic"
    );
    finish_gate.add_permits(1);
    assert_eq!(tasks.pop().unwrap().await.unwrap().unwrap(), b"part\r\n");
    let idle = pool.connection_stats();
    assert_eq!(idle.len(), 1);
    assert_eq!(idle[0].id, remaining_id);
    assert!(idle[0].authenticated && !idle[0].read_pending && !idle[0].write_pending);
    assert_eq!(idle[0].traffic.received, received + 3);

    let request = pool.clone();
    let task = tokio::spawn(async move {
        request
            .body(&ArticleRef::MessageId("next@test".into()))
            .await
    });
    let reused = wait_for(&pool, |connections| {
        connections.len() == 1
            && connections[0].read_pending
            && connections[0].traffic.received == received + 3 + PREFIX.len() as u64
    })
    .await;
    assert_eq!(
        reused[0].id, remaining_id,
        "reusing an idle socket keeps its identity"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(pool.connection_stats().is_empty());
    assert_eq!(pool.open_connections(), 0);
    assert_eq!(pool.active_connections(), 0);
    let final_totals = pool.traffic();
    drop(pool);
    assert_eq!(totals.snapshot(), final_totals);
}

#[tokio::test]
async fn cancelling_connection_setup_removes_its_snapshot_and_next_attempt_gets_a_new_id() {
    let addr = serve(|mut sock| async move {
        let mut byte = [0; 1];
        assert_eq!(sock.read(&mut byte).await.unwrap(), 0);
    })
    .await;
    let pool = Arc::new(Pool::new(endpoint(1, addr, 0, false)));
    for expected_id in [1, 2] {
        let request = pool.clone();
        let task = tokio::spawn(async move {
            request
                .body(&ArticleRef::MessageId("blocked@test".into()))
                .await
        });
        let pending = wait_for(&pool, |connections| {
            connections.len() == 1 && connections[0].read_pending
        })
        .await;
        assert_eq!(pending[0].id, expected_id);
        assert!(!pending[0].authenticated);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(pool.connection_stats().is_empty());
        assert_eq!(pool.open_connections(), 0);
        assert_eq!(pool.active_connections(), 0);
    }
}
