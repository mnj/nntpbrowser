use super::*;

#[tokio::test]
async fn overlapping_replacement_uses_activation_order_and_retains_the_old_file() {
    let (mut worker, _) = fixture_worker();
    let dest = worker.downloads.join("replace-volume");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("payload.par2"), INDEX).unwrap();
    let mut context = worker.recovery_source(None).unwrap();
    let set = rust_par2::parse(&dest.join("payload.par2")).unwrap();
    context.set_id = Some(
        set.recovery_set_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    );
    let payload = NewSegment {
        filename: "payload.bin".into(),
        part: 1,
        message_id: "payload@test".into(),
        begin: 0,
        end: 0,
        article: None,
    };
    let names = ["payload.vol0+1.par2", "payload.vol0+2.par2"];
    let candidates = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                NewSegment {
                    filename: (*name).into(),
                    part: 1,
                    message_id: format!("v{index}@test"),
                    begin: 0,
                    end: 0,
                    article: None,
                },
                Some(index as i64 + 1),
                1,
            )
        })
        .collect::<Vec<_>>();
    let id = worker
        .catalog
        .create_job_with_recovery(
            None,
            "replace",
            &dest,
            None,
            &[payload],
            Some(&context),
            &candidates,
        )
        .unwrap();
    let rows = worker.catalog.recovery_candidates(id).unwrap();
    worker
        .catalog
        .set_segment_state(
            worker.catalog.segments_for_job(id).unwrap()[0].id,
            "missing",
            None,
        )
        .unwrap();
    // Candidate creation order differs from its eventual request order.
    for row in rows.iter().rev() {
        worker
            .catalog
            .activate_recovery_candidates(id, &[row.id])
            .unwrap();
        let bytes = if row.segment.filename == names[0] {
            VOLUME
        } else {
            include_bytes!("../fixtures/par2/payload.vol0+2.par2").as_slice()
        };
        std::fs::write(dest.join(&row.segment.filename), bytes).unwrap();
        let segment = worker
            .catalog
            .segments_for_job(id)
            .unwrap()
            .into_iter()
            .find(|part| part.message_id == row.segment.message_id)
            .unwrap();
        worker
            .catalog
            .set_segment_bounds(segment.id, 0, bytes.len() as i64)
            .unwrap();
        worker
            .catalog
            .set_segment_state(segment.id, "written", None)
            .unwrap();
    }
    let parts = worker.catalog.segments_for_job(id).unwrap();
    let job = worker.catalog.job_by_id(id).unwrap().unwrap();
    worker.finish_job(&job, &parts).await;
    let done = worker.catalog.job_by_id(id).unwrap().unwrap();
    assert_eq!(done.state, "done", "{done:?}");
    assert_eq!(std::fs::read(dest.join("payload.bin")).unwrap(), DATA);
    assert!(dest.join(names[0]).exists());
    assert!(!dest.join(names[1]).exists());
    assert_eq!(
        std::fs::read(dest.join(".recovery-retired").join(names[1])).unwrap(),
        include_bytes!("../fixtures/par2/payload.vol0+2.par2")
    );
}

#[tokio::test]
async fn complete_unprotected_auxiliary_and_deferred_volumes_do_not_trigger_header_discovery() {
    for with_index in [false, true] {
        let (mut worker, _) = fixture_worker();
        let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
        let post = cached_first_header(&worker, sid);
        let group = worker
            .catalog
            .group_by_id(worker.catalog.post_by_id(post).unwrap().unwrap().group_id)
            .unwrap()
            .unwrap();
        let source = worker.catalog.post_by_id(post).unwrap().unwrap();
        worker
            .catalog
            .upsert_posts(
                group.id,
                &[IncomingPost {
                    group_key: source.group_key,
                    title: source.title,
                    poster: source.poster,
                    posted_at: 1,
                    files: vec![
                        IncomingFile {
                            name: "payload.bin".into(),
                            size: 8,
                            expected_segments: 1,
                            kind: store::FileKind::Data,
                        },
                        IncomingFile {
                            name: "notes.nfo".into(),
                            size: 5,
                            expected_segments: 1,
                            kind: store::FileKind::Nfo,
                        },
                        IncomingFile {
                            name: "payload.vol0+1.par2".into(),
                            size: 400,
                            expected_segments: 1,
                            kind: store::FileKind::Par2,
                        },
                    ],
                }],
            )
            .unwrap();
        let dest = worker
            .downloads
            .join(if with_index { "aux-index" } else { "no-par" });
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("payload.bin"), DATA).unwrap();
        std::fs::write(dest.join("notes.nfo"), b"notes").unwrap();
        if with_index {
            std::fs::write(dest.join("payload.par2"), INDEX).unwrap();
        }
        let parts = [("payload.bin", 8), ("notes.nfo", 5)].map(|(name, end)| NewSegment {
            filename: name.into(),
            part: 1,
            message_id: format!("{name}@test"),
            begin: 0,
            end,
            article: Some(10),
        });
        let candidate = NewSegment {
            filename: "payload.vol0+1.par2".into(),
            part: 1,
            message_id: "volume@test".into(),
            begin: 0,
            end: 0,
            article: Some(11),
        };
        let context = worker.recovery_source(Some(post)).unwrap();
        let id = worker
            .catalog
            .create_job_with_recovery(
                Some(post),
                "complete",
                &dest,
                None,
                &parts,
                Some(&context),
                &[(candidate, Some(1), 1)],
            )
            .unwrap();
        for part in worker.catalog.segments_for_job(id).unwrap() {
            worker
                .catalog
                .set_segment_state(part.id, "written", None)
                .unwrap();
        }
        let parts = worker.catalog.segments_for_job(id).unwrap();
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        worker.finish_job(&job, &parts).await;
        let done = worker.catalog.job_by_id(id).unwrap().unwrap();
        assert_eq!(done.state, "done", "{done:?}");
        assert!(worker.pools.is_empty());
        assert!(worker
            .catalog
            .recovery_scanned_ranges(id)
            .unwrap()
            .is_empty());
        assert!(worker
            .catalog
            .recovery_candidates(id)
            .unwrap()
            .iter()
            .all(|row| row.state == "deferred"));
    }
}
