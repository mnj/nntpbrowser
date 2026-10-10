use super::*;

#[test]
fn failed_unsubscribe_keeps_active_group_and_success_clears_its_state() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1)];
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, vec![post(1, 1)]));
    browser.selection.insert(1);
    browser.open_post = Some(1);
    browser.on_memory_preview(1, "one.jpg".into(), Arc::from(b"one".as_slice()));
    browser.set_subscription(1, "alt.test.1", false);
    browser.on_event(Event::Notice("unsubscribe: unsafe cache directory".into()));
    browser.on_event(Event::GroupRows {
        server_id: 1,
        groups: vec![group(1, 1)],
    });
    assert_eq!(browser.selected_group, Some(1));
    assert!(browser.groups[0].subscribed && !browser.posts.is_empty());
    assert!(browser.media_memory.contains_key(&1));
    browser.on_event(Event::GroupUnsubscribed {
        group_id: 1,
        post_ids: vec![1],
    });
    browser.on_event(Event::GroupRows {
        server_id: 1,
        groups: vec![],
    });
    assert!(browser.selected_group.is_none() && browser.posts.is_empty());
    assert!(browser.open_post.is_none() && browser.selection.is_empty());
    assert!(browser.media_memory.is_empty() && browser.media_memory_bytes == 0);
}

#[test]
fn removing_inactive_group_preserves_other_previews_and_rejects_old_decode_for_reused_id() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1), group(2, 1)];
    browser.pick_group(2, 1);
    browser.on_event(page(2, "", None, vec![post(2, 2)]));
    browser.on_memory_preview(1, "one.jpg".into(), Arc::from(b"one".as_slice()));
    browser.on_memory_preview(2, "two.jpg".into(), Arc::from(b"two".as_slice()));
    browser.media_pending.extend([1, 2]);
    browser.media_progress.insert(1, (1, 3));
    browser.media_progress.insert(2, (1, 3));
    browser.thumb_paths.insert(1, PathBuf::from("old.jpg"));
    browser.decoding.insert("post:1".into());
    browser
        .decode_refresh
        .insert("post:1".into(), PathBuf::from("old.jpg"));
    let generation = browser.media_generation;
    browser.on_event(Event::GroupUnsubscribed {
        group_id: 1,
        post_ids: vec![1],
    });
    assert_eq!(browser.selected_group, Some(2));
    assert_eq!(browser.media_generation, generation);
    assert_eq!(&*browser.media_memory[&2].bytes, b"two");
    assert_eq!(browser.media_memory_bytes, 3);
    assert_eq!(browser.media_pending, HashSet::from([2]));
    assert_eq!(browser.media_progress.len(), 1);
    assert!(!browser.thumb_paths.contains_key(&1));
    browser
        .pixel_tx
        .send(Pixels {
            key: "post:1".into(),
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        })
        .unwrap();
    browser.pump_pixels(&egui::Context::default());
    assert!(!browser.textures.contains_key("post:1"));
    assert!(!browser.decode_invalidated.contains("post:1"));
    // A fresh group can reuse SQLite IDs. A new reply remains eligible.
    browser
        .pixel_tx
        .send(Pixels {
            key: "post:1".into(),
            width: 1,
            height: 1,
            rgba: vec![0, 255, 0, 255],
        })
        .unwrap();
    browser.pump_pixels(&egui::Context::default());
    assert!(browser.textures.contains_key("post:1"));
}

#[test]
fn fresh_thumbnail_after_id_reuse_replaces_old_inflight_decode_without_accepting_it() {
    let mut browser = browser();
    browser.decoding.insert("post:1".into());
    browser.on_event(Event::GroupUnsubscribed {
        group_id: 1,
        post_ids: vec![1],
    });
    browser.on_event(Event::Thumb {
        post_id: 1,
        path: browser.handle.data_dir().join("new-thumbnail.jpg"),
    });
    browser
        .pixel_tx
        .send(Pixels {
            key: "post:1".into(),
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        })
        .unwrap();
    browser.pump_pixels(&egui::Context::default());
    assert!(!browser.textures.contains_key("post:1"));
    assert!(!browser.decode_invalidated.contains("post:1"));
    assert!(browser.decoding.contains("post:1"));
    assert!(!browser.decode_refresh.contains_key("post:1"));
}
