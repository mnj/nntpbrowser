use super::*;

#[test]
fn deleting_subscription_cascades_metadata_and_detaches_preserved_download_history() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.remove").unwrap();
    let other = catalog.subscribe(sid, "alt.keep").unwrap();
    for id in [gid, other] {
        catalog.set_group_marks(id, 10, 50, 30, Some(1)).unwrap();
        catalog
            .upsert_posts(
                id,
                &[incoming(
                    "key",
                    "Title",
                    1,
                    vec![file("photo.jpg", 3, 1, FileKind::Image)],
                )],
            )
            .unwrap();
    }
    let pid = catalog.post_ids_for_group(gid).unwrap()[0];
    let keep = catalog.post_ids_for_group(other).unwrap()[0];
    catalog
        .insert_thumb(pid, Path::new("outside-explicit-file.jpg"), 1, 1)
        .unwrap();
    let job = catalog
        .create_job(
            Some(pid),
            "Title",
            Path::new("saved-folder"),
            None,
            &[NewSegment {
                filename: "photo.jpg".into(),
                part: 1,
                message_id: "photo@test".into(),
                begin: 0,
                end: 3,
                article: Some(10),
            }],
        )
        .unwrap();
    catalog.set_job_state(job, "done", None).unwrap();
    let original = catalog.job_by_id(job).unwrap().unwrap();
    let segments = catalog.segments_for_job(job).unwrap();
    catalog.unsubscribe(sid, "alt.remove").unwrap();
    assert!(catalog.group_by_id(gid).unwrap().is_none());
    assert!(catalog.files_for_post(pid).unwrap().is_empty());
    assert!(catalog.thumb_for_post(pid).unwrap().is_none());
    assert!(catalog.post_ids_for_group(gid).unwrap().is_empty());
    let mut expected = original;
    expected.post_id = None;
    assert_eq!(catalog.job_by_id(job).unwrap(), Some(expected));
    assert_eq!(catalog.segments_for_job(job).unwrap(), segments);
    assert_eq!(catalog.post_ids_for_group(other).unwrap(), vec![keep]);
    assert_eq!(catalog.group_by_id(other).unwrap().unwrap().watermark, 30);
    let fresh = catalog.subscribe(sid, "alt.remove").unwrap();
    assert_eq!(catalog.group_by_id(fresh).unwrap().unwrap().watermark, 0);
    assert_eq!(catalog.post_count(fresh).unwrap(), 0);
    catalog.unsubscribe(sid, "missing").unwrap();
    drop(catalog);
    let _ = fs::remove_file(path);
}
