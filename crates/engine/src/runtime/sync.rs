//! Incremental header sync and bounded historical backfill.

use super::{now_unix, Event, Worker};
use std::collections::HashMap;
use store::{parse_overview_date, parse_subject, IncomingFile, IncomingPost, Record};

impl Worker {
    pub(super) fn load_posts_in_date_range(
        &self,
        group_id: i64,
        after: Option<(i64, i64)>,
        search: &str,
        from: Option<i64>,
        until: Option<i64>,
    ) {
        let fail = |error: String| {
            self.notice(error.clone());
            let _ = self.evt.send(Event::PostsDateFailed {
                group_id,
                after,
                search: search.to_owned(),
                from,
                until,
                error,
            });
        };
        if matches!((from, until), (Some(from), Some(until)) if from > until) {
            fail("posts: start date must be before or equal to end date".into());
            return;
        }
        if self.catalog.group_by_id(group_id).ok().flatten().is_none() {
            fail("posts: group was not found".into());
            return;
        }
        match self.catalog.page_posts_date_range(
            group_id,
            after,
            80,
            (!search.is_empty()).then_some(search),
            from,
            until,
        ) {
            Ok(posts) => {
                let jobs = self.catalog.list_jobs().unwrap_or_default();
                for post in &posts {
                    if let Ok(Some(thumb)) = self.catalog.thumb_for_post(post.id) {
                        let path = std::path::PathBuf::from(thumb.path);
                        if path.is_file() {
                            let _ = self.evt.send(Event::Thumb {
                                post_id: post.id,
                                path,
                            });
                        }
                    }
                    self.emit_cached_media_from_jobs(post.id, &jobs);
                }
                let _ = self.evt.send(Event::PostsDatePage {
                    group_id,
                    after,
                    search: search.to_owned(),
                    from,
                    until,
                    posts,
                });
            }
            Err(error) => fail(format!("posts: {error}")),
        }
    }

    pub(super) fn load_posts(&self, group_id: i64, after: Option<(i64, i64)>, search: &str) {
        if self.catalog.group_by_id(group_id).ok().flatten().is_none() {
            self.posts_failed(group_id, after, search, "posts: group was not found".into());
            return;
        }
        let search = if search.is_empty() {
            None
        } else {
            Some(search)
        };
        match self.catalog.page_posts(group_id, after, 80, search) {
            Ok(posts) => {
                let jobs = self.catalog.list_jobs().unwrap_or_default();
                for post in &posts {
                    if let Ok(Some(thumb)) = self.catalog.thumb_for_post(post.id) {
                        let path = std::path::PathBuf::from(thumb.path);
                        if path.is_file() {
                            let _ = self.evt.send(Event::Thumb {
                                post_id: post.id,
                                path,
                            });
                        }
                    }
                    self.emit_cached_media_from_jobs(post.id, &jobs);
                }
                let _ = self.evt.send(Event::PostsPage {
                    group_id,
                    search: search.unwrap_or_default().to_owned(),
                    after,
                    posts,
                });
            }
            Err(err) => self.posts_failed(
                group_id,
                after,
                search.unwrap_or_default(),
                format!("posts: {err}"),
            ),
        }
    }

    fn posts_failed(&self, group_id: i64, after: Option<(i64, i64)>, search: &str, error: String) {
        self.notice(error.clone());
        let _ = self.evt.send(Event::PostsFailed {
            group_id,
            search: search.to_owned(),
            after,
            error,
        });
    }

    pub(super) async fn sync_group(&mut self, group_id: i64) {
        self.sync_headers(group_id, None, true).await;
    }

    pub(super) async fn fetch_headers(&mut self, group_id: i64, limit: Option<u64>) {
        if limit == Some(0) {
            let error = "choose at least one article to fetch".to_string();
            self.notice(error.clone());
            let _ = self.evt.send(Event::HeaderFetchFinished {
                group_id,
                error: Some(error),
            });
            return;
        }
        self.sync_headers(group_id, limit, false).await;
    }

