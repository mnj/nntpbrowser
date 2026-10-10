use super::*;

impl Browser {
    pub(super) fn clear_group(&mut self) {
        self.media_generation = self.media_generation.wrapping_add(1);
        self.media_pending.clear();
        self.media_evicted.clear();
        self.media_progress.clear();
        self.handle
            .send(Command::PreviewContext(self.media_generation));
        self.selected_group = None;
        self.posts.clear();
        self.feed_indexes = Arc::new(Vec::new());
        self.feed_dirty = true;
        self.files.clear();
        self.open_post = None;
        self.preview_post = None;
        self.article_content = None;
        self.article_requested = None;
        self.article_loading = false;
        self.article_error = None;
        self.selection.clear();
        self.anchor = None;
        self.posts_full = false;
        self.posts_loading = false;
        self.append_posts = false;
        self.post_request = None;
        if self.header_task_group.is_none() {
            self.header_progress = None;
            self.header_cancel_pending = false;
        }
    }

    fn request_posts(&mut self, group_id: i64, after: Option<(i64, i64)>) {
        let (from, until) = self.active_dates();
        self.append_posts = after.is_some();
        self.posts_loading = true;
        self.post_request = Some(PostRequest {
            group_id,
            search: self.search.clone(),
            after,
            from,
            until,
        });
        if from.is_some() || until.is_some() {
            self.handle.send(Command::LoadPostsInDateRange {
                group_id,
                after,
                search: self.search.clone(),
                from,
                until,
            });
        } else {
            self.handle.send(Command::LoadPosts {
                group_id,
                after,
                search: self.search.clone(),
            });
        }
    }

    pub(super) fn active_dates(&self) -> (Option<i64>, Option<i64>) {
        if self.view == BrowseView::Gallery {
            self.gallery_dates.bounds()
        } else {
            (None, None)
        }
    }

    pub(super) fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub(super) fn filtered_posts(&mut self) -> Arc<Vec<usize>> {
        if self.feed_dirty {
            self.feed_indexes = Arc::new(
                self.posts
                    .iter()
                    .enumerate()
                    .filter(|(_, post)| {
                        (self.view != BrowseView::Gallery
                            || self.gallery_dates.matches(post.posted_at))
                            && passes_filter(
                                post.has_image,
                                post.has_video,
                                post.has_par2,
                                post.incomplete,
                                post.file_count,
                                self.filter,
                            )
                    })
                    .map(|(i, _)| i)
                    .collect(),
            );
            self.feed_dirty = false;
        }
        self.feed_indexes.clone()
    }

    pub(super) fn pick_group(&mut self, id: i64, server_id: i64) {
        let changed = self.selected_group != Some(id);
        if changed {
            self.clear_group();
        }
        self.selected_group = Some(id);
        self.selected_server = Some(server_id);
        self.screen = Screen::Browse;
        self.request_posts(id, None);
        if changed {
            // The worker checks the header cache, independently of filters or
            // the displayed page. Search and pagination never trigger a fetch.
            self.handle.send(Command::EnsureGroupHeaders(id));
        }
    }

    pub(super) fn focus_post(&mut self, post_id: i64) {
        if self.open_post != Some(post_id) {
            self.open_post = Some(post_id);
            self.files.clear();
            self.handle.send(Command::OpenPost(post_id));
        } else {
            self.open_post = Some(post_id);
        }
        if self.view == BrowseView::Articles && self.article_requested != Some(post_id) {
            self.article_requested = Some(post_id);
            self.article_loading = true;
            self.article_error = None;
            self.article_content = None;
            #[cfg(feature = "ui-preview")]
            if self.preview_fixture {
                return;
            }
            self.handle.send(Command::ReadArticle(post_id));
            self.request_media(&[post_id]);
        }
    }

    pub(super) fn fetch_selected_headers(&mut self) {
        if self.header_progress.is_some() {
            return;
        }
        let Some(id) = self.selected_group else {
            self.notice = "Pick a group in the sidebar first.".into();
            return;
        };
        let subscribed = self
            .groups
            .iter()
            .any(|group| group.id == id && group.subscribed);
        if !subscribed {
            self.notice = "Subscribe to the group before fetching headers.".into();
            return;
        }
        self.append_posts = false;
        self.post_request = None;
        self.posts_loading = true;
        self.header_progress = Some(HeaderProgress::default());
        self.header_task_group = Some(id);
        self.header_cancel_pending = false;
        let command = match self.header_scope {
            HeaderScope::New => Command::SyncGroup(id),
            HeaderScope::Latest => Command::FetchHeaders {
                group_id: id,
                limit: Some(self.header_limit),
            },
            HeaderScope::All => Command::FetchHeaders {
                group_id: id,
                limit: None,
            },
        };
        self.handle.send(command);
    }

