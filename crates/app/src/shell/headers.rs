//! Header-fetch controls for the active newsgroup; transfer progress is in the footer.
use super::*;

const HEADER_LIMITS: &[(u64, &str)] = &[
    (10_000, "10,000"),
    (25_000, "25,000"),
    (50_000, "50,000"),
    (100_000, "100,000"),
    (250_000, "250,000"),
    (500_000, "500,000"),
    (1_000_000, "1 million"),
    (2_500_000, "2.5 million"),
    (5_000_000, "5 million"),
    (10_000_000, "10 million"),
];

impl Browser {
    pub(super) fn header_controls(&mut self, ui: &mut egui::Ui) {
        let fetching = self.header_progress.is_some();
        ui.scope(|ui| {
            // Reserve the final control height before laying out labels and nested combos.
            ui.spacing_mut().interact_size.y = 32.0;
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::dim("Article headers"));
                ui.add_enabled_ui(!fetching, |ui| {
                    egui::ComboBox::from_id_salt("header-scope")
                        .width(124.0)
                        .selected_text(match self.header_scope {
                            HeaderScope::Latest => "Latest",
                            HeaderScope::New => "New only",
                            HeaderScope::All => "All available",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.header_scope, HeaderScope::Latest, "Latest");
                            ui.selectable_value(&mut self.header_scope, HeaderScope::New, "New since last fetch");
                            ui.selectable_value(&mut self.header_scope, HeaderScope::All, "All available on server");
                        })
                        .response
                        .on_hover_text("Latest gets the newest requested range. New only checks for newer articles. All available also fills older gaps. Cached ranges are skipped.");
                    if self.header_scope == HeaderScope::Latest {
                        let label = HEADER_LIMITS
                            .iter()
                            .find_map(|&(limit, label)| (limit == self.header_limit).then_some(label))
                            .map(str::to_owned)
                            .unwrap_or_else(|| self.header_limit.to_string());
                        egui::ComboBox::from_id_salt("header-limit")
                            .width(112.0)
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for &(limit, label) in HEADER_LIMITS {
                                    ui.selectable_value(&mut self.header_limit, limit, label);
                                }
                            })
                            .response
                            .on_hover_text("Number of newest article headers to fetch. Cached ranges are skipped.");
                        ui.label(theme::dim("headers"));
                    }
                });
                if ui.add_enabled(
                    self.selected_group.is_some() && !self.posts_loading && !fetching,
                    egui::Button::new(egui::RichText::new("Get article headers").color(theme::INK))
                        .fill(theme::ACCENT),
                ).clicked() {
                    self.fetch_selected_headers();
                }
            });
        });
        if self.posts_loading && !fetching {
            ui.add_space(4.0);
            ui.add(
                egui::ProgressBar::new(0.0)
                    .animate(true)
                    .text("Loading cached articles…")
                    .fill(theme::ACCENT)
                    .desired_width(ui.available_width()),
            );
        }
    }
}
