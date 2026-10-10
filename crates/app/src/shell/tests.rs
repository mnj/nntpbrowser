use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) fn browser() -> Browser {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "nntpbrowser-state-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    Browser::with_handle(Handle::start(dir))
}

fn post(id: i64, group_id: i64) -> PostRow {
    PostRow {
        id,
        group_id,
        group_key: id.to_string(),
        title: format!("Image {id}"),
        poster: "reader".into(),
        posted_at: 1000 - id,
        total_bytes: 1024,
        file_count: 1,
        segment_count: 1,
        incomplete: false,
        has_image: true,
        has_video: false,
        has_par2: false,
    }
}

fn group(id: i64, server_id: i64) -> GroupRow {
    GroupRow {
        id,
        server_id,
        name: format!("alt.test.{id}"),
        low: 1,
        high: 1000,
        watermark: 1000,
        subscribed: true,
        last_refresh: None,
    }
}

fn server(id: i64) -> ServerRow {
    ServerRow {
        id,
        name: format!("Server {id}"),
        host: "localhost".into(),
        port: 563,
        tls: true,
        starttls: false,
        username: None,
        password: None,
        priority: 1,
        max_connections: 8,
        pipeline_depth: 8,
        fill: false,
        enabled: false,
        header_compression: "plain".into(),
    }
}

fn page(group_id: i64, search: &str, after: Option<(i64, i64)>, posts: Vec<PostRow>) -> Event {
    Event::PostsPage {
        group_id,
        search: search.into(),
        after,
        posts,
    }
}

#[test]
fn pending_page_ignores_sync_first_page_and_replies_from_old_group_or_search() {
    let mut browser = browser();
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, (1..=80).map(|id| post(id, 1)).collect()));
    browser.load_more();
    let cursor = browser.post_request.as_ref().unwrap().after;
    browser.on_event(page(1, "", None, vec![post(900, 1)]));
    assert_eq!(browser.posts.len(), 80);
    assert!(browser.posts_loading && browser.append_posts);
    browser.on_event(page(1, "", cursor, vec![post(81, 1)]));
    assert_eq!(browser.posts.len(), 81);
    assert!(!browser.posts_loading);

    browser.pick_group(2, 2);
    browser.on_event(page(1, "", None, vec![post(901, 1)]));
    assert!(browser.posts.is_empty() && browser.posts_loading);
    browser.search = "landscape".into();
    browser.reload_posts();
    browser.on_event(page(2, "", None, vec![post(902, 2)]));
    assert!(browser.posts.is_empty() && browser.posts_loading);
    browser.on_event(page(2, "landscape", None, vec![post(903, 2)]));
    assert_eq!(browser.posts[0].id, 903);
    assert!(!browser.posts_loading);
}

#[test]
fn switching_search_ignores_an_older_page_with_the_same_search_when_returning() {
    let mut browser = browser();
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, (1..=80).map(|id| post(id, 1)).collect()));
    browser.load_more();
    let cursor = browser.post_request.as_ref().unwrap().after;
    browser.search = "new".into();
    browser.reload_posts();
    browser.search.clear();
    browser.reload_posts();
    browser.on_event(page(1, "", cursor, vec![post(81, 1)]));
    assert_eq!(browser.posts.len(), 80);
    assert!(browser.posts_loading);
    browser.on_event(page(1, "", None, vec![post(900, 1)]));
    assert_eq!(browser.posts.len(), 1);
    assert_eq!(browser.posts[0].id, 900);
}

#[test]
fn header_completion_only_reloads_matching_group_and_releases_errors() {
    let mut browser = browser();
    browser.pick_group(2, 2);
    browser.search = "landscape".into();
    browser.reload_posts();
    let pending = browser.post_request.clone();
    browser.on_event(Event::Notice(
        "synced 200 articles from alt.test.1 via plain".into(),
    ));
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: None,
    });
    assert!(browser.post_request == pending);
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: Some("timeout".into()),
    });
    assert!(browser.posts_loading);
    browser.groups = vec![group(2, 2)];
    browser.fetch_selected_headers();
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 2,
        error: Some("overview: timeout".into()),
    });
    assert!(!browser.posts_loading);
    assert!(browser.post_request.is_none());
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 2,
        error: None,
    });
    let pending = browser.post_request.as_ref().unwrap();
    assert_eq!(pending.group_id, 2);
    assert_eq!(pending.search, "landscape");
    assert!(pending.after.is_none() && browser.posts_loading);
}

