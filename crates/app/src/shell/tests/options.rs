use super::*;

#[test]
fn header_fetch_failure_shows_its_diagnostic_and_never_cancels_another_group() {
    let mut browser = browser();
    let mut configured = server(1);
    configured.password = Some("secret-reader-password".into());
    browser.servers = vec![configured];
    browser.groups = vec![group(1, 1)];
    browser.pick_group(1, 1);
    browser.fetch_selected_headers();
    browser.notice = "Fetching headers…".into();
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 2,
        error: Some("overview: different group failed".into()),
    });
    assert!(browser.posts_loading);
    assert_eq!(browser.notice, "Fetching headers…");
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: Some("overview: authentication failed secret-reader-password".into()),
    });
    assert!(!browser.posts_loading);
    assert!(browser.notice.contains("authentication failed"));
    assert!(!browser.notice.contains("secret-reader-password"));
}

#[test]
fn saving_a_new_disabled_server_preserves_the_checkbox_in_the_catalog_and_form() {
    let mut browser = browser();
    // Native CI may contend during cold engine startup. Keep each asynchronous
    // phase bounded independently so startup cannot consume the save budget.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let events = browser.handle.poll();
        let ready = events
            .iter()
            .any(|event| matches!(event, Event::Servers(_)));
        for event in events {
            browser.on_event(event);
        }
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine startup timed out; status: {}",
            browser.notice
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    browser.form.name = "Disabled server".into();
    browser.form.host = "localhost".into();
    browser.form.enabled = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    browser.save_server();
    loop {
        for event in browser.handle.poll() {
            browser.on_event(event);
        }
        if !browser.servers.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "server save timed out; status: {}",
            browser.notice
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(browser.servers.len(), 1);
    assert!(!browser.servers[0].enabled);
    assert!(!browser.form.enabled);
    assert_eq!(browser.editing, Some(browser.servers[0].id));
}

#[test]
fn header_progress_only_tracks_the_active_group_and_clears_after_its_failure() {
    let mut browser = browser();
    browser.groups = vec![group(1, 1), group(2, 2)];
    browser.pick_group(1, 1);
    browser.fetch_selected_headers();
    browser.on_event(Event::HeaderProgress {
        group_id: 2,
        received: 500,
        stored: 400,
        first: 100,
        last: 700,
        completed: 500,
        total: 601,
    });
    assert_eq!(browser.header_progress.as_ref().unwrap().received, 0);
    browser.on_event(Event::HeaderProgress {
        group_id: 1,
        received: 200,
        stored: 150,
        first: 10,
        last: 1000,
        completed: 400,
        total: 991,
    });
    let progress = browser.header_progress.as_ref().unwrap();
    assert_eq!(
        (
            progress.received,
            progress.stored,
            progress.first,
            progress.last
        ),
        (200, 150, 10, 1000)
    );
    assert_eq!((progress.completed, progress.total), (400, 991));
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 2,
        error: Some("timeout".into()),
    });
    assert!(browser.header_progress.is_some() && browser.posts_loading);
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 1,
        error: Some("overview: timeout".into()),
    });
    assert!(browser.header_progress.is_none() && !browser.posts_loading);
    assert_eq!(browser.notice, "overview: timeout");
}
