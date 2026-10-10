//! Session diagnostics. Secrets are removed before a message enters the history.
use super::*;

const ERROR_CAP: usize = 200;

pub(super) struct ErrorEntry {
    pub(super) timestamp: i64,
    pub(super) context: String,
    pub(super) message: String,
    repeats: usize,
}

impl Browser {
    pub(super) fn record_error(&mut self, message: &str, context: &str) {
        self.error_count = self.error_count.saturating_add(1);
        let message = self.scrub_text(message);
        let context = self.scrub_text(context);
        if let Some(last) = self
            .error_history
            .back_mut()
            .filter(|entry| entry.message == message && entry.context == context)
        {
            last.timestamp = Self::now_unix();
            last.repeats = last.repeats.saturating_add(1);
            return;
        }
        self.error_history.push_back(ErrorEntry {
            timestamp: Self::now_unix(),
            context,
            message,
            repeats: 1,
        });
        while self.error_history.len() > ERROR_CAP {
            self.error_history.pop_front();
        }
    }

    pub(super) fn observe_error(&mut self, event: &Event) {
        match event {
            Event::HeaderFetchFinished {
                group_id,
                error: Some(error),
            } => {
                self.record_error(
                    error,
                    &self.group_error_context("Article headers", *group_id),
                );
            }
            Event::PostsFailed {
                group_id, error, ..
            }
            | Event::PostsDateFailed {
                group_id, error, ..
            } => {
                self.record_error(
                    error,
                    &self.group_error_context("Cached articles", *group_id),
                );
            }
            Event::MediaCacheFailed { post_id, error } => {
                self.record_error(error, &self.post_error_context("Media preview", *post_id));
            }
            Event::PreviewMediaFailed {
                post_id,
                generation,
                error,
            } if *generation == self.media_generation => {
                self.record_error(error, &self.post_error_context("Media preview", *post_id));
            }
            Event::MediaSaveFailed { post_id, error } => {
                self.record_error(error, &self.post_error_context("Save media", *post_id));
            }
            Event::DownloadDeleteFailed { job_id, error } => {
                self.record_error(error, &format!("Remove download · job {job_id}"));
            }
            Event::ArticleReadFailed { post_id, error } => {
                self.record_error(error, &self.post_error_context("Read article", *post_id));
            }
            Event::Jobs(jobs) => {
                for job in jobs.iter().filter(|job| job.state == "failed") {
                    if let Some(error) = job.error.as_deref() {
                        // A Jobs snapshot repeats previous failures; record transitions only.
                        if self.jobs.iter().any(|old| {
                            old.id == job.id && old.state == "failed" && old.error == job.error
                        }) {
                            continue;
                        }
                        let secrets = [job.password.as_deref().unwrap_or("")];
                        self.record_error(
                            &scrub(error, &secrets),
                            &scrub(&format!("Download #{} · {}", job.id, job.name), &secrets),
                        );
                    }
                }
            }
            Event::Notice(message) if is_error_notice(message) => {
                self.remember_notice_error(message)
            }
            _ => {}
        }
    }

    pub(super) fn remember_notice_error(&mut self, notice: &str) {
        let message = self.scrub_text(notice);
        // Typed failures also have a Notice. Keep the richer typed context once.
        if self
            .error_history
            .iter()
            .rev()
            .take(8)
            .any(|entry| entry.message == message || message.ends_with(&entry.message))
        {
            return;
        }
        let lower = message.to_lowercase();
        let context = if lower.starts_with("connection")
            || lower.starts_with("connected,")
            || lower.contains("server:")
            || lower.starts_with("delete server cache:")
        {
            self.editing
                .or(self.selected_server)
                .and_then(|id| self.servers.iter().find(|server| server.id == id))
                .map(|server| format!("Server · {}", server.name))
                .unwrap_or_else(|| "Server settings".into())
        } else if lower.starts_with("group list:") {
            let id = self.group_fetching.or(self.picker_server);
            id.and_then(|id| self.servers.iter().find(|server| server.id == id))
                .map(|server| format!("Group list · {}", server.name))
                .unwrap_or_else(|| "Group list".into())
        } else {
            "Application".into()
        };
        self.record_error(&message, &context);
    }

