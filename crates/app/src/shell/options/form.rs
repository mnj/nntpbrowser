use super::*;

impl Browser {
    pub(in crate::shell) fn server_form(&mut self, ui: &mut egui::Ui) {
        let title = if self.editing.is_some() && !self.form.name.is_empty() {
            self.form.name.clone()
        } else {
            "New server".into()
        };
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(theme::heading(&title)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.form.enabled, "Enabled");
            });
        });
        ui.label(theme::dim(if self.editing.is_some() {
            "Edit this server’s connection and account."
        } else {
            "Enter the settings supplied by your Usenet provider."
        }));
        let page_id = egui::Id::new("settings-page");
        let mut page = ui
            .ctx()
            .data(|data| data.get_temp::<SettingsPage>(page_id))
            .unwrap_or_default();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            dense(ui);
            for (label, value) in [
                ("Connection", SettingsPage::Connection),
                ("Account", SettingsPage::Account),
                ("Downloads", SettingsPage::Downloads),
            ] {
                if segment(ui, label, page == value) {
                    page = value;
                }
            }
        });
        ui.ctx().data_mut(|data| data.insert_temp(page_id, page));
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(theme::BG)
            .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
            .corner_radius(10)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_min_width((ui.available_width() - 2.0).max(300.0));
                ui.set_min_height(214.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                match page {
                    SettingsPage::Connection => self.connection_settings(ui),
                    SettingsPage::Account => self.account_settings(ui),
                    SettingsPage::Downloads => self.download_settings(ui),
                }
            });
        ui.add_space(4.0);
        self.server_actions(ui);
    }

    fn connection_settings(&mut self, ui: &mut egui::Ui) {
        row(
            ui,
            "Display name",
            &mut self.form.name,
            false,
            "My news server",
        );
        row(
            ui,
            "Server address",
            &mut self.form.host,
            false,
            "news.example.com",
        );
        ui.horizontal(|ui| {
            label(ui, "Port");
            ui.add(egui::TextEdit::singleline(&mut self.form.port).desired_width(76.0));
        });
        ui.horizontal(|ui| {
            label(ui, "Security");
            let mut security = if self.form.starttls {
                "STARTTLS"
            } else if self.form.tls {
                "Implicit TLS"
            } else {
                "Plain NNTP"
            };
            let previous = security;
            egui::ComboBox::from_id_salt("server-security")
                .width(170.0)
                .selected_text(security)
                .show_ui(ui, |ui| {
                    for value in ["Implicit TLS", "STARTTLS", "Plain NNTP"] {
                        ui.selectable_value(&mut security, value, value);
                    }
                });
            if security != previous {
                self.form.tls = security == "Implicit TLS";
                self.form.starttls = security == "STARTTLS";
                if matches!(self.form.port.trim(), "119" | "563") {
                    self.form.port = if self.form.tls { "563" } else { "119" }.into();
                }
            }
        });
        ui.label(theme::dim(
            "Default ports: implicit TLS 563; STARTTLS or plain NNTP 119.",
        ));
        if let Some(server) = self
            .editing
            .and_then(|id| self.servers.iter().find(|server| server.id == id))
        {
            ui.add_space(4.0);
            ui.label(theme::dim(format!(
                "Header transport: {}",
                server.header_compression
            )));
        }
    }

    fn account_settings(&mut self, ui: &mut egui::Ui) {
        row(
            ui,
            "Username",
            &mut self.form.username,
            false,
            "Provider username",
        );
        row(
            ui,
            "Password",
            &mut self.form.password,
            true,
            "Provider password",
        );
        ui.add_space(6.0);
        ui.label(theme::dim("Use your Usenet provider’s login details. Leave both fields empty for anonymous access."));
        ui.label(theme::dim(
            "Your password stays hidden in the interface and connection errors.",
        ));
    }

    fn download_settings(&mut self, ui: &mut egui::Ui) {
        number_row(
            ui,
            "Connections",
            &mut self.form.connections,
            "Maximum simultaneous connections to this server.",
        );
        number_row(
            ui,
            "Pipeline depth",
            &mut self.form.pipeline,
            "Article requests sent per connection before waiting.",
        );
        number_row(
            ui,
            "Priority",
            &mut self.form.priority,
            "Lower numbers are tried first.",
        );
        ui.add_space(2.0);
        ui.checkbox(&mut self.form.fill, "Use as a fill server");
        ui.label(theme::dim(
            "Try this server only when the other servers are missing a file part.",
        ));
    }

    fn server_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            dense(ui);
            if primary(ui, "Save changes") {
                self.save_server();
            }
            if ui
                .add_enabled(self.editing.is_some(), egui::Button::new("Test connection"))
                .on_disabled_hover_text("Save this server before testing the connection.")
                .clicked()
            {
                if let Some(id) = self.editing {
                    self.handle.send(Command::TestServer(id));
                    self.notice = "Testing the connection…".into();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.confirm_delete
                    && ui
                        .add_enabled(
                            self.editing.is_some(),
                            egui::Button::new(
                                egui::RichText::new("Delete server").color(theme::DANGER),
                            ),
                        )
                        .clicked()
                {
                    self.confirm_delete = true;
                }
            });
        });
        if self.confirm_delete {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                dense(ui);
                ui.label(
                    egui::RichText::new("Delete this server and its saved groups?")
                        .size(13.0)
                        .color(theme::DANGER),
                );
                if ui.button("Delete").clicked() {
                    self.delete_editing();
                }
                if ui.button("Keep server").clicked() {
                    self.confirm_delete = false;
                }
            });
        } else {
            let notice = self.notice.to_lowercase();
            if [
                "saved the server",
                "deleted the server",
                "testing the connection",
                "connected, overview",
                "connected, list failed",
                "connection failed:",
                "test server:",
                "save server:",
                "name, host, and port",
                "update server:",
                "delete server:",
                "delete server cache:",
            ]
            .iter()
            .any(|prefix| notice.starts_with(prefix))
            {
                let color = if notice.contains("failed")
                    || notice.contains("error")
                    || notice.starts_with("name, host")
                {
                    theme::DANGER
                } else {
                    theme::DIM
                };
                ui.label(
                    egui::RichText::new(self.scrub_text(&self.notice))
                        .size(13.0)
                        .color(color),
                );
            } else {
                ui.label(theme::dim("Changes apply when you save."));
            }
        }
    }
}

fn label(ui: &mut egui::Ui, name: &str) {
    ui.add_sized([112.0, 30.0], egui::Label::new(theme::dim(name)));
}

fn row(ui: &mut egui::Ui, name: &str, value: &mut String, password: bool, hint: &str) {
    ui.horizontal(|ui| {
        label(ui, name);
        ui.add(
            egui::TextEdit::singleline(value)
                .password(password)
                .hint_text(hint)
                .desired_width(ui.available_width()),
        );
    });
}

fn number_row(ui: &mut egui::Ui, name: &str, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        label(ui, name);
        ui.add(egui::TextEdit::singleline(value).desired_width(76.0));
        ui.add(egui::Label::new(egui::RichText::new(hint).size(12.0).color(theme::DIM)).wrap());
    });
}
