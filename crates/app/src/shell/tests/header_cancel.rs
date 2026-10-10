use super::*;

#[test]
fn header_cancellation_keeps_active_search_and_does_not_record_an_error() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1)];
    browser.pick_group(1, 1);
    browser.view = BrowseView::Gallery;
    browser.search = "landscape".into();
    browser.reload_posts();
    let query = browser.post_request.clone();
    browser.fetch_selected_headers();
    browser.header_cancel_pending = true;
    browser.on_event(Event::HeaderFetchCancelled { group_id: 2 });
    assert!(browser.header_progress.is_some() && browser.header_cancel_pending);
    browser.on_event(Event::HeaderFetchCancelled { group_id: 1 });
    assert!(browser.header_progress.is_none() && !browser.header_cancel_pending);
    assert!(browser.post_request == query && browser.posts_loading);
    assert!(browser.error_history.is_empty() && browser.error_count == 0);
    assert!(browser.notice.contains("Cached headers kept"));
}

#[test]
fn header_task_stays_cancellable_after_switching_groups() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1), group(2, 1)];
    browser.pick_group(1, 1);
    browser.fetch_selected_headers();
    browser.pick_group(2, 1);
    let next_group_query = browser.post_request.clone();
    assert_eq!(browser.header_task_group, Some(1));
    assert!(browser.header_progress.is_some());
    // The sidebar action cannot replace the active task's cancellation target.
    browser.fetch_selected_headers();
    assert_eq!(browser.header_task_group, Some(1));
    browser.on_event(Event::HeaderProgress {
        group_id: 1,
        received: 5000,
        stored: 4000,
        first: 1,
        last: 10_000,
        completed: 5000,
        total: 10_000,
    });
    assert_eq!(browser.header_progress.as_ref().unwrap().stored, 4000);
    browser.header_cancel_pending = true;
    browser.on_event(Event::HeaderFetchCancelled { group_id: 1 });
    assert!(browser.header_task_group.is_none() && browser.header_progress.is_none());
    assert!(!browser.header_cancel_pending);
    assert!(browser.post_request == next_group_query && browser.posts_loading);
    assert_eq!(browser.selected_group, Some(2));
    assert_eq!(browser.error_count, 0);
}
