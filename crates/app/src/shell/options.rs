use super::*;

mod form;
mod global;
mod servers;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsSection {
    Appearance,
    Network,
    Storage,
    #[default]
    Servers,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SettingsPage {
    #[default]
    Connection,
    Account,
    Downloads,
}

impl Browser {
    pub(super) fn options_window(&mut self, ctx: &egui::Context) {
        if !self.options_open {
            return;
        }
        let viewport = ctx.content_rect().size();
        let mut close = false;
        egui::Window::new("Settings")
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size([
                (viewport.x - 64.0).clamp(340.0, 940.0),
                630.0_f32.min(viewport.y - 40.0),
            ])
            .min_width((viewport.x - 48.0).clamp(320.0, 700.0))
            .max_width((viewport.x - 32.0).max(320.0))
            .max_height((viewport.y - 40.0).max(400.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::heading("Settings"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("Close").clicked();
                    });
                });
                ui.label(theme::dim(
                    "Configure the interface, network, storage, and your news servers.",
                ));
                ui.add_space(8.0);
                let section_id = egui::Id::new("settings-section");
                let mut section = ctx
                    .data(|data| data.get_temp::<SettingsSection>(section_id))
                    .unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    dense(ui);
                    for (label, value) in [
                        ("Appearance", SettingsSection::Appearance),
                        ("Network", SettingsSection::Network),
                        ("Storage", SettingsSection::Storage),
                        ("Servers", SettingsSection::Servers),
                    ] {
                        if segment(ui, label, section == value) {
                            section = value;
                        }
                    }
                });
                ctx.data_mut(|data| data.insert_temp(section_id, section));
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                match section {
                    SettingsSection::Servers => self.options_body(ui),
                    _ => self.global_settings(ui, section),
                }
            });
        if close {
            self.options_open = false;
        }
    }

    pub(super) fn options_body(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(16.0, 10.0);
        if ui.available_width() < 680.0 {
            ui.horizontal_wrapped(|ui| {
                let current = self.editing;
                let name = self
                    .servers
                    .iter()
                    .find(|server| Some(server.id) == current)
                    .map(|server| server.name.as_str())
                    .unwrap_or("New server");
                let mut selected = current;
                egui::ComboBox::from_id_salt("compact-settings-servers")
                    .width(180.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for server in &self.servers {
                            ui.selectable_value(&mut selected, Some(server.id), &server.name);
                        }
                    });
                if selected != current {
                    if let Some(id) = selected {
                        self.edit_server(id);
                    }
                }
                if ui.button("+ Add server").clicked() {
                    self.begin_new_server();
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("compact-server-settings")
                .max_height(ui.available_height())
                .show(ui, |ui| self.server_form(ui));
            return;
        }
        let height = (ui.available_height() - 8.0).clamp(370.0, 440.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(222.0, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.server_list(ui),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().max(380.0), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| self.server_form(ui),
            );
        });
    }
}
