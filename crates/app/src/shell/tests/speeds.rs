use super::*;

fn job(id: i64, bytes_done: i64) -> JobRow {
    JobRow {
        id,
        post_id: None,
        name: "Transfer".into(),
        dest_dir: String::new(),
        dest_path: String::new(),
        state: "fetching".into(),
        bytes_done,
        bytes_total: 2_000_000,
        error: None,
        password: None,
    }
}

#[test]
fn job_snapshots_baseline_resumed_bytes_prune_removed_ids_and_reset_rollbacks() {
    let mut browser = browser();
    let start = Instant::now();
    browser.note_speeds_at(&[job(1, 1_000_000), job(2, 500)], start);
    assert_eq!(browser.speeds[&1].bytes_per_second(start), 0.0);

    let next = start + Duration::from_secs(1);
    browser.note_speeds_at(&[job(1, 1_010_000), job(2, 5_500)], next);
    assert_eq!(browser.speeds[&1].bytes_per_second(next), 2_000.0);
    assert_eq!(browser.speeds[&2].bytes_per_second(next), 1_000.0);

    // Retrying a job can reset its byte counter; an unrelated removed job must
    // also disappear from the estimator cache in the next complete snapshot.
    let retry = start + Duration::from_secs(2);
    browser.note_speeds_at(&[job(1, 100)], retry);
    assert_eq!(browser.speeds[&1].bytes_per_second(retry), 0.0);
    assert!(!browser.speeds.contains_key(&2));

    let resumed = start + Duration::from_secs(3);
    browser.note_speeds_at(&[job(1, 5_100)], resumed);
    assert_eq!(browser.speeds[&1].bytes_per_second(resumed), 1_000.0);
    // The row's current-time query expires an idle transfer without Jobs events.
    assert_eq!(
        browser.speeds[&1].bytes_per_second(start + Duration::from_secs(9)),
        0.0
    );

    browser.note_speeds_at(&[], start + Duration::from_secs(10));
    assert!(browser.speeds.is_empty());
    let reused = start + Duration::from_secs(11);
    browser.note_speeds_at(&[job(1, 1_500_000)], reused);
    assert_eq!(browser.speeds[&1].bytes_per_second(reused), 0.0);
}
