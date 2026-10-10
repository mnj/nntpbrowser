use super::*;

fn segment(name: &str, message_id: &str) -> NewSegment {
    NewSegment {
        filename: name.into(),
        part: 1,
        message_id: message_id.into(),
        begin: 0,
        end: 10,
        article: Some(1),
    }
}

#[test]
fn deleting_job_cascades_segments_preserves_other_entries_and_saved_file() {
    let path = temp_db();
    let saved = path.with_extension("saved.bin");
    fs::write(&saved, b"keep saved bytes").unwrap();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "one",
                "One",
                1,
                vec![file("one.bin", 10, 1, FileKind::Data)],
            )],
        )
        .unwrap();
    let post = catalog.page_posts(gid, None, 1, None).unwrap().remove(0);
    catalog
        .insert_thumb(post.id, Path::new("thumbs/one.jpg"), 2, 2)
        .unwrap();
    let id = catalog
        .create_job(
            Some(post.id),
            "One",
            saved.parent().unwrap(),
            None,
            &[
                segment(saved.file_name().unwrap().to_str().unwrap(), "one@test"),
                segment("two.bin", "two@test"),
            ],
        )
        .unwrap();
    let other = catalog
        .create_job(
            Some(post.id),
            "Other",
            Path::new("out"),
            None,
            &[segment("other.bin", "other@test")],
        )
        .unwrap();
    catalog.set_job_state(id, "done", None).unwrap();
    assert!(catalog.delete_job(id).unwrap());
    assert!(!catalog.delete_job(id).unwrap());
    assert!(!catalog.delete_job(i64::MAX).unwrap());
    assert!(catalog.job_by_id(id).unwrap().is_none());
    assert!(catalog.segments_for_job(id).unwrap().is_empty());
    assert_eq!(catalog.job_by_id(other).unwrap().unwrap().name, "Other");
    assert_eq!(catalog.segments_for_job(other).unwrap().len(), 1);
    assert_eq!(
        catalog.page_posts(gid, None, 1, None).unwrap().as_slice(),
        std::slice::from_ref(&post)
    );
    assert_eq!(catalog.files_for_post(post.id).unwrap().len(), 1);
    assert!(catalog.thumb_for_post(post.id).unwrap().is_some());
    assert_eq!(catalog.list_groups(sid).unwrap().len(), 1);
    assert_eq!(catalog.list_servers().unwrap().len(), 1);
    assert_eq!(fs::read(&saved).unwrap(), b"keep saved bytes");
    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    assert!(reopened.job_by_id(id).unwrap().is_none());
    assert!(reopened.segments_for_job(id).unwrap().is_empty());
    assert_eq!(reopened.list_jobs().unwrap().len(), 1);
    drop(reopened);
    fs::remove_file(saved).unwrap();
    let _ = fs::remove_file(path);
}

#[test]
fn job_and_segment_deletion_roll_back_together_on_constraint_error() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let id = catalog
        .create_job(
            None,
            "One",
            Path::new("out"),
            None,
            &[
                segment("one.bin", "one@test"),
                segment("two.bin", "two@test"),
            ],
        )
        .unwrap();
    catalog
        .lock()
        .execute_batch(&format!(
            "CREATE TRIGGER reject_segment_removal BEFORE DELETE ON segment
         WHEN OLD.job_id={id} BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END;"
        ))
        .unwrap();
    assert!(catalog.delete_job(id).is_err());
    assert!(catalog.job_by_id(id).unwrap().is_some());
    assert_eq!(catalog.segments_for_job(id).unwrap().len(), 2);
    catalog
        .lock()
        .execute_batch("DROP TRIGGER reject_segment_removal")
        .unwrap();
    assert!(catalog.delete_job(id).unwrap());
    assert!(catalog.job_by_id(id).unwrap().is_none());
    assert!(catalog.segments_for_job(id).unwrap().is_empty());
    drop(catalog);
    let _ = fs::remove_file(path);
}

#[test]
fn explicit_saved_file_history_is_complete_and_rejects_invalid_metadata() {
    let path = temp_db();
    let saved = path.with_extension("saved.png");
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "one",
                "One",
                1,
                vec![file("one.png", 7, 1, FileKind::Image)],
            )],
        )
        .unwrap();
    let post = catalog.page_posts(gid, None, 1, None).unwrap().remove(0);
    assert!(catalog.list_jobs().unwrap().is_empty());
    let id = catalog
        .record_saved_file(post.id, "One / image: saved", &saved, 7)
        .unwrap();
    let job = catalog.job_by_id(id).unwrap().unwrap();
    assert_eq!(job.post_id, Some(post.id));
    assert_eq!(job.name, "One / image: saved");
    assert_eq!(job.state, "done");
    assert_eq!((job.bytes_done, job.bytes_total), (7, 7));
    assert_eq!(job.dest_path, saved.to_str().unwrap());
    assert_eq!(job.dest_dir, saved.parent().unwrap().to_str().unwrap());
    assert_eq!(job.password, None);
    assert_eq!(job.error, None);
    assert!(catalog.segments_for_job(id).unwrap().is_empty());
    for invalid in [
        Path::new("relative.png"),
        Path::new("/"),
        Path::new("/tmp/../escape.png"),
        Path::new("/tmp/bad:name.png"),
        Path::new("/tmp/bad?.png"),
    ] {
        assert!(
            catalog
                .record_saved_file(post.id, "One", invalid, 7)
                .is_err(),
            "{invalid:?}"
        );
    }
    for name in ["", "  ", "bad\nname"] {
        assert!(catalog.record_saved_file(post.id, name, &saved, 7).is_err());
    }
    assert!(catalog
        .record_saved_file(post.id, "One", &saved, u64::MAX)
        .is_err());
    assert!(catalog
        .record_saved_file(i64::MAX, "One", &saved, 7)
        .is_err());
    assert_eq!(catalog.list_jobs().unwrap().len(), 1);
    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    assert_eq!(reopened.job_by_id(id).unwrap().unwrap(), job);
    assert!(reopened.delete_job(id).unwrap());
    assert!(reopened.page_posts(gid, None, 1, None).unwrap().len() == 1);
    drop(reopened);
    let _ = fs::remove_file(path);
}

#[test]
fn thumbnail_metadata_removal_is_scoped_and_idempotent() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[
                incoming("one", "One", 1, vec![]),
                incoming("two", "Two", 2, vec![]),
            ],
        )
        .unwrap();
    let posts = catalog.page_posts(gid, None, 2, None).unwrap();
    for post in &posts {
        catalog
            .insert_thumb(post.id, Path::new("thumb.jpg"), 2, 2)
            .unwrap();
    }
    assert!(catalog.remove_thumb_for_post(posts[0].id).unwrap());
    assert!(!catalog.remove_thumb_for_post(posts[0].id).unwrap());
    assert!(catalog.thumb_for_post(posts[0].id).unwrap().is_none());
    assert!(catalog.thumb_for_post(posts[1].id).unwrap().is_some());
    assert_eq!(catalog.page_posts(gid, None, 2, None).unwrap(), posts);
    drop(catalog);
    let _ = fs::remove_file(path);
}
