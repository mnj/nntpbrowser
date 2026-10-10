use super::*;
use state::DownloadsState;

impl Browser {
    pub(super) fn active_downloads(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &[JobRow],
        state: &mut DownloadsState,
    ) {
        let total = AggregateProgress::active(jobs);
        if total.count > 0 {
            theme::card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&total.detail).size(14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(&total.compact).color(total.color));
                    });
                });
                ui.add(
                    egui::ProgressBar::new(total.fraction.unwrap_or(0.0))
                        .animate(total.animate)
                        .fill(total.color)
                        .desired_width(ui.available_width())
                        .desired_height(8.0),
                );
            });
            ui.add_space(12.0);
        }
        if jobs.is_empty() {
            ui.add_space(28.0);
            ui.label(theme::heading(if state.search.is_empty() {
                "No unfinished downloads"
            } else {
                "No matching downloads"
            }));
            ui.label(theme::dim(
                "Download article files from a newsgroup, or import an NZB.",
            ));
            if ui.button("Browse newsgroups").clicked() {
                self.screen = Screen::Browse;
            }
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("active-downloads")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for job in jobs {
                    theme::card().show(ui, |ui| {
                        let progress = JobProgress::new(job);
                        ui.horizontal(|ui| {
                            let transfer_actions = match job.state.as_str() {
                                "paused" => 2,
                                "failed" | "cancelled" => 1,
                                _ if is_active(&job.state) => 2,
                                _ => 0,
                            };
                            let width = (ui.available_width()
                                - icons::row_width(4 + transfer_actions)
                                - 8.0)
                                .max(100.0);
                            download_title(ui, &job.name, width);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    self.download_actions(ui, job, state);
                                    match job.state.as_str() {
                                        "paused" => {
                                            if icons::button(ui, icons::Action::Cancel, true)
                                                .clicked()
                                            {
                                                self.handle.send(Command::Cancel(job.id));
                                            }
                                            if icons::button(ui, icons::Action::Resume, true)
                                                .clicked()
                                            {
                                                self.handle.send(Command::Resume(job.id));
                                            }
                                        }
                                        "failed" | "cancelled" => {
                                            if icons::button(ui, icons::Action::Retry, true)
                                                .clicked()
                                            {
                                                self.handle.send(Command::Resume(job.id));
                                            }
                                        }
                                        _ if is_active(&job.state) => {
                                            if icons::button(ui, icons::Action::Cancel, true)
                                                .clicked()
                                            {
                                                self.handle.send(Command::Cancel(job.id));
                                            }
                                            if icons::button(ui, icons::Action::Pause, true)
                                                .clicked()
                                            {
                                                self.handle.send(Command::Pause(job.id));
                                            }
                                        }
                                        _ => {}
                                    }
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(progress.stage).color(progress.color));
                            ui.label(theme::dim(format!(
                                "{}{}",
                                format_bytes(job.bytes_done),
                                if job.state == "recovery_fetching" {
                                    " transferred".into()
                                } else if job.bytes_total > 0 {
                                    format!(" / {}", format_bytes(job.bytes_total))
                                } else {
                                    " / unknown".into()
                                }
                            )));
                            if matches!(job.state.as_str(), "fetching" | "recovery_fetching") {
                                let bps = self
                                    .speeds
                                    .get(&job.id)
                                    .map(|rate| rate.bytes_per_second(Instant::now()))
                                    .unwrap_or(0.0);
                                if bps > 0.0 {
                                    ui.label(theme::dim(format!(
                                        "{} /s",
                                        format_bytes(bps as i64)
                                    )));
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(theme::dim(&progress.detail));
                                },
                            );
                        });
                        ui.add(
                            egui::ProgressBar::new(progress.fraction)
                                .animate(progress.animate)
                                .fill(progress.color)
                                .desired_width(ui.available_width())
                                .desired_height(8.0),
                        );
                        if let Some(error) = &job.error {
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(self.scrub_text(error))
                                    .color(theme::DANGER)
                                    .size(13.0),
                            );
                        }
                    });
                    ui.add_space(8.0);
                }
            });
    }
}
