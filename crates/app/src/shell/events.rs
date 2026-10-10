use super::*;

impl Browser {
    pub(super) fn pump(&mut self, ctx: &egui::Context) {
        #[cfg(feature = "ui-preview")]
        if self.preview_fixture {
            return;
        }
        for event in self.handle.poll() {
            self.on_event(event);
        }
        self.pump_pixels(ctx);
        self.pump_library();
        self.pump_group_fetches();
    }

    pub(super) fn pump_pixels(&mut self, ctx: &egui::Context) {
        while let Ok(pixels) = self.pixel_rx.try_recv() {
            self.decoding.remove(&pixels.key);
            if !self.memory_key_live(&pixels.key) {
                continue;
            }
            if let Some(path) = self.decode_refresh.remove(&pixels.key) {
                self.decode_invalidated.remove(&pixels.key);
                self.decode_failed.remove(&pixels.key);
                let max_edge = if pixels.key.starts_with("full:") {
                    2048
                } else {
                    512
                };
                self.decode_later_scaled(pixels.key, path, max_edge);
                continue;
            }
            if self.decode_invalidated.remove(&pixels.key) {
                continue;
            }
            if pixels.width == 0
                || pixels.rgba.len() != pixels.width as usize * pixels.height as usize * 4
            {
                self.decode_failed.insert(pixels.key);
                continue;
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [pixels.width as usize, pixels.height as usize],
                &pixels.rgba,
            );
            let handle = ctx.load_texture(&pixels.key, image, egui::TextureOptions::LINEAR);
            self.insert_texture(pixels.key, handle);
        }
    }

