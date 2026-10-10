use super::*;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum DownloadsTab {
    #[default]
    Active,
    Completed,
}

impl DownloadsTab {
    pub(super) fn includes(self, job: &JobRow) -> bool {
        (job.state == "done") == (self == Self::Completed)
    }
}

#[derive(Clone)]
pub(super) struct DeleteRequest {
    pub(super) job: JobRow,
    pub(super) file_count: usize,
    pub(super) scanning: bool,
}

#[derive(Default, Clone)]
pub(super) struct DownloadsState {
    pub(super) tab: DownloadsTab,
    pub(super) search: String,
    pub(super) import_open: bool,
    pub(super) delete: Option<DeleteRequest>,
}

pub(super) fn can_remove(job: &JobRow) -> bool {
    matches!(job.state.as_str(), "done" | "failed" | "cancelled")
}

fn review_matches(current: &JobRow, reviewed: &JobRow) -> bool {
    can_remove(current)
        && current.id == reviewed.id
        && current.name == reviewed.name
        && current.dest_dir == reviewed.dest_dir
        && current.dest_path == reviewed.dest_path
        && current.post_id == reviewed.post_id
}

impl Browser {
    pub(super) fn download_actions(
        &mut self,
        ui: &mut egui::Ui,
        job: &JobRow,
        state: &mut DownloadsState,
    ) {
        // The row uses right-to-left layout, placing deletion at its outer edge.
        let enabled = can_remove(job);
        if icons::button(ui, icons::Action::Delete, enabled).clicked() {
            state.delete = Some(DeleteRequest {
                job: job.clone(),
                file_count: self
                    .library
                    .items
                    .iter()
                    .filter(|item| item.job_id == job.id)
                    .count(),
                scanning: self.library.loading(),
            });
        }
        if icons::button(ui, icons::Action::Remove, enabled).clicked() {
            self.handle.send(Command::DeleteDownload {
                job_id: job.id,
                remove_files: false,
            });
        }
        if icons::button(ui, icons::Action::Copy, true).clicked() {
            ui.ctx().copy_text(job.dest_dir.clone());
        }
        if icons::button(ui, icons::Action::Folder, true).clicked() {
            open_system(Path::new(&job.dest_dir));
        }
    }

    pub(super) fn download_delete_dialog(
        &mut self,
        ctx: &egui::Context,
        state: &mut DownloadsState,
    ) {
        let Some(request) = state.delete.clone() else {
            return;
        };
        let current = self.jobs.iter().find(|job| job.id == request.job.id);
        let allowed = current.is_some_and(|job| review_matches(job, &request.job));
        let mut close = false;
        egui::Modal::new(egui::Id::new("delete-download")) .show(ctx,|ui| {
            ui.set_width(460.0);
            ui.label(theme::heading("Delete downloaded files?"));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(&request.job.name).color(theme::TEXT).strong());
            ui.label("This deletes the entire download folder and its contents, including extracted files, and removes it from Downloads.");
            ui.add_space(8.0);
            theme::card().show(ui,|ui|{
                ui.label(theme::dim(if request.scanning {"File count is still being scanned".into()} else if request.job.state!="done" {"File count is unavailable for this unfinished download".into()} else {format!("{} indexed file(s) · folder contents are deleted in full",request.file_count)}));
                ui.add(egui::Label::new(egui::RichText::new(&request.job.dest_dir).monospace()).wrap());
            });
            ui.add_space(8.0);ui.label(egui::RichText::new("This cannot be undone.").color(theme::DANGER));
            if !allowed {ui.label(theme::dim("This download changed. Close this dialog and review it again before deleting."));}
            ui.add_space(12.0);
            ui.horizontal(|ui|{
                if ui.button("Keep files").clicked(){close=true;}
                if ui.add_enabled(allowed,egui::Button::new(egui::RichText::new("Delete folder and files").color(theme::DANGER))).clicked(){
                    self.handle.send(Command::DeleteDownload{job_id:request.job.id,remove_files:true});close=true;
                }
            });
        });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close {
            state.delete = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_confirmation_never_applies_to_a_reused_job_or_changed_directory() {
        let reviewed = JobRow {
            id: 1,
            post_id: Some(7),
            name: "Photo".into(),
            dest_dir: "/downloads/photo-1".into(),
            dest_path: "/downloads/photo-1/photo.jpg".into(),
            state: "done".into(),
            bytes_done: 10,
            bytes_total: 10,
            error: None,
            password: None,
        };
        assert!(review_matches(&reviewed, &reviewed));
        let mut changed = reviewed.clone();
        changed.dest_dir = "/downloads/another-1".into();
        assert!(!review_matches(&changed, &reviewed));
        changed = reviewed.clone();
        changed.name = "Another download".into();
        assert!(!review_matches(&changed, &reviewed));
        changed = reviewed.clone();
        changed.state = "fetching".into();
        assert!(!review_matches(&changed, &reviewed));
        changed = reviewed.clone();
        changed.state = "paused".into();
        assert!(!review_matches(&changed, &reviewed));
    }
}
