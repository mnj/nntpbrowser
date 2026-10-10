use super::*;

impl Browser {
    pub(super) fn pump_library(&mut self) {
        let overlay_path = self.overlay.and_then(|slot| {
            self.library
                .items
                .iter()
                .filter(|item| item.kind == LibKind::Image)
                .nth(slot)
                .map(|item| item.path.clone())
        });
        if self.library.poll() {
            if let Some(path) = overlay_path {
                self.overlay = self
                    .library
                    .items
                    .iter()
                    .filter(|item| item.kind == LibKind::Image)
                    .position(|item| item.path == path);
            }
            self.textures
                .retain(|key, _| !key.starts_with("file:") && !key.starts_with("full:"));
            self.texture_order
                .retain(|key| !key.starts_with("file:") && !key.starts_with("full:"));
            self.decode_failed
                .retain(|key| !key.starts_with("file:") && !key.starts_with("full:"));
            for item in self.library.items.iter() {
                for prefix in ["file:", "full:"] {
                    let key = format!("{prefix}{}", item.path.display());
                    if self.decoding.contains(&key) {
                        self.decode_refresh.insert(key, item.path.clone());
                    }
                }
            }
        }
    }

    pub(super) fn lib_tile(
        &mut self,
        ui: &mut egui::Ui,
        item: &LibItem,
        index: usize,
        images: &[usize],
        width: f32,
    ) {
        let key = format!("file:{}", item.path.display());
        if item.kind == LibKind::Image {
            self.decode_later(key.clone(), item.path.clone());
        }
        let response = ui
            .scope_builder(
                egui::UiBuilder::new().layout(egui::Layout::top_down(egui::Align::Min)),
                |ui| {
                    ui.set_width(width);
                    theme::card().show(ui, |ui| {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(width - 24.0, 120.0),
                            egui::Sense::hover(),
                        );
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::same(8), theme::BG);
                        if let Some(tex) = self.texture(&key) {
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
                            let glyph = match item.kind {
                                LibKind::Image => "IMG",
                                LibKind::Video => "VIDEO",
                                LibKind::Packed => "PACKED",
                                LibKind::File => "FILE",
                            };
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                glyph,
                                FontId::new(16.0, egui::FontFamily::Proportional),
                                theme::DIM,
                            );
                        }
                        ui.add_space(6.0);
                        let name = item
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("file");
                        ui.label(egui::RichText::new(trim(name, 36)).size(14.0))
                            .on_hover_text(format!("{}\n{}", item.title, item.path.display()));
                        ui.label(theme::dim(format_bytes(item.size as i64)));
                    });
                },
            )
            .response;
        let response = ui.interact(
            response.rect,
            response.id.with("open"),
            egui::Sense::click(),
        );
        if response.clicked() {
            match item.kind {
                LibKind::Image => {
                    self.overlay = images.iter().position(|slot| *slot == index);
                }
                LibKind::Video | LibKind::Packed | LibKind::File => open_system(&item.path),
            }
        }
    }

    pub(super) fn overlay_ui(
        &mut self,
        ui: &mut egui::Ui,
        items: &[LibItem],
        images: &[usize],
        slot: usize,
    ) {
        let Some(index) = images.get(slot).copied() else {
            self.overlay = None;
            return;
        };
        let Some(item) = items.get(index) else {
            self.overlay = None;
            return;
        };
        let key = format!("full:{}", item.path.display());
        self.decode_later_scaled(key.clone(), item.path.clone(), 2048);
        let ctx = ui.ctx().clone();
        let next = ui.input(|input| input.key_pressed(egui::Key::ArrowRight));
        let prev = ui.input(|input| input.key_pressed(egui::Key::ArrowLeft));
        let close_key = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if next && !images.is_empty() {
            self.overlay = Some((slot + 1) % images.len());
        } else if prev && !images.is_empty() {
            self.overlay = Some((slot + images.len() - 1) % images.len());
        } else if close_key {
            self.overlay = None;
            return;
        }
        let path_name = item
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
            .to_string();
        let tex = self.texture(&key);
        egui::Window::new("saved-file-preview")
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Previous").clicked() && !images.is_empty() {
                        self.overlay = Some((slot + images.len() - 1) % images.len());
                    }
                    if ui.button("Next").clicked() && !images.is_empty() {
                        self.overlay = Some((slot + 1) % images.len());
                    }
                    if ui.button("Close").clicked() {
                        self.overlay = None;
                    }
                    ui.label(theme::dim(path_name));
                });
                if let Some(tex) = tex {
                    let size = fitted(tex.size_vec2(), egui::vec2(920.0, 640.0));
                    ui.image((tex.id(), size));
                } else {
                    ui.label(theme::dim("Decoding…"));
                }
            });
    }
}