    pub(super) fn reload_posts(&mut self) {
        self.feed_dirty = true;
        let Some(group_id) = self.selected_group else {
            self.notice = "Pick a group before searching it.".into();
            return;
        };
        self.request_posts(group_id, None);
    }

    pub(super) fn load_more(&mut self) {
        if self.posts_loading || !self.posts_full {
            return;
        }
        let Some(group_id) = self.selected_group else {
            return;
        };
        let Some(last) = self.posts.last() else {
            return;
        };
        self.request_posts(group_id, Some((last.posted_at, last.id)));
    }

    pub(super) fn download_ids(&mut self, ids: &[i64]) {
        if ids.is_empty() {
            self.notice = "Select an article to download.".into();
            return;
        }
        for id in ids {
            self.handle.send(Command::QueuePost { post_id: *id });
        }
        let noun = if ids.len() == 1 {
            "article"
        } else {
            "articles"
        };
        self.notice = format!("Queued {} {noun}.", ids.len());
    }

    pub(super) fn download_basket(&mut self) {
        let mut ids: Vec<i64> = self.selection.iter().copied().collect();
        ids.sort_unstable();
        self.download_ids(&ids);
    }

    pub(super) fn select_range(&mut self, ids: &[i64], to: i64) {
        let Some(anchor) = self.anchor else {
            self.selection.insert(to);
            self.anchor = Some(to);
            self.focus_post(to);
            return;
        };
        if let (Some(from), Some(end)) = (
            ids.iter().position(|id| *id == anchor),
            ids.iter().position(|id| *id == to),
        ) {
            let (lo, hi) = if from <= end {
                (from, end)
            } else {
                (end, from)
            };
            for id in &ids[lo..=hi] {
                self.selection.insert(*id);
            }
        } else {
            self.selection.insert(to);
        }
        self.focus_post(to);
    }

    pub(super) fn toggle_selected(&mut self, id: i64) {
        if !self.selection.insert(id) {
            self.selection.remove(&id);
        }
        self.anchor = Some(id);
    }

    pub(super) fn begin_new_server(&mut self) {
        self.options_open = true;
        self.editing = None;
        self.confirm_delete = false;
        self.form = Form::default();
    }

    pub(super) fn edit_server(&mut self, id: i64) {
        let Some(server) = self.servers.iter().find(|server| server.id == id).cloned() else {
            return;
        };
        self.options_open = true;
        self.editing = Some(id);
        self.confirm_delete = false;
        self.form = form_from(&server);
        self.selected_server = Some(id);
        self.group_fetches.push_back(id);
    }

    pub(super) fn save_server(&mut self) {
        let Some(server) = server_from_form(&self.form) else {
            self.notice = "Name, host, and port are required.".into();
            return;
        };
        if let Some(id) = self.editing {
            self.handle.send(Command::UpdateServer {
                id,
                server,
                enabled: self.form.enabled,
            });
            self.notice = "Saved the server.".into();
        } else {
            self.select_new = true;
            self.handle.send(Command::SaveServerEnabled {
                server,
                enabled: self.form.enabled,
            });
            self.notice = "Saved the server.".into();
        }
    }

    pub(super) fn delete_editing(&mut self) {
        let Some(id) = self.editing else {
            return;
        };
        self.handle.send(Command::DeleteServer(id));
        self.groups.retain(|group| group.server_id != id);
        self.samples.remove(&id);
        self.known_servers.remove(&id);
        if self.picker_server == Some(id) {
            self.picker_server = None;
            self.groups_open = false;
        }
        if self.selected_server == Some(id) {
            self.selected_server = None;
        }
        if self
            .selected_group
            .is_some_and(|selected| !self.groups.iter().any(|group| group.id == selected))
        {
            self.clear_group();
        }
        self.editing = None;
        self.form = Form::default();
        self.confirm_delete = false;
        self.notice = "Deleted the server.".into();
    }
    pub(super) fn queue_group(&mut self, limit: Option<u64>) {
        if let Some(group_id) = self.selected_group {
            self.handle.send(Command::QueueGroup {
                group_id,
                limit,
                search: self.search.clone(),
            });
            self.notice = "Queueing matching articles…".into();
        }
    }
}
