use super::*;

fn job(state: &str, done: i64, total: i64) -> JobRow {
    JobRow {
        id: 1,
        post_id: Some(1),
        name: "Example file".into(),
        dest_dir: "/tmp".into(),
        dest_path: "/tmp/example".into(),
        state: state.into(),
        bytes_done: done,
        bytes_total: total,
        error: None,
        password: None,
    }
}

#[test]
fn transfer_progress_distinguishes_unknown_size_and_processing() {
    let transfer = JobProgress::new(&job("fetching", 25, 100));
    assert_eq!(transfer.fraction, 0.25);
    assert_eq!(transfer.detail, "25% transferred");
    let unknown = JobProgress::new(&job("fetching", 500, 0));
    assert_eq!(unknown.fraction, 0.0);
    assert!(unknown.animate);
    assert_eq!(unknown.detail, "Size unknown");
    for state in [
        "decoding",
        "repair",
        "extract",
        "recovery_headers",
        "recovery_fetching",
    ] {
        let processing = JobProgress::new(&job(state, 100, 100));
        assert_eq!(processing.fraction, 0.0);
        assert!(processing.animate);
        assert!(!processing.detail.contains("100%"));
    }
    let done = JobProgress::new(&job("done", 100, 100));
    assert_eq!(done.fraction, 1.0);
    assert!(!done.animate);
}

#[test]
fn recovery_remains_active_and_header_discovery_cannot_look_complete() {
    let headers = job("recovery_headers", 100, 100);
    let recovery = job("recovery_fetching", 100, 200);
    assert!(is_active(&headers.state));
    assert!(is_active(&recovery.state));
    assert_eq!(JobProgress::new(&recovery).fraction, 0.0);
    let pending_headers = AggregateProgress::active(std::slice::from_ref(&headers));
    assert_eq!(pending_headers.active, 1);
    assert_eq!(pending_headers.fraction, None);
    assert!(pending_headers.animate);
    let both = AggregateProgress::active(&[headers, recovery, job("done", 1000, 1000)]);
    assert_eq!(both.active, 2);
    assert_eq!(both.count, 2);
    assert_eq!(both.fraction, None);
    assert!(both.stages.contains("finding missing parts"));
    assert!(both.stages.contains("fetching repair data"));
}

#[test]
fn aggregate_tracks_pending_bytes_without_history_or_false_completion() {
    let aggregate = AggregateProgress::new(&[
        job("fetching", 25, 100),
        job("queued", 0, 100),
        job("done", 1_000, 1_000),
        job("cancelled", 1_000, 1_000),
    ]);
    assert_eq!(aggregate.count, 2);
    assert_eq!(aggregate.fraction, Some(0.125));
    let unknown = AggregateProgress::new(&[job("fetching", 25, 100), job("queued", 0, 0)]);
    assert_eq!(unknown.fraction, None);
    assert!(unknown.detail.contains("unknown"));
    let processing = AggregateProgress::new(&[job("repair", 100, 100), job("extract", 100, 100)]);
    assert_eq!(processing.fraction, None);
    assert_eq!(processing.compact, "Processing");
    assert!(processing.animate);
    let paused = AggregateProgress::new(&[job("paused", 25, 100)]);
    assert!(!paused.animate);
    assert_eq!(paused.color, theme::WARN);
    let failed = AggregateProgress::new(&[job("failed", 25, 100)]);
    assert_eq!(failed.color, theme::DANGER);
}

#[test]
fn live_transfer_summary_excludes_paused_failed_cancelled_and_completed_history() {
    let progress = AggregateProgress::active(&[
        job("fetching", 25, 100),
        job("queued", 0, 100),
        job("paused", 500, 1000),
        job("failed", 900, 1000),
        job("done", 1000, 1000),
        job("cancelled", 1000, 1000),
    ]);
    assert_eq!(progress.active, 2);
    assert_eq!(progress.count, 2);
    assert_eq!(progress.fraction, Some(0.125));
    assert!(!progress.stages.contains("failed"));
    assert!(!progress.stages.contains("paused"));
}
