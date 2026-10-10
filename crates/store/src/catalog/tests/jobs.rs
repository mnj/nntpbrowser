use super::*;

#[test]
fn jobs_segments_and_thumbs() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "k",
                "Name",
                1,
                vec![file("file.rar", 10, 2, FileKind::Archive)],
            )],
        )
        .unwrap();
    let post_id = catalog.page_posts(gid, None, 1, None).unwrap()[0].id;
    let job_id = catalog
        .create_job(
            Some(post_id),
            "Name",
            Path::new("/tmp/out"),
            Some("secret"),
            &[
                NewSegment {
                    filename: "file.rar".into(),
                    part: 1,
                    message_id: "<a@x>".into(),
                    begin: 0,
                    end: 5,
                    article: Some(9),
                },
                NewSegment {
                    filename: "file.rar".into(),
                    part: 2,
                    message_id: "<b@x>".into(),
                    begin: 5,
                    end: 10,
                    article: None,
                },
            ],
        )
        .unwrap();
    let jobs = catalog.list_jobs().unwrap();
    assert_eq!(jobs[0].state, "queued");
    assert_eq!(jobs[0].dest_path, "/tmp/out/file.rar");
    assert_eq!(jobs[0].bytes_total, 10);
    assert_eq!(jobs[0].password.as_deref(), Some("secret"));
    let lookup = catalog.job_by_id(job_id).unwrap().unwrap();
    assert_eq!(lookup.id, job_id);
    assert_eq!(lookup.post_id, Some(post_id));
    assert_eq!(lookup.dest_path, jobs[0].dest_path);
    assert_eq!(lookup.password.as_deref(), Some("secret"));
    assert!(catalog.job_by_id(i64::MAX).unwrap().is_none());
    let segments = catalog.segments_for_job(job_id).unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].state, "pending");
    catalog
        .set_segment_state(segments[0].id, "written", Some(sid))
        .unwrap();
    catalog.set_job_state(job_id, "paused", None).unwrap();
    catalog.set_job_progress(job_id, 5, 10).unwrap();
    let jobs = catalog.list_jobs().unwrap();
    assert_eq!(jobs[0].state, "paused");
    assert_eq!(jobs[0].bytes_done, 5);
    let lookup = catalog.job_by_id(job_id).unwrap().unwrap();
    assert_eq!(lookup.state, "paused");
    assert_eq!(lookup.bytes_done, 5);
    let segments = catalog.segments_for_job(job_id).unwrap();
    assert_eq!(segments[0].state, "written");
    assert_eq!(segments[0].server_id, Some(sid));
    catalog
        .insert_thumb(post_id, Path::new("thumbs/1.jpg"), 256, 128)
        .unwrap();
    let thumb = catalog.thumb_for_post(post_id).unwrap().unwrap();
    assert_eq!(thumb.width, 256);
    assert_eq!(thumb.height, 128);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn unsafe_download_filenames_are_rejected_and_interrupted_jobs_resume() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let segment = |filename: &str| NewSegment {
        filename: filename.into(),
        part: 1,
        message_id: "<a@test>".into(),
        begin: 0,
        end: 10,
        article: None,
    };
    for filename in [
        "../escape",
        "a/b",
        "a\\b",
        "C:escape",
        ".",
        "..",
        "",
        "bad\0name",
        "trailing.",
    ] {
        assert!(
            catalog
                .create_job(
                    None,
                    "test",
                    Path::new("downloads"),
                    None,
                    &[segment(filename)]
                )
                .is_err(),
            "{filename:?}"
        );
    }
    assert!(catalog.list_jobs().unwrap().is_empty());
    let id = catalog
        .create_job(
            None,
            "test",
            Path::new("downloads"),
            None,
            &[segment("file.bin")],
        )
        .unwrap();
    let part = catalog.segments_for_job(id).unwrap()[0].id;
    catalog.set_segment_state(part, "fetching", None).unwrap();
    catalog.set_job_state(id, "fetching", None).unwrap();
    let paused = catalog
        .create_job(
            None,
            "paused",
            Path::new("downloads"),
            None,
            &[segment("paused.bin")],
        )
        .unwrap();
    catalog.set_job_state(paused, "paused", None).unwrap();
    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    assert_eq!(reopened.segments_for_job(id).unwrap()[0].state, "pending");
    let jobs = reopened.list_jobs().unwrap();
    assert_eq!(jobs[0].state, "queued");
    assert_eq!(jobs[1].state, "paused");
    drop(reopened);
    let _ = fs::remove_file(path);
}

#[test]
fn decoded_segment_bounds_persist_for_completion_validation() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let job = catalog
        .create_job(
            None,
            "media",
            Path::new("cache"),
            None,
            &[NewSegment {
                filename: "clip.mp4".into(),
                part: 1,
                message_id: "<clip@test>".into(),
                begin: 0,
                end: 0,
                article: None,
            }],
        )
        .unwrap();
    let id = catalog.segments_for_job(job).unwrap()[0].id;
    assert!(catalog.set_segment_bounds(id, -1, 5).is_err());
    assert!(catalog.set_segment_bounds(id, 5, 4).is_err());
    catalog.set_segment_bounds(id, 0, 10).unwrap();
    catalog.set_segment_state(id, "written", None).unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    let part = catalog.segments_for_job(job).unwrap().remove(0);
    assert_eq!(
        (part.begin, part.end, part.state.as_str()),
        (0, 10, "written")
    );
    drop(catalog);
    let _ = fs::remove_file(path);
}
