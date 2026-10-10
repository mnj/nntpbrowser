use super::*;

impl Browser {
    pub(super) fn lightbox(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.preview_post = None;
            return;
        }
        let Some(current) = self.preview_post else {
            return;
        };
        let posts = self.filtered_posts();
        let ids: Vec<i64> = if posts.iter().any(|i| self.posts[*i].id == current) {
            posts
                .iter()
                .map(|i| &self.posts[*i])
                .filter(|post| post.has_image || post.has_video)
                .map(|post| post.id)
                .collect()
        } else {
            self.posts
                .iter()
                .filter(|post| post.has_image || post.has_video)
                .map(|post| post.id)
                .collect()
        };
        let slot = ids.iter().position(|id| *id == current).unwrap_or(0);
        let next = ctx.input(|input| input.key_pressed(egui::Key::ArrowRight));
        let prev = ctx.input(|input| input.key_pressed(egui::Key::ArrowLeft));
        if next && !ids.is_empty() {
            self.preview_post = Some(ids[(slot + 1) % ids.len()]);
            if let Some(id) = self.preview_post {
                self.focus_post(id);
            }
        } else if prev && !ids.is_empty() {
            self.preview_post = Some(ids[(slot + ids.len() - 1) % ids.len()]);
            if let Some(id) = self.preview_post {
                self.focus_post(id);
            }
        }
        let current = self.preview_post.unwrap_or(current);
        let slot = ids.iter().position(|id| *id == current).unwrap_or(0);
        self.focus_post(current);
        // Opening the viewer is an explicit request even when scrolling previews are off.
        self.media_evicted.remove(&current);
        self.request_media(&[current]);
        let post = self.posts.iter().find(|p| p.id == current).cloned();
        let cached_path = self.media_paths.get(&current).cloned();
        let memory_name = self
            .media_memory
            .get(&current)
            .map(|preview| preview.name.clone());
        let is_video = memory_name.as_ref().map_or_else(
            || {
                cached_path.as_ref().map_or_else(
                    || {
                        post.as_ref()
                            .is_some_and(|post| post.has_video && !post.has_image)
                    },
                    |path| engine::classify(&path.to_string_lossy()) == engine::FileKind::Video,
                )
            },
            |name| engine::classify(name) == engine::FileKind::Video,
        );
        let full_key = format!("full:media:{current}");
        if let Some(path) = cached_path.as_ref().filter(|_| !is_video) {
            self.decode_later_scaled(full_key.clone(), path.clone(), 2048);
        }
        let thumb_key = format!("post:{current}");
        if let Some(path) = self.thumb_paths.get(&current).cloned() {
            self.decode_later(thumb_key.clone(), path);
        }
        let memory_key = self
            .media_memory
            .get(&current)
            .map(|preview| preview.key(current, true));
        let full_texture = self
            .preview_memory_texture(current, true)
            .or_else(|| self.texture(&full_key));
        let full_ready = full_texture.is_some();
        let tex = full_texture.or_else(|| self.texture(&thumb_key));
        egui::Window::new("article-preview")
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size([920.0, 600.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Previous").clicked() && !ids.is_empty() {
                        self.preview_post = Some(ids[(slot + ids.len() - 1) % ids.len()]);
                    }
                    if ui.button("Next").clicked() && !ids.is_empty() {
                        self.preview_post = Some(ids[(slot + 1) % ids.len()]);
                    }
                    if let Some(path) = cached_path.as_ref() {
                        if primary(ui, if is_video { "Open video" } else { "Open image" }) {
                            open_system(path);
                        }
                    } else if memory_name.is_some() {
                        if ui.add_enabled(!self.media_saving.contains(&current), egui::Button::new(egui::RichText::new(if is_video { "Save video" } else { "Save image" }).color(theme::INK)).fill(theme::ACCENT)).clicked() {
                            self.save_memory_preview(current);
                        }
                    } else if primary(ui, "Download this article") {
                        self.download_ids(&[current]);
                    }
                    if ui.button("Close").clicked() {
                        self.preview_post = None;
                    }
                });
                if let Some(post) = post.as_ref().filter(|post| post.id == current) {
                    ui.label(egui::RichText::new(&post.title).size(16.0));
                    ui.label(theme::dim(format!(
                        "{}   {}   {}",
                        post.poster,
                        format_stamp(post.posted_at),
                        format_bytes(post.total_bytes)
                    )));
                }
                if let Some(tex) = tex {
                    let size = fitted(
                        tex.size_vec2(),
                        egui::vec2(ui.available_width().min(880.0), (ctx.content_rect().height() - 240.0).clamp(120.0, 520.0)),
                    );
                    ui.image((tex.id(), size));
                }
                if is_video && cached_path.is_some() {
                    ui.label(theme::dim("Video saved. Open it in your video player."));
                } else if memory_name.is_some() {
                    ui.label(theme::dim(if is_video {
                        "Temporary video preview. Save it to keep it and open it in your player."
                    } else {
                        "Temporary image preview · held in memory until evicted or the app closes."
                    }));
                    if !full_ready && memory_key.as_ref().is_some_and(|key| !self.decode_failed.contains(key)) {
                        ui.spinner();
                        ui.label(theme::dim("Preparing preview…"));
                    } else if !full_ready && is_video {
                        ui.label(theme::dim("A poster is unavailable for this video. Save it to view it in your player."));
                    } else if !full_ready {
                        ui.label(theme::dim("This image cannot be displayed here. Save it to open it in your image viewer."));
                    }
                } else if cached_path.is_some() && !full_ready {
                    if self.decode_failed.contains(&full_key) {
                        ui.label(theme::dim("This image cannot be displayed here. Open the cached file to view it."));
                    } else {
                        ui.spinner();
                        ui.label(theme::dim("Preparing full image…"));
                    }
                } else if cached_path.is_none() {
                    if let Some(error) = self.media_errors.get(&current) {
                        ui.label(egui::RichText::new(self.scrub_text(error)).color(theme::WARN));
                    } else if let Some(job) = self.jobs.iter().find(|job| job.post_id == Some(current) && is_active(&job.state)) {
                        let progress = queue::JobProgress::new(job);
                        ui.add(egui::ProgressBar::new(progress.fraction)
                            .animate(progress.animate)
                            .fill(progress.color)
                            .text(format!("{} · {}", progress.stage, progress.detail)));
                    } else if self.media_pending.contains(&current) {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(theme::dim("Loading media into memory…"));
                        });
                    } else {
                        let hint = match post.as_ref() {
                            Some(post) if post.file_count > 1 => "This article contains multiple files. Download this article to save them.",
                            Some(post) if post.incomplete => "Some article parts are missing. Download this article to try the available files.",
                            Some(post) if post.total_bytes as u64 >= self.preview_limit() => "This media exceeds the preview limit. Download this article to save it.",
                            _ => "Download this article to view its media.",
                        };
                        ui.label(theme::dim(hint));
                    }
                }
                ui.add_space(6.0);
                let files = self.files.clone();
                for file in files.iter().take(12) {
                    ui.label(theme::dim(format!(
                        "{}   {}   {}",
                        file.kind.as_str(),
                        file.name,
                        format_bytes(file.size)
                    )));
                }
            });
    }
}