#[test]
fn failed_header_fetch_does_not_cancel_a_search_requested_during_the_fetch() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1)];
    browser.pick_group(1, 1);
    browser.fetch_selected_headers();
    browser.search = "new search".into();
    browser.reload_posts();
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: Some("timeout".into()),
    });
    assert!(browser.posts_loading && browser.post_request.is_some());
    browser.on_event(page(1, "new search", None, vec![post(10, 1)]));
    assert_eq!(browser.posts[0].id, 10);
    assert!(!browser.posts_loading);
}

#[test]
fn removing_the_active_server_resets_indexes_selection_and_pending_work() {
    let mut browser = browser();
    browser.servers = vec![server(1), server(2)];
    browser.groups = vec![group(1, 1), group(2, 2)];
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, vec![post(1, 1)]));
    browser.selection.insert(1);
    browser.anchor = Some(1);
    browser.open_post = Some(1);
    browser.preview_post = Some(1);
    assert_eq!(browser.filtered_posts().as_slice(), &[0]);
    browser.reload_posts();
    // Editing another server must not conceal deletion of the browsed server.
    browser.selected_server = Some(2);
    browser.on_event(Event::Servers(vec![server(2)]));
    assert!(browser.selected_group.is_none());
    assert!(browser.posts.is_empty() && browser.filtered_posts().is_empty());
    assert!(browser.selection.is_empty() && browser.anchor.is_none());
    assert!(browser.open_post.is_none() && browser.preview_post.is_none());
    assert!(browser.post_request.is_none() && !browser.posts_loading);
}

#[test]
fn unsubscribe_and_search_replacement_clear_disappearing_article_state() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1)];
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, vec![post(1, 1)]));
    browser.selection.insert(1);
    browser.anchor = Some(1);
    browser.open_post = Some(1);
    browser.preview_post = Some(1);
    browser.search = "different".into();
    browser.reload_posts();
    browser.on_event(page(1, "different", None, vec![post(2, 1)]));
    assert!(browser.selection.is_empty() && browser.anchor.is_none());
    assert!(browser.open_post.is_none() && browser.preview_post.is_none());
    browser.reload_posts();
    browser.on_event(Event::GroupRows {
        server_id: 1,
        groups: Vec::new(),
    });
    assert!(browser.selected_group.is_none() && !browser.posts_loading);
    assert!(browser.posts.is_empty() && browser.post_request.is_none());
}

#[test]
fn cached_media_is_not_fetched_again_after_its_texture_is_evicted() {
    let mut browser = browser();
    let path = browser.handle.data_dir().join("cached-thumbnail.jpg");
    std::fs::create_dir_all(browser.handle.data_dir()).unwrap();
    std::fs::write(&path, b"thumbnail fixture").unwrap();
    browser.posts = vec![post(1, 1)];
    browser.media_paths.insert(1, path);
    browser.request_media(&[1]);
    assert!(browser.media_pending.is_empty());
}

#[test]
fn automatic_media_rejects_unknown_or_large_single_files_without_limiting_visible_requests() {
    let mut browser = browser();
    browser.posts = (1..=10).map(|id| post(id, 1)).collect();
    browser.posts[0].total_bytes = 0;
    browser.posts[1].total_bytes = MEDIA_LIMIT as i64;
    browser.posts[2].has_image = false;
    browser.posts[2].has_video = true;
    browser.request_media(&(1..=10).collect::<Vec<_>>());
    assert_eq!(browser.media_pending.len(), 8);
    assert!(!browser.media_pending.contains(&1));
    assert!(!browser.media_pending.contains(&2));
    assert!(browser.media_pending.contains(&3));
    browser.request_media(&[7, 8, 9, 10]);
    assert_eq!(browser.media_pending, HashSet::from([7, 8, 9, 10]));
}

#[test]
fn article_body_replies_are_scoped_to_current_selection_and_bounded_for_display() {
    let mut browser = browser();
    browser.open_post = Some(2);
    browser.article_requested = Some(2);
    browser.article_loading = true;
    browser.on_event(Event::ArticleContent {
        post_id: 1,
        text: "old selection".into(),
        binary: false,
    });
    assert!(browser.article_loading && browser.article_content.is_none());
    browser.on_event(Event::ArticleReadFailed {
        post_id: 1,
        error: "old failure".into(),
    });
    assert!(browser.article_loading && browser.article_error.is_none());
    browser.on_event(Event::ArticleContent {
        post_id: 2,
        text: "日".repeat(25_000),
        binary: false,
    });
    let text = &browser.article_content.as_ref().unwrap().text;
    assert!(text.len() < 66_000 && text.contains("Display shortened"));
    assert!(!browser.article_loading);
}

