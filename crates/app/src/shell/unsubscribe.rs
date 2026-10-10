use super::*;

impl Browser {
    pub(super) fn on_group_unsubscribed(&mut self, group_id: i64, post_ids: &[i64]) {
        if self.selected_group == Some(group_id) {
            self.clear_group();
        }
        let ids: HashSet<_> = post_ids.iter().copied().collect();
        for &id in post_ids {
            self.evict_memory_preview(id);
            for key in [format!("post:{id}"), format!("full:media:{id}")] {
                // Keep in-flight keys until their old reply arrives. A new
                // thumbnail then records a fresh path in decode_refresh.
                if self.decoding.contains(&key) {
                    self.decode_invalidated.insert(key.clone());
                }
                self.decode_refresh.remove(&key);
                self.decode_failed.remove(&key);
                self.textures.remove(&key);
                self.texture_order.retain(|have| have != &key);
            }
            self.thumb_paths.remove(&id);
            self.media_paths.remove(&id);
            self.media_pending.remove(&id);
            self.media_progress.remove(&id);
            self.media_saving.remove(&id);
            self.media_errors.remove(&id);
            self.media_evicted.remove(&id);
            self.selection.remove(&id);
        }
        self.media_viewport = None;
        if self.open_post.is_some_and(|id| ids.contains(&id)) {
            self.open_post = None;
            self.files.clear();
        }
        if self.preview_post.is_some_and(|id| ids.contains(&id)) {
            self.preview_post = None;
        }
        if self.article_requested.is_some_and(|id| ids.contains(&id)) {
            self.article_requested = None;
            self.article_content = None;
            self.article_loading = false;
            self.article_error = None;
        }
        if self.anchor.is_some_and(|id| ids.contains(&id)) {
            self.anchor = None;
        }
        self.library.request(&self.jobs, true);
    }
}
