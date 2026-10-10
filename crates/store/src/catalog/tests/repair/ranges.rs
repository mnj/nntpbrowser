use super::*;

#[test]
fn discovery_ranges_merge_preserve_gaps_and_survive_failed_context_update() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let id = job(&catalog, &[]);
    catalog.set_recovery_context(id, &context()).unwrap();
    for (first, last) in [(100, 199), (300, 399), (200, 299), (0, 10), (500, 500)] {
        catalog.mark_recovery_scanned(id, first, last).unwrap();
    }
    assert_eq!(
        catalog.recovery_scanned_ranges(id).unwrap(),
        vec![(0, 10), (100, 399), (500, 500)]
    );
    let mut changed = context();
    changed.set_id = Some("0123456789abcdef0123456789abcdef".into());
    changed.discovery_ranges = vec![(400, 499), (9, 8)];
    assert!(catalog.set_recovery_context(id, &changed).is_err());
    assert_eq!(catalog.recovery_context(id).unwrap().unwrap().set_id, None);
    assert_eq!(
        catalog.recovery_scanned_ranges(id).unwrap(),
        vec![(0, 10), (100, 399), (500, 500)]
    );
    assert!(catalog.mark_recovery_scanned(id, 0, u64::MAX).is_err());
    catalog
        .mark_recovery_scanned(id, i64::MAX as u64, i64::MAX as u64)
        .unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog.recovery_scanned_ranges(id).unwrap(),
        vec![
            (0, 10),
            (100, 399),
            (500, 500),
            (i64::MAX as u64, i64::MAX as u64)
        ]
    );
}

#[test]
fn stale_snapshot_cannot_reset_ranges_exhaustion_or_pinned_set() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let id = job(&catalog, &[]);
    let mut current = context();
    current.set_id = Some("0123456789abcdef0123456789abcdef".into());
    current.discovery_exhausted = true;
    current.discovery_ranges = vec![(100, 200)];
    catalog.set_recovery_context(id, &current).unwrap();
    catalog.set_recovery_context(id, &context()).unwrap();
    assert_eq!(catalog.recovery_context(id).unwrap(), Some(current.clone()));
    current.set_id = Some("fedcba9876543210fedcba9876543210".into());
    assert!(catalog.set_recovery_context(id, &current).is_err());
    current.set_id = Some("not-a-set-id".into());
    assert!(catalog.set_recovery_context(id, &current).is_err());
}
