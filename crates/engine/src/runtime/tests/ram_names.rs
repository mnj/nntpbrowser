use super::media::add_media_post;
use super::*;
use store::{FileKind, IncomingFile, IncomingPost};

async fn settle(worker: &mut Worker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !worker.preview_pending.is_empty() {
        worker.drive_previews().await;
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn gallery_handles_complete_spaced_names_and_legacy_catalog_names_without_weakening_identity()
{
    for legacy in [false, true] {
        let addr = serve(|mut socket| async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            assert_eq!(
                read_line(&mut socket).await,
                "BODY <My_holiday_photo.jpg-1@test>"
            );
            let (encoded, _) = yenc_simd::encode_article(
                b"abcdef",
                r#"C:\Photos\My holiday photo.JPG"#,
                1,
                1,
                0,
                6,
            );
            write_multiline(&mut socket, &encoded).await;
            assert!(read_line(&mut socket).await.is_empty());
        })
        .await;
        let (mut worker, mut events) = fixture_worker();
        let sid = worker
            .catalog
            .insert_server(&fixture_server(addr.port()))
            .unwrap();
        let modern_id = add_media_post(
            &mut worker,
            sid,
            "Holiday",
            &[("My holiday photo.jpg", 6, 1)],
        );
        let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
        let pid = if legacy {
            let parsed = store::parse_subject_legacy(
                r#"Holiday [01/01] - "My holiday photo.jpg" yEnc (1/1)"#,
                "Poster",
            );
            worker
                .catalog
                .upsert_posts(
                    gid,
                    &[IncomingPost {
                        group_key: parsed.group_key.clone(),
                        title: "Legacy Holiday".into(),
                        poster: "Poster".into(),
                        posted_at: 1,
                        files: vec![IncomingFile {
                            name: "photo.jpg".into(),
                            size: 6,
                            expected_segments: 1,
                            kind: FileKind::Image,
                        }],
                    }],
                )
                .unwrap();
            worker
                .catalog
                .page_posts(gid, None, 80, Some("Legacy Holiday"))
                .unwrap()[0]
                .id
        } else {
            modern_id
        };
        worker.preview_media(pid, 1024, 0);
        settle(&mut worker).await;
        let bytes = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|event| match event {
                Event::PreviewMediaReady { name, bytes, .. } => {
                    assert_eq!(name, "My holiday photo.jpg");
                    Some(bytes)
                }
                Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
                _ => None,
            })
            .unwrap();
        assert_eq!(&*bytes, b"abcdef");
        assert!(worker.catalog.list_jobs().unwrap().is_empty());
        assert!(!worker.downloads.exists());
        worker.save_preview_media(pid, 0);
        let saved = std::iter::from_fn(|| events.try_recv().ok())
            .find_map(|event| {
                if let Event::MediaSaved { path, .. } = event {
                    Some(path)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(saved.file_name().unwrap(), "My holiday photo.jpg");
        assert_eq!(std::fs::read(saved).unwrap(), b"abcdef");
        if legacy {
            let parts = worker
                .segments_for_key(
                    sid,
                    "alt.test",
                    &worker.catalog.post_by_id(pid).unwrap().unwrap().group_key,
                )
                .unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].filename, "My holiday photo.jpg");
        }
    }
}

#[tokio::test]
async fn gallery_rejects_an_unrelated_attachment_name_in_any_part() {
    let addr = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <My_holiday.jpg-1@test>");
        assert_eq!(read_line(&mut socket).await, "BODY <My_holiday.jpg-2@test>");
        for (part, name, begin, bytes) in [
            (1, "My holiday.JPG", 0, b"abc"),
            (2, "Other holiday.jpg", 3, b"def"),
        ] {
            let (encoded, _) = yenc_simd::encode_article(bytes, name, part, 2, begin, 6);
            write_multiline(&mut socket, &encoded).await;
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1; // Transcript intentionally requires a FIFO window.
    let sid = worker.catalog.insert_server(&server).unwrap();
    let pid = add_media_post(&mut worker, sid, "Wrong", &[("My holiday.jpg", 6, 2)]);
    worker.preview_media(pid, 1024, 0);
    settle(&mut worker).await;
    let results = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
    assert!(results.iter().any(|event| matches!(event, Event::PreviewMediaFailed { error, .. } if error.contains("Other holiday.jpg") && error.contains("My holiday.jpg"))));
    assert!(!results
        .iter()
        .any(|event| matches!(event, Event::PreviewMediaReady { .. })));
    assert!(!worker.downloads.exists());
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
}
