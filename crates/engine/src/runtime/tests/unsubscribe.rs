use super::media::add_media_post;
use super::*;

#[tokio::test]
async fn unsubscribe_removes_scoped_caches_and_database_row_but_keeps_saved_files_and_jobs() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Photo", &[("photo.jpg", 3, 1)]);
    let gid = worker.catalog.list_groups(sid).unwrap()[0].id;
    let keep_gid = worker.catalog.subscribe(sid, "alt.keep").unwrap();
    worker
        .headers
        .append_range(
            sid,
            "alt.keep",
            &[store::Record {
                article: 20,
                date_unix: 1,
                date_raw: None,
                bytes: 3,
                lines: 1,
                message_id: "other@test".into(),
                subject: "Other".into(),
                from: "Poster".into(),
            }],
        )
        .unwrap();
    worker
        .catalog
        .upsert_posts(
            keep_gid,
            &[store::IncomingPost {
                group_key: "keep".into(),
                title: "Other".into(),
                poster: "Poster".into(),
                posted_at: 1,
                files: vec![],
            }],
        )
        .unwrap();
    let keep_pid = worker.catalog.post_ids_for_group(keep_gid).unwrap()[0];
    worker
        .store_article_body(pid, "photo@test", b"remove")
        .unwrap();
    worker
        .store_article_body(keep_pid, "other@test", b"keep")
        .unwrap();
    let body = worker.article_cache_path(pid, "photo@test");
    std::fs::write(body.with_extension("body.zst.pending"), b"pending").unwrap();
    let thumb_dir = worker.data_dir.join("thumbs");
    std::fs::create_dir(&thumb_dir).unwrap();
    std::fs::write(thumb_dir.join(format!("{pid}.jpg")), b"cached").unwrap();
    std::fs::write(thumb_dir.join(format!("{keep_pid}.jpg")), b"keep").unwrap();
    let saved_dir = worker.downloads.join("saved-media-file");
    std::fs::create_dir_all(&saved_dir).unwrap();
    let saved = saved_dir.join("photo.jpg");
    std::fs::write(&saved, b"saved").unwrap();
    // Metadata can point outside the cache; never use it as deletion authority.
    worker.catalog.insert_thumb(pid, &saved, 1, 1).unwrap();
    let job = worker
        .catalog
        .record_saved_file(pid, "Photo", &saved, 5)
        .unwrap();
    let preview = worker
        .downloads
        .join("preview")
        .join(format!("{pid}.image"));
    std::fs::create_dir_all(preview.parent().unwrap()).unwrap();
    std::fs::write(&preview, b"legacy preview").unwrap();
    let legacy_dir = worker
        .data_dir
        .join("media-cache")
        .join(pid.to_string())
        .join("1024/1");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join("photo.jpg"), b"legacy media").unwrap();
    let legacy_job = worker
        .catalog
        .create_job(Some(pid), "Legacy", &legacy_dir, None, &[])
        .unwrap();
    worker
        .catalog
        .set_job_state(legacy_job, "done", None)
        .unwrap();
    let header_dir = worker
        .data_dir
        .join("headers")
        .join(sid.to_string())
        .join("alt.test");
    std::fs::write(header_dir.join("pending.journal"), b"unfinished").unwrap();
    worker.retain_preview_media(&[pid, keep_pid], 0);
    worker.preview_media(pid, 1024, 0);
    worker.preview_media(keep_pid, 1024, 0);
    worker
        .on_command(Command::Unsubscribe {
            server_id: sid,
            name: "alt.test".into(),
        })
        .await;
    assert!(worker.catalog.group_by_id(gid).unwrap().is_none());
    assert!(!header_dir.exists());
    assert!(!body.with_extension("body.zst").exists());
    assert!(!body.with_extension("message-id").exists());
    assert!(!body.with_extension("body.zst.pending").exists());
    assert!(!thumb_dir.join(format!("{pid}.jpg")).exists());
    assert!(!preview.exists());
    assert!(!worker
        .data_dir
        .join("media-cache")
        .join(pid.to_string())
        .exists());
    assert_eq!(std::fs::read(saved).unwrap(), b"saved");
    assert_eq!(
        worker.catalog.job_by_id(job).unwrap().unwrap().post_id,
        None
    );
    assert_eq!(
        worker
            .catalog
            .job_by_id(legacy_job)
            .unwrap()
            .unwrap()
            .post_id,
        None
    );
    assert!(worker.catalog.thumb_for_post(pid).unwrap().is_none());
    assert_eq!(worker.preview_pending, HashSet::from([(keep_pid, 0)]));
    assert_eq!(worker.preview_queue.len(), 1);
    assert_eq!(worker.preview_wanted, Some(HashSet::from([keep_pid])));
    assert!(worker
        .article_cache_path(keep_pid, "other@test")
        .with_extension("body.zst")
        .exists());
    assert!(thumb_dir.join(format!("{keep_pid}.jpg")).exists());
    assert_eq!(worker.headers.watermark(sid, "alt.keep").unwrap(), 20);
    assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event,
        Event::GroupUnsubscribed { group_id, post_ids } if group_id == gid && post_ids == vec![pid])));
    let fresh = worker.catalog.subscribe(sid, "alt.test").unwrap();
    assert_eq!(
        worker
            .catalog
            .group_by_id(fresh)
            .unwrap()
            .unwrap()
            .watermark,
        0
    );
    assert_eq!(worker.headers.watermark(sid, "alt.test").unwrap(), 0);
    assert_eq!(worker.catalog.post_count(fresh).unwrap(), 0);
    let root = worker.data_dir.clone();
    drop(worker);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unsubscribe_refuses_symlinked_generated_cache_and_keeps_subscription_and_headers() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Photo", &[("photo.jpg", 3, 1)]);
    let gid = worker.catalog.list_groups(sid).unwrap()[0].id;
    let saved = worker.data_dir.join("explicit-file.jpg");
    std::fs::write(&saved, b"saved").unwrap();
    let thumbs = worker.data_dir.join("thumbs");
    std::fs::create_dir(&thumbs).unwrap();
    std::os::unix::fs::symlink(&saved, thumbs.join(format!("{pid}.jpg"))).unwrap();
    worker
        .on_command(Command::Unsubscribe {
            server_id: sid,
            name: "alt.test".into(),
        })
        .await;
    assert!(worker.catalog.group_by_id(gid).unwrap().unwrap().subscribed);
    assert_eq!(worker.headers.watermark(sid, "alt.test").unwrap(), 1);
    assert_eq!(std::fs::read(saved).unwrap(), b"saved");
    let replies: Vec<_> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert!(!replies
        .iter()
        .any(|event| matches!(event, Event::GroupUnsubscribed { .. })));
    assert!(replies.iter().any(|event| matches!(event, Event::Notice(text) if text.starts_with("unsubscribe:") && text.contains("unsafe"))));
    let root = worker.data_dir.clone();
    drop(worker);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unsubscribe_keeps_shared_legacy_cache_used_by_an_explicit_saved_file_alias() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let pid = add_media_post(&mut worker, sid, "Photo", &[("photo.jpg", 3, 1)]);
    let gid = worker.catalog.list_groups(sid).unwrap()[0].id;
    let dir = worker
        .data_dir
        .join("media-cache")
        .join(pid.to_string())
        .join("1024/1");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.jpg");
    std::fs::write(&file, b"saved").unwrap();
    let job = worker
        .catalog
        .create_job(Some(pid), "Legacy", &dir, None, &[])
        .unwrap();
    worker.catalog.set_job_state(job, "done", None).unwrap();
    let alias = worker.data_dir.join("explicit-folder-alias");
    std::os::unix::fs::symlink(&dir, &alias).unwrap();
    worker
        .catalog
        .record_saved_file(pid, "Explicit", &alias.join("photo.jpg"), 5)
        .unwrap();
    worker.unsubscribe_group(sid, "alt.test");
    assert!(worker.catalog.group_by_id(gid).unwrap().unwrap().subscribed);
    assert_eq!(std::fs::read(file).unwrap(), b"saved");
    assert!(
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event,
        Event::Notice(text) if text.starts_with("unsubscribe:") && text.contains("shared")))
    );
    let root = worker.data_dir.clone();
    drop(worker);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn unsubscribe_refuses_two_group_names_that_share_a_header_directory_slug() {
    let (mut worker, mut events) = fixture_worker();
    let sid = worker.catalog.insert_server(&fixture_server(119)).unwrap();
    let name = "unsafe/group";
    let alias = store::group_slug(name);
    let gid = worker.catalog.subscribe(sid, name).unwrap();
    let other = worker.catalog.subscribe(sid, &alias).unwrap();
    worker.unsubscribe_group(sid, name);
    assert!(worker.catalog.group_by_id(gid).unwrap().is_some());
    assert!(worker.catalog.group_by_id(other).unwrap().is_some());
    assert!(
        std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event,
        Event::Notice(text) if text.starts_with("unsubscribe:") && text.contains("shared")))
    );
    let root = worker.data_dir.clone();
    drop(worker);
    std::fs::remove_dir_all(root).unwrap();
}
