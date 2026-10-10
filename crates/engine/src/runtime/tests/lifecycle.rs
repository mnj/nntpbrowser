use super::*;

#[tokio::test]
async fn sync_queue_pause_and_bad_password() {
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let hits_body = hits.clone();
    let addr = serve(move |mut sock| {
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            let caps = read_line(&mut sock).await;
            assert!(caps.eq_ignore_ascii_case("CAPABILITIES"), "{caps}");
            write_all(
                &mut sock,
                b"101 caps\r\nVERSION 2\r\nREADER\r\nOVER\r\n.\r\n",
            )
            .await;
            let first = read_line(&mut sock).await;
            if first.starts_with("AUTHINFO") {
                write_all(&mut sock, b"481 no\r\n").await;
                return;
            }
            if first.starts_with("LIST ACTIVE") {
                write_all(&mut sock, b"215 list\r\nalt.test 2 1 y\r\n.\r\n").await;
                return;
            }
            assert_eq!(first, "GROUP alt.test");
            write_all(&mut sock, b"211 2 1 2 alt.test\r\n").await;
            let fmt = read_line(&mut sock).await;
            assert!(fmt.starts_with("LIST OVERVIEW.FMT"), "{fmt}");
            write_all(
                &mut sock,
                b"215 fields\r\nSubject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
            )
            .await;
            let xz = read_line(&mut sock).await;
            assert!(xz.starts_with("XZVER"), "{xz}");
            write_all(&mut sock, b"500 no\r\n").await;
            let feature = read_line(&mut sock).await;
            assert_eq!(feature, "XFEATURE COMPRESS GZIP");
            write_all(&mut sock, b"500 no\r\n").await;
            let over = read_line(&mut sock).await;
            assert_eq!(over, "OVER 1-2");
            write_all(
                &mut sock,
                b"224 overview\r\n\
1\tName [01/01] - \"file.bin\" yEnc (1/2)\tFoo\tTue, 10 Mar 2026 12:00:00 +0000\t<a@x>\t\t4\t1\r\n\
2\tName [01/01] - \"file.bin\" yEnc (2/2)\tFoo\tTue, 10 Mar 2026 12:00:01 +0000\t<b@x>\t\t6\t1\r\n\
.\r\n",
            )
            .await;
        }
    })
    .await;

    let body_addr = serve(move |mut sock| {
        let hits = hits_body.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            let caps = read_line(&mut sock).await;
            assert!(caps.eq_ignore_ascii_case("CAPABILITIES"), "{caps}");
            write_all(&mut sock, b"101 caps\r\nVERSION 2\r\nOVER\r\n.\r\n").await;
            loop {
                let cmd = read_line(&mut sock).await;
                if cmd.is_empty() {
                    break;
                }
                if !cmd.starts_with("BODY ") {
                    write_all(&mut sock, b"500 no\r\n").await;
                    continue;
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let first = cmd.contains("<a@x>");
                tokio::time::sleep(Duration::from_millis(350)).await;
                let payload: &[u8] = if first { b"abcd" } else { b"efghij" };
                let (encoded, _) = yenc_simd::encode_article(
                    payload,
                    "file.bin",
                    if first { 1 } else { 2 },
                    2,
                    if first { 0 } else { 4 },
                    10,
                );
                write_multiline(&mut sock, &encoded).await;
            }
        }
    })
    .await;

    let root = std::env::temp_dir().join(format!(
        "nntpbrowser-eng-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let handle = Handle::start(&root);
    let mut inbox = Vec::<Event>::new();
    let mut bad = NewServer {
        name: "bad".into(),
        host: "127.0.0.1".into(),
        port: addr.port(),
        tls: false,
        starttls: false,
        username: Some("reader".into()),
        password: Some("s3cret-pass".into()),
        priority: 1,
        max_connections: 2,
        pipeline_depth: 4,
        fill: false,
    };
    handle.send(Command::SaveServer(bad.clone()));
    let servers = wait_servers(&handle, &mut inbox).await;
    let bad_id = servers[0].id;
    handle.send(Command::TestServer(bad_id));
    let notice = wait_notice(&handle, &mut inbox, |text| {
        text.contains("connection failed")
    })
    .await;
    assert!(!notice.contains("s3cret-pass"), "{notice}");

    let bad_saved = bad.clone();
    bad.name = "main".into();
    bad.username = None;
    bad.password = None;
    bad.port = addr.port();
    handle.send(Command::SaveServer(bad));
    let servers = wait_servers(&handle, &mut inbox).await;
    let main = servers.iter().find(|server| server.name == "main").unwrap();
    handle.send(Command::Subscribe {
        server_id: main.id,
        name: "alt.test".into(),
    });
    let groups = wait_groups(&handle, &mut inbox).await;
    handle.send(Command::SyncGroup(groups[0].id));
    let notice = wait_notice(&handle, &mut inbox, |text| text.contains("synced")).await;
    assert!(notice.contains("plain"), "{notice}");
    let posts = wait_posts(&handle, &mut inbox).await;
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "Name");

    // Point downloads at the body server by replacing the saved host port.
    let main_edit = servers
        .into_iter()
        .find(|server| server.name == "main")
        .unwrap();
    let edited = NewServer {
        name: main_edit.name.clone(),
        host: "127.0.0.1".into(),
        port: body_addr.port(),
        tls: false,
        starttls: false,
        username: None,
        password: None,
        priority: 1,
        max_connections: 2,
        pipeline_depth: 4,
        fill: false,
    };
    handle.send(Command::UpdateServer {
        id: bad_id,
        server: bad_saved,
        enabled: false,
    });
    handle.send(Command::UpdateServer {
        id: main_edit.id,
        server: edited,
        enabled: true,
    });
    let _ = main_edit;
    handle.send(Command::QueuePost {
        post_id: posts[0].id,
    });
    let jobs = wait_jobs(&handle, &mut inbox, |jobs| {
        jobs.iter()
            .any(|job| job.state == "queued" || job.state == "fetching" || job.state == "paused")
    })
    .await;
    let job_id = jobs[0].id;
    handle.send(Command::Pause(job_id));
    tokio::time::sleep(Duration::from_millis(900)).await;
    let mid = hits.load(Ordering::SeqCst);
    assert!(
        mid <= 2,
        "pause exceeded the two already in-flight connections: {mid}"
    );
    handle.send(Command::Resume(job_id));
    let _ = wait_notice(&handle, &mut inbox, |text| text.contains("done")).await;
    assert!((2..=4).contains(&hits.load(Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&root);
}
