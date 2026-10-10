use super::*;
use state::DownloadsState;

impl Browser {
    pub(super) fn completed_downloads(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &[JobRow],
        state: &mut DownloadsState,
    ) {
        ui.horizontal(|ui| {
            ui.label(theme::dim(
                "Saved files stay on disk until you delete them.",
            ));
            if ui.button("Refresh files").clicked() {
                self.library.request(&self.jobs, true);
            }
            if self.library.loading() {
                ui.spinner();
            }
        });
        ui.add_space(8.0);
        if jobs.is_empty() {
            ui.add_space(28.0);
            ui.label(theme::heading(if state.search.is_empty() {
                "No saved downloads yet"
            } else {
                "No matching downloads"
            }));
            ui.label(theme::dim(
                "Completed downloads and media you explicitly save appear here.",
            ));
            return;
        }
        let items = self.library.items.clone();
        let images = self.library.images.clone();
        egui::ScrollArea::vertical().id_salt("completed-downloads").auto_shrink([false,false]).show(ui,|ui|{
            for (row,job) in jobs.iter().enumerate() {
                theme::card().show(ui,|ui|{
                    ui.horizontal(|ui|{
                        let width=(ui.available_width()-icons::row_width(4)-8.0).max(100.0);
                        download_title(ui,&job.name,width);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                            self.download_actions(ui,job,state);
                        });
                    });
                    let indexes=items.iter().enumerate().filter(|(_,item)|item.job_id==job.id).map(|(index,_)|index).collect::<Vec<_>>();
                    ui.label(theme::dim(format!("Completed · {} · {} saved file(s)",format_bytes(job.bytes_done),indexes.len())));
                    ui.add(egui::Label::new(theme::dim(&job.dest_dir)).truncate()).on_hover_text(&job.dest_dir);
                    egui::CollapsingHeader::new(format!("Files ({})",indexes.len())).id_salt(("download-files",job.id)).default_open(row==0).show(ui,|ui|{
                        if indexes.is_empty(){ui.label(theme::dim(if self.library.loading(){"Scanning saved files…"}else{"No files found at this folder. Refresh after changing files outside the app."}));return;}
                        let cols=grid_columns(ui.available_width(),170.0);let rows=indexes.len().div_ceil(cols);
                        let width=(ui.available_width()/cols as f32).max(140.0)-8.0;
                        egui::ScrollArea::vertical().id_salt(("saved-files",job.id)).max_height(250.0).auto_shrink([false,true]).show_rows(ui,210.0,rows,|ui,range|{
                            for row in range {ui.horizontal(|ui|{
                                for col in 0..cols {if let Some(&index)=indexes.get(row*cols+col){self.lib_tile(ui,&items[index],index,&images,width);}}
                            });}
                        });
                    });
                });ui.add_space(8.0);
            }
        });
        if let Some(slot) = self.overlay {
            self.overlay_ui(ui, &items, &images, slot);
        }
    }
}
