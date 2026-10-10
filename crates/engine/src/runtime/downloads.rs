//! Article queueing, segment downloads, and previews.

use super::{Command, Worker};
use nntp::ArticleRef;

impl Worker {
    pub(super) async fn drive_one(&mut self) -> bool {
        if self.network_timeout_dirty && self.transfers.is_empty() && self.preview_tasks.is_empty()
        {
            self.pools.clear();
            self.network_timeout_dirty = false;
        }
        let previews = self.drive_previews().await;
        self.drive_downloads().await || previews
    }

    pub(super) fn fail_job(&self, id: i64, error: String) {
        let _ = self.catalog.set_job_state(id, "failed", Some(&error));
        self.emit_jobs();
        self.notice(format!("job {id} failed: {error}"));
        if let Ok(jobs) = self.catalog.list_jobs() {
            if let Some(job) = jobs
                .iter()
                .find(|job| job.id == id && self.media_job_limit(job).is_some())
            {
                if let Some(post_id) = job.post_id {
                    let _ = self
                        .evt
                        .send(super::Event::MediaCacheFailed { post_id, error });
                }
            }
        }
    }

    pub(super) async fn body_from_farm(
        &mut self,
        id: &ArticleRef,
    ) -> Result<(i64, Vec<u8>), nntp::Error> {
        self.body_from_farm_limited(id, usize::MAX).await
    }

    pub(super) async fn body_from_farm_limited(
        &mut self,
        id: &ArticleRef,
        max_bytes: usize,
    ) -> Result<(i64, Vec<u8>), nntp::Error> {
        let mut servers = self.catalog.list_servers().unwrap_or_default();
        servers.retain(|server| server.enabled);
        servers.sort_by_key(|server| (server.fill, server.priority));
        for server in &servers {
            self.ensure_pool(server).await;
        }
        let mut last = nntp::Error::Missing;
        if servers.is_empty() {
            return Err(nntp::Error::Connect("no servers configured".into()));
        }
        for server in &servers {
            let pool = self.pools.get(&server.id).unwrap();
            match pool.body_limited(id, max_bytes).await {
                Ok(bytes) => return Ok((server.id, bytes)),
                Err(nntp::Error::Missing) => last = nntp::Error::Missing,
                Err(err) => return Err(err),
            }
        }
        Err(last)
    }

    pub(super) fn poll_controls(&mut self) {
        while let Ok(command) = self.cmd_rx.try_recv() {
            match command {
                Command::CancelHeaderFetch(id) => self.route_header_cancel(id),
                Command::Pause(id) => self.pause(id),
                Command::Resume(id) => self.resume(id),
                Command::Cancel(id) => self.cancel(id),
                Command::PreviewContext(generation) => self.set_preview_context(generation),
                Command::PreviewMedia {
                    post_id,
                    max_bytes,
                    generation,
                } => self.preview_media(post_id, max_bytes, generation),
                Command::RetainPreviewMedia {
                    post_ids,
                    generation,
                } => self.retain_preview_media(&post_ids, generation),
                other => self.deferred.push_back(other),
            }
        }
    }

    pub(super) fn pause(&mut self, id: i64) {
        let generation = self.transfer_generation.entry(id).or_default();
        *generation = generation.wrapping_add(1);
        self.paused.insert(id);
        self.cancelled.remove(&id);
        let _ = self.catalog.set_job_state(id, "paused", None);
        self.emit_jobs();
    }

    pub(super) fn resume(&mut self, id: i64) {
        let generation = self.transfer_generation.entry(id).or_default();
        *generation = generation.wrapping_add(1);
        self.paused.remove(&id);
        self.cancelled.remove(&id);
        if let Ok(segments) = self.catalog.segments_for_job(id) {
            for segment in segments
                .iter()
                .filter(|segment| matches!(segment.state.as_str(), "failed" | "missing"))
            {
                let _ = self.catalog.set_segment_state(segment.id, "pending", None);
            }
        }
        let _ = self.catalog.set_job_state(id, "queued", None);
        self.emit_jobs();
    }

    pub(super) fn cancel(&mut self, id: i64) {
        let generation = self.transfer_generation.entry(id).or_default();
        *generation = generation.wrapping_add(1);
        self.cancelled.insert(id);
        self.paused.remove(&id);
        let _ = self.catalog.set_job_state(id, "cancelled", None);
        self.emit_jobs();
    }
}
