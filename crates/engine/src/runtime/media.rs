//! Durable, size-limited Gallery downloads using the ordinary job queue.

use super::{Event, Worker};
use crate::decode::Part;
use std::collections::HashMap;
use std::path::Path;
use store::{FileKind, JobRow, SegmentRow};

pub(super) const MEDIA_CAP: u64 = 10 * 1024 * 1024;

impl Worker {
    pub(super) fn cache_media(&mut self, post_id: i64, max_bytes: u64) {
        self.emit_cached_media(post_id);
        let result = self.queue_media(post_id, max_bytes.min(MEDIA_CAP));
        if let Err(error) = result {
            let _ = self.evt.send(Event::MediaCacheFailed { post_id, error });
        }
    }

    fn queue_media(&self, post_id: i64, cap: u64) -> Result<(), String> {
        if cap == 0 {
            return Err("media cache limit must be positive".into());
        }
        let Some((server, group, post)) = self.find_post(post_id) else {
            return Err("article was not found".into());
        };
        let jobs = self.catalog.list_jobs().map_err(|err| err.to_string())?;
        if let Some(job) = jobs.iter().find(|job| job.post_id == Some(post_id)) {
            self.emit_jobs();
            return if matches!(job.state.as_str(), "failed" | "paused" | "cancelled") {
                Err(format!(
                    "existing download is {}; resume or retry it manually",
                    job.state
                ))
            } else {
                Ok(())
            };
        }
        let files = self
            .catalog
            .files_for_post(post_id)
            .map_err(|err| err.to_string())?;
        let eligible: HashMap<_, _> = files
            .iter()
            .filter(|file| {
                matches!(file.kind, FileKind::Image | FileKind::Video)
                    && file.size > 0
                    && (file.size as u64) < cap
            })
            .map(|file| (file.name.as_str(), file))
            .collect();
        if eligible.is_empty() {
            return Err(
                "no complete image or video with a known size within the cache limit".into(),
            );
        }
        let mut segments = self.segments_for_key(server.id, &group.name, &post.group_key)?;
        segments.retain(|segment| eligible.contains_key(segment.filename.as_str()));
        segments.dedup_by(|a, b| a.filename == b.filename && a.part == b.part);
        let complete: HashMap<_, _> = eligible
            .into_iter()
            .filter(|(name, file)| {
                let parts = segments
                    .iter()
                    .filter(|segment| segment.filename == *name)
                    .collect::<Vec<_>>();
                parts.len() as i64 == file.expected_segments.max(1)
                    && parts
                        .iter()
                        .enumerate()
                        .all(|(index, segment)| segment.part == index as i64 + 1)
            })
            .collect();
        segments.retain(|segment| complete.contains_key(segment.filename.as_str()));
        if segments.is_empty() {
            return Err("media headers are incomplete; fetch more headers first".into());
        }
        let total = complete
            .values()
            .fold(0i64, |sum, file| sum.saturating_add(file.size));
        let dest = self
            .data_dir
            .join("media-cache")
            .join(post_id.to_string())
            .join(cap.to_string())
            .join(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    .to_string(),
            );
        let id = self
            .catalog
            .create_job(Some(post_id), &post.title, &dest, None, &segments)
            .map_err(|err| err.to_string())?;
        self.catalog
            .set_job_progress(id, 0, total)
            .map_err(|err| err.to_string())?;
        self.emit_jobs();
        Ok(())
    }

    pub(super) fn media_job_limit(&self, job: &JobRow) -> Option<u64> {
        let relative = Path::new(&job.dest_dir)
            .strip_prefix(self.data_dir.join("media-cache"))
            .ok()?;
        let mut components = relative.iter();
        let post_id = components.next()?.to_str()?.parse::<i64>().ok()?;
        let limit = components.next()?.to_str()?.parse::<u64>().ok()?;
        let _nonce = components.next()?.to_str()?.parse::<u128>().ok()?;
        if components.next().is_some()
            || job.post_id != Some(post_id)
            || limit == 0
            || limit > MEDIA_CAP
        {
            return None;
        }
        Some(limit)
    }

    pub(super) fn emit_cached_media(&self, post_id: i64) {
        let Ok(jobs) = self.catalog.list_jobs() else {
            return;
        };
        self.emit_cached_media_from_jobs(post_id, &jobs);
    }

    pub(super) fn emit_cached_media_from_jobs(&self, post_id: i64, jobs: &[JobRow]) {
        if !jobs
            .iter()
            .any(|job| job.post_id == Some(post_id) && job.state == "done")
        {
            return;
        }
        let Ok(files) = self.catalog.files_for_post(post_id) else {
            return;
        };
        for job in jobs
            .iter()
            .filter(|job| job.post_id == Some(post_id) && job.state == "done")
        {
            for file in &files {
                if !matches!(file.kind, FileKind::Image | FileKind::Video) {
                    continue;
                }
                let path = Path::new(&job.dest_dir).join(&file.name);
                if std::fs::symlink_metadata(&path)
                    .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
                {
                    let _ = self.evt.send(Event::MediaCached { post_id, path });
                }
            }
        }
    }

    pub(super) fn validate_media_part(
        &self,
        job: &JobRow,
        segment: &SegmentRow,
        part: &Part,
        cap: u64,
    ) -> Result<(), String> {
        let end = part
            .begin
            .checked_add(part.data.len() as u64)
            .ok_or("media part offset overflow")?;
        if part.data.is_empty()
            || end > cap
            || part
                .file_size
                .is_some_and(|size| size == 0 || size > cap || end > size)
        {
            return Err("decoded media exceeds its cache limit or declared file size".into());
        }
        let prior = self
            .catalog
            .segments_for_job(job.id)
            .map_err(|err| err.to_string())?
            .iter()
            .filter(|other| {
                other.id != segment.id
                    && other.filename == segment.filename
                    && other.state == "written"
            })
            .fold(0u64, |sum, other| {
                sum.saturating_add(other.end.saturating_sub(other.begin).max(0) as u64)
            });
        if prior.saturating_add(part.data.len() as u64) > cap {
            return Err("media parts exceed the cache limit".into());
        }
        Ok(())
    }

    pub(super) fn finish_media_job(
        &self,
        job: &JobRow,
        segments: &[SegmentRow],
        cap: u64,
    ) -> Result<(), String> {
        let mut files: HashMap<&str, Vec<&SegmentRow>> = HashMap::new();
        for segment in segments {
            if segment.state != "written" {
                return Err("media download has missing or failed parts".into());
            }
            files.entry(&segment.filename).or_default().push(segment);
        }
        for (filename, parts) in &mut files {
            parts.sort_by_key(|part| part.begin);
            let mut end = 0;
            for part in parts {
                if part.begin != end || part.end <= part.begin {
                    return Err("media parts have gaps or overlapping offsets".into());
                }
                end = part.end;
            }
            let path = Path::new(&job.dest_dir).join(filename);
            let meta = std::fs::symlink_metadata(path).map_err(|err| err.to_string())?;
            if !meta.is_file() || meta.len() == 0 || meta.len() > cap || meta.len() != end as u64 {
                return Err("media file is incomplete or exceeds the cache limit".into());
            }
        }
        self.write_thumbs(Path::new(&job.dest_dir), job.post_id);
        self.catalog
            .set_job_progress(job.id, job.bytes_done, job.bytes_done)
            .map_err(|err| err.to_string())?;
        self.catalog
            .set_job_state(job.id, "done", None)
            .map_err(|err| err.to_string())?;
        self.emit_jobs();
        if let Some(post_id) = job.post_id {
            self.emit_cached_media(post_id);
        }
        Ok(())
    }
}
