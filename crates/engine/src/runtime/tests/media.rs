use super::*;
use store::{FileKind, IncomingFile, IncomingPost, Record};

pub(super) fn add_media_post(
    worker: &mut Worker,
    sid: i64,
    title: &str,
    files: &[(&str, i64, u32)],
) -> i64 {
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    let mut article = worker.headers.watermark(sid, "alt.test").unwrap() + 1;
    let mut records = Vec::new();
    let mut incoming = Vec::new();
    for &(name, size, parts) in files {
        incoming.push(IncomingFile {
            name: name.into(),
            size,
            expected_segments: parts as i64,
            kind: store::classify(name),
        });
        for part in 1..=parts {
            records.push(Record {
                article,
                date_unix: 1,
                date_raw: None,
                bytes: 1,
                lines: 1,
                message_id: format!("{}-{part}@test", name.replace(' ', "_")),
                subject: format!("{title} [01/01] - \"{name}\" yEnc ({part}/{parts})"),
                from: "Poster".into(),
            });
            article += 1;
        }
    }
    let key = store::parse_subject(&records[0].subject, "Poster").group_key;
    worker
        .headers
        .append_range(sid, "alt.test", &records)
        .unwrap();
    worker
        .catalog
        .upsert_posts(
            gid,
            &[IncomingPost {
                group_key: key,
                title: title.into(),
                poster: "Poster".into(),
                posted_at: 1,
                files: incoming,
            }],
        )
        .unwrap();
    worker
        .catalog
        .page_posts(gid, None, 80, Some(title))
        .unwrap()[0]
        .id
}

#[tokio::test]
async fn multipart_media_cache_reuses_selected_body_hydrates_full_file_and_deduplicates() {
    let mut output = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        16,
        8,
        image::Rgba([20, 40, 80, 255]),
    ))
    .write_to(&mut output, image::ImageFormat::Png)
    .unwrap();
    let raw = std::sync::Arc::new(output.into_inner());
    let payload = raw.clone();
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let addr = serve(move |mut sock| {
        let payload = payload.clone();
        let count = count.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() {
                    break;
                }
                count.fetch_add(1, Ordering::SeqCst);
                let part = if command == "BODY <picture.png-1@test>" {
                    1
                } else {
                    assert_eq!(command, "BODY <picture.png-2@test>");
                    2
                };
                let midpoint = payload.len() / 2;
                let begin = if part == 1 { 0 } else { midpoint };
                let bytes = if part == 1 {
                    &payload[..midpoint]
                } else {
                    &payload[midpoint..]
                };
                let (encoded, _) = yenc_simd::encode_article(
                    bytes,
                    "picture.png",
                    part,
                    2,
                    begin as u64,
                    payload.len() as u64,
                );
                write_multiline(&mut sock, &encoded).await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = add_media_post(
        &mut worker,
        sid,
        "Picture",
        &[
            ("picture.png", raw.len() as i64, 2),
            ("packed.zip", 1, 1),
            ("repair.par2", 1, 1),
        ],
    );
    worker.read_article(pid).await;
    worker.read_article(pid).await;
    assert!(worker
        .cached_article_body(pid, "unrelated@test", 4096)
        .is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(std::iter::from_fn(|| events.try_recv().ok()).any(
        |event| matches!(event,Event::ArticleContent { post_id,binary:true,.. } if post_id==pid)
    ));
    worker.cache_media(pid, 1024);
    worker.cache_media(pid, 1024);
    let jobs = worker.catalog.list_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].bytes_total, raw.len() as i64);
    assert_eq!(
        worker.catalog.segments_for_job(jobs[0].id).unwrap().len(),
        2
    );
    for _ in 0..3 {
        assert!(worker.drive_one().await);
    }
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    assert_eq!(job.state, "done");
    assert_eq!(hits.load(Ordering::SeqCst), 2); // Selected first part was reused.
    let path = Path::new(&job.dest_dir).join("picture.png");
    assert_eq!(std::fs::read(&path).unwrap(), *raw);
    assert_eq!(image::image_dimensions(&path).unwrap(), (16, 8));
    assert!(!Path::new(&job.dest_dir).join("packed.zip").exists());
    assert!(!Path::new(&job.dest_dir).join("repair.par2").exists());
    assert!(std::iter::from_fn(||events.try_recv().ok()).any(|event|matches!(event,Event::MediaCached { post_id,path:cached } if post_id==pid && cached==path)));
    worker.catalog = Catalog::open(worker.data_dir.join("catalog.sqlite")).unwrap();
    worker.open_post(pid);
    assert!(std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event,Event::MediaCached { post_id,.. } if post_id==pid)));
    let gid = worker.catalog.post_by_id(pid).unwrap().unwrap().group_id;
    worker.load_posts(gid, None, "");
    let hydrated = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(hydrated.iter().any(
        |event| matches!(event,Event::Thumb { post_id,path } if *post_id==pid && path.is_file())
    ));
    assert!(hydrated.iter().any(|event|matches!(event,Event::MediaCached { post_id,path } if *post_id==pid && path.is_file())));
    for state in [
        "queued",
        "fetching",
        "paused",
        "failed",
        "cancelled",
        "done",
    ] {
        worker.catalog.set_job_state(job.id, state, None).unwrap();
        worker.cache_media(pid, 1024);
        assert_eq!(worker.catalog.list_jobs().unwrap().len(), 1);
    }
}