    pub(super) fn on_event(&mut self, event: Event) {
        self.observe_error(&event);
        match event {
            Event::Servers(servers) => self.on_servers(servers),
            Event::Groups(groups) => self.on_groups(groups),
            Event::Posts(posts) => {
                if self.post_request.is_some()
                    && self.active_dates() == (None, None)
                    && posts
                        .iter()
                        .all(|post| Some(post.group_id) == self.selected_group)
                {
                    self.on_posts(posts);
                }
            }
            Event::PostsDatePage {
                group_id,
                search,
                after,
                from,
                until,
                posts,
            } => {
                let request = PostRequest {
                    group_id,
                    search,
                    after,
                    from,
                    until,
                };
                if self.post_request.as_ref() == Some(&request)
                    && self.selected_group == Some(group_id)
                    && self.search == request.search
                    && self.active_dates() == (from, until)
                {
                    self.on_posts(posts);
                }
            }
            Event::PostsDateFailed {
                group_id,
                search,
                after,
                from,
                until,
                error,
            } => {
                let request = PostRequest {
                    group_id,
                    search,
                    after,
                    from,
                    until,
                };
                if self.post_request.as_ref() == Some(&request) {
                    self.posts_loading = false;
                    self.append_posts = false;
                    self.post_request = None;
                    self.notice = self.scrub_text(&error);
                }
            }
            Event::PostsPage {
                group_id,
                search,
                after,
                posts,
            } => {
                let request = PostRequest {
                    group_id,
                    search,
                    after,
                    from: None,
                    until: None,
                };
                if self.post_request.as_ref() == Some(&request)
                    && self.selected_group == Some(group_id)
                    && self.search == request.search
                {
                    self.on_posts(posts);
                }
            }
            Event::HeaderProgress {
                group_id,
                received,
                stored,
                first,
                last,
                completed,
                total,
            } => {
                if self.header_task_group == Some(group_id)
                    || (self.header_task_group.is_none()
                        && (self.selected_group == Some(group_id) || (first == 0 && total == 0)))
                {
                    self.header_task_group = Some(group_id);
                    let progress = self
                        .header_progress
                        .get_or_insert_with(HeaderProgress::default);
                    progress.received = received;
                    progress.stored = stored;
                    progress.first = first;
                    progress.last = last;
                    progress.completed = completed.min(total);
                    progress.total = total;
                }
            }
            Event::HeaderFetchFinished { group_id, error } => {
                let progress = if self.header_task_group == Some(group_id) {
                    self.header_task_group = None;
                    self.header_cancel_pending = false;
                    self.header_progress.take()
                } else if self.header_task_group.is_none() && self.selected_group == Some(group_id)
                {
                    self.header_progress.take()
                } else {
                    None
                };
                if self.selected_group == Some(group_id) {
                    if let Some(error) = error {
                        self.notice = self.scrub_text(&error);
                        // A search may have queued a new LoadPosts while headers
                        // were being fetched. Its reply is still useful.
                        if self.post_request.is_none() {
                            self.posts_loading = false;
                            self.append_posts = false;
                        }
                    } else {
                        if let Some(progress) = progress {
                            self.notice = if progress.received == 0 && progress.total == 0 {
                                "Article headers are up to date.".into()
                            } else {
                                format!(
                                    "Cached {} article headers in {:.1}s.",
                                    progress.stored,
                                    progress.started.elapsed().as_secs_f32()
                                )
                            };
                        }
                        self.reload_posts();
                    }
                }
            }
            Event::HeaderFetchCancelled { group_id } => {
                if self.header_task_group == Some(group_id)
                    || (self.header_task_group.is_none() && self.selected_group == Some(group_id))
                {
                    self.header_task_group = None;
                    self.header_progress = None;
                    self.header_cancel_pending = false;
                    self.notice = "Header fetch cancelled. Cached headers kept.".into();
                    if self.selected_group == Some(group_id) {
                        self.reload_posts();
                    }
                }
            }
            Event::PostsFailed {
                group_id,
                search,
                after,
                error,
            } => {
                let request = PostRequest {
                    group_id,
                    search,
                    after,
                    from: None,
                    until: None,
                };
                if self.post_request.as_ref() == Some(&request) {
                    self.posts_loading = false;
                    self.append_posts = false;
                    self.post_request = None;
                    self.notice = self.scrub_text(&error);
                }
            }
            Event::GroupRows { server_id, groups } => self.on_group_rows(server_id, groups),
            Event::GroupUnsubscribed { group_id, post_ids } => {
                self.on_group_unsubscribed(group_id, &post_ids);
            }
            Event::Files { post_id, files } => {
                if self.open_post == Some(post_id) {
                    self.files = files;
                }
            }
            Event::Jobs(jobs) => {
                self.note_speeds(&jobs);
                self.library.request(&jobs, false);
                self.jobs = jobs;
            }
            Event::GroupSample {
                server_id,
                groups,
                received,
                done,
            } => self.on_sample(server_id, groups, received, done),
            Event::Notice(text) => {
                if text.starts_with("group list:") {
                    self.picker_loading = false;
                }
                if text.starts_with("save server:") {
                    self.select_new = false;
                }
                self.notice = self.scrub_text(&text);
            }
            Event::Thumb { post_id, path } => {
                self.thumb_paths.insert(post_id, path.clone());
                let key = format!("post:{post_id}");
                self.decode_failed.remove(&key);
                self.textures.remove(&key);
                self.texture_order.retain(|have| have != &key);
                if self.decoding.contains(&key) {
                    self.decode_refresh.insert(key, path);
                } else {
                    self.decode_later(key, path);
                }
            }
            Event::MediaCached { post_id, path } => self.on_media_cached(post_id, path),
            Event::PreviewMediaReady {
                post_id,
                generation,
                name,
                bytes,
            } => {
                if generation == self.media_generation {
                    self.on_memory_preview(post_id, name, bytes);
                }
            }
            Event::PreviewMediaProgress {
                post_id,
                generation,
                received,
                total,
            } => {
                if generation == self.media_generation && self.media_pending.contains(&post_id) {
                    self.media_progress.insert(post_id, (received, total));
                }
            }
            Event::PreviewMediaFailed {
                post_id,
                generation,
                error,
            } => {
                if generation == self.media_generation {
                    self.media_pending.remove(&post_id);
                    self.media_progress.remove(&post_id);
                    let error = self.scrub_text(&error);
                    self.media_errors.insert(post_id, error);
                }
            }
            Event::ConnectionStats {
                server_id,
                pool_id,
                connections,
            } => {
                if connections.is_empty() {
                    self.connections.remove(&(server_id, pool_id));
                } else {
                    self.connections.insert((server_id, pool_id), connections);
                }
            }
            Event::NetworkTraffic {
                server_id,
                pool_id,
                traffic,
                closed,
            } => {
                self.traffic.observe((server_id, pool_id), traffic, closed);
            }
            Event::NetworkStats {
                server_id,
                pool_id,
                open,
                active,
                bytes_per_second,
            } => {
                if active == 0 && open == 0 && bytes_per_second == 0.0 {
                    self.network_stats.remove(&(server_id, pool_id));
                } else {
                    self.network_stats
                        .insert((server_id, pool_id), (active, open, bytes_per_second));
                }
            }
            Event::DownloadDeleted {
                job_id,
                remove_files,
            } => {
                if let Some(job) = self
                    .jobs
                    .iter()
                    .find(|job| job.id == job_id)
                    .filter(|_| remove_files)
                {
                    let directory = Path::new(&job.dest_dir);
                    self.media_paths
                        .retain(|_, path| !path.starts_with(directory));
                    self.thumb_paths
                        .retain(|_, path| !path.starts_with(directory));
                    if let Some(post_id) = job.post_id.filter(|post_id| {
                        !self.jobs.iter().any(|other| {
                            other.id != job_id
                                && other.post_id == Some(*post_id)
                                && other.state == "done"
                        })
                    }) {
                        self.thumb_paths.remove(&post_id);
                        for key in [format!("post:{post_id}"), format!("full:media:{post_id}")] {
                            self.textures.remove(&key);
                            self.texture_order.retain(|have| have != &key);
                        }
                    }
                }
                self.overlay = None;
                self.notice = if remove_files {
                    "Download files deleted."
                } else {
                    "Download removed from list; files retained."
                }
                .into();
            }
            Event::DownloadDeleteFailed { error, .. } => {
                self.notice = self.scrub_text(&format!("remove download failed: {error}"));
            }
            Event::MediaSaved { post_id, path } => {
                self.media_saving.remove(&post_id);
                self.on_media_cached(post_id, path);
            }
            Event::MediaSaveFailed { post_id, error } => {
                self.media_saving.remove(&post_id);
                self.notice = self.scrub_text(&format!("save media failed: {error}"));
            }
            Event::MediaCacheFailed { post_id, error } => {
                self.media_pending.remove(&post_id);
                let error = self.scrub_text(&error);
                self.media_errors.insert(post_id, error);
            }
            Event::ArticleContent {
                post_id,
                mut text,
                binary,
            } => {
                if self.open_post == Some(post_id) && self.article_requested == Some(post_id) {
                    self.article_loading = false;
                    self.article_error = None;
                    if text.len() > 65_536 {
                        let mut end = 65_536;
                        while !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        text.truncate(end);
                        text.push_str(
                            "\n\n[Display shortened; the complete article body is cached on disk.]",
                        );
                    }
                    self.article_content = Some(ArticleContent { text, binary });
                }
            }
            Event::ArticleReadFailed { post_id, error } => {
                if self.open_post == Some(post_id) && self.article_requested == Some(post_id) {
                    self.article_loading = false;
                    self.article_error = Some(self.scrub_text(&error));
                }
            }
        }
    }

