use super::*;

impl Browser {
    pub(super) fn articles(&mut self, ui: &mut egui::Ui, posts: &[usize]) {
        if posts.is_empty() {
            let sentence = if self.header_progress.is_some() {
                "Fetching article headers…"
            } else if self.posts_loading {
                "Loading articles…"
            } else if self.posts.is_empty() {
                "No cached articles yet. Choose a range above and press Get article headers."
            } else {
                "Nothing in this filter. Try All, or load older headers."
            };
            ui.label(theme::dim(sentence));
            ui.add_space(8.0);
            if self.posts_full {
                ui.label(theme::dim("Older articles may match this filter."));
                if ui
                    .add_enabled(
                        !self.posts_loading,
                        egui::Button::new("Show more cached articles"),
                    )
                    .clicked()
                {
                    self.load_more();
                }
            }
            return;
        }
        let jobs = self.jobs.clone();
        let now = Self::now_unix();
        let ids: Vec<i64> = posts.iter().map(|i| self.posts[*i].id).collect();
        let available = ui.available_height();
        let splitter = 8.0;
        let pane_min = 184.0;
        let table_h = (available * self.article_split)
            .clamp(140.0, (available - pane_min - splitter).max(140.0));
        self.column_header(ui, &ids);
        let mut near_end = false;
        egui::ScrollArea::vertical()
            .id_salt("articles")
            .max_height((table_h - 28.0).max(80.0))
            .auto_shrink([false, false])
            .show_rows(ui, 30.0, posts.len(), |ui, range| {
                if range.end + 2 >= posts.len() {
                    near_end = true;
                }
                for index in range {
                    let post = self.posts[posts[index]].clone();
                    self.article_row(ui, &post, &ids, &jobs);
                }
            });
        if near_end {
            self.load_more();
        }

        let (split_rect, split) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), splitter),
            egui::Sense::click_and_drag(),
        );
        let line = if split.hovered() || split.dragged() {
            theme::ACCENT
        } else {
            theme::HAIRLINE
        };
        ui.painter().hline(
            split_rect.x_range(),
            split_rect.center().y,
            egui::Stroke::new(1.0, line),
        );
        if split.dragged() {
            let next = self.article_split + split.drag_delta().y / available.max(1.0);
            self.article_split = next.clamp(0.28, 0.78);
        }
        if split.double_clicked() {
            self.article_split = 0.56;
        }
        self.article_pane(ui, &jobs, now);
    }

    pub(super) fn column_header(&mut self, ui: &mut egui::Ui, ids: &[i64]) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(0), theme::SURFACE);
        let box_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.center().y - 8.0),
            egui::vec2(16.0, 16.0),
        );
        let all_on = !ids.is_empty() && ids.iter().all(|id| self.selection.contains(id));
        let toggle = ui.interact(
            box_rect,
            egui::Id::new("article-select-all"),
            egui::Sense::click(),
        );
        if toggle.clicked() {
            if all_on {
                for id in ids {
                    self.selection.remove(id);
                }
            } else {
                for id in ids {
                    self.selection.insert(*id);
                }
            }
        }
        paint_check(ui, box_rect, all_on, toggle.hovered());
        let cols = columns_for(rect.width());
        paint_cells(
            ui,
            rect.left() + 32.0,
            rect,
            &[
                (cols.date, "Date", theme::DIM, false),
                (cols.title, "Title", theme::DIM, false),
                (cols.size, "Size", theme::DIM, false),
                (cols.files, "Files", theme::DIM, false),
                (cols.parts, "Parts", theme::DIM, false),
                (cols.poster, "Poster", theme::DIM, false),
                (cols.status, "Status", theme::DIM, false),
            ],
        );
        toggle.on_hover_text("Select the articles loaded in this window");
    }

    pub(super) fn article_row(
        &mut self,
        ui: &mut egui::Ui,
        post: &PostRow,
        ids: &[i64],
        jobs: &[JobRow],
    ) {
        ui.push_id(post.id, |ui| {
            let width = ui.available_width();
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
            let selected = self.open_post == Some(post.id);
            let checked = self.selection.contains(&post.id);
            if ui.is_rect_visible(rect) {
                let bg = if selected {
                    theme::SELECT
                } else if response.hovered() {
                    theme::HOVER
                } else {
                    Color32::TRANSPARENT
                };
                if bg != Color32::TRANSPARENT {
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::same(0), bg);
                }
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom(),
                    egui::Stroke::new(1.0, theme::HAIRLINE),
                );
                if checked {
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                        egui::CornerRadius::same(0),
                        theme::ACCENT,
                    );
                }
                let box_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 8.0, rect.center().y - 8.0),
                    egui::vec2(16.0, 16.0),
                );
                let box_resp = ui.interact(box_rect, ui.id().with("tick"), egui::Sense::click());
                paint_check(ui, box_rect, checked, box_resp.hovered());
                let (label, color) = status_of(jobs, post);
                let date = format_stamp(post.posted_at);
                let size = format_bytes(post.total_bytes);
                let files = post.file_count.to_string();
                let parts = if post.segment_count > 0 {
                    post.segment_count.to_string()
                } else {
                    "unknown".into()
                };
                let cols = columns_for(rect.width());
                paint_cells(
                    ui,
                    rect.left() + 32.0,
                    rect,
                    &[
                        (cols.date, date.as_str(), theme::DIM, false),
                        (cols.title, post.title.as_str(), theme::TEXT, false),
                        (cols.size, size.as_str(), theme::TEXT, true),
                        (cols.files, files.as_str(), theme::DIM, true),
                        (cols.parts, parts.as_str(), theme::DIM, true),
                        (cols.poster, post.poster.as_str(), theme::DIM, false),
                        (cols.status, label, color, false),
                    ],
                );
                let box_clicked = box_resp.clicked();
                let clicked = response.clicked();
                let double = response.double_clicked();
                if box_clicked {
                    self.toggle_selected(post.id);
                } else if double {
                    self.focus_post(post.id);
                    self.download_ids(&[post.id]);
                } else if clicked {
                    let mods = ui.input(|input| input.modifiers);
                    if mods.shift {
                        self.select_range(ids, post.id);
                    } else if mods.command || mods.ctrl {
                        self.toggle_selected(post.id);
                        self.focus_post(post.id);
                    } else {
                        self.anchor = Some(post.id);
                        self.focus_post(post.id);
                    }
                }
            }
            response.clone().on_hover_text(format!(
                "{}\n{}\n{}   {}   {} files   {} parts",
                post.title,
                post.poster,
                format_stamp(post.posted_at),
                format_bytes(post.total_bytes),
                post.file_count,
                post.segment_count
            ));
            let post_id = post.id;
            let title = post.title.clone();
            response.context_menu(|ui| {
                if ui.button("Download").clicked() {
                    self.download_ids(&[post_id]);
                    ui.close();
                }
                if ui.button("Copy title").clicked() {
                    ui.ctx().copy_text(title);
                    ui.close();
                }
            });
        });
    }
}
