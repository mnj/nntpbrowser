use super::*;

#[test]
fn invalid_candidates_roll_back_entire_new_job_and_reject_bad_bounds_or_ids() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let good = segment("data.par2", 1, "par@test");
    for bad in [
        NewSegment {
            filename: "../evil".into(),
            ..good.clone()
        },
        NewSegment {
            message_id: "bad\r\nBODY".into(),
            ..good.clone()
        },
        NewSegment {
            part: 0,
            ..good.clone()
        },
        NewSegment {
            begin: 11,
            end: 10,
            ..good.clone()
        },
        NewSegment {
            article: Some(-1),
            ..good.clone()
        },
    ] {
        assert!(catalog
            .create_job_with_recovery(
                None,
                "Article",
                Path::new("downloads/article"),
                None,
                &[segment("data.bin", 1, "data@test")],
                Some(&context()),
                &[(bad, Some(1), 1)]
            )
            .is_err());
        assert!(catalog.list_jobs().unwrap().is_empty());
    }
    let id = job(&catalog, &[]);
    assert!(catalog
        .add_recovery_candidates(id, &[(good.clone(), Some(0), 1)])
        .is_err());
    assert!(catalog
        .add_recovery_candidates(id, &[(NewSegment { part: 2, ..good }, Some(1), 1)])
        .is_err());
    assert!(catalog.recovery_candidates(id).unwrap().is_empty());
    assert!(catalog
        .create_job(
            None,
            "oversized",
            Path::new("folder"),
            None,
            &[
                NewSegment {
                    end: i64::MAX,
                    ..segment("one.bin", 1, "one@test")
                },
                segment("two.bin", 1, "two@test")
            ]
        )
        .is_err());
}
