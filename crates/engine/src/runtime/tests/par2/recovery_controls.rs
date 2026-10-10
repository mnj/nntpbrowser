use super::*;
use crate::runtime::recovery::RecoveryStep;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Clone, Copy, Debug)]
enum HeldCommand {
    Group,
    Overview,
}

#[tokio::test]
async fn interrupted_recovery_search_discards_its_socket_and_retries_the_uncommitted_range() {
    for held_command in [HeldCommand::Group, HeldCommand::Overview] {
        for cancel in [false, true] {
            let first = Arc::new(AtomicBool::new(true));
            let (held_tx, mut held_rx) = tokio::sync::mpsc::unbounded_channel();
            let (closed_tx, mut closed_rx) = tokio::sync::mpsc::unbounded_channel();
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = requests.clone();
            let addr = serve(move |mut sock| {
                let first = first.clone();
                let held_tx = held_tx.clone();
                let closed_tx = closed_tx.clone();
                let observed = observed.clone();
                async move {
                    write_all(&mut sock, b"201 ready\r\n").await;
                    loop {
                        let command = read_line(&mut sock).await;
                        if command.is_empty() { return; }
                        if command == "GROUP alt.test" || command.starts_with("OVER ") {
                            observed.lock().unwrap().push(command.clone());
                        }
                        let should_hold = match held_command {
                            HeldCommand::Group => command == "GROUP alt.test",
                            HeldCommand::Overview => command.starts_with("OVER "),
                        };
                        if should_hold && first.swap(false, Ordering::SeqCst) {
                            held_tx.send(()).unwrap();
                            assert!(read_line(&mut sock).await.is_empty(), "interrupted socket was reused");
                            closed_tx.send(()).unwrap();
                            return;
                        }
                        match command.as_str() {
                            "CAPABILITIES" => write_all(&mut sock, b"101 caps\r\nOVER\r\n.\r\n").await,
                            "GROUP alt.test" => write_all(&mut sock, b"211 20 1 20 alt.test\r\n").await,
                            "LIST OVERVIEW.FMT" => write_all(&mut sock, b"500 standard format\r\n").await,
                            "OVER 1-9" => write_all(&mut sock, b"224 overview\r\n5\tSet [01/01] - \"payload.bin\" yEnc (2/2)\tPoster\tThu, 01 Jan 1970 00:00:01 +0000\t<payload2@test>\t\t4\t1\r\n.\r\n").await,
                            other if other.starts_with("XZVER ") || other.starts_with("XFEATURE ") => write_all(&mut sock, b"500 unsupported\r\n").await,
                            other => panic!("unexpected recovery command: {other}"),
                        }
                    }
                }
            }).await;
            let (mut worker, _events) = fixture_worker();
            let (commands, rx) = crossbeam_channel::unbounded();
            worker.cmd_rx = rx;
            let sid = worker
                .catalog
                .insert_server(&fixture_server(addr.port()))
                .unwrap();
            let pid = cached_first_header(&worker, sid);
            worker.queue_post(pid);
            let job = worker.catalog.list_jobs().unwrap().remove(0);
            let part = worker.catalog.segments_for_job(job.id).unwrap().remove(0);
            worker
                .catalog
                .set_segment_state(part.id, "written", None)
                .unwrap();
            let parts = worker.catalog.segments_for_job(job.id).unwrap();
            let control = tokio::spawn(async move {
                held_rx.recv().await.unwrap();
                commands
                    .send(if cancel {
                        Command::Cancel(job.id)
                    } else {
                        Command::Pause(job.id)
                    })
                    .unwrap();
                std::time::Instant::now()
            });
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                worker.prepare_recovery(&job, &parts, &["payload.bin".into()], false),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result, RecoveryStep::Halted, "{held_command:?}");
            assert!(control.await.unwrap().elapsed() < Duration::from_secs(1));
            tokio::time::timeout(Duration::from_secs(1), closed_rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(worker
                .catalog
                .recovery_scanned_ranges(job.id)
                .unwrap()
                .is_empty());
            assert_eq!(
                worker.catalog.job_by_id(job.id).unwrap().unwrap().state,
                if cancel { "cancelled" } else { "paused" }
            );
            assert_eq!(worker.pools[&sid].connection_stats().len(), 0);

            // Durable context survives a process-style worker restart; the
            // interrupted GROUP/OVER request never became a completed range.
            let catalog_path = worker.data_dir.join("catalog.sqlite");
            let header_path = worker.data_dir.join("headers");
            let downloads = worker.downloads.clone();
            drop(worker);
            let (mut worker, _events) = fixture_worker();
            worker.catalog = Catalog::open(catalog_path).unwrap();
            worker.headers = HeaderStore::open(header_path).unwrap();
            worker.downloads = downloads;
            worker.resume(job.id);
            let job = worker.catalog.job_by_id(job.id).unwrap().unwrap();
            let parts = worker.catalog.segments_for_job(job.id).unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                worker.prepare_recovery(&job, &parts, &["payload.bin".into()], false),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result, RecoveryStep::Scheduled);
            assert_eq!(
                worker.catalog.recovery_scanned_ranges(job.id).unwrap(),
                vec![(1, 9)]
            );
            let resumed_parts = worker.catalog.segments_for_job(job.id).unwrap();
            assert!(
                resumed_parts
                    .iter()
                    .any(
                        |part| part.message_id.trim_matches(['<', '>']) == "payload2@test"
                            && part.part == 2
                            && part.state == "pending"
                    ),
                "{resumed_parts:?}"
            );
            let requests = requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|line| line.as_str() == "GROUP alt.test")
                    .count(),
                2
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|line| line.as_str() == "OVER 1-9")
                    .count(),
                if matches!(held_command, HeldCommand::Overview) {
                    2
                } else {
                    1
                }
            );
        }
    }
}

#[tokio::test]
async fn stale_blocking_recovery_waits_for_its_writer_and_discards_the_result() {
    let (mut worker, _events) = fixture_worker();
    let id = worker
        .catalog
        .create_job(
            None,
            "held repair",
            &worker.downloads.join("held"),
            None,
            &[],
        )
        .unwrap();
    let (commands, rx) = crossbeam_channel::unbounded();
    worker.cmd_rx = rx;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (settled_tx, settled_rx) = tokio::sync::oneshot::channel();
    let writer = tokio::task::spawn_blocking(move || {
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        settled_tx.send(()).unwrap();
        "repaired"
    });
    let control = tokio::spawn(async move {
        started_rx.await.unwrap();
        commands.send(Command::Pause(id)).unwrap();
        tokio::time::sleep(Duration::from_millis(70)).await;
        commands.send(Command::Resume(id)).unwrap();
        tokio::time::sleep(Duration::from_millis(70)).await;
        release_tx.send(()).unwrap();
    });
    assert!(worker.recovery_blocking(id, 0, writer).await.is_none());
    settled_rx.await.unwrap();
    control.await.unwrap();
    assert_eq!(
        worker.catalog.job_by_id(id).unwrap().unwrap().state,
        "queued"
    );
    assert!(!worker.recovery_current(id, 0));
}