#[test]
fn media_failures_retry_after_updated_headers_and_server_removal_drops_cached_identity() {
    let mut browser = browser();
    browser.servers = vec![server(1)];
    browser.pick_group(1, 1);
    browser.on_event(page(1, "", None, vec![post(1, 1)]));
    browser
        .media_errors
        .insert(1, "Missing article part".into());
    browser.reload_posts();
    browser.on_event(page(1, "", None, vec![post(1, 1)]));
    assert!(browser.media_errors.contains_key(&1));
    browser.reload_posts();
    let mut updated = post(1, 1);
    updated.segment_count = 2;
    browser.on_event(page(1, "", None, vec![updated]));
    assert!(!browser.media_errors.contains_key(&1));
    browser
        .media_paths
        .insert(1, PathBuf::from("old-image.jpg"));
    browser
        .thumb_paths
        .insert(1, PathBuf::from("old-thumb.jpg"));
    browser.on_event(Event::Servers(Vec::new()));
    assert!(browser.media_paths.is_empty() && browser.thumb_paths.is_empty());
}

#[test]
fn a_new_thumbnail_recovers_decode_failure_and_defers_stale_pixels() {
    let mut browser = browser();
    let key = "post:1".to_owned();
    browser.decode_failed.insert(key.clone());
    browser.decoding.insert(key.clone());
    let path = browser.handle.data_dir().join("replaced-thumbnail.jpg");
    browser.on_event(Event::Thumb {
        post_id: 1,
        path: path.clone(),
    });
    assert!(!browser.decode_failed.contains(&key));
    assert_eq!(browser.decode_refresh.get(&key), Some(&path));
    browser
        .pixel_tx
        .send(Pixels {
            key: key.clone(),
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        })
        .unwrap();
    browser.pump_pixels(&egui::Context::default());
    assert!(!browser.textures.contains_key(&key));
    assert!(!browser.decode_refresh.contains_key(&key));
    assert!(browser.decoding.contains(&key));
}

#[test]
fn article_loading_error_only_releases_the_matching_query_and_cursor() {
    let mut browser = browser();
    browser.pick_group(2, 2);
    browser.search = "landscape".into();
    browser.reload_posts();
    for (group_id, search, after) in [
        (1, "landscape", None),
        (2, "old", None),
        (2, "landscape", Some((99, 10))),
    ] {
        browser.on_event(Event::PostsFailed {
            group_id,
            search: search.into(),
            after,
            error: "posts: timeout".into(),
        });
        assert!(browser.posts_loading && browser.post_request.is_some());
    }
    browser.on_event(Event::PostsFailed {
        group_id: 2,
        search: "landscape".into(),
        after: None,
        error: "posts: timeout".into(),
    });
    assert!(!browser.posts_loading && browser.post_request.is_none());
}

mod errors;
mod gallery;
mod header_cancel;
mod library;
mod options;
mod speeds;
mod unsubscribe;

#[test]
fn gallery_ram_cache_is_bounded_and_drops_evicted_decode_results() {
    let mut browser = browser();
    let bytes: Arc<[u8]> = vec![0; 9 * 1024 * 1024].into();
    for id in 1..=7 {
        browser.on_memory_preview(id, format!("{id}.jpg"), bytes.clone());
    }
    let stale = browser.media_memory[&1].key(1, false);
    browser.request_media(&[2]);
    browser.on_memory_preview(8, "8.jpg".into(), bytes.clone());
    assert!(!browser.media_memory.contains_key(&1));
    browser.posts = vec![post(1, 1)];
    browser.request_media(&[1]);
    assert!(
        !browser.media_pending.contains(&1),
        "an evicted visible tile must not refetch every frame"
    );
    assert!(browser.media_memory.contains_key(&2));
    assert!(browser.media_memory_bytes <= 64 * 1024 * 1024);
    assert!(browser.jobs.is_empty() && browser.media_paths.is_empty());
    browser
        .pixel_tx
        .send(Pixels {
            key: stale.clone(),
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        })
        .unwrap();
    browser.pump_pixels(&egui::Context::default());
    assert!(!browser.textures.contains_key(&stale));
    browser.preferences_applied.preview_cache_mib = 8;
    browser.trim_memory_previews();
    assert!(browser.media_memory.is_empty() && browser.media_memory_bytes == 0);
}

