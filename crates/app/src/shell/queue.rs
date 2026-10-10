use super::*;

mod active;
mod completed;
mod icons;
mod progress;
mod state;
pub(super) use progress::{AggregateProgress, JobProgress};
use state::{DownloadsState, DownloadsTab};

fn download_title(ui: &mut egui::Ui, name: &str, width: f32) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add(egui::Label::new(egui::RichText::new(name).size(17.0)).truncate())
                .on_hover_text(name);
        },
    );
}

impl Browser {
    pub(super) fn queue_screen(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::new("downloads-state");
        let mut state = ui
            .ctx()
            .data_mut(|data| data.get_temp::<DownloadsState>(id))
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(theme::heading("Downloads"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Import NZB…").clicked() {
                    state.import_open = !state.import_open;
                }
            });
        });
        ui.label(theme::dim("Manage transfers and saved files in one place."));
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            let pending = self.jobs.iter().filter(|job| job.state != "done").count();
            let completed = self.jobs.len() - pending;
            for (tab, label, count) in [
                (DownloadsTab::Active, "Transfers", pending),
                (DownloadsTab::Completed, "Completed", completed),
            ] {
                if ui
                    .selectable_label(state.tab == tab, format!("{label}  {count}"))
                    .clicked()
                {
                    state.tab = tab;
                }
            }
            ui.add_space(12.0);
            ui.add(
                egui::TextEdit::singleline(&mut state.search)
                    .hint_text("Search downloads…")
                    .desired_width(200.0),
            );
        });
        ui.add_space(12.0);
        if state.import_open {
            theme::card().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.nzb_path)
                            .hint_text("Path to an .nzb file…")
                            .desired_width((ui.available_width() - 120.0).max(80.0)),
                    );
                    if primary(ui, "Import") {
                        self.import_nzb();
                    }
                });
            });
            ui.add_space(12.0);
        }
        let query = state.search.to_lowercase();
        let jobs = self
            .jobs
            .iter()
            .filter(|job| state.tab.includes(job) && job.name.to_lowercase().contains(&query))
            .cloned()
            .collect::<Vec<_>>();
        match state.tab {
            DownloadsTab::Active => self.active_downloads(ui, &jobs, &mut state),
            DownloadsTab::Completed => self.completed_downloads(ui, &jobs, &mut state),
        }
        self.download_delete_dialog(ui.ctx(), &mut state);
        ui.ctx().data_mut(|data| data.insert_temp(id, state));
    }

    pub(super) fn import_nzb(&mut self) {
        let path = PathBuf::from(self.nzb_path.trim());
        if path.as_os_str().is_empty() {
            self.notice = "Choose an NZB path first.".into();
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(xml) => {
                let dest = self.configured_downloads_dir();
                self.handle.send(Command::ImportNzb { xml, dest });
            }
            Err(error) => self.notice = format!("nzb file: {error}"),
        }
    }
}

#[cfg(feature = "ui-preview")]
pub(super) fn preview_completed(ctx: &egui::Context, delete: Option<(JobRow, usize)>) {
    let mut state = DownloadsState {
        tab: DownloadsTab::Completed,
        ..Default::default()
    };
    state.delete = delete.map(|(job, file_count)| state::DeleteRequest {
        job,
        file_count,
        scanning: false,
    });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("downloads-state"), state));
}
