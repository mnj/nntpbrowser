use super::*;

pub(super) mod dates;

const TILE_GAP: f32 = 8.0;

pub(super) struct GridGeometry {
    pub(super) columns: usize,
    pub(super) tile_width: f32,
    pub(super) image_height: f32,
    pub(super) row_height: f32,
}

impl GridGeometry {
    pub(super) fn new(ui: &egui::Ui, preferred_width: f32) -> Self {
        let available = ui.available_width().max(1.0);
        // Round to the nearest requested density. Flooring would stretch a
        // single "large" card across almost twice its requested width.
        let columns = (available / preferred_width).round().max(1.0) as usize;
        let tile_width =
            ((available - TILE_GAP * (columns - 1) as f32) / columns as f32).clamp(1.0, 360.0);
        let image_height = ((tile_width - 20.0).max(1.0) * 2.0 / 3.0).round();
        let metadata = ui.fonts_mut(|fonts| {
            fonts.row_height(&FontId::proportional(14.0)) * 2.0
                + fonts.row_height(&FontId::proportional(12.0))
                + 8.0
        });
        // Explicit spacing plus the largest selected border keep actual card
        // height within show_rows' pitch, including narrow, truncated captions.
        let row_height = (image_height + metadata + 6.0 + 20.0).ceil();
        Self {
            columns,
            tile_width,
            image_height,
            row_height,
        }
    }
}

