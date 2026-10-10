use super::*;

#[tokio::test]
async fn crc_failed_and_missing_bodies_are_repaired_from_local_recovery_volumes() {
    for corrupt in [true, false] {
        let addr = serve(move |mut sock| async move {
            write_all(&mut sock, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut sock).await, "CAPABILITIES");
            write_all(&mut sock, b"101 caps\r\n.\r\n").await;
            loop {
                let command = read_line(&mut sock).await;
                if command.is_empty() {
                    break;
                }
                let part = match command.as_str() {
                    "BODY <part1@test>" => 1,
                    "BODY <part2@test>" => 2,
                    _ => panic!("unexpected {command}"),
                };
                if part == 2 && !corrupt {
                    write_all(&mut sock, b"430 missing\r\n").await;
                    continue;
                }
                let begin = if part == 1 { 0 } else { 4 };
                let (mut body, _) = yenc_simd::encode_article(
                    &DATA[begin..begin + 4],
                    "payload.bin",
                    part,
                    2,
                    begin as u64,
                    8,
                );
                if part == 2 {
                    let pos = body
                        .windows(7)
                        .position(|slice| slice == b"pcrc32=")
                        .unwrap()
                        + 7;
                    body[pos] = if body[pos] == b'0' { b'1' } else { b'0' };
                    assert!(crate::decode::decode_body(&body).is_err());
                }
                write_multiline(&mut sock, &body).await;
            }
        })
        .await;
        let (mut worker, _) = fixture_worker();
        worker
            .catalog
            .insert_server(&fixture_server(addr.port()))
            .unwrap();
        let dest = worker
            .downloads
            .join(if corrupt { "bad-crc" } else { "missing-body" });
        local_recovery(&dest);
        let parts = (1..=2)
            .map(|part| NewSegment {
                filename: "payload.bin".into(),
                part,
                message_id: format!("part{part}@test"),
                begin: 0,
                end: 0,
                article: None,
            })
            .collect::<Vec<_>>();
        let id = worker
            .catalog
            .create_job(None, "recover", &dest, None, &parts)
            .unwrap();
        let done = drive_until_terminal(&mut worker, id).await;
        assert_eq!(done.state, "done", "{done:?}");
        assert_eq!(done.bytes_done, 8);
        assert_eq!(std::fs::read(dest.join("payload.bin")).unwrap(), DATA);
        let states = worker.catalog.segments_for_job(id).unwrap();
        assert_eq!(states[1].state, if corrupt { "failed" } else { "missing" });
        assert_eq!(
            crate::postproc::repair_dir(&dest).unwrap(),
            crate::postproc::RepairReport::Clean
        );
    }
}

#[tokio::test]
async fn unrelated_clean_par2_set_cannot_certify_an_unresolved_download() {
    let (mut worker, _) = fixture_worker();
    let dest = worker.downloads.join("wrong-set");
    local_recovery(&dest);
    std::fs::write(dest.join("payload.bin"), DATA).unwrap();
    let id = worker
        .catalog
        .create_job(
            None,
            "other",
            &dest,
            None,
            &[NewSegment {
                filename: "unprotected.bin".into(),
                part: 1,
                message_id: "other@test".into(),
                begin: 0,
                end: 0,
                article: None,
            }],
        )
        .unwrap();
    let parts = worker.catalog.segments_for_job(id).unwrap();
    worker
        .catalog
        .set_segment_state(parts[0].id, "failed", None)
        .unwrap();
    let parts = worker.catalog.segments_for_job(id).unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    worker.finish_job(&job, &parts).await;
    let failed = worker.catalog.job_by_id(id).unwrap().unwrap();
    assert_eq!(failed.state, "failed");
    assert!(failed.error.unwrap().contains("does not protect"));
}

#[test]
fn malformed_empty_sets_and_bad_recovery_data_never_report_success() {
    let (worker, _) = fixture_worker();
    let dir = worker.downloads.join("empty-set");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("empty.par2"), &INDEX[..92]).unwrap();
    assert!(crate::postproc::repair_dir(&dir).is_err());
    let dir = worker.downloads.join("bad-recovery");
    local_recovery(&dir);
    std::fs::write(
        dir.join("payload.vol0+1.par2"),
        include_bytes!("../fixtures/par2/bad-recovery.par2"),
    )
    .unwrap();
    assert!(crate::postproc::repair_dir(&dir).is_err());
}

#[cfg(unix)]
#[test]
fn repair_rejects_linked_volumes_sources_and_final_directory() {
    use std::os::unix::fs::symlink;
    let (worker, _) = fixture_worker();
    let dir = worker.downloads.join("volume-link");
    local_recovery(&dir);
    let outside = worker.data_dir.join("external.par2");
    std::fs::write(&outside, VOLUME).unwrap();
    symlink(&outside, dir.join("external.vol1+1.par2")).unwrap();
    assert!(crate::postproc::repair_dir(&dir)
        .unwrap_err()
        .to_string()
        .contains("regular local files"));
    let dir = worker.downloads.join("source-link");
    local_recovery(&dir);
    let outside = worker.data_dir.join("external.bin");
    std::fs::write(&outside, b"unaltered").unwrap();
    symlink(&outside, dir.join("payload.bin")).unwrap();
    assert!(crate::postproc::repair_dir(&dir).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"unaltered");
    let linked = worker.downloads.join("directory-link");
    symlink(&dir, &linked).unwrap();
    assert!(crate::postproc::repair_dir(&linked)
        .unwrap_err()
        .to_string()
        .contains("regular download directory"));
}
