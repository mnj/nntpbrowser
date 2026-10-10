use super::*;

#[tokio::test]
async fn out_of_order_uu_articles_are_staged_then_streamed_in_part_order() {
    let addr = serve(|mut sock| async move {
        write_all(&mut sock, b"201 ready\r\n").await;
        assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
        write_all(&mut sock, b"101 caps\r\nPIPELINING\r\n.\r\n").await;
        loop {
            let command = read_line(&mut sock).await;
            if command.is_empty() {
                break;
            }
            let body = match command.as_str() {
                "BODY <uu1@test>" => {
                    tokio::time::sleep(Duration::from_millis(60)).await;
                    b"begin 644 a spaced file.bin\r\n#0V%T\r\n".as_slice()
                }
                "BODY <uu2@test>" => b"!00``\r\n`\r\nend\r\n".as_slice(),
                _ => panic!("unexpected {command}"),
            };
            write_multiline(&mut sock, body).await;
        }
    })
    .await;
    let (mut worker, _) = fixture_worker();
    let mut server = fixture_server(addr.port());
    server.max_connections = 2;
    server.pipeline_depth = 1;
    worker.catalog.insert_server(&server).unwrap();
    let dest = worker.downloads.join("multipart-uu");
    let parts = (1..=2)
        .map(|part| NewSegment {
            filename: "a spaced file.bin".into(),
            part,
            message_id: format!("uu{part}@test"),
            begin: 0,
            end: 0,
            article: None,
        })
        .collect::<Vec<_>>();
    let id = worker
        .catalog
        .create_job(None, "UU multipart", &dest, None, &parts)
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    loop {
        worker.drive_one().await;
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        if job.state == "done" {
            assert_eq!(job.bytes_done, 4);
            break;
        }
        assert_ne!(job.state, "failed", "{job:?}");
        assert!(std::time::Instant::now() < deadline, "{job:?}");
    }
    assert_eq!(
        std::fs::read(dest.join("a spaced file.bin")).unwrap(),
        b"CatA"
    );
    assert!(!dest.join(format!(".uu-parts-{id}")).exists());
}

#[tokio::test]
async fn uu_finalization_is_repeatable_and_stages_survive_until_done() {
    let (worker, _) = fixture_worker();
    let dest = worker.downloads.join("uu-recovery");
    let parts = (1..=2)
        .map(|part| NewSegment {
            filename: "file.bin".into(),
            part,
            message_id: format!("uu{part}@test"),
            begin: 0,
            end: 0,
            article: None,
        })
        .collect::<Vec<_>>();
    let id = worker
        .catalog
        .create_job(None, "UU recovery", &dest, None, &parts)
        .unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    let segments = worker.catalog.segments_for_job(id).unwrap();
    worker
        .stage_uu_segment(&job, &segments[1], 0, b"!00``\n`\nend\n")
        .unwrap();
    worker
        .stage_uu_segment(&job, &segments[0], 0, b"begin 644 file.bin\n#0V%T\n")
        .unwrap();
    for _ in 0..3 {
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        let segments = worker.catalog.segments_for_job(id).unwrap();
        worker.finish_uu_downloads(&job, &segments).await.unwrap();
        assert_eq!(worker.catalog.job_by_id(id).unwrap().unwrap().bytes_done, 4);
        assert_eq!(std::fs::read(dest.join("file.bin")).unwrap(), b"CatA");
        assert!(dest
            .join(format!(".uu-parts-{id}/{}.body", segments[0].id))
            .exists());
        assert!(dest
            .join(format!(".uu-parts-{id}/{}.body", segments[1].id))
            .exists());
        worker.cleanup_uu_stage(&job);
        assert!(dest.join(format!(".uu-parts-{id}")).exists());
    }
    worker.catalog.set_job_state(id, "done", None).unwrap();
    worker.cleanup_completed_uu_stages();
    assert!(!dest.join(format!(".uu-parts-{id}")).exists());
    assert_eq!(worker.catalog.job_by_id(id).unwrap().unwrap().bytes_done, 4);
}
