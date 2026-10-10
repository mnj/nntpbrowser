//! Compact flat previews and namespace sections.
use super::{
    data::{self, Group},
    egui, sort_groups, theme, State, Style,
};
use std::collections::BTreeMap;

pub(super) fn header(ui: &mut egui::Ui, style: Style) {
    let (row, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), egui::Sense::hover());
    let width = row.width() - 104.0;
    let painter = ui.painter();
    painter.text(
        row.left_center() + egui::vec2(7.0, 0.0),
        egui::Align2::LEFT_CENTER,
        if style == Style::Columns {
            "Namespace"
        } else {
            "Newsgroup"
        },
        egui::FontId::proportional(12.0),
        theme::DIM,
    );
    if style == Style::Columns {
        painter.text(
            row.left_center() + egui::vec2(width * 0.55, 0.0),
            egui::Align2::LEFT_CENTER,
            "Name",
            egui::FontId::proportional(12.0),
            theme::DIM,
        );
    }
    painter.text(
        row.right_center() - egui::vec2(9.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        "Articles",
        egui::FontId::proportional(12.0),
        theme::DIM,
    );
    painter.line_segment(
        [row.left_bottom(), row.right_bottom()],
        egui::Stroke::new(1.0, theme::HAIRLINE),
    );
}

pub(super) fn paint(ui: &mut egui::Ui, group: &Group, style: Style) {
    paint_name(ui, group, style, &group.name);
}

pub(super) fn paint_leaf(ui: &mut egui::Ui, group: &Group) {
    paint_name(ui, group, Style::Flat, data::split(&group.name).1);
}

fn paint_name(ui: &mut egui::Ui, group: &Group, style: Style, display: &str) {
    let (row, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 27.0), egui::Sense::hover());
    response.on_hover_text(&group.name);
    let count_left = row.right() - 104.0;
    let name_clip = egui::Rect::from_min_max(row.min, egui::pos2(count_left - 6.0, row.bottom()))
        .intersect(ui.clip_rect());
    if style == Style::Bands {
        let color = family_color(&group.name);
        ui.painter()
            .rect_filled(row.shrink(0.5), 4.0, color.gamma_multiply(0.09));
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                row.min + egui::vec2(0.0, 3.0),
                egui::vec2(3.0, row.height() - 6.0),
            ),
            2.0,
            color,
        );
    }
    let (namespace, name) = data::split(display);
    let left = row.left() + 8.0;
    match style {
        Style::Indent | Style::Muted => {
            let indent = if style == Style::Indent {
                (group.name.matches('.').count().saturating_sub(1).min(4) * 9) as f32
            } else {
                0.0
            };
            let prefix = if namespace.is_empty() {
                String::new()
            } else {
                format!("{namespace}.")
            };
            styled(
                ui,
                name_clip,
                row.center().y,
                left + indent,
                &[(&prefix, theme::DIM), (name, theme::TEXT)],
            );
        }
        Style::Tags => {
            if namespace.is_empty() {
                styled(ui, name_clip, row.center().y, left, &[(name, theme::TEXT)]);
            } else {
                let tag_width = (namespace.chars().count() as f32 * 6.1 + 14.0)
                    .min((name_clip.width() * 0.62).max(40.0));
                let tag = egui::Rect::from_min_size(
                    egui::pos2(left, row.top() + 4.0),
                    egui::vec2(tag_width, 19.0),
                );
                ui.painter().rect_filled(tag, 4.0, theme::RAISED);
                styled(
                    ui,
                    tag.intersect(name_clip),
                    row.center().y,
                    left + 5.0,
                    &[(namespace, theme::DIM)],
                );
                styled(
                    ui,
                    name_clip,
                    row.center().y,
                    left + tag_width + 8.0,
                    &[(name, theme::TEXT)],
                );
            }
        }
        Style::Columns => {
            let middle = row.left() + (row.width() - 104.0) * 0.55;
            let namespace_clip =
                egui::Rect::from_min_max(name_clip.min, egui::pos2(middle - 10.0, row.bottom()))
                    .intersect(name_clip);
            styled(
                ui,
                namespace_clip,
                row.center().y,
                left,
                &[(namespace, theme::DIM)],
            );
            styled(
                ui,
                name_clip,
                row.center().y,
                middle,
                &[(name, theme::TEXT)],
            );
        }
        _ => {
            styled(
                ui,
                name_clip,
                row.center().y,
                left,
                &[(display, theme::TEXT)],
            );
        }
    }
    ui.painter().text(
        row.right_center() - egui::vec2(9.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        data::count(group.articles),
        egui::FontId::monospace(12.0),
        theme::DIM,
    );
}

fn styled(
    ui: &egui::Ui,
    clip: egui::Rect,
    center: f32,
    left: f32,
    parts: &[(&str, egui::Color32)],
) {
    let mut job = egui::text::LayoutJob::default();
    for &(text, color) in parts {
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(13.5),
                color,
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = (clip.right() - left).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    ui.painter().with_clip_rect(clip).galley(
        egui::pos2(left, center - galley.size().y * 0.5),
        galley,
        theme::TEXT,
    );
}

pub(super) fn sections(ui: &mut egui::Ui, groups: &[Group], state: &State) {
    let mut namespaces = BTreeMap::<&str, Vec<Group>>::new();
    for group in groups {
        namespaces
            .entry(data::split(&group.name).0)
            .or_default()
            .push(group.clone());
    }
    for (namespace, mut rows) in namespaces {
        sort_groups(&mut rows, state);
        ui.add_space(7.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 25.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 4.0, theme::RAISED);
        styled(
            ui,
            rect.intersect(ui.clip_rect()),
            rect.center().y,
            rect.left() + 8.0,
            &[(
                &format!(
                    "{}  ·  {} groups",
                    if namespace.is_empty() {
                        "Top-level groups"
                    } else {
                        namespace
                    },
                    rows.len()
                ),
                theme::ACCENT,
            )],
        );
        for group in rows {
            paint_leaf(ui, &group);
        }
    }
}

fn family_color(name: &str) -> egui::Color32 {
    let family = name.split('.').take(2).collect::<Vec<_>>().join(".");
    let colors = [
        theme::ACCENT,
        egui::Color32::from_rgb(119, 166, 225),
        egui::Color32::from_rgb(183, 145, 217),
        theme::WARN,
    ];
    colors[family.bytes().fold(0usize, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(byte as usize)
    }) % colors.len()]
}
