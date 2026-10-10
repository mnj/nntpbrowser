//! Server group discovery and the virtualized subscription picker.
use super::*;

mod names;
pub(super) mod sort;

impl Browser {
    pub(super) fn open_group_picker(&mut self, server_id: i64) {
        self.groups_open = true;
        self.picker_server = Some(server_id);
        self.picker_filter.clear();
        self.picker_name.clear();
        self.picker_indexes.clear();
        self.picker_dirty = true;
        self.fetch_group_list(server_id);
    }

    fn fetch_group_list(&mut self, server_id: i64) {
        self.picker_loading = true;
        self.samples.insert(
            server_id,
            SampleSet {
                groups: Vec::new(),
                received: 0,
                articles: 0,
                done: false,
                started: Instant::now(),
            },
        );
        self.handle.send(Command::ListActive {
            server_id,
            wildmat: "*".into(),
        });
    }

    pub(super) fn set_subscription(&mut self, server_id: i64, name: &str, subscribed: bool) {
        self.picker_dirty = true;
        self.handle.send(if subscribed {
            Command::Subscribe {
                server_id,
                name: name.into(),
            }
        } else {
            Command::Unsubscribe {
                server_id,
                name: name.into(),
            }
        });
        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| subscribed && group.server_id == server_id && group.name == name)
        {
            group.subscribed = subscribed;
        }
        // Unsubscribe is acknowledged only after cache cleanup succeeds.
    }

    pub(super) fn groups_window(&mut self, ctx: &egui::Context) {
        if !self.groups_open {
            return;
        }
        let mut open = true;
        let available = ctx.content_rect().size() - egui::vec2(48.0, 48.0);
        let initial = egui::vec2(
            available.x.clamp(620.0, 1120.0),
            available.y.clamp(380.0, 740.0),
        );
        egui::Window::new("Subscribe to newsgroups")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(initial)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .min_size(egui::vec2(initial.x.min(980.0), initial.y.min(640.0)))
            .show(ctx, |ui| {
                // Reserve the same viewport during discovery and after completion.
                ui.set_min_size(ui.available_size());
                self.groups_body(ui);
            });
        self.groups_open = open;
    }

    fn groups_body(&mut self, ui: &mut egui::Ui) {
        dense(ui);
        let previous = self.picker_server;
        ui.horizontal(|ui| {
            let name = self
                .servers
                .iter()
                .find(|s| Some(s.id) == self.picker_server)
                .map(|s| s.name.as_str())
                .unwrap_or("Choose a server");
            egui::ComboBox::from_id_salt("subscribe-server")
                .selected_text(name)
                .show_ui(ui, |ui| {
                    for server in &self.servers {
                        ui.selectable_value(&mut self.picker_server, Some(server.id), &server.name);
                    }
                });
            if ui
                .add_enabled(!self.picker_loading, egui::Button::new("Refresh list"))
                .clicked()
            {
                if let Some(id) = self.picker_server {
                    self.fetch_group_list(id);
                }
            }
            if ui
                .button("Compare layouts")
                .on_hover_text("Preview eight ways to display this group list")
                .clicked()
            {
                self.open_group_layouts(ui.ctx());
            }
        });
        if self.picker_server != previous {
            if let Some(id) = self.picker_server {
                self.open_group_picker(id);
            }
        }
        let Some(server_id) = self.picker_server else {
            return;
        };
        if let Some(server) = self.servers.iter().find(|s| s.id == server_id) {
            ui.label(theme::dim(format!(
                "{}:{} · {}",
                server.host,
                server.port,
                if server.starttls {
                    "STARTTLS"
                } else if server.tls {
                    "Implicit TLS"
                } else {
                    "plain"
                }
            )));
        }
        let Some(sample) = self.samples.get(&server_id) else {
            return;
        };
        let elapsed = sample.started.elapsed().as_secs_f64();
        ui.add(
            egui::ProgressBar::new(if sample.done { 1.0 } else { 0.0 })
                .animate(self.picker_loading)
                .fill(theme::ACCENT)
                .desired_width(ui.available_width()),
        );
        if self.picker_loading {
            ui.label(theme::dim(format!(
                "{} groups · {} articles · {:.1}s · {:.0} groups/s",
                sample.received,
                sample.articles,
                elapsed,
                sample.received as f64 / elapsed.max(0.1)
            )));
            ui.label(theme::dim("Downloading the server’s group list…"));
            return;
        }
        if !sample.done {
            ui.label(
                egui::RichText::new(
                    "The group list could not be completed. See the status bar, then retry.",
                )
                .color(theme::WARN),
            );
            return;
        }
        let subscribed: HashSet<String> = self
            .groups
            .iter()
            .filter(|g| g.server_id == server_id && g.subscribed)
            .map(|g| g.name.clone())
            .collect();
        let count = sample
            .groups
            .iter()
            .filter(|g| subscribed.contains(&g.name))
            .count();
        ui.label(theme::dim(format!(
            "{} groups · {} articles · {} subscribed",
            sample.groups.len(),
            sample.articles,
            count
        )));
        ui.add(
            egui::TextEdit::singleline(&mut self.picker_filter)
                .hint_text("Filter newsgroups locally…")
                .desired_width(ui.available_width()),
        );
        if self.picker_dirty || self.picker_applied_filter != self.picker_filter {
            self.picker_indexes = filtered_group_indexes(&sample.groups, &self.picker_filter);
            self.picker_sort
                .apply(&mut self.picker_indexes, &sample.groups, &subscribed);
            names::rebuild(ui.ctx(), &sample.groups, &self.picker_indexes);
            self.picker_applied_filter = self.picker_filter.clone();
            self.picker_dirty = false;
        }
        ui.label(theme::dim(format!(
            "{} matching groups",
            self.picker_indexes.len()
        )));
        let name_layouts = names::cached(ui.ctx());
        // Only visible rows are copied; the full server list stays in one allocation.
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
        let name_w = (rect.width() - 215.0).max(80.0);
        let mut left = rect.left();
        for (column, width, title, right) in [
            (sort::Column::Subscribed, 30.0, "✓", false),
            (sort::Column::Name, name_w, "Newsgroup", false),
            (sort::Column::Articles, 90.0, "Articles", true),
            (sort::Column::Posting, 85.0, "Posting", false),
        ] {
            let cell = egui::Rect::from_min_size(
                egui::pos2(left, rect.top()),
                egui::vec2(width, rect.height()),
            );
            let response = ui.interact(
                cell,
                ui.id().with(("group-sort", left.to_bits())),
                egui::Sense::click(),
            );
            let label = self.picker_sort.heading(column, title);
            paint_cells(
                ui,
                left,
                cell,
                &[(
                    width,
                    &label,
                    if response.hovered() {
                        theme::TEXT
                    } else {
                        theme::DIM
                    },
                    right,
                )],
            );
            if response.clicked() {
                self.picker_sort.choose(column);
                self.picker_dirty = true;
            }
            response
                .on_hover_text(if column == sort::Column::Subscribed {
                    "Sort by subscription"
                } else {
                    "Click to sort; click again to reverse"
                })
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            left += width;
        }
        egui::ScrollArea::vertical()
            .id_salt("subscribe-list")
            .max_height((ui.available_height() - 58.0).max(100.0))
            .auto_shrink([false, false])
            .show_rows(ui, 28.0, self.picker_indexes.len(), |ui, range| {
                for row in range {
                    let group = self.samples[&server_id].groups[self.picker_indexes[row]].clone();
                    ui.push_id((&group.name, server_id), |ui| {
                        ui.horizontal(|ui| {
                            let mut on = subscribed.contains(&group.name);
                            if ui.checkbox(&mut on, "").changed() {
                                self.set_subscription(server_id, &group.name, on);
                            }
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 24.0),
                                egui::Sense::hover(),
                            );
                            let articles = group.posts().to_string();
                            let status = sort::posting_label(group.status);
                            let name_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(name_w, rect.height()),
                            );
                            names::paint(
                                ui,
                                name_rect,
                                &group.name,
                                name_layouts.get(row).copied(),
                            );
                            paint_cells(
                                ui,
                                rect.left() + name_w,
                                rect,
                                &[
                                    (90.0, &articles, theme::DIM, true),
                                    (85.0, status, theme::DIM, false),
                                ],
                            );
                            response.on_hover_text(format!(
                                "{}\n{} articles · {}",
                                group.name, articles, status
                            ));
                        });
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.picker_name)
                    .hint_text("Or enter an exact group name")
                    .desired_width((ui.available_width() - 120.0).max(100.0)),
            );
            if ui.button("Subscribe").clicked() {
                let name = self.picker_name.trim().to_string();
                if !name.is_empty() && !name.contains(char::is_whitespace) {
                    self.set_subscription(server_id, &name, true);
                    self.picker_name.clear();
                }
            }
        });
    }
}

pub(super) fn finish_sample(sample: &mut SampleSet) {
    sample
        .groups
        .retain(|g| !g.name.is_empty() && !g.name.contains(char::is_whitespace));
    sample
        .groups
        .sort_by_cached_key(|g| (g.name.to_ascii_lowercase(), g.name.clone()));
    sample.groups.dedup_by(|a, b| a.name == b.name);
    sample.articles = sample
        .groups
        .iter()
        .map(ListedGroup::posts)
        .fold(0, u64::saturating_add);
}

fn filtered_group_indexes(groups: &[ListedGroup], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    groups
        .iter()
        .enumerate()
        .filter(|(_, g)| needle.is_empty() || g.name.to_ascii_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}
