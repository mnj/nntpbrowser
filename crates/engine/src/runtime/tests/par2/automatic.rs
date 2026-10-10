use super::*;
use std::sync::{Arc, Mutex};

async fn recovery_peer(
    missing_payload: bool,
    headers: bool,
    direct_part: bool,
    missing_volume: u8,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let logged = calls.clone();
    let addr = serve(move |mut sock| {
        let calls = logged.clone();
        async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() { break; }
                calls.lock().unwrap().push(command.clone());
                match command.as_str() {
                    "CAPABILITIES" => write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await,
                    "GROUP alt.test" => write_all(&mut sock, b"211 4 10 13 alt.test\r\n").await,
                    "LIST OVERVIEW.FMT" => write_all(&mut sock, b"500 unsupported\r\n").await,
                    command if command.starts_with("OVER ") => {
                        assert!(headers);
                        let files = if direct_part { vec![(11, "payload.bin", "payload2@test", "2/2")] } else {
                            vec![(11, "payload.par2", "index@test", "1/1"), (12, "payload.vol0+1.par2", "volume@test", "1/1"), (13, "payload.vol0+2.par2", "fallback@test", "1/1")]
                        };
                        let mut response = "224 overview\r\n".to_owned();
                        for (article, name, id, counter) in files {
                            response.push_str(&format!("{article}\tSet [01/01] - \"{name}\" yEnc ({counter})\tPoster\tThu, 01 Jan 1970 00:00:01 +0000\t<{id}>\t\t400\t1\r\n"));
                        }
                        response.push_str(".\r\n"); write_all(&mut sock,response.as_bytes()).await;
                    }
                    command if command.starts_with("BODY ") => {
                        let (name, data, part, total, begin, size) = match command {
                            "BODY <payload1@test>" => ("payload.bin", if headers { &DATA[..4] } else { DATA }, 1, if headers {2}else{1}, 0, 8),
                            "BODY <payload2@test>" => ("payload.bin", &DATA[4..], 2, 2, 4, 8),
                            "BODY <index@test>" => ("payload.par2", INDEX, 1, 1, 0, INDEX.len() as u64),
                            "BODY <volume@test>" => {
                                if missing_volume == 1 { write_all(&mut sock,b"430 missing\r\n").await; continue; }
                                ("payload.vol0+1.par2", match missing_volume {2=> include_bytes!("../fixtures/par2/wrong-set.par2").as_slice(),3=>include_bytes!("../fixtures/par2/bad-recovery.par2").as_slice(),_=>VOLUME}, 1, 1, 0, VOLUME.len() as u64)
                            },
                            "BODY <fallback@test>" => ("payload.vol0+2.par2", include_bytes!("../fixtures/par2/payload.vol0+2.par2").as_slice(), 1, 1, 0, 476),
                            _ => panic!("unexpected {command}"),
                        };
                        if name == "payload.bin" && missing_payload { write_all(&mut sock,b"430 missing\r\n").await; continue; }
                        let (body,_) = yenc_simd::encode_article(data,name,part,total,begin,size);
                        write_multiline(&mut sock,&body).await;
                    }
                    _ => panic!("unexpected {command}"),
                }
            }
        }
    }).await;
    (addr, calls)
}

#[tokio::test]
async fn cached_missing_headers_discover_index_and_minimum_recovery_volume_without_global_cache_changes(
) {
    let (addr, calls) = recovery_peer(false, true, false, 0).await;
    let (mut worker, _) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 1;
    let sid = worker.catalog.insert_server(&server).unwrap();
    let post = cached_first_header(&worker, sid);
    worker.queue_post(post);
    let id = worker.catalog.list_jobs().unwrap()[0].id;
    let done = drive_until_terminal(&mut worker, id).await;
    assert_eq!(done.state, "done", "{done:?}");
    assert_eq!(
        std::fs::read(std::path::Path::new(&done.dest_dir).join("payload.bin")).unwrap(),
        DATA
    );
    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|command| command.starts_with("OVER ")));
    assert!(calls.contains(&"BODY <index@test>".to_owned()));
    assert!(calls.contains(&"BODY <volume@test>".to_owned()));
    assert!(!calls.contains(&"BODY <fallback@test>".to_owned()));
    assert_eq!(
        worker.headers.stored_ranges(sid, "alt.test").unwrap(),
        vec![(10, 10)]
    );
    assert!(worker
        .catalog
        .recovery_context(id)
        .unwrap()
        .unwrap()
        .set_id
        .is_some());
}