#[test]
fn gallery_memory_events_ignore_stale_context_without_consuming_pending_requests() {
    let mut browser = browser();
    browser.media_generation = 4;
    browser.media_pending.insert(1);
    browser.on_event(Event::PreviewMediaReady {
        post_id: 1,
        generation: 3,
        name: "old.jpg".into(),
        bytes: Arc::from(&b"old"[..]),
    });
    browser.on_event(Event::PreviewMediaFailed {
        post_id: 1,
        generation: 3,
        error: "old error".into(),
    });
    assert!(browser.media_pending.contains(&1));
    assert!(browser.media_memory.is_empty() && browser.media_errors.is_empty());
    browser.on_event(Event::PreviewMediaProgress {
        post_id: 1,
        generation: 4,
        received: 2,
        total: 3,
    });
    assert_eq!(browser.media_progress.get(&1), Some(&(2, 3)));
    browser.on_event(Event::PreviewMediaReady {
        post_id: 1,
        generation: 4,
        name: "new.jpg".into(),
        bytes: Arc::from(&b"new"[..]),
    });
    assert!(!browser.media_pending.contains(&1));
    assert!(browser.media_progress.is_empty());
    assert_eq!(&*browser.media_memory[&1].bytes, b"new");
}

#[test]
fn network_status_combines_pools_and_removes_retired_pool_activity() {
    let mut browser = browser();
    for (server_id, pool_id, active, bytes_per_second) in
        [(1, 1, 2, 1024.0), (2, 2, 3, 2048.0), (1, 3, 1, 512.0)]
    {
        browser.on_event(Event::NetworkStats {
            server_id,
            pool_id,
            open: active,
            active,
            bytes_per_second,
        });
    }
    assert_eq!(
        browser
            .network_stats
            .values()
            .map(|(active, _, _)| active)
            .sum::<usize>(),
        6
    );
    assert_eq!(
        browser
            .network_stats
            .values()
            .map(|(_, _, bps)| bps)
            .sum::<f64>(),
        3584.0
    );
    browser.on_event(Event::NetworkStats {
        server_id: 1,
        pool_id: 1,
        open: 0,
        active: 0,
        bytes_per_second: 0.0,
    });
    assert_eq!(browser.network_stats.len(), 2);
    assert_eq!(browser.network_stats[&(1, 3)], (1, 1, 512.0));
    browser.on_event(Event::NetworkStats {
        server_id: 1,
        pool_id: 3,
        open: 4,
        active: 0,
        bytes_per_second: 0.0,
    });
    assert_eq!(
        browser.network_stats[&(1, 3)],
        (0, 4, 0.0),
        "idle open sockets remain visible"
    );
}

#[test]
fn removing_download_history_keeps_cached_files_but_file_deletion_clears_disk_previews() {
    let mut browser = browser();
    browser.jobs.push(JobRow {
        id: 5,
        post_id: Some(1),
        name: "Image".into(),
        dest_dir: "/tmp/nntpbrowser-owned/Image-1".into(),
        dest_path: "/tmp/nntpbrowser-owned/Image-1/image.jpg".into(),
        state: "done".into(),
        bytes_done: 3,
        bytes_total: 3,
        error: None,
        password: None,
    });
    browser
        .media_paths
        .insert(1, PathBuf::from(&browser.jobs[0].dest_path));
    browser
        .thumb_paths
        .insert(1, PathBuf::from("/tmp/nntpbrowser-owned/thumbs/1.jpg"));
    browser.on_memory_preview(1, "image.jpg".into(), Arc::from(&b"ram"[..]));
    browser.on_event(Event::DownloadDeleted {
        job_id: 5,
        remove_files: false,
    });
    assert!(browser.media_paths.contains_key(&1) && browser.thumb_paths.contains_key(&1));
    browser.on_event(Event::DownloadDeleted {
        job_id: 5,
        remove_files: true,
    });
    assert!(!browser.media_paths.contains_key(&1) && !browser.thumb_paths.contains_key(&1));
    assert_eq!(&*browser.media_memory[&1].bytes, b"ram");
}
