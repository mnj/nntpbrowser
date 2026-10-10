use super::*;

#[tokio::test]
async fn first_selection_downloads_exactly_latest_ten_thousand_with_initial_activity() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock,b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await,"CAPABILITIES");
        write_all(&mut sock,b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await,"GROUP alt.test");
        write_all(&mut sock,b"211 50000 1 50000 alt.test\r\n").await;
        assert_eq!(read_line(&mut sock).await,"LIST OVERVIEW.FMT");
        write_all(&mut sock,b"500 unsupported\r\n").await;
        assert_eq!(read_line(&mut sock).await,"OVER 40001-50000");
        let mut response = String::from("224 overview\r\n");
        for article in 40001..=50000 {
            response.push_str(&format!("{article}\tArticle {article}\tPoster\tTue, 10 Mar 2026 12:00:00 +0000\t<{article}@test>\t\t10\t1\r\n"));
        }
        response.push_str(".\r\n");
        write_all(&mut sock,response.as_bytes()).await;
    }).await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.on_command(Command::EnsureGroupHeaders(gid)).await;
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 10_000);
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        matches!(results.first(),Some(Event::HeaderProgress{group_id,received:0,stored:0,completed:0,total:0,first:0,last:0}) if *group_id==gid)
    );
    assert_eq!(results.iter().filter(|event|matches!(event,Event::HeaderFetchFinished{group_id,error:None}if *group_id==gid)).count(),1);
    assert!(results.iter().any(|event| matches!(
        event,
        Event::HeaderProgress {
            received: 10_000,
            stored: 10_000,
            completed: 10_000,
            total: 10_000,
            ..
        }
    )));
}

#[tokio::test]
async fn cached_ranges_avoid_network_even_when_catalog_and_filtered_views_are_empty() {
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let requests = hits.clone();
    let addr = serve(move |_sock| {
        requests.fetch_add(1, Ordering::SeqCst);
        async {}
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker
        .headers
        .append_range(
            sid,
            "alt.test",
            &[store::Record {
                article: 42,
                date_unix: 1,
                date_raw: None,
                bytes: 10,
                lines: 1,
                message_id: "42@test".into(),
                subject: "Cached article".into(),
                from: "Poster".into(),
            }],
        )
        .unwrap();
    assert_eq!(worker.catalog.post_count(gid).unwrap(), 0);
    worker
        .on_command(Command::LoadPostsInDateRange {
            group_id: gid,
            after: None,
            search: "nothing".into(),
            from: Some(10),
            until: Some(20),
        })
        .await;
    while events.try_recv().is_ok() {}
    for _ in 0..2 {
        worker.on_command(Command::EnsureGroupHeaders(gid)).await;
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(worker.pools.is_empty());
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(results.is_empty());
}

#[tokio::test]
async fn invalid_cache_manifest_and_unknown_group_emit_one_scoped_error_without_refetch() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(9)).unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    assert!(worker
        .headers
        .stored_ranges(sid, "alt.test")
        .unwrap()
        .is_empty());
    let manifest = worker
        .data_dir
        .join("headers")
        .join(sid.to_string())
        .join("alt.test/manifest.json");
    std::fs::write(&manifest, b"broken manifest").unwrap();
    for group_id in [gid, gid + 100] {
        worker
            .on_command(Command::EnsureGroupHeaders(group_id))
            .await;
        let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|event|matches!(event,Event::HeaderFetchFinished{group_id:id,error:Some(_)}if *id==group_id)).count(),1);
        assert_eq!(
            results
                .iter()
                .filter(|event| matches!(event, Event::Notice(_)))
                .count(),
            1
        );
        assert!(!results
            .iter()
            .any(|event| matches!(event, Event::HeaderProgress { .. })));
    }
    assert!(worker.pools.is_empty());
    assert_eq!(std::fs::read(manifest).unwrap(), b"broken manifest");
}

#[tokio::test]
async fn automatic_network_failure_keeps_progress_and_emits_only_one_completion() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "GROUP alt.test");
        write_all(&mut sock, b"411 no such group\r\n").await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker.on_command(Command::EnsureGroupHeaders(gid)).await;
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        matches!(results.first(),Some(Event::HeaderProgress{group_id,total:0,..})if *group_id==gid)
    );
    assert_eq!(results.iter().filter(|event|matches!(event,Event::HeaderFetchFinished{group_id,error:Some(_)}if *group_id==gid)).count(),1);
    assert!(worker
        .headers
        .stored_ranges(sid, "alt.test")
        .unwrap()
        .is_empty());
}
