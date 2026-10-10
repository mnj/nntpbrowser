use super::*;

fn job(worker: &Worker, dest: &Path, name: &str, state: &str) -> i64 {
    let id = worker
        .catalog
        .create_job(
            None,
            name,
            dest,
            None,
            &[NewSegment {
                filename: "article.bin".into(),
                part: 1,
                message_id: format!("{}@test", dest.display()),
                begin: 0,
                end: 4,
                article: None,
            }],
        )
        .unwrap();
    worker.catalog.set_job_state(id, state, None).unwrap();
    id
}

fn failed(events: &mut UnboundedReceiver<Event>, id: i64) -> String {
    match events.try_recv().unwrap() {
        Event::DownloadDeleteFailed { job_id, error } => {
            assert_eq!(job_id, id);
            error
        }
        event => panic!("unexpected event: {event:?}"),
    }
}

#[tokio::test]
async fn explicit_folder_removal_deletes_extracted_files_and_emits_before_jobs() {
    let (mut worker, mut events) = fixture_worker();
    let folder = worker.downloads.join("Article-100");
    let sibling = worker.downloads.join("Sibling-101");
    std::fs::create_dir_all(folder.join("extracted/nested")).unwrap();
    std::fs::create_dir_all(folder.join("thumbs")).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(folder.join("article.bin"), b"data").unwrap();
    std::fs::write(folder.join("extracted/nested/photo.png"), b"image").unwrap();
    std::fs::write(folder.join("thumbs/poster.jpg"), b"thumb").unwrap();
    std::fs::write(sibling.join("keep.bin"), b"keep").unwrap();
    let id = job(&worker, &folder, "Article", "done");
    let other = job(&worker, &sibling, "Sibling", "failed");
    worker
        .on_command(Command::DeleteDownload {
            job_id: id,
            remove_files: true,
        })
        .await;
    assert!(!folder.exists());
    assert_eq!(std::fs::read(sibling.join("keep.bin")).unwrap(), b"keep");
    assert!(worker.catalog.segments_for_job(id).unwrap().is_empty());
    assert!(
        matches!(events.try_recv().unwrap(), Event::DownloadDeleted { job_id, remove_files: true } if job_id == id)
    );
    assert!(
        matches!(events.try_recv().unwrap(), Event::Jobs(jobs) if jobs.len()==1 && jobs[0].id==other)
    );
}

#[tokio::test]
async fn history_only_retains_files_and_active_or_paused_jobs_require_cancel() {
    let (mut worker, mut events) = fixture_worker();
    let legacy = worker.downloads.join("legacy-shared-folder");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("keep.bin"), b"keep").unwrap();
    let id = job(&worker, &legacy, "Legacy", "done");
    worker.delete_download(id, false);
    assert_eq!(std::fs::read(legacy.join("keep.bin")).unwrap(), b"keep");
    assert!(worker.catalog.job_by_id(id).unwrap().is_none());
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::DownloadDeleted { .. }
    ));
    assert!(matches!(events.try_recv().unwrap(), Event::Jobs(_)));
    for state in [
        "queued", "fetching", "decoding", "repair", "extract", "paused",
    ] {
        let id = job(
            &worker,
            &worker.downloads.join("Active-10"),
            "Active",
            state,
        );
        for files in [false, true] {
            worker.delete_download(id, files);
            assert!(failed(&mut events, id).contains("cancel"));
            assert_eq!(worker.catalog.job_by_id(id).unwrap().unwrap().state, state);
        }
    }
}

#[tokio::test]
async fn shared_and_overlapping_folders_are_refused_including_canonical_aliases() {
    let (mut worker, mut events) = fixture_worker();
    let folder = worker.downloads.join("Article-100");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("keep.bin"), b"keep").unwrap();
    let id = job(&worker, &folder, "Article", "done");
    for alias in [
        folder.clone(),
        folder.join("../Article-100"),
        folder.join("extracted"),
        worker.downloads.clone(),
    ] {
        let other = job(&worker, &alias, "Other", "done");
        worker.delete_download(id, true);
        assert!(failed(&mut events, id).contains("another download"));
        assert_eq!(std::fs::read(folder.join("keep.bin")).unwrap(), b"keep");
        worker.catalog.delete_job(other).unwrap();
    }
    assert!(worker.catalog.job_by_id(id).unwrap().is_some());
}

