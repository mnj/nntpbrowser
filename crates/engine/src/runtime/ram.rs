//! Gallery previews have no disk or download-queue side effects.

use super::{Event, Worker};
use std::{
    path::Path,
    sync::{Arc, Weak},
};
use store::NewSegment;

mod fetch;
mod headers;
use fetch::fetch_preview;

const MAX_PREVIEW: u64 = 10 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    host: String,
    port: u16,
    tls: bool,
    starttls: bool,
    group: String,
    key: String,
    name: String,
    parts: Vec<(i64, String)>,
}

pub(super) struct CachedMedia {
    post_id: i64,
    identity: Identity,
    bytes: Weak<[u8]>,
}

pub(super) struct PreviewIntent {
    post_id: i64,
    generation: u64,
    max_bytes: u64,
}

pub(super) struct PreviewRequest {
    post_id: i64,
    generation: u64,
    cap: u64,
    size: u64,
    identity: Identity,
    segments: Vec<NewSegment>,
}

pub(super) struct PreviewResult {
    request: PreviewRequest,
    bytes: Result<Arc<[u8]>, String>,
}

pub(super) struct ActivePreview {
    post_id: i64,
    generation: u64,
    abort: tokio::task::AbortHandle,
}

impl Worker {
    pub(super) fn stop_legacy_media_jobs(&self) {
        if let Ok(jobs) = self.catalog.list_jobs() {
            for job in jobs.iter().filter(|job| {
                matches!(job.state.as_str(), "queued" | "fetching")
                    && Path::new(&job.dest_dir).starts_with(self.data_dir.join("media-cache"))
            }) {
                let _ = self.catalog.set_job_state(
                    job.id,
                    "cancelled",
                    Some("Gallery previews now stay in memory until Save"),
                );
            }
        }
    }

    pub(super) fn set_preview_context(&mut self, generation: u64) {
        if self.preview_context == generation {
            return;
        }
        self.preview_context = generation;
        self.preview_wanted = None;
        self.preview_tasks.abort_all();
        self.preview_active.clear();
        self.preview_queue.clear();
        self.preview_pending.clear();
    }

    pub(super) fn purge_group_previews(&mut self, post_ids: &std::collections::HashSet<i64>) {
        self.preview_cache
            .retain(|entry| !post_ids.contains(&entry.post_id));
        self.preview_queue
            .retain(|intent| !post_ids.contains(&intent.post_id));
        self.preview_pending
            .retain(|(post_id, _)| !post_ids.contains(post_id));
        if let Some(wanted) = &mut self.preview_wanted {
            wanted.retain(|post_id| !post_ids.contains(post_id));
        }
        self.preview_active.retain(|_, active| {
            if post_ids.contains(&active.post_id) {
                active.abort.abort();
                false
            } else {
                true
            }
        });
        // Completed JoinSet replies are ignored when their task ID no longer
        // exists in preview_active, including if SQLite reuses a removed post ID.
    }

