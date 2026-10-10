use super::*;

#[test]
fn version_two_migration_and_restart_keep_requested_candidates_and_paused_jobs() {
    let path = temp_db();
    let old = Connection::open(&path).unwrap();
    old.execute_batch(super::super::super::schema::SCHEMA)
        .unwrap();
    old.execute("PRAGMA user_version=2", []).unwrap();
    drop(old);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog
            .lock()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        3
    );
    let id = job(&catalog, &[segment("data.bin", 1, "data@test")]);
    catalog.set_recovery_context(id, &context()).unwrap();
    catalog
        .add_recovery_candidates(
            id,
            &[(segment("data.vol0+2.par2", 1, "par@test"), Some(2), 2)],
        )
        .unwrap();
    let candidate = catalog.recovery_candidates(id).unwrap()[0].id;
    catalog
        .activate_recovery_candidates(id, &[candidate])
        .unwrap();
    let parts = catalog.segments_for_job(id).unwrap();
    catalog
        .set_segment_state(parts[0].id, "fetching", None)
        .unwrap();
    catalog
        .set_job_state(id, "recovery_fetching", None)
        .unwrap();
    let paused = job(&catalog, &[]);
    catalog.set_job_state(paused, "paused", None).unwrap();
    let scan = job(&catalog, &[]);
    catalog
        .set_job_state(scan, "recovery_headers", None)
        .unwrap();
    catalog.mark_recovery_scanned(id, 10, 20).unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.job_by_id(id).unwrap().unwrap().state, "queued");
    assert_eq!(catalog.job_by_id(scan).unwrap().unwrap().state, "queued");
    assert_eq!(catalog.job_by_id(paused).unwrap().unwrap().state, "paused");
    assert_eq!(catalog.segments_for_job(id).unwrap()[0].state, "pending");
    assert_eq!(
        catalog.recovery_candidates(id).unwrap()[0].state,
        "requested"
    );
    assert_eq!(catalog.recovery_scanned_ranges(id).unwrap(), vec![(10, 20)]);
    assert_eq!(
        catalog
            .activate_recovery_candidates(id, &[candidate])
            .unwrap(),
        0
    );
}

#[test]
fn interrupted_schema_migration_does_not_advance_version_or_leave_partial_tables() {
    let path = temp_db();
    let old = Connection::open(&path).unwrap();
    old.execute_batch(super::super::super::schema::SCHEMA)
        .unwrap();
    old.execute("PRAGMA user_version=2", []).unwrap();
    // A conflicting schema object forces an error midway through migration.
    old.execute(
        "CREATE VIEW job_recovery AS SELECT id AS job_id FROM job",
        [],
    )
    .unwrap();
    drop(old);
    assert!(Catalog::open(&path).is_err());
    let old = Connection::open(&path).unwrap();
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(old.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('recovery_candidate','recovery_scan','recovery_group_deleted')",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    old.execute("DROP VIEW job_recovery", []).unwrap();
    drop(old);
    assert!(Catalog::open(&path).is_ok());
}
