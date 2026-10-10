use super::*;

impl Browser {
    pub(super) fn browse(&mut self, ui: &mut egui::Ui) {
        self.feed_bar(ui);
        if self.view == BrowseView::Gallery && self.gallery_dates.show(ui, Self::now_unix()) {
            self.reload_posts();
        }
        let posts = self.filtered_posts();
        ui.add_space(4.0);
        if self.servers.is_empty() {
            self.placeholder(
                ui,
                "Add a server, then choose the newsgroups you want to browse.",
                "Add a server",
                true,
            );
            return;
        }
        if self.selected_group.is_none() {
            ui.add_space(28.0);
            ui.label(theme::heading("Browse"));
            ui.add_space(8.0);
            ui.label(theme::dim(
                "Choose a subscribed group on the left, or open Subscribe to find groups on a server.",
            ));
            ui.label(theme::dim(
                "Articles puts the list on top and the article underneath. Gallery scrolls that group as tiles.",
            ));
            return;
        }
        match self.view {
            BrowseView::Articles => self.articles(ui, &posts),
            BrowseView::Gallery => self.gallery(ui, &posts),
        }
    }

    pub(super) fn placeholder(
        &mut self,
        ui: &mut egui::Ui,
        sentence: &str,
        button: &str,
        add_server: bool,
    ) {
        ui.add_space(36.0);
        ui.label(egui::RichText::new(sentence).color(theme::DIM).size(16.0));
        ui.add_space(12.0);
        if primary(ui, button) {
            if add_server {
                self.begin_new_server();
            } else {
                self.options_open = true;
            }
        }
    }

    pub(super) fn feed_bar(&mut self, ui: &mut egui::Ui) {
        dense(ui);
        ui.horizontal_wrapped(|ui| {
            let filter_name = match self.filter {
                FeedFilter::All => "All articles",
                FeedFilter::Images => "Images",
                FeedFilter::Media => "Images and videos",
                FeedFilter::Sets => "Several files",
                FeedFilter::Repair => "Repair files",
                FeedFilter::Partial => "Missing parts",
            };
            ui.menu_button(format!("Filter: {filter_name}"), |ui| {
                for (filter, label, description) in [
                    (FeedFilter::All, "All articles", "Show every cached article."),
                    (FeedFilter::Media, "Images and videos", "Articles containing images or videos."),
                    (FeedFilter::Images, "Images", "Articles containing image files."),
                    (FeedFilter::Sets, "Several files", "Articles containing more than one file."),
                    (FeedFilter::Repair, "Repair files (PAR2)", "Articles containing PAR2 data used to repair damaged or missing file parts."),
                    (FeedFilter::Partial, "Missing file parts", "Articles whose files have fewer cached parts than expected. Older headers may contain the missing parts."),
                ] {
                    if ui.selectable_label(self.filter == filter, label).on_hover_text(description).clicked() {
                        self.filter = filter;
                        self.feed_dirty = true;
                        ui.close();
                    }
                }
            });
            ui.label(theme::dim(format!("{} cached articles shown", self.filtered_posts().len())))
                .on_hover_text("Articles currently displayed from your local header cache. This is not the server's total article count.");
            if self.posts_full && ui.button("Show more cached articles").clicked() {
                self.load_more();
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let reserved = if self.selection.is_empty() {
                340.0
            } else {
                455.0
            };
            let edit = egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search articles in this group…")
                .desired_width((ui.available_width() - reserved).max(120.0))
                .show(ui);
            if edit.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.reload_posts();
            }
            if !self.search.is_empty() && ui.small_button("×").on_hover_text("Clear search").clicked() {
                self.search.clear();
                self.reload_posts();
            }
            if !self.selection.is_empty() && ui.button("Clear selection").clicked() {
                self.selection.clear();
            }
            let label = if self.selection.is_empty() {
                "Download selected".into()
            } else { format!("Download selected ({})", self.selection.len()) };
            if ui.add_enabled(!self.selection.is_empty(),
                egui::Button::new(egui::RichText::new(label).color(theme::INK)).fill(theme::ACCENT))
                .on_hover_text("Download files from the articles you checked in the list or gallery.")
                .clicked() {
                self.download_basket();
            }
            ui.menu_button("Download range", |ui| {
                ui.set_max_width(320.0);
                ui.label("Download files from cached articles");
                ui.label(theme::dim("All file types in this group's cache matching your search, including articles not shown."));
                ui.add(
                    egui::DragValue::new(&mut self.download_limit)
                        .range(1..=u64::MAX)
                        .prefix("Newest "),
                );
                if ui.button("Download newest N articles").clicked() {
                    self.queue_group(Some(self.download_limit));
                    ui.close();
                }
                if ui.button("Download all cached matches").clicked() {
                    self.queue_group(None);
                    ui.close();
                }
            });
        });
        self.header_controls(ui);
        ui.add_space(6.0);
        ui.separator();
    }
}
