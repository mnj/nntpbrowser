//! Native comparison dialog; all controls edit context-local preview state.
use super::{egui, rows, sorted, state_id, theme, tree, Browser, Sort, State, Style, STYLES};

pub(super) fn show(browser: &mut Browser, ctx: &egui::Context) {
    if !browser.group_layouts_open {
        return;
    }
    let mut open = true;
    let mut close = false;
    let mut state = ctx.data_mut(|data| data.get_temp::<State>(state_id()).unwrap_or_default());
    let available = ctx.content_rect().size();
    let width = (available.x - 80.0).clamp(680.0, 1060.0);
    let preview_height = (available.y - 440.0).clamp(230.0, 400.0);
    egui::Window::new("Compare group layouts")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(width)
        .show(ctx, |ui| {
            ui.set_min_width(width - 32.0);
            ui.add(egui::Label::new(theme::dim(&state.source)).truncate())
                .on_hover_text(&state.source);
            ui.add_space(6.0);
            let mut divider = None;
            let body = ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(190.0);
                    ui.label(egui::RichText::new("LAYOUT").size(11.0).color(theme::DIM));
                    ui.spacing_mut().item_spacing.y = 6.0;
                    for (index, style) in STYLES.into_iter().enumerate() {
                        let selected = state.style == style;
                        if ui
                            .add_sized(
                                [190.0, 34.0],
                                egui::Button::new(
                                    egui::RichText::new(format!(
                                        "{}  {}",
                                        index + 1,
                                        style.title()
                                    ))
                                    .size(13.0)
                                    .color(theme::TEXT),
                                )
                                .selected(selected),
                            )
                            .on_hover_text(style.caption())
                            .clicked()
                        {
                            state.style = style;
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Same groups, different presentation.")
                            .size(12.0)
                            .color(theme::DIM),
                    );
                });
                divider = Some(ui.cursor().min.x + 3.0);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width((width - 260.0).max(370.0));
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(state.style.title())
                                .size(20.0)
                                .strong()
                                .color(theme::TEXT),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(theme::dim("Sort"));
                        egui::ComboBox::from_id_salt("layout-preview-sort")
                            .selected_text(match state.sort {
                                Sort::Name => "Name",
                                Sort::Articles => "Articles",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut state.sort, Sort::Name, "Name");
                                ui.selectable_value(&mut state.sort, Sort::Articles, "Articles");
                            });
                        if ui
                            .button(if state.descending {
                                "↓ Descending"
                            } else {
                                "↑ Ascending"
                            })
                            .clicked()
                        {
                            state.descending = !state.descending;
                        }
                        ui.label(
                            egui::RichText::new(format!("{} groups", state.groups.len()))
                                .size(12.0)
                                .color(theme::DIM),
                        );
                    });
                    ui.label(
                        egui::RichText::new(state.style.caption())
                            .size(12.0)
                            .color(theme::DIM),
                    );
                    ui.add_space(3.0);
                    let groups = sorted(&state);
                    egui::Frame::new()
                        .fill(theme::BG)
                        .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
                        .corner_radius(8)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            rows::header(ui, state.style);
                            egui::ScrollArea::vertical()
                                .id_salt(("layout-preview-list", state.style as u8))
                                .max_height(preview_height)
                                .min_scrolled_height(preview_height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    if groups.is_empty() {
                                        ui.label(theme::dim(
                                            "No matching groups in this snapshot.",
                                        ));
                                    } else {
                                        match state.style {
                                            Style::Tree => tree::paint(ui, &groups, &state),
                                            Style::Sections => rows::sections(ui, &groups, &state),
                                            style => {
                                                for group in groups {
                                                    rows::paint(ui, &group, style);
                                                }
                                            }
                                        }
                                    }
                                });
                        });
                });
            });
            if let Some(x) = divider {
                ui.painter().line_segment(
                    [
                        egui::pos2(x, body.response.rect.top()),
                        egui::pos2(x, body.response.rect.bottom()),
                    ],
                    egui::Stroke::new(1.0, theme::HAIRLINE),
                );
            }
            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(
                        "Comparison only · subscriptions and settings stay unchanged.",
                    )
                    .size(12.0)
                    .color(theme::DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = ui.button("Close").clicked();
                });
            });
        });
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
    browser.group_layouts_open = open && !close;
}
