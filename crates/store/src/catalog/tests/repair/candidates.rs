use super::*;

#[test]
fn activation_restart_preserves_attempts_and_written_parts() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let data = segment("data.bin", 1, "data@test");
    let parity = segment("data.vol00+02.par2", 1, "parity@test");
    let id = catalog
        .create_job_with_recovery(
            None,
            "Article",
            Path::new("downloads/article"),
            None,
            &[data],
            Some(&context()),
            &[(parity.clone(), Some(2), 2)],
        )
        .unwrap();
    let candidate = catalog.recovery_candidates(id).unwrap().remove(0);
    assert_eq!(candidate.expected_parts, 2);
    assert_eq!(candidate.state, "deferred");
    assert_eq!(catalog.segments_for_job(id).unwrap().len(), 1);
    assert_eq!(
        catalog
            .activate_recovery_candidates(id, &[candidate.id])
            .unwrap(),
        1
    );
    let active = catalog
        .segments_for_job(id)
        .unwrap()
        .into_iter()
        .find(|part| part.filename == parity.filename)
        .unwrap();
    catalog
        .set_segment_state(active.id, "written", Some(3))
        .unwrap();
    catalog.set_segment_bounds(active.id, 4, 9).unwrap();
    let original = catalog.segments_for_job(id).unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog
            .activate_recovery_candidates(id, &[candidate.id])
            .unwrap(),
        0
    );
    assert_eq!(
        catalog
            .add_recovery_candidates(id, &[(parity, Some(2), 2)])
            .unwrap(),
        0
    );
    assert_eq!(catalog.segments_for_job(id).unwrap(), original);
    assert_eq!(
        catalog.recovery_candidates(id).unwrap()[0].state,
        "requested"
    );
    assert_eq!(
        catalog
            .add_recovery_candidates(id, &[(candidate.segment.clone(), Some(2), 3)])
            .unwrap(),
        0
    );
    let updated = catalog.recovery_candidates(id).unwrap().remove(0);
    assert_eq!(updated.expected_parts, 3);
    assert_eq!(updated.state, "requested");
    catalog
        .exhaust_recovery_candidates(id, &[candidate.id])
        .unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog.recovery_candidates(id).unwrap()[0].state,
        "exhausted"
    );
    assert_eq!(
        catalog
            .activate_recovery_candidates(id, &[candidate.id])
            .unwrap(),
        0
    );
}

#[test]
fn deferral_is_atomic_and_does_not_remove_already_requested_or_written_parts() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let id = job(
        &catalog,
        &[
            segment("a.par2", 1, "a@test"),
            segment("b.par2", 1, "b@test"),
            segment("c.par2", 1, "c@test"),
        ],
    );
    let parts = catalog.segments_for_job(id).unwrap();
    catalog
        .set_segment_state(parts[1].id, "written", None)
        .unwrap();
    catalog
        .set_segment_state(parts[2].id, "fetching", None)
        .unwrap();
    assert!(catalog
        .defer_job_segments(id, &[(parts[0].id, Some(2), 1), (parts[1].id, Some(0), 1)])
        .is_ok());
    assert_eq!(catalog.segments_for_job(id).unwrap().len(), 2);
    let candidate = catalog.recovery_candidates(id).unwrap().remove(0);
    catalog
        .activate_recovery_candidates(id, &[candidate.id])
        .unwrap();
    let requested = catalog
        .segments_for_job(id)
        .unwrap()
        .into_iter()
        .find(|part| part.message_id == "a@test")
        .unwrap();
    assert_eq!(
        catalog
            .defer_job_segments(id, &[(requested.id, Some(2), 1)])
            .unwrap(),
        0
    );
    assert_eq!(catalog.segments_for_job(id).unwrap().len(), 3);
    let other = job(
        &catalog,
        &[
            segment("other.par2", 1, "other@test"),
            segment("second.par2", 1, "second@test"),
        ],
    );
    let parts = catalog.segments_for_job(other).unwrap();
    assert!(catalog
        .defer_job_segments(
            other,
            &[(parts[0].id, Some(2), 1), (parts[1].id, Some(0), 1)]
        )
        .is_err());
    assert_eq!(catalog.segments_for_job(other).unwrap().len(), 2);
    assert!(catalog.recovery_candidates(other).unwrap().is_empty());
}

#[test]
fn append_replaces_terminal_alternate_identity_but_preserves_active_and_written() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let id = job(
        &catalog,
        &[
            segment("data.bin", 1, "old@test"),
            segment("data.bin", 2, "two@test"),
            segment("data.bin", 3, "three@test"),
        ],
    );
    let parts = catalog.segments_for_job(id).unwrap();
    catalog
        .set_segment_state(parts[0].id, "missing", None)
        .unwrap();
    catalog
        .set_segment_state(parts[1].id, "written", Some(5))
        .unwrap();
    catalog.set_segment_bounds(parts[1].id, 10, 20).unwrap();
    let replacement = segment("data.bin", 1, "alternate@test");
    assert_eq!(
        catalog
            .append_job_segments(
                id,
                &[
                    replacement.clone(),
                    segment("data.bin", 2, "fake-two@test"),
                    segment("data.bin", 3, "fake-three@test")
                ]
            )
            .unwrap(),
        1
    );
    let current = catalog.segments_for_job(id).unwrap();
    assert_eq!(current.len(), 3);
    assert!(!current.iter().any(|part| part.message_id == "old@test"));
    assert!(current
        .iter()
        .any(|part| part.message_id == "alternate@test" && part.state == "pending"));
    assert!(current.iter().any(|part| part.id == parts[1].id
        && part.state == "written"
        && part.begin == 10
        && part.end == 20
        && part.server_id == Some(5)));
    assert_eq!(catalog.append_job_segments(id, &[replacement]).unwrap(), 0);
    let bad = NewSegment {
        filename: "../evil".into(),
        ..segment("safe", 4, "evil@test")
    };
    assert!(catalog
        .append_job_segments(id, &[segment("data.bin", 4, "four@test"), bad])
        .is_err());
    assert_eq!(catalog.segments_for_job(id).unwrap(), current);
}
