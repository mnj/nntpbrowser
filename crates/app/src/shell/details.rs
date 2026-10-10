use super::*;

impl Browser {
    pub(super) fn article_pane(&mut self, ui: &mut egui::Ui, jobs: &[JobRow], now: i64) {
        let post = self
            .open_post
            .and_then(|id| self.posts.iter().find(|p| p.id == id))
            .cloned();
        let Some(post) = post else {
            ui.add_space(20.0);
            ui.label(theme::heading("Article details"));
            ui.label(theme::dim(
                "Select an article to see its files, metadata and preview.",
            ));
            return;
        };
        let height = ui.available_height();
        theme::card().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("ARTICLE DETAILS")
                        .size(11.0)
                        .color(theme::DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary(ui, "Download this article") {
                        self.download_ids(&[post.id]);
                    }
                });
            });
            ui.add_space(4.0);
            if !post.has_image
                && !post.has_video
                && (self.article_loading
                    || self.article_error.is_some()
                    || self
                        .article_content
                        .as_ref()
                        .is_some_and(|body| !body.binary))
            {
                ui.label(egui::RichText::new(&post.title).size(18.0));
                ui.label(theme::dim(format!(
                    "{} · {}",
                    post.poster,
                    format_stamp(post.posted_at)
                )));
                self.article_body(ui, (height - 112.0).max(64.0));
                return;
            }
            let inner_h = (height - 82.0).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), inner_h),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(222.0, inner_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.preview_column(ui, &post),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), inner_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.detail_column(ui, &post, jobs, now, inner_h),
                    );
                },
            );
        });
    }

    fn article_body(&mut self, ui: &mut egui::Ui, height: f32) {
        if self.article_loading {
            ui.add(
                egui::ProgressBar::new(0.0)
                    .animate(true)
                    .text("Downloading selected article…")
                    .fill(theme::ACCENT)
                    .desired_width(ui.available_width()),
            );
        } else if let Some(error) = &self.article_error {
            ui.label(egui::RichText::new(error).color(theme::WARN));
            if ui.button("Retry article body").clicked() {
                self.article_requested = None;
                if let Some(id) = self.open_post {
                    self.focus_post(id);
                }
            }
        } else if let Some(body) = &self.article_content {
            egui::ScrollArea::vertical()
                .id_salt(("article-body", self.open_post))
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(&body.text).selectable(true).wrap());
                });
        }
    }

    pub(super) fn preview_column(&mut self, ui: &mut egui::Ui, post: &PostRow) {
        let key = format!("post:{}", post.id);
        if let Some(path) = self.thumb_paths.get(&post.id).cloned() {
            self.decode_later(key.clone(), path);
        }
        let media_key = format!("full:media:{}", post.id);
        if let Some(path) = self
            .media_paths
            .get(&post.id)
            .cloned()
            .filter(|_| post.has_image)
        {
            self.decode_later_scaled(media_key.clone(), path, 2048);
        }
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(210.0, (ui.available_height() - 28.0).clamp(70.0, 140.0)),
            egui::Sense::click(),
        );
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(10), theme::BG);
        if let Some(tex) = self
            .preview_memory_texture(post.id, true)
            .or_else(|| self.texture(&media_key))
            .or_else(|| self.texture(&key))
        {
            let size = fitted(tex.size_vec2(), rect.size() - egui::vec2(12.0, 12.0));
            let image = egui::Rect::from_center_size(rect.center(), size);
            ui.painter().image(
                tex.id(),
                image,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph(post),
                FontId::new(18.0, egui::FontFamily::Name("semibold".into())),
                theme::DIM,
            );
        }
        if response.clicked() && post.has_image {
            self.preview_post = Some(post.id);
        }
        if post.has_image && ui.button("Open preview").clicked() {
            self.preview_post = Some(post.id);
        }
        if post.has_video && ui.button("Open video").clicked() {
            if let Some(path) = self.media_paths.get(&post.id) {
                open_system(path);
            } else {
                self.preview_post = Some(post.id);
            }
        }
        if self.media_memory.contains_key(&post.id)
            && !self.media_paths.contains_key(&post.id)
            && ui
                .add_enabled(
                    !self.media_saving.contains(&post.id),
                    egui::Button::new("Save media"),
                )
                .clicked()
        {
            self.save_memory_preview(post.id);
        }
        if self.article_loading || self.media_pending.contains(&post.id) {
            ui.label(theme::dim("Loading selected article…"));
        }
    }

    pub(super) fn detail_column(
        &mut self,
        ui: &mut egui::Ui,
        post: &PostRow,
        jobs: &[JobRow],
        now: i64,
        height: f32,
    ) {
        ui.set_max_width(ui.available_width());
        ui.add(
            egui::Label::new(
                egui::RichText::new(&post.title)
                    .family(egui::FontFamily::Name("semibold".into()))
                    .size(20.0)
                    .color(theme::TEXT),
            )
            .truncate(),
        )
        .on_hover_text(&post.title);
        ui.label(theme::dim(format!(
            "{}   {}   {}",
            post.poster,
            format_stamp(post.posted_at),
            age(post.posted_at, now)
        )));
        let (label, color) = status_of(jobs, post);
        ui.label(
            egui::RichText::new(format!(
                "{}   {}   {} files   {}   {label}",
                format_bytes(post.total_bytes),
                if post.incomplete {
                    "incomplete"
                } else {
                    "complete"
                },
                post.file_count,
                if post.segment_count > 0 {
                    format!("{} parts", post.segment_count)
                } else {
                    "Parts: unknown".into()
                }
            ))
            .color(color)
            .size(13.0),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("FILES  ({})", self.files.len()))
                .size(11.0)
                .color(theme::DIM),
        );
        let files = self.files.clone();
        if files.is_empty() {
            ui.label(theme::dim("No files recorded for this article."));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("article-files")
            .max_height((height - 100.0).max(48.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for file in &files {
                    self.file_line(ui, file);
                }
            });
    }

    pub(super) fn file_line(&mut self, ui: &mut egui::Ui, file: &FileRow) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
        let kind = file.kind.as_str();
        let kind_color = match kind {
            "image" => theme::ACCENT,
            "par2" => theme::WARN,
            "archive" => theme::TEXT,
            _ => theme::DIM,
        };
        let size = format_bytes(file.size);
        let parts = if file.expected_segments > 0 {
            format!("{} parts", file.expected_segments)
        } else {
            "Parts: unknown".into()
        };
        let name_w = (rect.width() - 250.0).max(80.0);
        paint_cells(
            ui,
            rect.left(),
            rect,
            &[
                (72.0, kind, kind_color, false),
                (name_w, file.name.as_str(), theme::TEXT, false),
                (72.0, size.as_str(), theme::DIM, true),
                (94.0, parts.as_str(), theme::DIM, false),
            ],
        );
    }
}