#[test]
fn automatic_media_rejects_unknown_boundary_sizes_and_bad_decoded_bounds() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    for (title, size) in [("Unknown", 0), ("Boundary", 1024), ("TooLarge", 1025)] {
        let pid = add_media_post(&mut worker, sid, title, &[("photo.png", size, 1)]);
        worker.cache_media(pid, 1024);
        assert!(worker.catalog.list_jobs().unwrap().is_empty());
        assert!(std::iter::from_fn(|| events.try_recv().ok())
            .any(|event| matches!(event,Event::MediaCacheFailed { post_id,.. } if post_id==pid)));
    }
    let pid = add_media_post(&mut worker, sid, "Video", &[("clip.mp4", 100, 2)]);
    worker.cache_media(pid, 1024);
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    assert_eq!(
        worker.catalog.files_for_post(pid).unwrap()[0].kind,
        FileKind::Video
    );
    assert_eq!(job.bytes_total, 100);
    let segment = worker.catalog.segments_for_job(job.id).unwrap().remove(0);
    let huge = crate::decode::Part {
        filename: Some("clip.mp4".into()),
        begin: 0,
        file_size: Some(100_000_000),
        data: vec![1],
        crc32: 0,
    };
    assert!(worker
        .validate_media_part(&job, &segment, &huge, 1024)
        .is_err());
    assert!(!Path::new(&job.dest_dir).join("clip.mp4").exists());
    let offset = crate::decode::Part {
        file_size: Some(1024),
        begin: 1024,
        ..huge
    };
    assert!(worker
        .validate_media_part(&job, &segment, &offset, 1024)
        .is_err());
}

#[tokio::test]
async fn automatic_media_rejects_false_decoded_size_without_preallocating() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "BODY <clip.mp4-1@test>");
        let (encoded, _) = yenc_simd::encode_article(b"a", "clip.mp4", 1, 2, 0, 100_000_000);
        write_multiline(&mut sock, &encoded).await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = add_media_post(&mut worker, sid, "FalseSize", &[("clip.mp4", 2, 2)]);
    worker.cache_media(pid, 1024);
    assert!(worker.drive_one().await);
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    assert_eq!(job.state, "failed");
    assert!(!Path::new(&job.dest_dir).join("clip.mp4").exists());
    assert!(std::iter::from_fn(|| events.try_recv().ok())
        .any(|event| matches!(event,Event::MediaCacheFailed { post_id,.. } if post_id==pid)));
}