    pub(super) async fn sync_headers(
        &mut self,
        group_id: i64,
        limit: Option<u64>,
        incremental: bool,
    ) {
        self.active_header_fetch = Some(group_id);
        self.header_fetch_cancelled = false;
        self.consume_deferred_header_cancel(group_id);
        self.poll_controls();
        let result = if self.header_fetch_cancelled {
            Ok(())
        } else {
            self.sync_headers_inner(group_id, limit, incremental).await
        };
        self.active_header_fetch = None;
        if std::mem::take(&mut self.header_fetch_cancelled) {
            let _ = self.evt.send(Event::HeaderFetchCancelled { group_id });
            return;
        }
        let error = result.err();
        if let Some(error) = &error {
            self.notice(error.clone());
        }
        let _ = self
            .evt
            .send(Event::HeaderFetchFinished { group_id, error });
    }

    async fn sync_headers_inner(
        &mut self,
        group_id: i64,
        limit: Option<u64>,
        incremental: bool,
    ) -> Result<(), String> {
        self.headers
            .recover_catalog(&self.catalog)
            .map_err(|err| format!("recover headers: {err}"))?;
        let Some((server, group)) = self.find_group(group_id) else {
            return Err("group is not subscribed".into());
        };
        self.ensure_pool(&server).await;
        let server_id = server.id;
        let group_name = group.name.clone();
        let gid = group.id;
        let pool = self.pools.get(&server_id).cloned().expect("server pool");
        let Some(status) = self.wait_header(Box::pin(pool.group(&group_name))).await else {
            return Ok(());
        };
        let status = match status {
            Ok(status) => status,
            Err(err) => {
                return Err(format!("group: {err}"));
            }
        };
        // GROUP's count is only an estimate. Providers may return zero while
        // still exposing a populated low/high article range.
        if status.high < status.low || status.high == 0 {
            let _ = self.catalog.set_group_marks(
                gid,
                status.low as i64,
                status.high as i64,
                group.watermark,
                Some(now_unix()),
            );
            self.notice(format!("{group_name} is empty"));
            self.emit_groups(server_id);
            self.load_posts(gid, None, "");
            return Ok(());
        }
        let watermark = match self.headers.watermark(server_id, &group_name) {
            Ok(mark) => mark,
            Err(err) => {
                return Err(format!("watermark: {err}"));
            }
        };
        let start = if incremental {
            status.low.max(watermark.saturating_add(1))
        } else {
            limit
                .map(|n| status.high.saturating_sub(n.saturating_sub(1)))
                .unwrap_or(status.low)
                .max(status.low)
        };
        let ranges = match self.headers.stored_ranges(server_id, &group_name) {
            Ok(ranges) => missing_ranges(start, status.high, &ranges),
            Err(err) => {
                return Err(format!("header ranges: {err}"));
            }
        };
        let total = ranges.iter().fold(0u64, |total, &(first, last)| {
            total.saturating_add(last.saturating_sub(first).saturating_add(1))
        });
        let mut completed = 0u64;
        let _ = self.evt.send(Event::HeaderProgress {
            group_id,
            received: 0,
            stored: 0,
            completed,
            total,
            first: start,
            last: status.high,
        });
        if ranges.is_empty() {
            self.headers
                .compact_if_needed(server_id, &group_name)
                .map_err(|err| format!("compact headers: {err}"))?;
            let _ = self.catalog.set_group_marks(
                gid,
                status.low as i64,
                status.high as i64,
                watermark as i64,
                Some(now_unix()),
            );
            self.notice(format!("{group_name} is up to date"));
            self.emit_groups(server_id);
            self.load_posts(gid, None, "");
            return Ok(());
        }
        let mut stored = 0u64;
        let mut received = 0u64;
        let pool = self.pools.get(&server_id).cloned().expect("server pool");
        let mut strategy = pool.header_strategy().await;
        let mut failed = None;
        for (first, last) in ranges {
            // Bound compressed responses too: XZVER is buffered at the wire layer.
            let mut first = first;
            loop {
                let chunk_last = first.saturating_add(19_999).min(last);
                let prior_completed = completed;
                let window_len = chunk_last.saturating_sub(first).saturating_add(1);
                let mut records = Vec::with_capacity(2000);
                let mut posts: HashMap<String, IncomingPost> = HashMap::new();
                let mut write_error = None;
                let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel();
                let fetch = Box::pin(pool.fetch_overview(first, chunk_last, move |line| {
                    let _ = line_tx.send(line);
                }));
                let result = self
                    .wait_header_stream(fetch, line_rx, |worker, line| {
                        if write_error.is_some()
                            || line.article < first
                            || line.article > chunk_last
                        {
                            return;
                        }
                        let record = record_from_line(&line);
                        completed = completed.max(
                            prior_completed
                                .saturating_add(line.article - first + 1)
                                .min(total),
                        );
                        received += 1;
                        absorb_post(&mut posts, &record);
                        records.push(record);
                        if records.len() >= 2000 {
                            let n = records.len() as u64;
                            match worker.flush_sync(
                                server_id,
                                &group_name,
                                gid,
                                &mut records,
                                &mut posts,
                            ) {
                                Ok(()) => stored += n,
                                Err(err) => write_error = Some(err),
                            }
                        }
                        if received.is_multiple_of(250) {
                            let _ = worker.evt.send(Event::HeaderProgress {
                                group_id,
                                received,
                                stored,
                                completed,
                                total,
                                first: start,
                                last: status.high,
                            });
                        }
                    })
                    .await;
                let Some(result) = result else {
                    // Only complete durable batches are retained. The trailing
                    // in-memory records must not turn an unfinished range into
                    // a cached span on cancellation.
                    let _ = self.evt.send(Event::HeaderProgress {
                        group_id,
                        received,
                        stored,
                        completed,
                        total,
                        first: start,
                        last: status.high,
                    });
                    self.emit_groups(server_id);
                    return Ok(());
                };
                if let Some(err) = write_error {
                    failed = Some(format!("sync write: {err}"));
                    break;
                }
                // A partial response remains useful. Persist the trailing records
                // before reporting a failure so the next fetch can resume.
                let n = records.len() as u64;
                if let Err(err) =
                    self.flush_sync(server_id, &group_name, gid, &mut records, &mut posts)
                {
                    failed = Some(format!("sync write: {err}"));
                    break;
                }
                stored += n;
                if result.is_ok() || matches!(&result, Err(nntp::Error::Missing)) {
                    completed = prior_completed.saturating_add(window_len).min(total);
                }
                let _ = self.evt.send(Event::HeaderProgress {
                    group_id,
                    received,
                    stored,
                    completed,
                    total,
                    first: start,
                    last: status.high,
                });
                match result {
                    Ok(found) => strategy = found,
                    Err(nntp::Error::Missing) => {}
                    Err(err) => {
                        failed = Some(format!("overview: {err}"));
                        break;
                    }
                }
                self.poll_controls();
                if self.header_fetch_cancelled {
                    return Ok(());
                }
                // Dispatch current viewport intents between network windows. Header
                // and preview requests share the same bounded connection pool.
                self.drive_previews().await;
                if chunk_last == last {
                    break;
                }
                first = chunk_last + 1;
            }
            if failed.is_some() {
                break;
            }
        }
        // Include the final partial offset when a storage error interrupted a
        // window before its normal completion event could be emitted.
        let _ = self.evt.send(Event::HeaderProgress {
            group_id,
            received,
            stored,
            completed,
            total,
            first: start,
            last: status.high,
        });
        let high_mark = self
            .headers
            .watermark(server_id, &group_name)
            .unwrap_or(watermark);
        let _ = self.catalog.set_group_marks(
            gid,
            status.low as i64,
            status.high as i64,
            high_mark as i64,
            Some(now_unix()),
        );
        let _ = self
            .catalog
            .set_header_compression(server_id, strategy.as_str());
        self.emit_groups(server_id);
        self.load_posts(gid, None, "");
        if let Some(err) = failed {
            Err(err)
        } else {
            // Compact once after all durable batch journals have cleared.
            // Doing this per batch repeatedly recompresses the growing prefix.
            self.headers
                .compact_if_needed(server_id, &group_name)
                .map_err(|err| format!("headers saved but compaction failed: {err}"))?;
            self.notice(format!(
                "synced {stored} articles from {group_name} via {}",
                strategy.as_str()
            ));
            Ok(())
        }
    }

