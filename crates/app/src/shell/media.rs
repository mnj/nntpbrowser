//! Temporary Gallery media lives in a bounded RAM cache until explicitly saved.
use super::*;

const ENTRY_CAP: usize = 128;

pub(super) struct MemoryPreview {
    pub(super) name: String,
    pub(super) bytes: Arc<[u8]>,
    pub(super) version: u64,
}

impl MemoryPreview {
    pub(super) fn key(&self, id: i64, full: bool) -> String {
        format!(
            "{}ram:{id}:{}",
            if full { "full:" } else { "" },
            self.version
        )
    }
}

impl Browser {
    pub(super) fn request_media(&mut self, ids: &[i64]) {
        #[cfg(feature = "ui-preview")]
        if self.preview_fixture {
            return;
        }
        if self
            .media_viewport
            .as_ref()
            .is_none_or(|(generation, wanted)| {
                *generation != self.media_generation || wanted != ids
            })
        {
            self.handle.send(Command::RetainPreviewMedia {
                post_ids: ids.to_vec(),
                generation: self.media_generation,
            });
            self.media_viewport = Some((self.media_generation, ids.to_vec()));
            self.media_pending.retain(|id| ids.contains(id));
            self.media_progress.retain(|id, _| ids.contains(id));
        }
        for &id in ids {
            if self.media_memory.contains_key(&id) {
                self.media_order.retain(|have| *have != id);
                self.media_order.push_back(id);
                continue;
            }
            let eligible = self
                .posts
                .iter()
                .find(|post| post.id == id)
                .is_some_and(|post| {
                    (post.has_image || post.has_video)
                        && (post.file_count > 1
                            || (post.total_bytes > 0
                                && (post.total_bytes as u64) < self.preview_limit()))
                });
            if !eligible
                || self.media_evicted.contains(&id)
                || self.media_errors.contains_key(&id)
                || self.media_paths.get(&id).is_some_and(|path| path.is_file())
                || self
                    .jobs
                    .iter()
                    .any(|job| job.post_id == Some(id) && is_active(&job.state))
                || !self.media_pending.insert(id)
            {
                continue;
            }
            self.handle.send(Command::PreviewMedia {
                post_id: id,
                max_bytes: self.preview_limit(),
                generation: self.media_generation,
            });
        }
    }

    pub(super) fn on_memory_preview(&mut self, id: i64, name: String, bytes: Arc<[u8]>) {
        self.media_pending.remove(&id);
        self.media_progress.remove(&id);
        self.media_errors.remove(&id);
        if bytes.is_empty() || bytes.len() as u64 >= self.preview_limit() {
            self.media_errors
                .insert(id, "Media preview exceeds its memory limit.".into());
            return;
        }
        self.evict_memory_preview(id);
        self.media_evicted.remove(&id);
        self.media_version = self.media_version.wrapping_add(1);
        self.media_memory_bytes += bytes.len();
        self.media_memory.insert(
            id,
            MemoryPreview {
                name,
                bytes,
                version: self.media_version,
            },
        );
        self.media_order.push_back(id);
        self.trim_memory_previews();
    }

    pub(super) fn preview_limit(&self) -> u64 {
        (self.preferences_applied.preview_limit_mib * 1024 * 1024).min(MEDIA_LIMIT)
    }

    pub(super) fn trim_memory_previews(&mut self) {
        while self.media_memory_bytes > self.preferences_applied.preview_cache_mib * 1024 * 1024
            || self.media_memory.len() > ENTRY_CAP
        {
            let Some(old) = self.media_order.front().copied() else {
                break;
            };
            self.evict_memory_preview(old);
            self.media_evicted.insert(old);
        }
    }

    pub(super) fn evict_memory_preview(&mut self, id: i64) {
        if let Some(preview) = self.media_memory.remove(&id) {
            self.media_memory_bytes = self.media_memory_bytes.saturating_sub(preview.bytes.len());
            for key in [preview.key(id, false), preview.key(id, true)] {
                self.textures.remove(&key);
                self.texture_order.retain(|have| have != &key);
                self.decode_failed.remove(&key);
            }
        }
        self.media_order.retain(|have| *have != id);
    }

    pub(super) fn memory_key_live(&self, key: &str) -> bool {
        !key.starts_with("ram:") && !key.starts_with("full:ram:")
            || self.media_memory.iter().any(|(id, preview)| {
                key == preview.key(*id, false) || key == preview.key(*id, true)
            })
    }

    pub(super) fn preview_memory_texture(&mut self, id: i64, full: bool) -> Option<TextureHandle> {
        let preview = self.media_memory.get(&id)?;
        let key = preview.key(id, full);
        let (bytes, name) = (preview.bytes.clone(), preview.name.clone());
        self.media_order.retain(|have| *have != id);
        self.media_order.push_back(id);
        self.decode_memory_later(key.clone(), name, bytes, if full { 2048 } else { 256 });
        self.texture(&key)
    }

    pub(super) fn save_memory_preview(&mut self, id: i64) {
        if self.media_memory.contains_key(&id) && self.media_saving.insert(id) {
            self.handle.send(Command::SavePreviewMedia {
                post_id: id,
                generation: self.media_generation,
            });
        }
    }

    pub(super) fn on_media_cached(&mut self, id: i64, path: PathBuf) {
        self.media_pending.remove(&id);
        self.media_progress.remove(&id);
        self.media_errors.remove(&id);
        // Explicit downloads may contain multiple files; prefer an image.
        if self.media_paths.get(&id).is_some_and(|have| have == &path) {
            return;
        }
        let image = |p: &Path| engine::classify(&p.to_string_lossy()) == engine::FileKind::Image;
        if self
            .media_paths
            .get(&id)
            .is_some_and(|have| have.is_file() && image(have))
        {
            return;
        }
        self.media_paths.insert(id, path.clone());
        let key = format!("full:media:{id}");
        self.textures.remove(&key);
        self.texture_order.retain(|have| have != &key);
        self.decode_failed.remove(&key);
        if self.decoding.contains(&key) {
            self.decode_refresh.insert(key, path);
        }
    }
}
