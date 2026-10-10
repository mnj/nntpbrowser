use super::*;

#[tokio::test]
async fn header_progress_excludes_cached_spans_and_finishes_sparse_and_missing_windows() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 2 1 10 alt.test\r\n").await;
        assert_eq!(read_line(&mut sock).await, "LIST OVERVIEW.FMT");
        write_all(&mut sock, b"500 unsupported\r\n").await;
        assert_eq!(read_line(&mut sock).await, "OVER 1-3");
        write_all(&mut sock, b"423 no articles\r\n").await;
        assert_eq!(read_line(&mut sock).await, "OVER 7-10");
        write_all(&mut sock, b"224 overview\r\n7\tarticle7\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<7@test>\t\t10\t1\r\n9\tarticle9\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<9@test>\t\t10\t1\r\n.\r\n").await;
    }).await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    let cached = (4..=6)
        .map(|article| store::Record {
            article,
            date_unix: 1,
            date_raw: None,
            bytes: 10,
            lines: 1,
            message_id: format!("{article}@test"),
            subject: format!("article{article}"),
            from: "Poster".into(),
        })
        .collect::<Vec<_>>();
    worker
        .headers
        .append_range(sid, "alt.test", &cached)
        .unwrap();
    worker.ensure_pool(&worker.server(sid).unwrap()).await;
    worker
        .pools
        .get(&sid)
        .unwrap()
        .set_header_strategy(nntp::HeaderStrategy::Plain)
        .await;
    worker.fetch_headers(gid, None).await;
    let progress = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            Event::HeaderProgress {
                group_id,
                received,
                stored,
                completed,
                total,
                ..
            } => {
                assert_eq!(group_id, gid);
                assert!(completed <= total);
                Some((completed, total, received, stored))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(progress.first(), Some(&(0, 7, 0, 0)));
    assert!(progress.contains(&(3, 7, 0, 0))); // Entire first window is absent.
    assert_eq!(progress.last(), Some(&(7, 7, 2, 2))); // Sparse trailing slots still scanned.
    assert!(progress.windows(2).all(|pair| pair[0].0 <= pair[1].0));
}

#[tokio::test]
async fn header_progress_retains_partial_scanned_offset_on_network_failure() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"211 2 1 10 alt.test\r\n").await;
        assert_eq!(read_line(&mut sock).await, "LIST OVERVIEW.FMT");
        write_all(&mut sock, b"500 unsupported\r\n").await;
        assert_eq!(read_line(&mut sock).await, "OVER 1-10");
        write_all(&mut sock, b"224 overview\r\n2\tarticle2\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<2@test>\t\t10\t1\r\n").await;
        // Connection closes without the overview terminator.
    }).await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.ensure_pool(&worker.server(sid).unwrap()).await;
    worker
        .pools
        .get(&sid)
        .unwrap()
        .set_header_strategy(nntp::HeaderStrategy::Plain)
        .await;
    worker.fetch_headers(gid, None).await;
    let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    let progress = events
        .iter()
        .filter_map(|event| match event {
            Event::HeaderProgress {
                completed,
                total,
                received,
                stored,
                ..
            } => Some((*completed, *total, *received, *stored)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(progress.last(), Some(&(2, 10, 1, 1)));
    assert!(events.iter().any(|event| matches!(event, Event::HeaderFetchFinished { group_id,error:Some(_) } if *group_id==gid)));
}

#[tokio::test]
async fn latest_headers_can_expand_then_backfill_without_duplicates() {
    let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = requested.clone();
    let addr = serve(move |mut sock| {
        let seen = seen.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            loop {
                let cmd = read_line(&mut sock).await;
                if cmd.is_empty() { break; }
                if cmd == "GROUP alt.test" {
                    write_all(&mut sock, b"211 0 1 10 alt.test\r\n").await;
                } else if cmd == "LIST OVERVIEW.FMT" || cmd.starts_with("XZVER") || cmd.starts_with("XFEATURE") {
                    write_all(&mut sock, b"500 unsupported\r\n").await;
                } else if let Some(range) = cmd.strip_prefix("OVER ") {
                    seen.lock().unwrap().push(range.to_owned());
                    let (first, last) = range.split_once('-').unwrap();
                    let first: u64 = first.parse().unwrap();
                    let last: u64 = last.parse().unwrap();
                    let mut response = String::from("224 overview\r\n");
                    for n in first..=last {
                        response.push_str(&format!("{n}\tarticle {n}\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<{n}@test>\t\t10\t1\r\n"));
                    }
                    response.push_str(".\r\n");
                    write_all(&mut sock, response.as_bytes()).await;
                } else { panic!("unexpected command: {cmd}"); }
            }
        }
    }).await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.fetch_headers(gid, Some(3)).await;
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 3);
    let progress = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            Event::HeaderProgress {
                group_id,
                received,
                stored,
                completed,
                total,
                first,
                last,
            } => {
                assert_eq!(group_id, gid);
                Some((received, stored, completed, total, first, last))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(progress.first(), Some(&(0, 0, 0, 3, 8, 10)));
    assert_eq!(progress.last(), Some(&(3, 3, 3, 3, 8, 10)));
    worker.fetch_headers(gid, Some(5)).await;
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 5);
    worker.fetch_headers(gid, None).await;
    worker.fetch_headers(gid, None).await;
    assert_eq!(&*requested.lock().unwrap(), &["8-10", "6-7", "1-5"]);
    let posts = worker.catalog.page_posts(gid, None, 80, None).unwrap();
    assert_eq!(posts.len(), 10);
    assert_eq!(posts.iter().map(|post| post.total_bytes).sum::<i64>(), 100);
    assert_eq!(
        worker.headers.stored_ranges(sid, "alt.test").unwrap(),
        [(1, 5), (6, 7), (8, 10)]
    );
    worker.queue_group(gid, Some(3), "").await;
    assert_eq!(worker.catalog.list_jobs().unwrap().len(), 3);
    worker.queue_group(gid, None, "article 1").await;
    assert_eq!(worker.catalog.list_jobs().unwrap().len(), 5);
}

#[tokio::test]
async fn group_discovery_emits_start_batches_and_completion() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "LIST ACTIVE *");
        let mut response = String::from("215 active\r\njunk\r\n");
        for n in 0..1001 {
            response.push_str(&format!("alt.group{n} 20 1 y\r\n"));
        }
        response.push_str(".\r\n");
        write_all(&mut sock, response.as_bytes()).await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    worker.list_active(sid, "*").await;
    let mut samples = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::GroupSample {
            server_id,
            groups,
            received,
            done,
        } = event
        {
            assert_eq!(server_id, sid);
            samples.push((groups.len(), received, done));
        }
    }
    assert_eq!(
        samples,
        [
            (0, 0, false),
            (450, 450, false),
            (450, 900, false),
            (101, 1001, true)
        ]
    );
}