#[tokio::test]
async fn discovered_missing_payload_header_is_downloaded_before_parity() {
    let (addr, calls) = recovery_peer(false, true, true, 0).await;
    let (mut worker, _) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    worker.queue_post(cached_first_header(&worker, sid));
    let id = worker.catalog.list_jobs().unwrap()[0].id;
    let done = drive_until_terminal(&mut worker, id).await;
    assert_eq!(done.state, "done", "{done:?}");
    let parts = worker.catalog.segments_for_job(id).unwrap();
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().all(|part| part.state == "written"));
    assert!(calls
        .lock()
        .unwrap()
        .contains(&"BODY <payload2@test>".into()));
}

#[tokio::test]
async fn nzb_downloads_index_but_defers_volumes_until_payload_needs_repair_and_falls_back() {
    for (missing, fail_first) in [(false, 0), (true, 0), (true, 1), (true, 2), (true, 3)] {
        let (addr, calls) = recovery_peer(missing, false, false, fail_first).await;
        let (mut worker, _) = fixture_worker();
        worker
            .catalog
            .insert_server(&fixture_server(addr.port()))
            .unwrap();
        let files = [
            ("payload.bin", "payload1@test"),
            ("payload.par2", "index@test"),
            ("payload.vol0+1.par2", "volume@test"),
            ("payload.vol0+2.par2", "fallback@test"),
        ];
        let xml=format!("<nzb>{}</nzb>",files.iter().map(|(name,id)|format!("<file poster=\"Poster\" date=\"1\" subject=\"Set [01/01] - &quot;{name}&quot; yEnc (1/1)\"><groups><group>alt.test</group></groups><segments><segment bytes=\"400\" number=\"1\">{id}</segment></segments></file>")).collect::<String>());
        let dest = worker.downloads.clone();
        worker.import_nzb(&xml, &dest);
        let id = worker.catalog.list_jobs().unwrap()[0].id;
        let done = drive_until_terminal(&mut worker, id).await;
        assert_eq!(done.state, "done", "{done:?}");
        assert_eq!(
            std::fs::read(std::path::Path::new(&done.dest_dir).join("payload.bin")).unwrap(),
            DATA
        );
        if fail_first == 3 {
            assert!(std::path::Path::new(&done.dest_dir)
                .join(".recovery-retired/payload.vol0+1.par2")
                .is_file());
        }
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&"BODY <index@test>".into()));
        assert_eq!(calls.contains(&"BODY <volume@test>".into()), missing);
        assert_eq!(
            calls.contains(&"BODY <fallback@test>".into()),
            fail_first != 0
        );
        assert!(!calls.iter().any(|command| command.starts_with("OVER ")));
    }
}

#[tokio::test]
async fn explicitly_requested_par2_index_completes_without_downloading_its_protected_payload() {
    let (addr, calls) = recovery_peer(false, false, false, 0).await;
    let (mut worker, _) = fixture_worker();
    worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let id = worker
        .catalog
        .create_job(
            None,
            "index",
            &worker.downloads.join("index-only"),
            None,
            &[NewSegment {
                filename: "payload.par2".into(),
                part: 1,
                message_id: "index@test".into(),
                begin: 0,
                end: 0,
                article: None,
            }],
        )
        .unwrap();
    let done = drive_until_terminal(&mut worker, id).await;
    assert_eq!(done.state, "done", "{done:?}");
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|command| command.starts_with("BODY "))
            .count(),
        1
    );
    assert!(!std::path::Path::new(&done.dest_dir)
        .join("payload.bin")
        .exists());
}
