use super::*;

impl Browser {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        dense(ui);
        ui.horizontal_centered(|ui| {
            let (mark, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
            ui.painter().rect_filled(mark, 8.0, theme::ACCENT);
            ui.painter().text(
                mark.center(),
                egui::Align2::CENTER_CENTER,
                "N",
                FontId::proportional(22.0),
                theme::INK,
            );
            if ui.available_width() > 900.0 {
                ui.label(
                    egui::RichText::new("NNTPBrowser")
                        .family(egui::FontFamily::Name("semibold".into()))
                        .size(21.0),
                );
            }
            ui.add_space(12.0);
            for (label, view) in [
                ("Articles", BrowseView::Articles),
                ("Gallery", BrowseView::Gallery),
            ] {
                if nav_tab(
                    ui,
                    label,
                    self.screen == Screen::Browse && self.view == view,
                ) {
                    self.choose_browse_view(view);
                }
            }
            ui.add_space(12.0);
            let group_width = (ui.available_width() - 205.0).clamp(150.0, 460.0);
            self.active_group_name(ui, group_width);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Settings").clicked() {
                    self.options_open = true;
                }
                if nav_tab(ui, "Downloads", self.screen == Screen::Queue) {
                    self.screen = Screen::Queue;
                    self.overlay = None;
                    self.preview_post = None;
                }
            });
        });
    }

    fn choose_browse_view(&mut self, view: BrowseView) {
        let changed = self.view != view;
        self.screen = Screen::Browse;
        self.overlay = None;
        self.view = view;
        if changed {
            self.preview_post = None;
            self.feed_dirty = true;
            if view == BrowseView::Gallery {
                self.filter = FeedFilter::Media;
            }
            if self.gallery_dates.bounds() != (None, None) {
                self.reload_posts();
            }
        }
        if view == BrowseView::Articles {
            if let Some(id) = self.open_post {
                self.focus_post(id);
            }
        }
    }

    fn active_group_name(&self, ui: &mut egui::Ui, width: f32) {
        let name = self
            .groups
            .iter()
            .find(|group| Some(group.id) == self.selected_group)
            .map(|group| group.name.as_str())
            .unwrap_or("Choose a newsgroup on the left");
        let server = self
            .servers
            .iter()
            .find(|server| Some(server.id) == self.selected_server)
            .map(|server| server.name.as_str());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 34.0), egui::Sense::hover());
        clip_text(
            ui,
            rect,
            name,
            theme::TEXT,
            FontId::new(17.0, egui::FontFamily::Name("semibold".into())),
        );
        ui.interact(
            rect,
            ui.id().with("active-group-name"),
            egui::Sense::hover(),
        )
        .on_hover_text(
            server.map_or_else(|| name.to_owned(), |server| format!("{server}\n{name}")),
        );
    }

    pub(super) fn content(&mut self, ui: &mut egui::Ui) {
        match self.screen {
            Screen::Browse => self.browse(ui),
            Screen::Queue => self.queue_screen(ui),
        }
    }
}
