use super::*;
use store::{NewSegment, SegmentRow};

const POSTER: &str = "Poster <poster@test>";
const POSTED: i64 = 1_700_000_000;

fn record(name: &str, part: u32, total: u32, article: u64) -> Record {
    Record {
        article,
        date_unix: POSTED,
        date_raw: None,
        bytes: 100,
        lines: 1,
        message_id: format!("{article}@test"),
        subject: format!("Set [01/03] - \"{name}\" yEnc ({part}/{total})"),
        from: POSTER.into(),
    }
}

fn known(name: &str, part: i64, state: &str) -> SegmentRow {
    SegmentRow {
        id: part,
        job_id: 1,
        filename: name.into(),
        part,
        message_id: format!("known{part}@test"),
        begin: 0,
        end: 0,
        article: Some(100_000),
        state: state.into(),
        server_id: Some(1),
    }
}

fn discovery(segments: &[SegmentRow]) -> Discovery {
    let key = store::parse_subject(&record("set.rar", 1, 3, 100_000).subject, POSTER).group_key;
    Discovery::new(&key, POSTER, &["set.rar".into()], segments)
        .unwrap()
        .with_posted_at(Some(POSTED))
}

#[test]
fn matching_requires_original_identity_poster_exact_stem_time_and_safe_name() {
    let mut discovery = discovery(&[known("set.rar", 1, "written")]);
    for name in [
        "set2.par2",
        "set-extra.par2",
        "../set.par2",
        "set.vol1+0.par2",
        "set.vol1+2junk.par2",
    ] {
        discovery.ingest(record(name, 1, 1, 100_001));
    }
    let mut wrong_poster = record("set.par2", 1, 1, 100_001);
    wrong_poster.from = "Other poster".into();
    discovery.ingest(wrong_poster);
    let mut wrong_title = record("set.par2", 1, 1, 100_001);
    wrong_title.subject = wrong_title.subject.replace("Set [", "Other set [");
    discovery.ingest(wrong_title);
    let mut old = record("set.par2", 1, 1, 100_001);
    old.date_unix -= 8 * 86_400;
    discovery.ingest(old);
    let mut unknown_date = record("set.par2", 1, 1, 100_001);
    unknown_date.date_unix = 0;
    discovery.ingest(unknown_date);
    for id in [
        "bad id@test".into(),
        "bad\u{0007}@test".into(),
        "x".repeat(498),
    ] {
        let mut invalid = record("set.par2", 1, 1, 100_001);
        invalid.message_id = id;
        discovery.ingest(invalid);
    }
    assert!(discovery.partial_candidates().is_empty());
    discovery.ingest(record("set.par2", 1, 1, 100_001));
    discovery.ingest(record("set.VOL000+004.PAR2", 1, 1, 100_002));
    let candidates = discovery.partial_candidates();
    assert_eq!(candidates.len(), 2);
    assert!(candidates
        .iter()
        .any(|candidate| candidate.recovery_blocks == Some(4)));
}

#[test]
fn known_parity_stem_overrides_inference_and_complete_parts_survive_restart() {
    let mut original = discovery(&[known("authoritative.par2", 1, "pending")]);
    original.ingest(record("set.par2", 1, 1, 100_001));
    original.ingest(record("authoritative.vol0+2.par2", 2, 2, 100_002));
    assert!(original
        .partial_candidates()
        .iter()
        .all(|candidate| candidate.segments.len() < candidate.expected_parts as usize));
    let partial = original.partial_candidates();
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].expected_parts, 2);
    let durable = partial
        .iter()
        .flat_map(|candidate| {
            candidate
                .segments
                .iter()
                .cloned()
                .map(|segment| (segment, candidate.expected_parts))
        })
        .collect::<Vec<_>>();
    let mut restarted = discovery(&[known("authoritative.par2", 1, "pending")]);
    restarted.seed_candidates(&durable).unwrap();
    restarted.ingest(record("authoritative.vol0+2.par2", 1, 2, 100_001));
    let complete = restarted.partial_candidates();
    assert_eq!(complete.len(), 1);
    assert_eq!(
        complete[0]
            .segments
            .iter()
            .map(|segment| segment.part)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    restarted.ingest(record("authoritative.vol0+2.par2", 1, 3, 100_003));
    assert!(restarted.partial_candidates().is_empty());
}