    fn on_posts(&mut self, posts: Vec<PostRow>) {
        // More headers may complete a file which previously could not be
        // previewed. Retry only when its metadata changes, avoiding a loop
        // for unchanged unavailable articles.
        for post in &posts {
            if self.posts.iter().any(|old| {
                old.id == post.id
                    && (old.segment_count != post.segment_count
                        || old.file_count != post.file_count
                        || old.total_bytes != post.total_bytes
                        || old.incomplete != post.incomplete)
            }) {
                self.media_errors.remove(&post.id);
            }
        }
        let full = posts.len() >= 80;
        if self.append_posts {
            let mut seen: HashSet<i64> = self.posts.iter().map(|p| p.id).collect();
            self.posts
                .extend(posts.into_iter().filter(|p| seen.insert(p.id)));
        } else {
            self.posts = posts;
            self.selection
                .retain(|id| self.posts.iter().any(|p| p.id == *id));
            if self
                .anchor
                .is_some_and(|id| !self.posts.iter().any(|p| p.id == id))
            {
                self.anchor = None;
            }
            if self
                .preview_post
                .is_some_and(|id| !self.posts.iter().any(|p| p.id == id))
            {
                self.preview_post = None;
            }
            if self
                .open_post
                .is_some_and(|id| !self.posts.iter().any(|p| p.id == id))
            {
                self.open_post = None;
                self.files.clear();
                self.article_content = None;
                self.article_requested = None;
                self.article_loading = false;
                self.article_error = None;
            }
        }
        self.feed_dirty = true;
        self.posts_full = full;
        self.posts_loading = false;
        self.append_posts = false;
        self.post_request = None;
    }

    fn on_group_rows(&mut self, server_id: i64, groups: Vec<GroupRow>) {
        self.picker_dirty = true;
        self.groups.retain(|g| g.server_id != server_id);
        self.groups.extend(groups);
        if self.group_fetching == Some(server_id) {
            self.group_fetching = None;
            self.group_fetch_at = None;
        }
        if self
            .selected_group
            .is_some_and(|id| !self.groups.iter().any(|g| g.id == id && g.subscribed))
        {
            self.clear_group();
        }
    }

