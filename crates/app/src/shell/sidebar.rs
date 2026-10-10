use super::*;

impl Browser {
    pub(super) fn tree(&mut self, ui: &mut egui::Ui) {
        dense(ui);
        ui.horizontal(|ui| {
            ui.label(theme::heading("Newsgroups"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if panel_toggle(ui, true).clicked() {
                    self.sidebar_open = false;
                }
            });
        });
        ui.add_space(2.0);
        ui.label(theme::dim("Subscribed groups"));
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);
        if self.servers.is_empty() {
            ui.label(theme::dim("No servers yet."));
            ui.label(theme::dim("Add a server in Settings to get started."));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("group-tree")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let servers = self.servers.clone();
                for server in servers {
                    self.server_node(ui, &server);
                    ui.add_space(4.0);
                }
            });
    }

    pub(super) fn minimized_tree(&mut self, ui: &mut egui::Ui) {
        dense(ui);
        ui.vertical_centered(|ui| {
            if panel_toggle(ui, false).clicked() {
                self.sidebar_open = true;
            }
            ui.add_space(10.0);
            let count = self.groups.iter().filter(|group| group.subscribed).count();
            ui.label(
                egui::RichText::new(count.to_string())
                    .size(14.0)
                    .color(theme::DIM),
            )
            .on_hover_text(format!("{count} subscribed newsgroups"));
        });
    }

    pub(super) fn server_node(&mut self, ui: &mut egui::Ui, server: &ServerRow) {
        let subscribed = self
            .groups
            .iter()
            .filter(|group| group.server_id == server.id && group.subscribed)
            .count();
        let mut title = format!(
            "{}  {}",
            if server.enabled { "●" } else { "○" },
            server.name
        );
        if subscribed > 0 {
            title.push_str(&format!("  ({subscribed})"));
        }
        if server.fill {
            title.push_str("  · fill");
        }
        if !server.enabled {
            title.push_str("  · off");
        }
        let text = if server.enabled {
            egui::RichText::new(title).size(15.0).color(theme::TEXT)
        } else {
            egui::RichText::new(title).size(15.0).color(theme::DIM)
        };
        let header = egui::CollapsingHeader::new(text)
            .id_salt(("server", server.id))
            .default_open(self.servers.len() == 1 || self.selected_server == Some(server.id))
            .show(ui, |ui| self.server_body(ui, server.id));
        let server_id = server.id;
        header.header_response.context_menu(|ui| {
            if ui.button("Edit server").clicked() {
                self.edit_server(server_id);
                ui.close();
            }
        });
    }

    pub(super) fn server_body(&mut self, ui: &mut egui::Ui, server_id: i64) {
        let mut groups: Vec<GroupRow> = self
            .groups
            .iter()
            .filter(|group| group.server_id == server_id && group.subscribed)
            .cloned()
            .collect();
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        if groups.is_empty() {
            ui.label(theme::dim("Subscribe opens the server’s group list."));
        }
        for group in groups {
            let selected = self.selected_group == Some(group.id);
            let response = ui.add_sized(
                [ui.available_width(), 29.0],
                egui::Button::new((
                    egui::RichText::new(&group.name)
                        .size(14.0)
                        .color(if selected { theme::ACCENT } else { theme::TEXT }),
                    egui::Atom::grow(),
                ))
                .gap(0.0)
                .fill(if selected {
                    theme::SELECT
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(egui::Stroke::NONE)
                .wrap_mode(egui::TextWrapMode::Truncate),
            );
            if response.clicked() {
                self.pick_group(group.id, server_id);
            }
            response.clone().on_hover_text(format!(
                "{}\nHeaders through article {}",
                group.name, group.watermark
            ));
            response.context_menu(|ui| {
                if ui.button("Get article headers").clicked() {
                    self.pick_group(group.id, server_id);
                    self.fetch_selected_headers();
                    ui.close();
                }
                if ui.button("Unsubscribe").clicked() {
                    self.set_subscription(server_id, &group.name, false);
                    ui.close();
                }
                if ui.button("Copy name").clicked() {
                    ui.ctx().copy_text(group.name.clone());
                    ui.close();
                }
            });
        }
        ui.add_space(6.0);
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("+ Subscribe to groups").color(theme::ACCENT),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE),
            )
            .clicked()
        {
            self.open_group_picker(server_id);
        }
    }
}

/// A drawn panel icon keeps the collapse control crisp at every interface scale.
fn panel_toggle(ui: &mut egui::Ui, expanded: bool) -> egui::Response {
    let label = if expanded {
        "Collapse newsgroups panel"
    } else {
        "Expand newsgroups panel"
    };
    let response = ui.add_sized(
        [34.0, 34.0],
        egui::Button::new("")
            .fill(theme::RAISED)
            .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
            .corner_radius(8),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(response.rect.shrink(1.0), 7, theme::SELECT);
    }
    let icon = egui::Rect::from_center_size(response.rect.center(), egui::vec2(20.0, 17.0));
    let color = if response.hovered() || response.has_focus() {
        theme::ACCENT
    } else {
        theme::DIM
    };
    let stroke = egui::Stroke::new(1.4, color);
    painter.rect_stroke(icon, 3, stroke, egui::StrokeKind::Inside);
    let divider = icon.left() + 6.0;
    painter.line_segment(
        [
            egui::pos2(divider, icon.top() + 1.0),
            egui::pos2(divider, icon.bottom() - 1.0),
        ],
        stroke,
    );
    let center = egui::pos2(icon.left() + 13.0, icon.center().y);
    let direction = if expanded { -1.0 } else { 1.0 };
    painter.add(egui::Shape::line(
        vec![
            center + egui::vec2(-direction * 1.5, -3.0),
            center + egui::vec2(direction * 1.5, 0.0),
            center + egui::vec2(-direction * 1.5, 3.0),
        ],
        stroke,
    ));
    response
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