    pub(super) fn flush_sync(
        &self,
        server_id: i64,
        group: &str,
        group_id: i64,
        records: &mut Vec<Record>,
        posts: &mut HashMap<String, IncomingPost>,
    ) -> Result<(), String> {
        if !records.is_empty() {
            let batch: Vec<IncomingPost> = posts.values().cloned().collect();
            self.headers
                .append_catalog_batch(&self.catalog, server_id, group_id, group, records, &batch)
                .map_err(|err| err.to_string())?;
            records.clear();
            posts.clear();
        }
        Ok(())
    }
}

fn missing_ranges(first: u64, last: u64, stored: &[(u64, u64)]) -> Vec<(u64, u64)> {
    if first > last {
        return Vec::new();
    }
    let mut cursor = first;
    let mut missing = Vec::new();
    for &(low, high) in stored {
        if high < cursor {
            continue;
        }
        if low > last {
            break;
        }
        if low > cursor {
            missing.push((cursor, low - 1));
        }
        if high >= last {
            return missing;
        }
        cursor = cursor.max(high + 1);
    }
    if cursor <= last {
        missing.push((cursor, last));
    }
    missing
}

fn record_from_line(line: &nntp::OverviewLine) -> Record {
    let date_unix = parse_overview_date(&line.date).unwrap_or(0);
    Record {
        article: line.article,
        date_unix,
        date_raw: if date_unix == 0 && !line.date.is_empty() {
            Some(line.date.clone())
        } else {
            None
        },
        bytes: line.bytes,
        lines: u32::try_from(line.lines).unwrap_or(u32::MAX),
        message_id: line.message_id.clone(),
        subject: line.subject.clone(),
        from: line.from.clone(),
    }
}

fn absorb_post(posts: &mut HashMap<String, IncomingPost>, record: &Record) {
    let parsed = parse_subject(&record.subject, &record.from);
    let filename = parsed
        .filename
        .clone()
        .unwrap_or_else(|| parsed.title.clone());
    let expected = i64::from(parsed.total.unwrap_or(0));
    let entry = posts
        .entry(parsed.group_key.clone())
        .or_insert_with(|| IncomingPost {
            group_key: parsed.group_key.clone(),
            title: parsed.title.clone(),
            poster: record.from.clone(),
            posted_at: record.date_unix,
            files: Vec::new(),
        });
    if record.date_unix > entry.posted_at {
        entry.posted_at = record.date_unix;
    }
    entry.title = parsed.title;
    if let Some(file) = entry.files.iter_mut().find(|file| file.name == filename) {
        file.size = file
            .size
            .saturating_add(i64::try_from(record.bytes).unwrap_or(i64::MAX));
        file.expected_segments = file.expected_segments.max(expected);
        file.kind = parsed.kind;
    } else {
        entry.files.push(IncomingFile {
            name: filename,
            size: i64::try_from(record.bytes).unwrap_or(i64::MAX),
            expected_segments: expected,
            kind: parsed.kind,
        });
    }
}
