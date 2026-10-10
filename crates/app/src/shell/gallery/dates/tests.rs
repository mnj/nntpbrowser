use super::*;

#[test]
fn custom_range_includes_last_second_and_rejects_invalid_calendar_dates() {
    let mut dates = DateFilter {
        preset: Preset::Custom,
        from_text: "2024-02-29".into(),
        until_text: "2024-02-29".into(),
        ..Default::default()
    };
    dates.apply_custom();
    let (from, until) = dates.bounds();
    assert_eq!(until.unwrap() - from.unwrap(), DAY - 1);
    assert_eq!(day_label(until.unwrap()), "2024-02-29");
    let valid = dates.bounds();
    dates.from_text = "2023-02-29".into();
    dates.apply_custom();
    assert!(dates.error.is_some());
    assert_eq!(dates.bounds(), valid);
    dates.from_text = "2024-03-01".into();
    dates.apply_custom();
    assert!(dates.error.as_ref().unwrap().contains("before"));
    assert_eq!(dates.bounds(), valid);
    dates.from_text = "2024-2-29".into();
    dates.apply_custom();
    assert!(dates.error.is_some());
}

#[test]
fn presets_use_utc_calendar_days_and_clear_releases_bounds() {
    let now = parse_day("2026-10-10").unwrap().unwrap() + 42_000;
    let mut dates = DateFilter::default();
    for (preset, from) in [
        (Preset::Today, "2026-10-10"),
        (Preset::Week, "2026-10-04"),
        (Preset::Month, "2026-09-11"),
    ] {
        dates.preset = preset;
        dates.apply_preset(now);
        assert_eq!(day_label(dates.from.unwrap()), from);
        assert_eq!(dates.until.unwrap(), now - 42_000 + DAY - 1);
    }
    dates.clear();
    assert_eq!(dates.bounds(), (None, None));
    dates.preset = Preset::Custom;
    dates.until_text = "2026-10-10".into();
    dates.apply_custom();
    assert_eq!(dates.from, None);
    assert_eq!(dates.until.unwrap(), now - 42_000 + DAY - 1);
}

#[test]
fn applying_dates_hides_nine_hundred_day_old_thumbnails_before_query_completes() {
    use crate::shell::{BrowseView, PostRequest};
    let now = parse_day("2026-10-10").unwrap().unwrap() + 42_000;
    let mut browser = crate::shell::tests::browser();
    browser.view = BrowseView::Gallery;
    browser.selected_group = Some(1);
    let post = |id, posted_at| engine::PostRow {
        id,
        posted_at,
        group_id: 1,
        group_key: id.to_string(),
        title: "Image".into(),
        poster: "Reader".into(),
        total_bytes: 1000,
        file_count: 1,
        segment_count: 1,
        incomplete: false,
        has_image: true,
        has_video: false,
        has_par2: false,
    };
    browser.posts = vec![post(1, now - 900 * DAY), post(2, now - 3 * DAY), post(3, 0)];
    assert_eq!(browser.filtered_posts().len(), 3);
    browser.gallery_dates.preset = Preset::Month;
    browser.gallery_dates.apply_preset(now);
    browser.reload_posts();
    assert!(browser.posts_loading);
    assert_eq!(browser.filtered_posts().as_slice(), &[1]);
    let (from, until) = browser.gallery_dates.bounds();
    assert!(
        browser.post_request
            == Some(PostRequest {
                group_id: 1,
                search: String::new(),
                after: None,
                from,
                until,
            })
    );
    browser.on_event(engine::Event::PostsDatePage {
        group_id: 1,
        search: String::new(),
        after: None,
        from: None,
        until: None,
        posts: vec![post(1, now - 900 * DAY)],
    });
    assert_eq!(browser.filtered_posts().as_slice(), &[1]);
    assert!(browser.posts_loading);
    browser.on_event(engine::Event::PostsDatePage {
        group_id: 1,
        search: String::new(),
        after: None,
        from,
        until,
        posts: vec![post(2, now - 3 * DAY)],
    });
    assert_eq!(browser.posts.len(), 1);
    assert!(!browser.posts_loading);
}

#[test]
fn automatic_header_fetch_preserves_search_and_dates_and_ignores_unfiltered_page() {
    use engine::Event;
    let mut browser = crate::shell::tests::browser();
    browser.view = crate::shell::BrowseView::Gallery;
    browser.search = "landscape".into();
    browser.gallery_dates.preset = Preset::Month;
    browser.gallery_dates.apply_preset(1_800_000_000);
    browser.pick_group(1, 1);
    let request = browser.post_request.clone().unwrap();
    browser.on_event(Event::PostsDatePage {
        group_id: 1,
        search: request.search.clone(),
        after: None,
        from: request.from,
        until: request.until,
        posts: vec![],
    });
    assert!(!browser.posts_loading);
    browser.on_event(Event::HeaderProgress {
        group_id: 1,
        received: 0,
        stored: 0,
        first: 0,
        last: 0,
        completed: 0,
        total: 0,
    });
    assert!(browser.header_progress.is_some());
    // Header synchronization publishes an unfiltered page. It must not
    // replace the active Gallery query, even when that query returned empty.
    browser.on_event(Event::PostsPage {
        group_id: 1,
        search: String::new(),
        after: None,
        posts: vec![],
    });
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: None,
    });
    assert!(browser.header_progress.is_none());
    assert!(browser.posts_loading);
    assert!(browser.post_request.as_ref() == Some(&request));
}

#[test]
fn cancelling_headers_reloads_current_date_query_without_recording_an_error() {
    let mut browser = crate::shell::tests::browser();
    browser.view = crate::shell::BrowseView::Gallery;
    browser.selected_group = Some(1);
    browser.search = "landscape".into();
    browser.gallery_dates.preset = Preset::Month;
    browser.gallery_dates.apply_preset(1_800_000_000);
    browser.reload_posts();
    let query = browser.post_request.clone();
    browser.on_event(engine::Event::HeaderProgress {
        group_id: 1,
        received: 0,
        stored: 0,
        first: 0,
        last: 0,
        completed: 0,
        total: 0,
    });
    browser.header_cancel_pending = true;
    browser.on_event(engine::Event::HeaderFetchCancelled { group_id: 1 });
    assert!(browser.header_progress.is_none() && !browser.header_cancel_pending);
    assert!(browser.post_request == query && browser.posts_loading);
    assert_eq!(browser.error_count, 0);
}