impl Browser {
    pub(super) fn gallery(&mut self, ui: &mut egui::Ui, posts: &[usize]) {
        ui.horizontal_wrapped(|ui| {
            dense(ui);
            ui.checkbox(&mut self.gallery_auto, format!("Preview media under {} MiB", self.preferences_applied.preview_limit_mib))
                .on_hover_text(format!("Load visible images and videos into a temporary {} MiB memory cache. Use Save in the viewer to keep a file.", self.preferences_applied.preview_cache_mib));
            ui.label(theme::dim("Tile size"));
            let mut tile_width = self.preferences_applied.gallery_tile_width;
            let response = ui.scope(|ui| {
                ui.spacing_mut().slider_width = 120.0;
                ui.add(egui::Slider::new(&mut tile_width, 140.0..=360.0)
                    .show_value(false).step_by(10.0))
            }).inner.on_hover_text("Resize thumbnails from compact to large.");
            if response.changed() {
                self.set_gallery_tile_width(tile_width, false);
            }
            if response.drag_stopped() || (response.changed() && !response.dragged()) {
                self.set_gallery_tile_width(tile_width, true);
            }
            if ui.add_enabled(!posts.is_empty(), egui::Button::new("Select shown articles")).clicked() {
                for index in posts {
                    self.selection.insert(self.posts[*index].id);
                }
            }
            ui.label(theme::dim(
                "Double-click to view. Save explicitly to keep media.",
            ));
        });
        if posts.is_empty() {
            if self.preview_post.is_none() {
                self.request_media(&[]);
            }
            ui.label(theme::dim(if self.header_progress.is_some() {
                "Fetching article headers…"
            } else if self.posts_loading {
                "Loading articles…"
            } else if self.posts.is_empty() {
                "No cached articles yet. Choose a range above and press Get article headers."
            } else {
                "Nothing in this filter."
            }));
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
        let geometry = GridGeometry::new(ui, self.preferences_applied.gallery_tile_width);
        let cols = geometry.columns;
        let rows = posts.len().div_ceil(cols);
        let mut visible = Vec::new();
        let mut near_end = false;
        egui::ScrollArea::vertical()
            .id_salt("gallery")
            .auto_shrink([false, false])
            .show_rows(ui, geometry.row_height, rows, |ui, range| {
                if range.end + 1 >= rows {
                    near_end = true;
                }
                for row in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = TILE_GAP;
                        ui.set_min_height(geometry.row_height);
                        for col in 0..cols {
                            if let Some(index) = posts.get(row * cols + col) {
                                let post = self.posts[*index].clone();
                                let rect = self.tile(ui, &post, &ids, &jobs, &geometry, now);
                                if ui.is_rect_visible(rect) && (post.has_image || post.has_video) {
                                    visible.push(post.id);
                                }
                            }
                        }
                    });
                }
            });
        if near_end {
            self.load_more();
        }
        // The open viewer owns request priority until it closes.
        if self.preview_post.is_none() {
            if self.gallery_auto {
                // A viewport larger than the RAM budget must not refetch its
                // evicted tiles every frame; scrolling away permits a later retry.
                self.media_evicted.retain(|id| visible.contains(id));
                self.request_media(&visible);
            } else {
                self.request_media(&[]);
            }
        }
    }

    pub(super) fn tile(
        &mut self,
        ui: &mut egui::Ui,
        post: &PostRow,
        ids: &[i64],
        jobs: &[JobRow],
        geometry: &GridGeometry,
        now: i64,
    ) -> egui::Rect {
        let width = geometry.tile_width;
        let key = format!("post:{}", post.id);
        if let Some(path) = self.thumb_paths.get(&post.id).cloned() {
            self.decode_later(key.clone(), path);
        }
        let texture = self
            .preview_memory_texture(post.id, false)
            .or_else(|| self.texture(&key));
        ui.push_id(("tile", post.id), |ui| {
            ui.set_width(width);
            let selected = self.open_post == Some(post.id);
            let border = if selected {
                theme::ACCENT
            } else {
                theme::HAIRLINE
            };
            egui::Frame::new()
                .fill(theme::SURFACE)
                .stroke(egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2((width - 20.0).max(1.0), geometry.image_height),
                            egui::Sense::click(),
                        );
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::same(8), theme::BG);
                        if let Some(tex) = texture {
                            let size = fitted(tex.size_vec2(), rect.size() - egui::vec2(8.0, 8.0));
                            let image = egui::Rect::from_center_size(rect.center(), size);
                            ui.painter().image(
                                tex.id(),
                                image,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                        } else {
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                glyph(post),
                                FontId::new(16.0, egui::FontFamily::Name("semibold".into())),
                                theme::DIM,
                            );
                        }
                        if let Some(job) = jobs
                            .iter()
                            .find(|job| job.post_id == Some(post.id) && is_active(&job.state))
                        {
                            let progress = queue::JobProgress::new(job);
                            let bar = egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 4.0, rect.bottom() - 22.0),
                                egui::vec2(rect.width() - 8.0, 18.0),
                            );
                            ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                                ui.add(
                                    egui::ProgressBar::new(progress.fraction)
                                        .animate(progress.animate)
                                        .fill(progress.color)
                                        .text(progress.stage)
                                        .desired_width(bar.width()),
                                );
                            });
                        } else if self.media_pending.contains(&post.id) {
                            let (received, total) = self
                                .media_progress
                                .get(&post.id)
                                .copied()
                                .unwrap_or_default();
                            let bar = egui::Rect::from_min_size(
                                egui::pos2(rect.left() + 4.0, rect.bottom() - 22.0),
                                egui::vec2(rect.width() - 8.0, 18.0),
                            );
                            ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                                ui.add(
                                    egui::ProgressBar::new(if total > 0 {
                                        received as f32 / total as f32
                                    } else {
                                        0.0
                                    })
                                    .animate(true)
                                    .fill(theme::ACCENT)
                                    .text("Loading preview…")
                                    .desired_width(bar.width()),
                                );
                            });
                        } else if self.media_paths.contains_key(&post.id) {
                            ui.painter().text(
                                egui::pos2(rect.right() - 8.0, rect.top() + 8.0),
                                egui::Align2::RIGHT_TOP,
                                "Saved",
                                FontId::proportional(12.0),
                                theme::ACCENT,
                            );
                        }
                        let (status, status_color) = status_of(jobs, post);
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let mut on = self.selection.contains(&post.id);
                            if ui.checkbox(&mut on, "").changed() {
                                if on {
                                    self.selection.insert(post.id);
                                } else {
                                    self.selection.remove(&post.id);
                                }
                                self.anchor = Some(post.id);
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.set_width((width - 48.0).max(40.0));
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&post.title)
                                            .size(14.0)
                                            .color(theme::TEXT),
                                    )
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(theme::dim(format!(
                                        "{}   {}",
                                        format_bytes(post.total_bytes),
                                        format_stamp(post.posted_at)
                                    )))
                                    .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!(
                                            "{}   {}   {}",
                                            age(post.posted_at, now),
                                            match post.segment_count {
                                                0 => "Parts: unknown".into(),
                                                1 => "1 part".into(),
                                                n => format!("{n} parts"),
                                            },
                                            status
                                        ))
                                        .color(status_color)
                                        .size(12.0),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                        if response.double_clicked() {
                            self.focus_post(post.id);
                            if post.has_image || post.has_video {
                                self.preview_post = Some(post.id);
                            }
                        } else if response.clicked() {
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
                        response.on_hover_text(format!("{}\n{}", post.title, post.poster));
                    });
                })
                .response
                .rect
        })
        .inner
    }
}
