use super::*;

impl Browser {
    pub(super) fn global_settings(&mut self, ui: &mut egui::Ui, section: SettingsSection) {
        ui.spacing_mut().item_spacing.y = 12.0;
        egui::ScrollArea::vertical()
            .id_salt("global-settings")
            .max_height((ui.available_height() - 72.0).max(180.0))
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(theme::BG)
                    .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
                    .corner_radius(10)
                    .inner_margin(18)
                    .show(ui, |ui| {
                        ui.set_min_width((ui.available_width() - 2.0).max(280.0));
                        ui.set_min_height(330.0);
                        match section {
                            SettingsSection::Appearance => self.appearance_preferences(ui),
                            SettingsSection::Network => self.network_preferences(ui),
                            SettingsSection::Storage => self.storage_preferences(ui),
                            SettingsSection::Servers => {}
                        }
                    });
            });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if primary(ui, "Apply preferences") {
                self.save_preferences(ui.ctx());
            }
            if self.notice.starts_with("Preferences saved")
                || self.notice.starts_with("preferences failed:")
            {
                ui.label(
                    egui::RichText::new(self.scrub_text(&self.notice))
                        .size(13.0)
                        .color(if self.notice.starts_with("preferences failed:") {
                            theme::DANGER
                        } else {
                            theme::DIM
                        }),
                );
            } else {
                ui.label(theme::dim(
                    "Saved preferences are restored when you next open the app.",
                ));
            }
        });
    }

    fn appearance_preferences(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::heading("Appearance"));
        ui.label(theme::dim(
            "Adjust the interface and your default browsing view.",
        ));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            setting_label(ui, "Interface scale");
            ui.add(
                egui::Slider::new(&mut self.preferences.ui_scale, 0.75..=2.0)
                    .step_by(0.05)
                    .suffix("×"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            setting_label(ui, "Default view");
            egui::ComboBox::from_id_salt("preference-default-view")
                .width(170.0)
                .selected_text(if self.preferences.default_gallery {
                    "Gallery"
                } else {
                    "Articles"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preferences.default_gallery, false, "Articles");
                    ui.selectable_value(&mut self.preferences.default_gallery, true, "Gallery");
                });
        });
        ui.checkbox(
            &mut self.preferences.sidebar_open,
            "Show the newsgroup sidebar by default",
        );
        ui.checkbox(
            &mut self.preferences.gallery_auto,
            "Load Gallery media previews automatically",
        );
        ui.label(theme::dim(
            "Previews stay in RAM. Files are saved only when you choose Save or a download action.",
        ));
    }

    fn network_preferences(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::heading("Network"));
        ui.label(theme::dim(
            "Limits for temporary Gallery previews and NNTP connections.",
        ));
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            setting_label(ui, "Preview file limit");
            ui.add(
                egui::DragValue::new(&mut self.preferences.preview_limit_mib)
                    .range(1..=10)
                    .suffix(" MiB"),
            );
            ui.label(theme::dim("Maximum size of one automatic preview."));
        });
        ui.horizontal_wrapped(|ui| {
            setting_label(ui, "Preview memory");
            ui.add(
                egui::DragValue::new(&mut self.preferences.preview_cache_mib)
                    .range(8..=64)
                    .suffix(" MiB"),
            );
            ui.label(theme::dim("Budget for original encoded media bytes."));
        });
        ui.label(theme::dim(
            "Previews open connections on demand using each server’s configured connection limit.",
        ));
        ui.horizontal_wrapped(|ui| {
            setting_label(ui, "NNTP timeout");
            ui.add(
                egui::DragValue::new(&mut self.preferences.nntp_timeout_secs)
                    .range(2..=120)
                    .suffix(" s"),
            );
            ui.label(theme::dim("Used when opening new server connections."));
        });
        ui.add_space(4.0);
        ui.label(theme::dim("Per-server connection counts, pipelining, security, and account settings are under Servers."));
    }

    fn storage_preferences(&mut self, ui: &mut egui::Ui) {
        ui.label(theme::heading("Storage"));
        ui.label(theme::dim(
            "Use absolute folder paths, or leave a field empty for its default.",
        ));
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Download directory").size(14.0));
        ui.add(
            egui::TextEdit::singleline(&mut self.preferences.downloads_dir)
                .hint_text("Default: data directory / downloads")
                .desired_width(ui.available_width()),
        );
        ui.label(theme::dim(
            "Applies to new saves and downloads. Existing jobs keep their current destination.",
        ));
        ui.label(egui::RichText::new("Data directory").size(14.0));
        ui.add(
            egui::TextEdit::singleline(&mut self.preferences.data_dir)
                .hint_text("Default platform data directory")
                .desired_width(ui.available_width()),
        );
        ui.label(theme::dim(
            "Data directory applies after restart; existing data remains in the current folder.",
        ));
        ui.separator();
        for (label, path) in [
            ("Current data", self.handle.data_dir().to_path_buf()),
            ("Catalog", self.handle.data_dir().join("catalog.sqlite")),
            ("Headers", self.handle.data_dir().join("headers")),
            (
                "Article bodies",
                self.handle.data_dir().join("article-bodies"),
            ),
            ("Thumbnails", self.handle.data_dir().join("thumbs")),
            ("Downloads", self.configured_downloads_dir()),
            (
                "Preferences",
                self.handle.data_dir().join("preferences.json"),
            ),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [100.0, 18.0],
                    egui::Label::new(egui::RichText::new(label).size(12.0).color(theme::DIM)),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(path.to_string_lossy())
                            .monospace()
                            .size(12.0),
                    )
                    .truncate()
                    .selectable(true),
                )
                .on_hover_text(path.to_string_lossy());
            });
        }
        ui.label(theme::dim(
            "Gallery previews remain in RAM; they do not use these folders until you save them.",
        ));
    }
}

fn setting_label(ui: &mut egui::Ui, name: &str) {
    ui.add_sized([160.0, 28.0], egui::Label::new(theme::dim(name)));
}
