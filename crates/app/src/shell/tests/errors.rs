use super::*;

#[test]
fn scoped_error_history_survives_success_and_deduplicates_its_notice() {
    let mut browser = browser();
    let mut configured = server(1);
    configured.password = Some("private-server-password".into());
    browser.servers = vec![configured];
    browser.groups = vec![group(2, 1)];
    browser.on_event(Event::HeaderFetchFinished {
        group_id: 2,
        error: Some("overview: authentication failed private-server-password".into()),
    });
    browser.on_event(Event::Notice(
        "overview: authentication failed private-server-password".into(),
    ));
    assert_eq!(browser.error_history.len(), 1);
    assert_eq!(browser.error_count, 1);
    let entry = browser.error_history.back().unwrap();
    assert!(entry.context.contains("alt.test.2"));
    assert!(!entry.message.contains("private-server-password"));
    assert!(entry.timestamp > 0);
    browser.on_event(Event::Notice("queued Error document.pdf (1 parts)".into()));
    assert_eq!(browser.error_history.len(), 1);
    for index in 0..205 {
        browser.record_error(&format!("Failure {index}"), "Application");
    }
    assert_eq!(browser.error_history.len(), 200);
    assert_eq!(browser.error_count, 206);
    assert_eq!(browser.error_history.front().unwrap().message, "Failure 5");
}

#[test]
fn failed_job_history_redacts_first_snapshot_and_ignores_repeated_snapshots() {
    let mut browser = browser();
    let job = JobRow {
        id: 5,
        post_id: None,
        name: "Downloaded file".into(),
        dest_dir: "".into(),
        dest_path: "".into(),
        state: "failed".into(),
        bytes_done: 0,
        bytes_total: 100,
        error: Some("extraction failed with new-job-secret".into()),
        password: Some("new-job-secret".into()),
    };
    browser.on_event(Event::Jobs(vec![job.clone()]));
    assert_eq!(browser.error_history.len(), 1);
    assert!(!browser.error_history[0].message.contains("new-job-secret"));
    browser.on_event(Event::Jobs(vec![job.clone()]));
    assert_eq!(browser.error_history.len(), 1);
    browser.error_history.clear();
    browser.on_event(Event::Jobs(vec![job]));
    assert!(browser.error_history.is_empty());
}

#[test]
fn ram_preview_error_history_ignores_stale_generations_and_records_save_failures() {
    let mut browser = browser();
    browser.media_generation = 4;
    browser.posts = vec![post(7, 1)];
    browser.form.password = "preview-secret".into();
    browser.observe_error(&Event::PreviewMediaFailed {
        post_id: 7,
        generation: 3,
        error: "stale preview failed".into(),
    });
    assert!(browser.error_history.is_empty());
    browser.observe_error(&Event::PreviewMediaFailed {
        post_id: 7,
        generation: 4,
        error: "preview failed preview-secret".into(),
    });
    assert_eq!(browser.error_history.len(), 1);
    assert_eq!(browser.error_history[0].context, "Media preview · Image 7");
    assert!(!browser.error_history[0].message.contains("preview-secret"));
    browser.observe_error(&Event::MediaSaveFailed {
        post_id: 7,
        error: "save media: permission denied".into(),
    });
    assert_eq!(browser.error_history.len(), 2);
    assert_eq!(browser.error_history[1].context, "Save media · Image 7");
}