    pub(super) fn on_servers(&mut self, servers: Vec<ServerRow>) {
        let ids: HashSet<i64> = servers.iter().map(|server| server.id).collect();
        if self.servers.iter().any(|server| !ids.contains(&server.id)) {
            // SQLite may reuse deleted article identifiers. Never associate
            // media from a removed server with a newly created article.
            self.media_paths.clear();
            self.media_generation = self.media_generation.wrapping_add(1);
            self.handle
                .send(Command::PreviewContext(self.media_generation));
            self.media_memory.clear();
            self.media_order.clear();
            self.media_evicted.clear();
            self.media_memory_bytes = 0;
            self.media_progress.clear();
            self.media_saving.clear();
            self.media_pending.clear();
            self.media_errors.clear();
            self.thumb_paths.clear();
            self.textures.clear();
            self.texture_order.clear();
            self.decode_failed.clear();
            self.decode_refresh.clear();
        }
        self.groups.retain(|group| ids.contains(&group.server_id));
        self.samples.retain(|id, _| ids.contains(id));
        self.known_servers.retain(|id| ids.contains(id));
        for server in &servers {
            if self.known_servers.insert(server.id) {
                self.group_fetches.push_back(server.id);
            }
        }
        if self.selected_server.is_some_and(|id| !ids.contains(&id)) {
            self.selected_server = None;
        }
        if self.selected_group.is_some_and(|id| {
            !self
                .groups
                .iter()
                .any(|group| group.id == id && group.subscribed)
        }) {
            self.clear_group();
        }
        if self.select_new {
            if let Some(server) = servers.iter().max_by_key(|server| server.id) {
                self.editing = Some(server.id);
                self.form = form_from(server);
                self.selected_server = Some(server.id);
            }
            self.select_new = false;
        } else if self.editing.is_some_and(|id| !ids.contains(&id)) {
            self.editing = None;
            self.form = Form::default();
            self.confirm_delete = false;
        }
        self.servers = servers;
    }

    pub(super) fn on_groups(&mut self, groups: Vec<GroupRow>) {
        if let Some(group) = groups.first() {
            let server_id = group.server_id;
            self.on_group_rows(server_id, groups);
        } else if let Some(server_id) = self.group_fetching.take() {
            self.on_group_rows(server_id, groups);
        }
    }

    pub(super) fn on_sample(
        &mut self,
        server_id: i64,
        groups: Vec<ListedGroup>,
        received: u64,
        done: bool,
    ) {
        if received == 0 && !done {
            self.samples.insert(
                server_id,
                SampleSet {
                    groups: Vec::new(),
                    received: 0,
                    articles: 0,
                    done: false,
                    started: Instant::now(),
                },
            );
        }
        let sample = self.samples.entry(server_id).or_insert_with(|| SampleSet {
            groups: Vec::new(),
            received: 0,
            articles: 0,
            done: false,
            started: Instant::now(),
        });
        sample.articles = sample.articles.saturating_add(
            groups
                .iter()
                .map(ListedGroup::posts)
                .fold(0u64, u64::saturating_add),
        );
        sample.groups.extend(groups);
        sample.received = received;
        sample.done = done;
        if done {
            finish_sample(sample);
        }
        if self.picker_server == Some(server_id) {
            self.picker_loading = !done;
            self.picker_dirty = true;
        }
    }

    pub(super) fn pump_group_fetches(&mut self) {
        if let Some(at) = self.group_fetch_at {
            if at.elapsed() > Duration::from_secs(3) {
                self.group_fetching = None;
                self.group_fetch_at = None;
            }
        }
        if self.group_fetching.is_some() {
            return;
        }
        if let Some(id) = self.group_fetches.pop_front() {
            self.group_fetching = Some(id);
            self.group_fetch_at = Some(Instant::now());
            self.handle.send(Command::ShowGroups(id));
        }
    }

    pub(super) fn note_speeds(&mut self, jobs: &[JobRow]) {
        self.note_speeds_at(jobs, Instant::now());
    }

    pub(super) fn note_speeds_at(&mut self, jobs: &[JobRow], now: Instant) {
        let retained: HashSet<_> = jobs.iter().map(|job| job.id).collect();
        self.speeds.retain(|id, _| retained.contains(id));
        for job in jobs {
            let bytes = job.bytes_done.max(0) as u64;
            self.speeds
                .entry(job.id)
                .and_modify(|rate| rate.observe(bytes, now))
                .or_insert_with(|| TransferRate::new(bytes, now));
        }
    }

    pub(super) fn scrub_text(&self, text: &str) -> String {
        let mut secrets = Vec::new();
        if self.form.password.len() >= 3 {
            secrets.push(self.form.password.clone());
        }
        for server in &self.servers {
            if let Some(password) = &server.password {
                secrets.push(password.clone());
            }
        }
        scrub(text, &secrets)
    }
}
