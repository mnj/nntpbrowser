use super::media::add_media_post;
use super::*;

async fn settle(worker: &mut Worker) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !worker.preview_pending.is_empty() {
        worker.drive_previews().await;
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn ordered_multipart_uu_preview_uses_fill_server_for_an_earlier_missing_part() {
    let primary = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <My_photo.jpg-1@test>");
        assert_eq!(read_line(&mut socket).await, "BODY <My_photo.jpg-2@test>");
        write_all(&mut socket, b"430 first part missing\r\n").await;
        write_multiline(&mut socket, b"#9&5F\r\n`\r\nend\r\n").await;
    })
    .await;
    let fill = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <My_photo.jpg-1@test>");
        write_multiline(&mut socket, b"begin 644 My photo.JPG\r\n#86)C\r\n").await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let mut primary_config = fixture_server(primary.port());
    primary_config.max_connections = 1; // Keep the two-request FIFO/fill transcript.
    let sid = worker.catalog.insert_server(&primary_config).unwrap();
    let mut fill_config = fixture_server(fill.port());
    fill_config.fill = true;
    worker.catalog.insert_server(&fill_config).unwrap();
    let pid = add_media_post(&mut worker, sid, "Uu Photo", &[("My photo.jpg", 6, 2)]);
    worker.preview_media(pid, 1024, 0);
    settle(&mut worker).await;
    let bytes = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::PreviewMediaReady { name, bytes, .. } => {
                assert_eq!(name, "My photo.jpg");
                Some(bytes)
            }
            Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
            _ => None,
        })
        .unwrap();
    assert_eq!(&*bytes, b"abcdef");
    assert!(!worker.downloads.exists());
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
    worker.save_preview_media(pid, 0);
    let path = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| {
            if let Event::MediaSaved { path, .. } = event {
                Some(path)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"abcdef");
}

#[tokio::test]
async fn multipart_uu_preview_rejects_wrong_names_missing_end_and_false_header_size() {
    for (name, footer, payload) in [
        ("other.jpg", "`\r\nend\r\n", "#9&5F\r\n"),
        ("photo.jpg", "", "#9&5F\r\n"),
        ("photo.jpg", "`\r\nend\r\n", "#9&5F\r\n"),
    ] {
        let addr = serve(move |mut socket| async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\n.\r\n").await;
            assert_eq!(read_line(&mut socket).await, "BODY <photo.jpg-1@test>");
            assert_eq!(read_line(&mut socket).await, "BODY <photo.jpg-2@test>");
            write_multiline(
                &mut socket,
                format!("begin 644 {name}\r\n#86)C\r\n").as_bytes(),
            )
            .await;
            write_multiline(&mut socket, format!("{payload}{footer}").as_bytes()).await;
        })
        .await;
        let (mut worker, mut events) = fixture_worker();
        let mut server = fixture_server(addr.port());
        server.max_connections = 1;
        let sid = worker.catalog.insert_server(&server).unwrap();
        // Decoder's cap is derived from user limit, not the potentially inaccurate
        // overview size. The final case exceeds cap despite a small header size.
        let pid = add_media_post(&mut worker, sid, "Bad Uu", &[("photo.jpg", 4, 2)]);
        worker.preview_media(
            pid,
            if name == "photo.jpg" && !footer.is_empty() {
                5
            } else {
                1024
            },
            0,
        );
        settle(&mut worker).await;
        let events = std::iter::from_fn(|| events.try_recv().ok()).collect::<Vec<_>>();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::PreviewMediaFailed { .. })));
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::PreviewMediaReady { .. })));
        assert!(!worker.downloads.exists());
    }
}

#[tokio::test]
async fn begin_base64_gallery_attachment_decodes_without_disk_side_effects() {
    let addr = serve(|mut socket| async move {
        write_all(&mut socket, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
        write_all(&mut socket, b"101 caps\r\n.\r\n").await;
        assert_eq!(read_line(&mut socket).await, "BODY <photo.png-1@test>");
        write_multiline(
            &mut socket,
            b"begin-base64 644 photo.PNG\r\nYWJjZGVm\r\n====\r\n",
        )
        .await;
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = add_media_post(&mut worker, sid, "Base64 photo", &[("photo.png", 6, 1)]);
    worker.preview_media(pid, 1024, 0);
    settle(&mut worker).await;
    let bytes = std::iter::from_fn(|| events.try_recv().ok())
        .find_map(|event| match event {
            Event::PreviewMediaReady { bytes, .. } => Some(bytes),
            Event::PreviewMediaFailed { error, .. } => panic!("{error}"),
            _ => None,
        })
        .unwrap();
    assert_eq!(&*bytes, b"abcdef");
    assert!(!worker.downloads.exists());
    assert!(worker.catalog.list_jobs().unwrap().is_empty());
}
