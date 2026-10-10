use super::*;

#[tokio::test]
async fn unavailable_auxiliary_parity_is_tolerated_only_after_all_payloads_verify() {
    for damaged in [false, true] {
        let (mut worker, _) = fixture_worker();
        let dest = worker.downloads.join("optional-parity");
        local_recovery(&dest);
        std::fs::write(
            dest.join("payload.bin"),
            if damaged { b"ABCD\0\0\0\0" } else { DATA },
        )
        .unwrap();
        let mut parts = vec![NewSegment {
            filename: "payload.bin".into(),
            part: 1,
            message_id: "p1@test".into(),
            begin: 0,
            end: if damaged { 4 } else { 8 },
            article: None,
        }];
        if damaged {
            parts.push(NewSegment {
                filename: "payload.bin".into(),
                part: 2,
                message_id: "p2@test".into(),
                begin: 0,
                end: 0,
                article: None,
            });
        }
        parts.push(NewSegment {
            filename: "payload.vol1+99.par2".into(),
            part: 1,
            message_id: "par@test".into(),
            begin: 0,
            end: 0,
            article: None,
        });
        let id = worker
            .catalog
            .create_job(None, "optional", &dest, None, &parts)
            .unwrap();
        for part in worker.catalog.segments_for_job(id).unwrap() {
            let state = if part.message_id == "p1@test" {
                "written"
            } else if damaged {
                "failed"
            } else {
                "missing"
            };
            worker
                .catalog
                .set_segment_state(part.id, state, None)
                .unwrap();
        }
        let parts = worker.catalog.segments_for_job(id).unwrap();
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        worker.finish_job(&job, &parts).await;
        let done = worker.catalog.job_by_id(id).unwrap().unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        assert_eq!(done.bytes_done, 8);
        assert_eq!(std::fs::read(dest.join("payload.bin")).unwrap(), DATA);
        assert!(!dest.join("payload.vol1+99.par2").exists());
    }
}

#[tokio::test]
async fn optional_parity_cannot_be_ignored_without_a_usable_set_covering_every_payload() {
    for mode in ["absent", "malformed", "unprotected"] {
        let (mut worker, _) = fixture_worker();
        let dest = worker.downloads.join(mode);
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("payload.bin"), DATA).unwrap();
        let mut parts = vec![
            NewSegment {
                filename: "payload.bin".into(),
                part: 1,
                message_id: "p@test".into(),
                begin: 0,
                end: 8,
                article: None,
            },
            NewSegment {
                filename: "payload.vol1+99.par2".into(),
                part: 1,
                message_id: "par@test".into(),
                begin: 0,
                end: 0,
                article: None,
            },
        ];
        match mode {
            "malformed" => std::fs::write(dest.join("bad.par2"), b"not PAR2").unwrap(),
            "unprotected" => {
                local_recovery(&dest);
                std::fs::write(dest.join("unprotected.nfo"), b"text").unwrap();
                parts.push(NewSegment {
                    filename: "unprotected.nfo".into(),
                    part: 1,
                    message_id: "nfo@test".into(),
                    begin: 0,
                    end: 4,
                    article: None,
                });
            }
            _ => {}
        }
        let id = worker
            .catalog
            .create_job(None, mode, &dest, None, &parts)
            .unwrap();
        for part in worker.catalog.segments_for_job(id).unwrap() {
            worker
                .catalog
                .set_segment_state(
                    part.id,
                    if part.filename.ends_with(".par2") {
                        "missing"
                    } else {
                        "written"
                    },
                    None,
                )
                .unwrap();
        }
        let parts = worker.catalog.segments_for_job(id).unwrap();
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        worker.finish_job(&job, &parts).await;
        let failed = worker.catalog.job_by_id(id).unwrap().unwrap();
        assert_eq!(failed.state, "failed", "{mode}: {failed:?}");
        if mode == "unprotected" {
            assert!(failed.error.unwrap().contains("does not protect"));
        }
    }
}

#[tokio::test]
async fn parity_only_download_requires_the_requested_recovery_file() {
    let (mut worker, _) = fixture_worker();
    let dest = worker.downloads.join("parity-only");
    local_recovery(&dest);
    std::fs::write(dest.join("payload.bin"), DATA).unwrap();
    let id = worker
        .catalog
        .create_job(
            None,
            "requested parity",
            &dest,
            None,
            &[NewSegment {
                filename: "payload.vol1+99.par2".into(),
                part: 1,
                message_id: "par@test".into(),
                begin: 0,
                end: 0,
                article: None,
            }],
        )
        .unwrap();
    let part = worker.catalog.segments_for_job(id).unwrap().remove(0);
    worker
        .catalog
        .set_segment_state(part.id, "missing", None)
        .unwrap();
    let parts = worker.catalog.segments_for_job(id).unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    worker.finish_job(&job, &parts).await;
    let failed = worker.catalog.job_by_id(id).unwrap().unwrap();
    assert_eq!(failed.state, "failed");
    assert!(failed.error.unwrap().contains("incomplete or damaged"));
    assert!(!dest.join("payload.vol1+99.par2").exists());
}