#[tokio::test]
async fn selected_text_is_readable_cached_and_does_not_cross_reused_post_ids() {
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let addr = serve(move |mut sock| {
        let count = count.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
            let command = read_line(&mut sock).await;
            count.fetch_add(1, Ordering::SeqCst);
            let text = if command == "BODY <first.txt-1@test>" {
                b"Hello\r\nWe begin reading now.\r\n.literal\r\n".as_slice()
            } else {
                assert_eq!(command, "BODY <second.txt-1@test>");
                b"New provider article\r\n".as_slice()
            };
            write_multiline(&mut sock, text).await;
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = add_media_post(&mut worker, sid, "First", &[("first.txt", 10, 1)]);
    worker.read_article(pid).await;
    worker.read_article(pid).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(std::iter::from_fn(||events.try_recv().ok()).any(|event|matches!(event,Event::ArticleContent { post_id,text,binary:false } if post_id==pid && text=="Hello\r\nWe begin reading now.\r\n.literal\r\n")));
    worker.delete_server(sid);
    let reused = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let new_pid = add_media_post(&mut worker, reused, "Second", &[("second.txt", 10, 1)]);
    assert_eq!(new_pid, pid);
    worker.read_article(new_pid).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert!(std::iter::from_fn(||events.try_recv().ok()).any(|event|matches!(event,Event::ArticleContent { text,binary:false,.. } if text=="New provider article\r\n")));
}

#[tokio::test]
async fn manual_downloads_run_before_earlier_gallery_jobs() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await;
        assert_eq!(read_line(&mut sock).await, "BODY <manual@test>");
        let (encoded, _) = yenc_simd::encode_article(b"manual", "manual.bin", 1, 1, 0, 6);
        write_multiline(&mut sock, &encoded).await;
    })
    .await;
    let (mut worker, _events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1;
    let sid = worker.catalog.insert_server(&server).unwrap();
    let pid = add_media_post(&mut worker, sid, "Auto", &[("clip.mp4", 10, 1)]);
    worker.cache_media(pid, 1024);
    let automatic = worker.catalog.list_jobs().unwrap().remove(0).id;
    let manual = worker
        .catalog
        .create_job(
            None,
            "Manual",
            &worker.downloads.join("manual"),
            None,
            &[NewSegment {
                filename: "manual.bin".into(),
                part: 1,
                message_id: "manual@test".into(),
                begin: 0,
                end: 6,
                article: None,
            }],
        )
        .unwrap();
    assert!(worker.drive_one().await);
    assert_eq!(
        worker.catalog.segments_for_job(manual).unwrap()[0].state,
        "written"
    );
    assert_eq!(
        worker.catalog.segments_for_job(automatic).unwrap()[0].state,
        "pending"
    );
}

#[test]
fn media_completion_rejects_sparse_preallocated_file() {
    let (mut worker, _events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Gap", &[("clip.mp4", 100, 2)]);
    worker.cache_media(pid, 1024);
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    let mut parts = worker.catalog.segments_for_job(job.id).unwrap();
    parts[0].state = "written".into();
    parts[0].begin = 0;
    parts[0].end = 40;
    parts[1].state = "written".into();
    parts[1].begin = 60;
    parts[1].end = 100;
    std::fs::create_dir_all(&job.dest_dir).unwrap();
    std::fs::write(Path::new(&job.dest_dir).join("clip.mp4"), vec![0; 100]).unwrap();
    assert!(worker
        .finish_media_job(&job, &parts, 1024)
        .unwrap_err()
        .contains("gaps"));
    assert_ne!(worker.catalog.list_jobs().unwrap()[0].state, "done");
}

#[test]
fn manual_resume_retries_failed_media_segments_and_clears_cancellation() {
    let (mut worker, _events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Retry", &[("clip.mp4", 10, 2)]);
    worker.cache_media(pid, 1024);
    let job = worker.catalog.list_jobs().unwrap().remove(0);
    let parts = worker.catalog.segments_for_job(job.id).unwrap();
    worker
        .catalog
        .set_segment_state(parts[0].id, "written", None)
        .unwrap();
    worker
        .catalog
        .set_segment_state(parts[1].id, "missing", None)
        .unwrap();
    worker.cancel(job.id);
    worker.resume(job.id);
    assert!(!worker.cancelled.contains(&job.id));
    let parts = worker.catalog.segments_for_job(job.id).unwrap();
    assert_eq!(parts[0].state, "written");
    assert_eq!(parts[1].state, "pending");
    assert_eq!(worker.catalog.list_jobs().unwrap()[0].state, "queued");
}