    pub(super) fn retain_preview_media(&mut self, post_ids: &[i64], generation: u64) {
        if generation != self.preview_context {
            return;
        }
        let wanted: std::collections::HashMap<i64, usize> = post_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(rank, post_id)| (post_id, rank))
            .collect();
        self.preview_wanted = Some(wanted.keys().copied().collect());
        self.preview_queue.retain(|request| {
            request.generation == generation && wanted.contains_key(&request.post_id)
        });
        self.preview_queue
            .make_contiguous()
            .sort_by_key(|request| wanted[&request.post_id]);
        self.preview_active.retain(|_, active| {
            if active.generation == generation && wanted.contains_key(&active.post_id) {
                true
            } else {
                active.abort.abort();
                false
            }
        });
        self.preview_pending = self
            .preview_queue
            .iter()
            .map(|request| (request.post_id, request.generation))
            .chain(
                self.preview_active
                    .values()
                    .map(|active| (active.post_id, active.generation)),
            )
            .collect();
    }

    pub(super) fn preview_media(&mut self, post_id: i64, max_bytes: u64, generation: u64) {
        if !self.preview_is_wanted(post_id, generation)
            || self.preview_pending.contains(&(post_id, generation))
        {
            return;
        }
        // Queue only intent. Header scans and cache identity validation happen after
        // command draining, so newer viewport retention can discard obsolete work.
        self.preview_cache
            .retain(|entry| entry.bytes.strong_count() > 0);
        if self.preview_queue.len() >= 128
            || self.preview_cache.len() + self.preview_pending.len() >= 256
        {
            self.preview_failed(post_id, generation, "preview request queue is full".into());
            return;
        }
        self.preview_pending.insert((post_id, generation));
        self.preview_queue.push_back(PreviewIntent {
            post_id,
            generation,
            max_bytes,
        });
    }

    fn preview_is_wanted(&self, post_id: i64, generation: u64) -> bool {
        generation == self.preview_context
            && self
                .preview_wanted
                .as_ref()
                .is_none_or(|wanted| wanted.contains(&post_id))
    }

    pub(super) async fn drive_previews(&mut self) -> bool {
        let mut worked = false;
        while let Some(result) = self.preview_tasks.try_join_next_with_id() {
            worked = true;
            match result {
                Ok((task_id, result)) => {
                    if self.preview_active.remove(&task_id).is_none() {
                        continue;
                    }
                    let request = result.request;
                    self.preview_pending
                        .remove(&(request.post_id, request.generation));
                    if !self.preview_is_wanted(request.post_id, request.generation) {
                        continue;
                    }
                    match result.bytes {
                        Ok(bytes) => {
                            self.preview_cache
                                .retain(|entry| entry.post_id != request.post_id);
                            self.preview_cache.push_back(CachedMedia {
                                post_id: request.post_id,
                                identity: request.identity.clone(),
                                bytes: Arc::downgrade(&bytes),
                            });
                            // The UI owns strong references. Never evict metadata for a live
                            // preview: Save must remain available while its bytes are visible.
                            self.preview_cache
                                .retain(|entry| entry.bytes.strong_count() > 0);
                            let _ = self.evt.send(Event::PreviewMediaReady {
                                post_id: request.post_id,
                                generation: request.generation,
                                name: request.identity.name,
                                bytes,
                            });
                        }
                        Err(error) => {
                            self.preview_failed(request.post_id, request.generation, error)
                        }
                    }
                }
                Err(error) => {
                    if let Some(active) = self.preview_active.remove(&error.id()) {
                        self.preview_pending
                            .remove(&(active.post_id, active.generation));
                        if !error.is_cancelled() && active.generation == self.preview_context {
                            self.preview_failed(
                                active.post_id,
                                active.generation,
                                format!("preview worker: {error}"),
                            );
                        }
                    }
                }
            }
        }
        if self.preview_queue.is_empty() {
            return worked;
        }
        let mut servers = self.catalog.list_servers().unwrap_or_default();
        servers.retain(|server| server.enabled);
        servers.sort_by_key(|server| (server.fill, server.priority));
        for server in &servers {
            self.ensure_pool(server).await;
        }
        let pools = servers
            .iter()
            .filter_map(|server| self.pools.get(&server.id).cloned())
            .collect::<Vec<_>>();
        let capacity = pools
            .iter()
            .fold(0usize, |sum, pool| {
                sum.saturating_add(pool.max_connections())
            })
            .max(1);
        self.poll_controls();
        let mut intents = Vec::new();
        while intents.len() < capacity.saturating_sub(self.preview_active.len()) {
            let Some(intent) = self.preview_queue.pop_front() else {
                break;
            };
            worked = true;
            let post_id = intent.post_id;
            let generation = intent.generation;
            if !self.preview_is_wanted(post_id, generation) {
                self.preview_pending.remove(&(post_id, generation));
                continue;
            }
            intents.push(intent);
        }
        let generation = self.preview_context;
        let prepared = self.prepare_preview_batch(intents);
        // Retention may have changed while a large OSF group was scanned. Do
        // not launch a request that the newer viewport has already discarded.
        self.poll_controls();
        for (post_id, result) in prepared {
            if !self.preview_is_wanted(post_id, generation) {
                continue;
            }
            if self
                .preview_queue
                .iter()
                .any(|intent| intent.post_id == post_id && intent.generation == generation)
            {
                continue;
            }
            self.preview_pending.insert((post_id, generation));
            let request = match result {
                Ok(request) => request,
                Err(error) => {
                    self.preview_pending.remove(&(post_id, generation));
                    self.preview_failed(post_id, generation, error);
                    continue;
                }
            };
            if let Some(bytes) = self
                .preview_cache
                .iter()
                .find(|entry| entry.post_id == post_id && entry.identity == request.identity)
                .and_then(|entry| entry.bytes.upgrade())
                .filter(|bytes| bytes.len() as u64 <= request.cap)
            {
                self.preview_pending.remove(&(post_id, generation));
                let _ = self.evt.send(Event::PreviewMediaReady {
                    post_id,
                    generation,
                    name: request.identity.name,
                    bytes,
                });
                continue;
            }
            let abort =
                self.preview_tasks
                    .spawn(fetch_preview(request, pools.clone(), self.evt.clone()));
            self.preview_active.insert(
                abort.id(),
                ActivePreview {
                    post_id,
                    generation,
                    abort,
                },
            );
            worked = true;
        }
        worked
    }

    fn preview_failed(&self, post_id: i64, generation: u64, error: String) {
        let _ = self.evt.send(Event::PreviewMediaFailed {
            post_id,
            generation,
            error,
        });
    }

    pub(super) fn save_preview_media(&mut self, post_id: i64, generation: u64) {
        let result = self.save_memory_media(post_id, generation);
        match result {
            Ok(path) => {
                self.notice(format!("saved media to {}", path.display()));
                let _ = self.evt.send(Event::MediaSaved { post_id, path });
                self.emit_jobs();
            }
            Err(error) => {
                let _ = self.evt.send(Event::MediaSaveFailed { post_id, error });
            }
        }
    }

    fn save_memory_media(
        &self,
        post_id: i64,
        generation: u64,
    ) -> Result<std::path::PathBuf, String> {
        if generation != self.preview_context {
            return Err("preview context changed; reopen this media before saving".into());
        }
        let request = self.prepare_preview(post_id, MAX_PREVIEW, generation)?;
        let bytes = self
            .preview_cache
            .iter()
            .find(|entry| entry.post_id == post_id && entry.identity == request.identity)
            .and_then(|entry| entry.bytes.upgrade())
            .ok_or("preview is no longer in memory; reopen it before saving")?;
        let name = &request.identity.name;
        if name.is_empty()
            || matches!(name.as_str(), "." | "..")
            || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
            || name.chars().any(char::is_control)
            || name.ends_with(['.', ' '])
        {
            return Err("media filename is invalid".into());
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = self
            .downloads
            .join(format!("saved-media-{post_id}-{nonce}"));
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let path = dir.join(name);
        let pending = dir.join(".saving");
        std::fs::write(&pending, &bytes).map_err(|error| error.to_string())?;
        std::fs::rename(pending, &path).map_err(|error| error.to_string())?;
        if let Err(error) = self
            .catalog
            .record_saved_file(post_id, name, &path, bytes.len() as u64)
        {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_dir(&dir);
            return Err(format!("record saved media: {error}"));
        }
        Ok(path)
    }
}