    fn group_error_context(&self, action: &str, id: i64) -> String {
        self.groups
            .iter()
            .find(|group| group.id == id)
            .map(|group| {
                let server = self
                    .servers
                    .iter()
                    .find(|server| server.id == group.server_id)
                    .map(|server| server.name.as_str())
                    .unwrap_or("Unknown server");
                format!("{action} · {server} / {}", group.name)
            })
            .unwrap_or_else(|| format!("{action} · group #{id}"))
    }

    fn post_error_context(&self, action: &str, id: i64) -> String {
        self.posts
            .iter()
            .find(|post| post.id == id)
            .map(|post| format!("{action} · {}", post.title))
            .unwrap_or_else(|| format!("{action} · article #{id}"))
    }

    pub(super) fn errors_window(&mut self, ctx: &egui::Context) {
        if !self.errors_open {
            return;
        }
        let mut open = true;
        let mut close = false;
        egui::Window::new("Error history")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .default_size([760.0, 450.0])
            .min_width(400.0)
            .max_height((ctx.content_rect().height() - 80.0).max(240.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::heading("Error history"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.button("Close").clicked();
                        if ui
                            .add_enabled(!self.error_history.is_empty(), egui::Button::new("Clear"))
                            .clicked()
                        {
                            self.error_history.clear();
                            self.error_count = 0;
                            self.notice = "History cleared.".into();
                        }
                        if ui
                            .add_enabled(
                                !self.error_history.is_empty(),
                                egui::Button::new("Copy all"),
                            )
                            .clicked()
                        {
                            ui.ctx().copy_text(
                                self.error_history
                                    .iter()
                                    .map(|entry| {
                                        format!(
                                            "{} · {}\n{}",
                                            format_stamp(entry.timestamp),
                                            entry.context,
                                            entry.message
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n\n"),
                            );
                        }
                    });
                });
                ui.label(theme::dim(format!(
                    "{} errors since clearing · newest first · last {ERROR_CAP} entries retained",
                    self.error_count
                )));
                ui.add_space(8.0);
                if self.error_history.is_empty() {
                    ui.label(theme::dim(
                        "No errors recorded. Failed actions will appear here.",
                    ));
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("error-history")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for entry in self.error_history.iter().rev() {
                                theme::card().show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(format_stamp(entry.timestamp))
                                                .size(12.0)
                                                .color(theme::DIM),
                                        );
                                        if entry.repeats > 1 {
                                            ui.label(theme::dim(format!(
                                                "Repeated {} times",
                                                entry.repeats
                                            )));
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.small_button("Copy").clicked() {
                                                    ui.ctx().copy_text(format!(
                                                        "{} · {}\n{}",
                                                        format_stamp(entry.timestamp),
                                                        entry.context,
                                                        entry.message
                                                    ));
                                                }
                                            },
                                        );
                                    });
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&entry.context).size(14.0),
                                        )
                                        .selectable(true)
                                        .wrap(),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&entry.message)
                                                .size(14.0)
                                                .color(theme::DANGER),
                                        )
                                        .selectable(true)
                                        .wrap(),
                                    );
                                });
                                ui.add_space(8.0);
                            }
                        });
                }
            });
        self.errors_open = open && !close;
    }
}

pub(super) fn is_error_notice(notice: &str) -> bool {
    let lower = notice.to_lowercase();
    if [
        "queued ",
        "synced ",
        "imported nzb,",
        "connected, overview",
        "job ",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        && !lower.contains(" failed:")
    {
        return false;
    }
    if lower.starts_with("preview: deferred")
        || lower.starts_with("preview: image needs a full download")
        || lower.starts_with("preview: no single-part image")
    {
        return false;
    }
    lower.contains("failed")
        || lower.contains("error")
        || [
            "save server:",
            "startup:",
            "catalog:",
            "headers:",
            "recover headers:",
            "update server:",
            "delete server:",
            "delete server cache:",
            "subscribe:",
            "unsubscribe:",
            "files:",
            "groups:",
            "group list:",
            "queue:",
            "nzb:",
            "nzb file:",
            "nzb queue:",
            "preview:",
            "server is gone",
            "article was not found",
            "no headers matched",
            "group is not subscribed",
            "name, host, and port",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}
