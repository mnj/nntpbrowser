use super::*;

#[tokio::test]
async fn one_worker_owns_a_data_directory_until_its_handle_is_dropped() {
    let (fixture, _events) = fixture_worker();
    let dir = fixture.data_dir.clone();
    drop(fixture);
    let first = Handle::start(&dir);
    let mut inbox = Vec::new();
    wait_event("first worker startup", &first, &mut inbox, |event| {
        matches!(event, Event::Servers(_)).then_some(())
    })
    .await;
    let second = Handle::start(&dir);
    let notice = wait_notice(&second, &mut Vec::new(), |text| {
        text.starts_with("startup:")
    })
    .await;
    assert!(notice.contains("already in use"));
    assert!(!second
        .poll()
        .iter()
        .any(|event| matches!(event, Event::Servers(_))));
    drop(second);
    drop(first);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match super::super::startup::lock_data_dir(&dir) {
            Ok(lock) => {
                drop(lock);
                break;
            }
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(err) => panic!("worker did not release its data lock: {err}"),
        }
    }
    let replacement = Handle::start(&dir);
    wait_event(
        "replacement worker startup",
        &replacement,
        &mut Vec::new(),
        |event| matches!(event, Event::Servers(_)).then_some(()),
    )
    .await;
}

#[test]
fn deleting_server_cleans_cache_before_its_identifier_can_be_reused() {
    let (mut worker, _events) = fixture_worker();
    let id = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    worker
        .headers
        .append_range(
            id,
            "alt.test",
            &[store::Record {
                article: 50,
                date_unix: 1,
                date_raw: None,
                bytes: 10,
                lines: 1,
                message_id: "one@test".into(),
                subject: "article".into(),
                from: "Poster".into(),
            }],
        )
        .unwrap();
    worker.delete_server(id);
    assert!(worker.catalog.server_by_id(id).unwrap().is_none());
    let reused = worker.catalog.insert_server(&fixture_server(120)).unwrap();
    assert_eq!(reused, id);
    assert!(worker
        .headers
        .stored_ranges(reused, "alt.test")
        .unwrap()
        .is_empty());

    // An unremovable cache must retain its catalog row rather than permit reuse.
    std::fs::remove_dir_all(worker.data_dir.join("headers").join(id.to_string())).unwrap();
    std::fs::write(
        worker.data_dir.join("headers").join(id.to_string()),
        b"blocked",
    )
    .unwrap();
    worker.delete_server(id);
    assert!(worker.catalog.server_by_id(id).unwrap().is_some());
}

#[tokio::test]
async fn missing_segments_and_failed_extraction_are_failed_jobs() {
    let (mut worker, _events) = fixture_worker();
    let dir = worker.downloads.join("missing");
    std::fs::create_dir_all(&dir).unwrap();
    let segment = NewSegment {
        filename: "missing.bin".into(),
        part: 1,
        message_id: "<missing@test>".into(),
        begin: 0,
        end: 10,
        article: None,
    };
    let id = worker
        .catalog
        .create_job(None, "missing", &dir, None, &[segment])
        .unwrap();
    let segments = worker.catalog.segments_for_job(id).unwrap();
    worker
        .catalog
        .set_segment_state(segments[0].id, "missing", None)
        .unwrap();
    let segments = worker.catalog.segments_for_job(id).unwrap();
    let job = worker
        .catalog
        .list_jobs()
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    worker.finish_job(&job, &segments).await;
    assert_eq!(worker.catalog.list_jobs().unwrap()[0].state, "failed");

    let dir = worker.downloads.join("broken-archive");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.zip"), b"not a zip").unwrap();
    let segment = NewSegment {
        filename: "broken.zip".into(),
        part: 1,
        message_id: "<zip@test>".into(),
        begin: 0,
        end: 9,
        article: None,
    };
    let id = worker
        .catalog
        .create_job(None, "archive", &dir, None, &[segment])
        .unwrap();
    let segments = worker.catalog.segments_for_job(id).unwrap();
    worker
        .catalog
        .set_segment_state(segments[0].id, "written", None)
        .unwrap();
    let segments = worker.catalog.segments_for_job(id).unwrap();
    let job = worker
        .catalog
        .list_jobs()
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    worker.finish_job(&job, &segments).await;
    let job = worker
        .catalog
        .list_jobs()
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap();
    assert_eq!(job.state, "failed");
    assert!(job.error.unwrap().contains("extract"));
}

#[tokio::test]
async fn failed_header_and_post_requests_keep_their_context() {
    let (mut worker, mut events) = fixture_worker();
    worker.sync_group(42).await;
    worker.fetch_headers(43, Some(0)).await;
    worker.load_posts(44, Some((100, 7)), "needle");
    let mut completed = Vec::new();
    let mut failed_page = None;
    while let Ok(event) = events.try_recv() {
        match event {
            Event::HeaderFetchFinished { group_id, error } => {
                completed.push((group_id, error.is_some()))
            }
            Event::PostsFailed {
                group_id,
                search,
                after,
                error,
            } => {
                failed_page = Some((group_id, search, after, error));
            }
            _ => {}
        }
    }
    assert_eq!(completed, [(42, true), (43, true)]);
    let failed_page = failed_page.unwrap();
    assert_eq!(
        (failed_page.0, failed_page.1.as_str(), failed_page.2),
        (44, "needle", Some((100, 7)))
    );
    assert!(failed_page.3.contains("not found"));
}

#[test]
fn independent_nzb_imports_get_distinct_subdirectories() {
    let (mut worker, _events) = fixture_worker();
    let base = worker.data_dir.join("custom-destination");
    let xml = r#"<nzb><file subject="Example [01/01] - &quot;file.bin&quot; yEnc (1/1)" poster="Poster" date="1"><groups><group>alt.test</group></groups><segments><segment bytes="12" number="1">one@test</segment></segments></file></nzb>"#;
    worker.import_nzb(xml, &base);
    worker.import_nzb(xml, &base);
    let jobs = worker.catalog.list_jobs().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_ne!(jobs[0].dest_dir, jobs[1].dest_dir);
    for job in jobs {
        assert_eq!(Path::new(&job.dest_dir).parent(), Some(base.as_path()));
        assert_eq!(
            worker.catalog.segments_for_job(job.id).unwrap()[0].filename,
            "file.bin"
        );
    }
}

#[test]
fn single_article_queue_rejects_damaged_header_blocks() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    let record = store::Record {
        article: 1,
        date_unix: 1,
        date_raw: None,
        bytes: 10,
        lines: 1,
        message_id: "one@test".into(),
        subject: "article".into(),
        from: "Poster".into(),
    };
    worker
        .headers
        .append_range(sid, "alt.test", &[record])
        .unwrap();
    worker
        .catalog
        .upsert_posts(
            gid,
            &[store::IncomingPost {
                group_key: store::parse_subject("article", "Poster").group_key,
                title: "article".into(),
                poster: "Poster".into(),
                posted_at: 1,
                files: Vec::new(),
            }],
        )
        .unwrap();
    let pid = worker.catalog.page_posts(gid, None, 1, None).unwrap()[0].id;
    let dir = worker
        .data_dir
        .join("headers")
        .join(sid.to_string())
        .join(store::group_slug("alt.test"));
    let osf = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "osf"))
        .unwrap();
    let mut bytes = std::fs::read(&osf).unwrap();
    bytes[42] ^= 1; // First OSF block checksum, leaving framing intact.
    std::fs::write(osf, bytes).unwrap();
    worker.queue_post(pid);
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
    assert!(std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event, Event::Notice(text) if text.contains("damaged"))));
}
