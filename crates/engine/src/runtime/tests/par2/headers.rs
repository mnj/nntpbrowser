use super::*;

#[tokio::test]
async fn missing_cached_headers_never_complete_without_independent_file_verification() {
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let calls = hits.clone();
    let addr = serve(move |mut sock| {
        let calls = calls.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() {
                    break;
                }
                if command == "GROUP alt.test" {
                    write_all(&mut sock, b"211 1 10 10 alt.test\r\n").await;
                    continue;
                }
                assert_eq!(command, "BODY <payload1@test>");
                calls.fetch_add(1, Ordering::SeqCst);
                let (body, _) = yenc_simd::encode_article(&DATA[..4], "payload.bin", 1, 2, 0, 8);
                write_multiline(&mut sock, &body).await;
            }
        }
    })
    .await;
    let (mut worker, _) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = cached_first_header(&worker, sid);
    worker.queue_post(pid);
    let id = worker.catalog.list_jobs().unwrap()[0].id;
    let failed = drive_until_terminal(&mut worker, id).await;
    assert_eq!(failed.state, "failed");
    assert!(failed.error.unwrap().contains("incomplete or damaged"));
    assert_eq!(hits.load(Ordering::SeqCst), 1); // Bounded discovery found no uncached positions.

    // The same cached-header subset is recoverable when a complete local set
    // independently certifies the whole file, not just the queued part count.
    local_recovery(std::path::Path::new(&failed.dest_dir));
    worker.resume(id);
    let done = drive_until_terminal(&mut worker, id).await;
    assert_eq!(done.state, "done", "{done:?}");
    assert_eq!(done.bytes_done, 8);
    assert_eq!(
        std::fs::read(std::path::Path::new(&done.dest_dir).join("payload.bin")).unwrap(),
        DATA
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn written_part_count_alone_cannot_hide_byte_gaps() {
    let (mut worker, _) = fixture_worker();
    let dest = worker.downloads.join("byte-gap");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("payload.bin"), b"AB\0\0EFGH").unwrap();
    let parts = [(1, 0, 2), (2, 4, 8)].map(|(part, begin, end)| NewSegment {
        filename: "payload.bin".into(),
        part,
        message_id: format!("part{part}@test"),
        begin,
        end,
        article: None,
    });
    let id = worker
        .catalog
        .create_job(None, "gap", &dest, None, &parts)
        .unwrap();
    for part in worker.catalog.segments_for_job(id).unwrap() {
        worker
            .catalog
            .set_segment_state(part.id, "written", None)
            .unwrap();
    }
    let parts = worker.catalog.segments_for_job(id).unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    worker.finish_job(&job, &parts).await;
    assert_eq!(
        worker.catalog.job_by_id(id).unwrap().unwrap().state,
        "failed"
    );
}

#[tokio::test]
async fn no_segments_and_complete_bytes_with_missing_header_parts_do_not_complete() {
    let (mut worker, _) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
    let pid = cached_first_header(&worker, sid);
    worker.queue_post(pid);
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    let dest = std::path::Path::new(&job.dest_dir);
    std::fs::create_dir_all(dest).unwrap();
    std::fs::write(dest.join("payload.bin"), DATA).unwrap();
    let part = worker.catalog.segments_for_job(job.id).unwrap().remove(0);
    worker.catalog.set_segment_bounds(part.id, 0, 8).unwrap();
    worker
        .catalog
        .set_segment_state(part.id, "written", None)
        .unwrap();
    let parts = worker.catalog.segments_for_job(job.id).unwrap();
    worker.finish_job(&job, &parts).await;
    assert_eq!(
        worker.catalog.job_by_id(job.id).unwrap().unwrap().state,
        "failed"
    );
    local_recovery(dest);
    worker.finish_job(&job, &parts).await;
    assert_eq!(
        worker.catalog.job_by_id(job.id).unwrap().unwrap().state,
        "done"
    );

    let empty = worker
        .catalog
        .create_job(None, "empty", &worker.downloads.join("empty"), None, &[])
        .unwrap();
    let job = worker.catalog.job_by_id(empty).unwrap().unwrap();
    worker.finish_job(&job, &[]).await;
    assert_eq!(
        worker.catalog.job_by_id(empty).unwrap().unwrap().state,
        "failed"
    );
}

#[tokio::test]
async fn whole_file_restored_from_par2_is_counted_when_no_header_part_was_queued() {
    let (mut worker, _) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
    let pid = cached_first_header(&worker, sid);
    let dest = worker.downloads.join("whole-file");
    local_recovery(&dest);
    let id = worker
        .catalog
        .create_job(
            Some(pid),
            "whole",
            &dest,
            None,
            &[NewSegment {
                filename: "payload.par2".into(),
                part: 1,
                message_id: "par@test".into(),
                begin: 0,
                end: INDEX.len() as i64,
                article: None,
            }],
        )
        .unwrap();
    let part = worker.catalog.segments_for_job(id).unwrap().remove(0);
    worker
        .catalog
        .set_segment_state(part.id, "written", None)
        .unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    let parts = worker.catalog.segments_for_job(id).unwrap();
    worker.finish_job(&job, &parts).await;
    let done = worker.catalog.job_by_id(id).unwrap().unwrap();
    assert_eq!(done.state, "done", "{done:?}");
    assert_eq!(done.bytes_done, INDEX.len() as i64 + 8);
    assert_eq!(std::fs::read(dest.join("payload.bin")).unwrap(), DATA);
}
