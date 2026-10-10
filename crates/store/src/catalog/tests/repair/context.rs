use super::*;

#[test]
fn deleting_sources_exhausts_discovery_without_destroying_explicit_downloads() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let id = job(&catalog, &[segment("data.bin", 1, "data@test")]);
    let source = JobRecoveryContext {
        server_id: Some(sid),
        group_id: Some(gid),
        group: "alt.test".into(),
        poster: "Poster".into(),
        group_key: "key".into(),
        posted_at: Some(42),
        ..context()
    };
    catalog.set_recovery_context(id, &source).unwrap();
    catalog
        .add_recovery_candidates(
            id,
            &[(segment("data.vol0+1.par2", 1, "par@test"), Some(1), 1)],
        )
        .unwrap();
    catalog.mark_recovery_scanned(id, 10, 20).unwrap();
    catalog.unsubscribe(sid, "alt.test").unwrap();
    let recovered = catalog.recovery_context(id).unwrap().unwrap();
    assert_eq!(recovered.group_id, None);
    assert!(recovered.discovery_exhausted);
    assert_eq!(recovered.group, "alt.test");
    assert_eq!(recovered.posted_at, Some(42));
    let reused = catalog.subscribe(sid, "alt.new").unwrap();
    assert_eq!(reused, gid);
    catalog.set_recovery_context(id, &source).unwrap();
    assert_eq!(
        catalog.recovery_context(id).unwrap().unwrap().group_id,
        None
    );
    assert_eq!(catalog.recovery_candidates(id).unwrap().len(), 1);
    assert_eq!(catalog.segments_for_job(id).unwrap().len(), 1);
    catalog.delete_server(sid).unwrap();
    assert_eq!(
        catalog.recovery_context(id).unwrap().unwrap().server_id,
        None
    );
    catalog.delete_job(id).unwrap();
    assert!(catalog.recovery_context(id).unwrap().is_none());
    assert!(catalog.recovery_candidates(id).unwrap().is_empty());
    assert!(catalog.recovery_scanned_ranges(id).unwrap().is_empty());
}

#[test]
fn shared_volume_name_hints_are_bounded_and_do_not_accept_paths() {
    assert_eq!(recovery_volume_range("File.VOL000+002.PAR2"), Some((0, 2)));
    assert_eq!(recovery_volume_blocks("File.vol100+5.par2"), Some(5));
    for name in [
        "file.par2",
        "file.voltage.par2",
        "../file.vol0+1.par2",
        "file\\x.vol0+1.par2",
        "file.vol-1+1.par2",
        "file.vol0+0.par2",
        "file.vol0+1+2.par2",
        "file.vol4294967295+1.par2",
    ] {
        assert_eq!(recovery_volume_range(name), None, "{name}");
    }
    assert_eq!(
        recovery_volume_blocks(&format!("{}.vol0+1.par2", "a".repeat(256))),
        None
    );
}