#[test]
fn missing_payload_headers_allow_alternates_but_never_repeat_finished_parts() {
    let mut discovery = discovery(&[
        known("set.rar", 1, "written"),
        known("set.rar", 2, "missing"),
    ]);
    discovery.ingest(record("set.rar", 1, 3, 100_001));
    discovery.ingest(record("set.rar", 2, 3, 100_002));
    discovery.ingest(record("set.rar", 3, 3, 100_003));
    discovery.ingest(record("set2.rar", 3, 3, 100_004));
    let mut old_missing = record("set.rar", 2, 3, 100_000);
    old_missing.message_id = "known2@test".into();
    discovery.ingest(old_missing);
    assert_eq!(
        discovery
            .missing_payload_segments()
            .iter()
            .map(|segment| segment.part)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[test]
fn nearest_duplicate_header_and_single_part_without_counter_are_supported() {
    let mut discovery = discovery(&[known("set.rar", 1, "written")]);
    discovery.ingest(record("set.par2", 1, 1, 95_000));
    let mut near = record("set.par2", 1, 1, 100_001);
    near.subject = "Set [01/03] - \"set.par2\" yEnc".into();
    discovery.ingest(near);
    let candidate = &discovery.partial_candidates()[0];
    assert_eq!(candidate.expected_parts, 1);
    assert_eq!(candidate.segments[0].article, Some(100_001));
}

#[test]
fn progressive_plan_skips_cached_and_durable_empty_ranges_and_is_finite() {
    let attempted = [(90_000, 95_000)];
    let stored = [(99_000, 101_000)];
    let plan = DiscoveryPlan::new(&[100_000], 1, 2_000_000, &stored, &attempted);
    assert!(!plan.windows.is_empty());
    let mut seen = std::collections::BTreeSet::new();
    for &(low, high) in &plan.windows {
        assert!(high >= low && high - low < OVERVIEW_BATCH);
        for article in low..=high {
            assert!(seen.insert(article), "duplicate request for {article}");
            assert!(!stored
                .iter()
                .chain(&attempted)
                .any(|&(first, last)| first <= article && article <= last));
        }
    }
    assert!(seen.len() as u64 <= SEARCH_BUDGET - 5_001);
    assert!(plan.windows.len() <= 200);
    assert!(plan.windows[0].1 <= 110_000); // Nearby before expanding outward.
    let exhausted = DiscoveryPlan::new(&[100_000], 1, 2_000_000, &[], &[(1, SEARCH_BUDGET)]);
    assert!(exhausted.windows.is_empty());
}

#[test]
fn empty_or_extreme_ranges_do_not_overflow_or_expand_unboundedly() {
    assert!(DiscoveryPlan::new(&[], 0, u64::MAX, &[], &[])
        .windows
        .is_empty());
    for anchor in [0, u64::MAX] {
        let plan = DiscoveryPlan::new(&[anchor], 0, u64::MAX, &[], &[]);
        assert!(plan.windows.len() <= 200);
        assert!(plan
            .windows
            .iter()
            .all(|(low, high)| high >= low && high - low < OVERVIEW_BATCH));
    }
}

#[test]
fn invalid_seeded_candidate_metadata_is_rejected() {
    let mut discovery = discovery(&[]);
    let segment = NewSegment {
        filename: "set.vol0+2.par2".into(),
        part: 3,
        message_id: "3@test".into(),
        begin: 0,
        end: 0,
        article: Some(100_001),
    };
    assert!(discovery.seed_candidates(&[(segment, 2)]).is_err());
}

#[test]
fn exact_posting_title_can_name_the_index_but_its_neighboring_prefix_cannot() {
    let header = record("unrelated-file.bin", 1, 2, 100_000);
    let key = store::parse_subject(&header.subject, POSTER).group_key;
    let mut discovery = Discovery::new(&key, POSTER, &["unrelated-file.bin".into()], &[]).unwrap();
    discovery.ingest(record("set2.par2", 1, 1, 100_001));
    assert!(discovery.partial_candidates().is_empty());
    discovery.ingest(record("Set.par2", 1, 1, 100_002));
    assert_eq!(
        discovery.partial_candidates()[0].segments[0].filename,
        "Set.par2"
    );
}

#[tokio::test]
async fn real_overview_batch_retains_only_requested_articles_and_rejects_provider_oversend() {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    for mode in 0..3 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(b"201 ready\r\n").await.unwrap();
            let (read, mut write) = socket.into_split();
            let mut reader = BufReader::new(read);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                match line.trim() {
                    "CAPABILITIES" => write
                        .write_all(b"101 caps\r\nVERSION 2\r\nOVER\r\n.\r\n")
                        .await
                        .unwrap(),
                    "GROUP alt.test" => write.write_all(b"211 2 10 11 alt.test\r\n").await.unwrap(),
                    "LIST OVERVIEW.FMT" => write
                        .write_all(b"500 use standard columns\r\n")
                        .await
                        .unwrap(),
                    "OVER 10-10" => {
                        if mode == 2 {
                            write
                                .write_all(b"423 no articles in this range\r\n")
                                .await
                                .unwrap();
                            continue;
                        }
                        write.write_all(b"224 overview\r\n").await.unwrap();
                        let mut response = String::new();
                        for index in 0..if mode == 1 { 5_001 } else { 2 } {
                            let article = if mode == 1 { 10 } else { 10 + index };
                            response.push_str(&format!("{article}\tSet - \"set.par2\" yEnc\t{POSTER}\tTue, 14 Nov 2023 22:13:20 +0000\t<{index}@test>\t\t10\t1\r\n"));
                        }
                        // An oversending peer never terminates its response;
                        // the bounded helper must discard it instead of drain.
                        if mode != 1 {
                            response.push_str(".\r\n");
                        }
                        write.write_all(response.as_bytes()).await.unwrap();
                    }
                    other => panic!("unexpected recovery command: {other}"),
                }
            }
        });
        let pool = Pool::new(nntp::ServerEndpoint {
            id: 1,
            config: nntp::Config {
                host: "127.0.0.1".into(),
                port,
                tls: nntp::TlsMode::Plain,
                username: None,
                password: None,
                timeout: Duration::from_secs(2),
            },
            priority: 1,
            fill: false,
            max_connections: 1,
            pipeline_depth: 1,
        });
        pool.set_header_strategy(nntp::HeaderStrategy::Plain).await;
        pool.group("alt.test").await.unwrap();
        let result = overview_batch(&pool, 10, 10).await;
        if mode == 1 {
            assert!(result.unwrap_err().contains("bounded header batch"));
        } else if mode == 2 {
            assert!(result.unwrap().is_empty());
        } else {
            let records = result.unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].article, 10);
            assert_eq!(records[0].date_unix, POSTED);
        }
        drop(pool);
        server.await.unwrap();
    }
}
