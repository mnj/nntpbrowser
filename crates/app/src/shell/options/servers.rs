use super::*;

impl Browser {
    pub(in crate::shell) fn server_list(&mut self, ui: &mut egui::Ui) {
        ui.set_width(222.0);
        ui.label(
            egui::RichText::new("NEWS SERVERS")
                .size(12.0)
                .color(theme::DIM),
        );
        ui.add_space(2.0);
        if ui
            .add_sized([222.0, 36.0], egui::Button::new("+ Add server"))
            .clicked()
        {
            self.begin_new_server();
        }
        ui.add_space(4.0);
        let servers = self.servers.clone();
        egui::ScrollArea::vertical()
            .id_salt("option-servers")
            .max_height((ui.available_height() - 32.0).max(200.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if servers.is_empty() {
                    ui.label(theme::dim("Add a server to start browsing newsgroups."));
                }
                for server in servers {
                    let selected = self.editing == Some(server.id);
                    let response = egui::Frame::new()
                        .fill(if selected { theme::SELECT } else { theme::BG })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if selected {
                                theme::ACCENT
                            } else {
                                theme::HAIRLINE
                            },
                        ))
                        .corner_radius(10)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(190.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.add(
                                egui::Label::new(egui::RichText::new(&server.name).size(17.0))
                                    .truncate(),
                            );
                            ui.add(
                                egui::Label::new(theme::dim(format!(
                                    "{}:{}",
                                    server.host, server.port
                                )))
                                .truncate(),
                            );
                            let status = if !server.enabled {
                                "Disabled"
                            } else if server.fill {
                                "Enabled · fill server"
                            } else {
                                "Enabled"
                            };
                            ui.label(egui::RichText::new(status).size(12.0).color(
                                if server.enabled {
                                    theme::ACCENT
                                } else {
                                    theme::DIM
                                },
                            ));
                        })
                        .response;
                    let response = ui.interact(
                        response.rect,
                        response.id.with("select"),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        self.edit_server(server.id);
                    }
                    response
                        .on_hover_text(format!("{}\n{}:{}", server.name, server.host, server.port));
                    ui.add_space(2.0);
                }
            });
    }
}
