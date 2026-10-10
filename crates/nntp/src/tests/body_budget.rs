use super::*;
use crate::{BodyBudget, Pool};

async fn wait_until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_bodies_share_one_aggregate_budget_and_overflow_discards_sockets() {
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let release = gate.clone();
    let addr = serve(move |mut socket| {
        let gate = gate.clone();
        async move {
            greet(&mut socket).await;
            capabilities(&mut socket, "").await;
            assert!(read_line(&mut socket).await.starts_with("BODY "));
            write_all(&mut socket, b"222 body\r\nabcd\r\n").await;
            gate.acquire().await.unwrap().forget();
            write_all(&mut socket, b"efgh\r\n.\r\n").await;
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.max_connections = 3;
    let pool = Arc::new(Pool::new(ep));
    assert_eq!(pool.max_connections(), 3);
    let budget = BodyBudget::new(20);
    let mut tasks = tokio::task::JoinSet::new();
    for id in 0..3 {
        let pool = pool.clone();
        let budget = budget.clone();
        tasks.spawn(async move {
            pool.body_many_budgeted(
                &[ArticleRef::MessageId(format!("{id}@test"))],
                &[100],
                &budget,
            )
            .await
            .unwrap()
            .pop()
            .unwrap()
        });
    }
    wait_until(|| budget.bytes_used() == 18 && pool.active_connections() == 3).await;
    assert_eq!(budget.limit(), 20);
    release.add_permits(3);
    let mut completed = 0;
    let mut rejected = 0;
    while let Some(result) = tasks.join_next().await {
        assert!(budget.bytes_used() <= budget.limit());
        match result.unwrap() {
            Ok(bytes) => {
                assert_eq!(bytes, b"abcd\r\nefgh\r\n");
                completed += 1;
            }
            Err(Error::BodyTooLarge(20)) => rejected += 1,
            other => panic!("unexpected reply: {other:?}"),
        }
    }
    assert!(completed <= 1 && rejected >= 2);
    assert_eq!(completed + rejected, 3);
    assert_eq!(
        budget.bytes_used(),
        completed * 12,
        "only complete bodies stay charged"
    );
    assert_eq!(pool.active_connections(), 0);
    assert_eq!(
        pool.open_connections(),
        completed,
        "overflow sockets cannot re-enter the idle pool"
    );
    assert_eq!(pool.take_active_peak(), 3);
}

#[tokio::test]
async fn cancelled_partial_body_refunds_unstuffed_bytes_and_releases_its_connection() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let (connections, drops) = (attempts.clone(), closed.clone());
    let addr = serve(move |mut socket| {
        let (connections, drops) = (connections.clone(), drops.clone());
        async move {
            let attempt = connections.fetch_add(1, Ordering::SeqCst);
            greet(&mut socket).await;
            capabilities(&mut socket, "").await;
            assert!(read_line(&mut socket).await.starts_with("BODY "));
            if attempt == 0 {
                write_all(&mut socket, b"222 body\r\n..dot\r\n").await;
                let mut byte = [0];
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
                        .await
                        .unwrap()
                        .unwrap(),
                    0
                );
                drops.fetch_add(1, Ordering::SeqCst);
            } else {
                write_all(&mut socket, b"222 body\r\nok\r\n.\r\n").await;
            }
        }
    })
    .await;
    let mut ep = endpoint(1, addr, 1, false);
    ep.max_connections = 1;
    let pool = Arc::new(Pool::new(ep));
    let budget = BodyBudget::new(6);
    let (task_pool, task_budget) = (pool.clone(), budget.clone());
    let task = tokio::spawn(async move {
        task_pool
            .body_many_budgeted(
                &[ArticleRef::MessageId("cancel@test".into())],
                &[100],
                &task_budget,
            )
            .await
    });
    // Count the unstuffed leading dot and canonical CRLF, not the extra wire dot.
    wait_until(|| budget.bytes_used() == 6).await;
    assert_eq!(pool.active_connections(), 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_until(|| closed.load(Ordering::SeqCst) == 1).await;
    assert_eq!(budget.bytes_used(), 0);
    assert_eq!(pool.active_connections(), 0);
    assert_eq!(pool.open_connections(), 0);
    let replies = pool
        .body_many_budgeted(
            &[ArticleRef::MessageId("again@test".into())],
            &[100],
            &budget,
        )
        .await
        .unwrap();
    assert_eq!(replies[0].as_ref().unwrap(), b"ok\r\n");
    assert_eq!(budget.bytes_used(), 4);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn truncated_response_refunds_prefix_before_pool_retries_with_the_same_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let count = attempts.clone();
    let addr = serve(move |mut socket| {
        let count = count.clone();
        async move {
            let attempt = count.fetch_add(1, Ordering::SeqCst);
            greet(&mut socket).await;
            capabilities(&mut socket, "").await;
            assert_eq!(read_line(&mut socket).await, "BODY <retry@test>");
            if attempt == 0 {
                write_all(&mut socket, b"222 body\r\nok\r\n").await;
            } else {
                write_all(&mut socket, b"222 body\r\nok\r\n.\r\n").await;
            }
        }
    })
    .await;
    let pool = Pool::new(endpoint(1, addr, 1, false));
    let budget = BodyBudget::new(4);
    let replies = pool
        .body_many_budgeted(
            &[ArticleRef::MessageId("retry@test".into())],
            &[100],
            &budget,
        )
        .await
        .unwrap();
    assert_eq!(replies[0].as_ref().unwrap(), b"ok\r\n");
    assert_eq!(budget.bytes_used(), 4);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn budgeted_pipeline_keeps_successful_bytes_and_refunds_only_later_partial_response() {
    let addr = serve(|mut socket| async move {
        greet(&mut socket).await;
        capabilities(&mut socket, "PIPELINING\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <first@test>");
        assert_eq!(read_line(&mut socket).await, "BODY <second@test>");
        write_all(
            &mut socket,
            b"222 body\r\nabcd\r\n.\r\n222 body\r\nefgh\r\n",
        )
        .await;
    })
    .await;
    let mut connection = Connection::connect(&plain_cfg(addr)).await.unwrap();
    let budget = BodyBudget::new(12);
    let ids = [
        ArticleRef::MessageId("first@test".into()),
        ArticleRef::MessageId("second@test".into()),
    ];
    let replies = connection
        .body_pipeline_budgeted(&ids, &[100, 100], &budget)
        .await
        .unwrap();
    assert_eq!(replies[0].as_ref().unwrap(), b"abcd\r\n");
    assert!(replies[1].is_err());
    assert_eq!(budget.bytes_used(), 6);
    drop(replies);
    assert_eq!(
        budget.bytes_used(),
        6,
        "successful charges are cumulative, not tied to buffer lifetime"
    );
}
