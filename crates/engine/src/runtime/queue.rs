//! Queue construction and bounded bulk downloads.

use super::recovery::split_parity;
use super::Worker;
use crate::nzb;
use std::collections::HashMap;
use std::path::Path;
use store::{parse_subject, NewSegment};

impl Worker {
    pub(super) fn queue_post(&mut self, post_id: i64) {
        let Some((server, group, post)) = self.find_post(post_id) else {
            self.notice("article was not found".into());
            return;
        };
        let segments = match self.segments_for_key(server.id, &group.name, &post.group_key) {
            Ok(segments) => segments,
            Err(err) => {
                self.notice(format!("queue: {err}"));
                return;
            }
        };
        if segments.is_empty() {
            self.notice("no headers matched this article".into());
            return;
        }
        let dest = self
            .downloads
            .join(format!("{}-{}", sanitize(&post.title), post.id));
        let context = match self.recovery_source(Some(post.id)) {
            Ok(context) => context,
            Err(error) => {
                self.notice(format!("queue: {error}"));
                return;
            }
        };
        let expected = match self.catalog.files_for_post(post.id) {
            Ok(files) => files
                .into_iter()
                .map(|file| (file.name, file.expected_segments))
                .collect::<HashMap<_, _>>(),
            Err(error) => {
                self.notice(format!("queue: {error}"));
                return;
            }
        };
        let (active, candidates) = split_parity(&segments, &expected);
        match self.catalog.create_job_with_recovery(
            Some(post.id),
            &post.title,
            &dest,
            None,
            &active,
            Some(&context),
            &candidates,
        ) {
            Ok(id) => {
                self.notice(format!("queued {} ({} parts)", post.title, segments.len()));
                let _ = id;
                self.emit_jobs();
            }
            Err(err) => self.notice(format!("queue: {err}")),
        }
    }

    pub(super) async fn queue_group(&mut self, group_id: i64, limit: Option<u64>, search: &str) {
        let Some((server, group)) = self.find_group(group_id) else {
            self.notice("group is not subscribed".into());
            return;
        };
        match self.catalog.queue_group_headers(
            &self.headers,
            server.id,
            group_id,
            &group.name,
            limit,
            (!search.is_empty()).then_some(search),
            &self.downloads,
        ) {
            Ok(queued) => {
                self.emit_jobs();
                self.notice(format!("queued {queued} articles from {}", group.name));
            }
            Err(err) => self.notice(format!("queue: {err}")),
        }
        self.poll_controls();
    }

    pub(super) fn import_nzb(&mut self, xml: &str, dest: &Path) {
        let parsed = match nzb::parse(xml) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.notice(format!("nzb: {err}"));
                return;
            }
        };
        let mut segments = Vec::new();
        let mut expected = HashMap::new();
        for file in &parsed.files {
            let parsed_name = parse_subject(&file.subject, &file.poster);
            let filename = parsed_name
                .filename
                .unwrap_or_else(|| parsed_name.title.clone());
            expected.insert(
                filename.clone(),
                i64::from(parsed_name.total.unwrap_or(file.segments.len() as u32)),
            );
            for segment in &file.segments {
                let bytes = segment.bytes;
                let begin = segments
                    .iter()
                    .filter(|existing: &&NewSegment| existing.filename == filename)
                    .map(|existing| existing.end)
                    .max()
                    .unwrap_or(0);
                segments.push(NewSegment {
                    filename: filename.clone(),
                    part: segment.number as i64,
                    message_id: segment.message_id.clone(),
                    begin,
                    end: begin + bytes as i64,
                    article: None,
                });
            }
        }
        let name = parsed
            .files
            .first()
            .map(|file| parse_subject(&file.subject, &file.poster).title)
            .unwrap_or_else(|| "nzb".into());
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let clean: String = sanitize(&name).chars().take(120).collect();
        let dest = dest.join(format!("{clean}-{nonce}"));
        let (active, candidates) = split_parity(&segments, &expected);
        let context = match self.recovery_source(None) {
            Ok(context) => context,
            Err(error) => {
                self.notice(format!("nzb queue: {error}"));
                return;
            }
        };
        match self.catalog.create_job_with_recovery(
            None,
            &name,
            &dest,
            parsed.password.as_deref(),
            &active,
            Some(&context),
            &candidates,
        ) {
            Ok(_) => {
                self.notice(format!("imported NZB, {} segments", segments.len()));
                self.emit_jobs();
            }
            Err(err) => self.notice(format!("nzb queue: {err}")),
        }
    }

    pub(super) fn segments_for_key(
        &self,
        server_id: i64,
        group: &str,
        group_key: &str,
    ) -> Result<Vec<NewSegment>, String> {
        let mut segments = Vec::new();
        let bad = self
            .headers
            .for_each(server_id, group, |record| {
                let parsed = parse_subject(&record.subject, &record.from);
                if parsed.group_key != group_key {
                    return;
                }
                let filename = parsed.filename.unwrap_or(parsed.title);
                segments.push(NewSegment {
                    filename,
                    part: i64::from(parsed.part.unwrap_or(1)),
                    message_id: record.message_id,
                    begin: 0,
                    end: 0,
                    article: i64::try_from(record.article).ok(),
                });
            })
            .map_err(|err| err.to_string())?;
        if !bad.is_empty() {
            return Err("stored headers contain damaged blocks; download was not queued".into());
        }
        segments.sort_by(|a, b| (&a.filename, a.part).cmp(&(&b.filename, b.part)));
        Ok(segments)
    }
}

pub(super) fn sanitize(title: &str) -> String {
    let mut out = String::new();
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' ') {
            out.push(ch);
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        "article".into()
    } else {
        out
    }
}