#[tokio::test]
async fn missing_known_folder_can_be_removed_but_unknown_and_reserved_paths_are_retained() {
    let (mut worker, mut events) = fixture_worker();
    let missing = worker.downloads.join("Article-100");
    let id = job(&worker, &missing, "Article", "cancelled");
    worker.delete_download(id, true);
    assert!(worker.catalog.job_by_id(id).unwrap().is_none());
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::DownloadDeleted { .. }
    ));
    assert!(matches!(events.try_recv().unwrap(), Event::Jobs(_)));
    for path in [
        worker.downloads.join("unknown"),
        worker.downloads.clone(),
        worker.data_dir.clone(),
        PathBuf::from("/"),
    ] {
        let id = job(&worker, &path, "Article", "failed");
        worker.delete_download(id, true);
        assert!(!failed(&mut events, id).is_empty());
        assert!(worker.catalog.job_by_id(id).unwrap().is_some());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_folder_is_refused_and_nested_link_does_not_remove_its_target() {
    use std::os::unix::fs::symlink;
    let (mut worker, mut events) = fixture_worker();
    let outside = worker.data_dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.bin"), b"keep").unwrap();
    let folder = worker.downloads.join("Article-100");
    std::fs::create_dir_all(&worker.downloads).unwrap();
    symlink(&outside, &folder).unwrap();
    let id = job(&worker, &folder, "Article", "done");
    worker.delete_download(id, true);
    assert!(failed(&mut events, id).contains("symbolic link"));
    assert!(worker.catalog.job_by_id(id).unwrap().is_some());
    assert_eq!(std::fs::read(outside.join("keep.bin")).unwrap(), b"keep");
    std::fs::remove_file(&folder).unwrap();
    std::fs::create_dir_all(&folder).unwrap();
    symlink(&outside, folder.join("linked-extraction")).unwrap();
    worker.delete_download(id, true);
    assert!(!folder.exists());
    assert_eq!(std::fs::read(outside.join("keep.bin")).unwrap(), b"keep");
}

#[tokio::test]
async fn last_completed_output_removes_shared_generated_thumbnail() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker
        .catalog
        .upsert_posts(
            gid,
            &[store::IncomingPost {
                group_key: "one".into(),
                title: "Article".into(),
                poster: "author".into(),
                posted_at: 1,
                files: vec![],
            }],
        )
        .unwrap();
    let post = worker
        .catalog
        .page_posts(gid, None, 1, None)
        .unwrap()
        .remove(0);
    let first = worker.downloads.join(format!("Article-{}", post.id));
    let second = worker
        .downloads
        .join(format!("saved-media-{}-123", post.id));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let one = worker
        .catalog
        .create_job(Some(post.id), "Article", &first, None, &[])
        .unwrap();
    worker.catalog.set_job_state(one, "done", None).unwrap();
    let saved = second.join("image.png");
    std::fs::write(&saved, b"image").unwrap();
    let two = worker
        .catalog
        .record_saved_file(post.id, "image.png", &saved, 5)
        .unwrap();
    let thumbnail = worker
        .data_dir
        .join("thumbs")
        .join(format!("{}.jpg", post.id));
    std::fs::create_dir_all(thumbnail.parent().unwrap()).unwrap();
    std::fs::write(&thumbnail, b"thumbnail").unwrap();
    worker
        .catalog
        .insert_thumb(post.id, &thumbnail, 2, 2)
        .unwrap();
    worker.delete_download(one, true);
    assert!(thumbnail.exists());
    assert!(worker.catalog.thumb_for_post(post.id).unwrap().is_some());
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::DownloadDeleted {
            remove_files: true,
            ..
        }
    ));
    assert!(matches!(events.try_recv().unwrap(), Event::Jobs(_)));
    worker.delete_download(two, false);
    assert!(thumbnail.exists());
    assert!(worker.catalog.thumb_for_post(post.id).unwrap().is_some());
    let two = worker
        .catalog
        .record_saved_file(post.id, "image.png", &saved, 5)
        .unwrap();
    worker.delete_download(two, true);
    assert!(!thumbnail.exists());
    assert!(worker.catalog.thumb_for_post(post.id).unwrap().is_none());
    assert!(worker.catalog.post_by_id(post.id).unwrap().is_some());
}

#[tokio::test]
async fn explicit_delete_accepts_owned_completed_legacy_media_cache_folder() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(1)).unwrap();
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    worker
        .catalog
        .upsert_posts(
            gid,
            &[store::IncomingPost {
                group_key: "legacy".into(),
                title: "Legacy".into(),
                poster: "author".into(),
                posted_at: 1,
                files: vec![],
            }],
        )
        .unwrap();
    let post = worker
        .catalog
        .page_posts(gid, None, 1, None)
        .unwrap()
        .remove(0);
    let folder = worker
        .data_dir
        .join("media-cache")
        .join(post.id.to_string())
        .join("1024")
        .join("12345");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("image.png"), b"cached image").unwrap();
    let id = worker
        .catalog
        .create_job(Some(post.id), "Legacy", &folder, None, &[])
        .unwrap();
    worker.catalog.set_job_state(id, "done", None).unwrap();
    worker.delete_download(id, true);
    assert!(!folder.exists());
    assert!(worker.data_dir.join("media-cache").is_dir());
    assert!(worker.catalog.job_by_id(id).unwrap().is_none());
    assert!(
        matches!(events.try_recv().unwrap(), Event::DownloadDeleted { job_id, remove_files: true } if job_id == id)
    );
}
