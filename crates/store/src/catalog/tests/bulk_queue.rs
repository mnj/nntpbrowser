use super::*;

#[test]
fn bulk_queue_matches_latest_filtered_posts_and_streams_all_segments() {
    let path = temp_db();
    let root = path.with_extension("headers");
    let catalog = Catalog::open(&path).unwrap();
    let headers = crate::HeaderStore::open(&root).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let mut records = Vec::new();
    for (index, title) in ["Alpha", "Beta", "Gamma"].iter().enumerate() {
        let subject = format!("{title} - \"{title}.bin\" yEnc (1/2)");
        let parsed = crate::parse_subject(&subject, "Poster");
        catalog
            .upsert_posts(
                gid,
                &[IncomingPost {
                    group_key: parsed.group_key,
                    title: title.to_string(),
                    poster: "Poster".into(),
                    posted_at: index as i64,
                    files: vec![file(&format!("{title}.bin"), 20, 2, FileKind::Data)],
                }],
            )
            .unwrap();
        for part in 1..=2 {
            let article = index as u64 * 2 + part;
            records.push(crate::Record {
                article,
                date_unix: index as i64,
                date_raw: None,
                bytes: 10,
                lines: 1,
                message_id: format!("<{article}@test>"),
                subject: format!("{title} - \"{title}.bin\" yEnc ({part}/2)"),
                from: "Poster".into(),
            });
        }
    }
    headers.append_range(sid, "alt.test", &records).unwrap();
    assert_eq!(
        catalog
            .queue_group_headers(
                &headers,
                sid,
                gid,
                "alt.test",
                Some(2),
                None,
                Path::new("downloads")
            )
            .unwrap(),
        2
    );
    let jobs = catalog.list_jobs().unwrap();
    assert_eq!(
        jobs.iter().map(|job| job.name.as_str()).collect::<Vec<_>>(),
        vec!["Gamma", "Beta"]
    );
    for job in jobs {
        assert_eq!(catalog.segments_for_job(job.id).unwrap().len(), 2);
    }
    assert_eq!(
        catalog
            .queue_group_headers(
                &headers,
                sid,
                gid,
                "alt.test",
                None,
                Some("Alpha"),
                Path::new("downloads")
            )
            .unwrap(),
        1
    );
    assert_eq!(
        catalog
            .queue_group_headers(
                &headers,
                sid,
                gid,
                "alt.test",
                Some(0),
                None,
                Path::new("downloads")
            )
            .unwrap(),
        0
    );
    // Damage a block: a bulk operation must roll back its entire queue batch.
    let osf = root.join(format!("{sid}/alt.test/r_1_6.osf"));
    let mut bytes = fs::read(&osf).unwrap();
    bytes[50] ^= 0xff;
    fs::write(osf, bytes).unwrap();
    // A filter matching nothing should not scan unrelated damaged headers.
    assert_eq!(
        catalog
            .queue_group_headers(
                &headers,
                sid,
                gid,
                "alt.test",
                None,
                Some("does-not-exist"),
                Path::new("downloads")
            )
            .unwrap(),
        0
    );
    assert!(catalog
        .queue_group_headers(
            &headers,
            sid,
            gid,
            "alt.test",
            None,
            None,
            Path::new("downloads")
        )
        .is_err());
    assert_eq!(catalog.list_jobs().unwrap().len(), 3);
    drop(catalog);
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bulk_queue_atomically_snapshots_source_and_defers_volumes_but_keeps_par_only_jobs() {
    let path = temp_db();
    let headers = crate::HeaderStore::open(path.with_extension("headers")).unwrap();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let mut records = Vec::new();
    for (title, files) in [
        (
            "Payload",
            vec![
                ("payload.bin", 1),
                ("payload.par2", 1),
                ("payload.vol00+10.par2", 2),
            ],
        ),
        ("Parity only", vec![("only.vol00+10.par2", 1)]),
    ] {
        let first_subject = format!("{title} - \"{}\" yEnc (1/{})", files[0].0, files[0].1);
        catalog
            .upsert_posts(
                gid,
                &[IncomingPost {
                    group_key: crate::parse_subject(&first_subject, "Poster").group_key,
                    title: title.into(),
                    poster: "Poster".into(),
                    posted_at: 123,
                    files: files
                        .iter()
                        .map(|(name, parts)| file(name, 10, *parts, crate::classify(name)))
                        .collect(),
                }],
            )
            .unwrap();
        for (name, parts) in files {
            for part in 1..=parts {
                let article = records.len() as u64 + 1;
                records.push(crate::Record {
                    article,
                    date_unix: 123,
                    date_raw: None,
                    bytes: 10,
                    lines: 1,
                    message_id: format!("<{article}@test>"),
                    subject: format!("{title} - \"{name}\" yEnc ({part}/{parts})"),
                    from: "Poster".into(),
                });
            }
        }
    }
    headers.append_range(sid, "alt.test", &records).unwrap();
    assert_eq!(
        catalog
            .queue_group_headers(
                &headers,
                sid,
                gid,
                "alt.test",
                None,
                None,
                Path::new("downloads")
            )
            .unwrap(),
        2
    );
    let jobs = catalog.list_jobs().unwrap();
    let payload = jobs.iter().find(|job| job.name == "Payload").unwrap();
    let parity_only = jobs.iter().find(|job| job.name == "Parity only").unwrap();
    assert_eq!(
        catalog
            .segments_for_job(payload.id)
            .unwrap()
            .iter()
            .map(|part| part.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["payload.bin", "payload.par2"]
    );
    let candidates = catalog.recovery_candidates(payload.id).unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates
        .iter()
        .all(|candidate| candidate.expected_parts == 2
            && candidate.recovery_blocks == Some(10)
            && candidate.state == "deferred"));
    assert!(catalog
        .recovery_candidates(parity_only.id)
        .unwrap()
        .is_empty());
    assert_eq!(catalog.segments_for_job(parity_only.id).unwrap().len(), 1);
    let source = catalog.recovery_context(payload.id).unwrap().unwrap();
    assert_eq!(source.group_id, Some(gid));
    assert_eq!(source.server_id, Some(sid));
    assert_eq!(source.poster, "Poster");
    assert_eq!(source.posted_at, Some(123));
    // Unsubscribe before the first scheduler pass cannot lose the original
    // identity or recreate the removed subscription through header discovery.
    catalog.unsubscribe(sid, "alt.test").unwrap();
    let source = catalog.recovery_context(payload.id).unwrap().unwrap();
    assert_eq!(source.group_id, None);
    assert!(source.discovery_exhausted);
    assert_eq!(catalog.recovery_candidates(payload.id).unwrap(), candidates);
}
