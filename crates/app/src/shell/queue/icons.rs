//! Small, scale-independent download actions with named hit targets.
use super::*;

const SIZE: f32 = 32.0;
const GAP: f32 = 6.0;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Pause,
    Resume,
    Cancel,
    Retry,
    Folder,
    Copy,
    Remove,
    Delete,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Pause => "Pause download",
            Self::Resume => "Resume download",
            Self::Cancel => "Cancel download",
            Self::Retry => "Retry download",
            Self::Folder => "Open download folder",
            Self::Copy => "Copy folder path",
            Self::Remove => "Remove from list — keep files on disk",
            Self::Delete => "Delete download folder and files…",
        }
    }
}

pub(super) fn row_width(count: usize) -> f32 {
    SIZE * count as f32 + GAP * count.saturating_sub(1) as f32
}

pub(super) fn button(ui: &mut egui::Ui, action: Action, enabled: bool) -> egui::Response {
    ui.spacing_mut().item_spacing.x = GAP;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(egui::vec2(SIZE, SIZE))
            .fill(theme::RAISED)
            .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
            .corner_radius(8),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, action.label())
    });
    let danger = matches!(action, Action::Delete);
    let hovered = response.hovered() || response.has_focus();
    if enabled && hovered {
        ui.painter().rect_filled(
            response.rect.shrink(1.0),
            7,
            if danger {
                theme::DANGER.gamma_multiply(0.16)
            } else {
                theme::SELECT
            },
        );
    }
    let color = if !enabled {
        theme::DIM.gamma_multiply(0.35)
    } else if danger {
        theme::DANGER
    } else if hovered {
        theme::ACCENT
    } else {
        theme::DIM
    };
    draw(ui.painter(), response.rect.center(), action, color);
    response
        .on_hover_text(action.label())
        .on_disabled_hover_text(format!(
            "{}\nCancel the transfer before removing it.",
            action.label()
        ))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw(painter: &egui::Painter, center: egui::Pos2, action: Action, color: Color32) {
    let stroke = egui::Stroke::new(1.5, color);
    let point = |x, y| center + egui::vec2(x, y);
    let line = |from: (f32, f32), to: (f32, f32)| {
        painter.line_segment([point(from.0, from.1), point(to.0, to.1)], stroke);
    };
    let outline = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|(x, y)| point(*x, *y)).collect(),
            stroke,
        ));
    };
    let rect = |min: (f32, f32), max: (f32, f32)| {
        painter.rect_stroke(
            egui::Rect::from_min_max(point(min.0, min.1), point(max.0, max.1)),
            2,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match action {
        Action::Pause => {
            for x in [-5.0, 2.0] {
                painter.rect_filled(
                    egui::Rect::from_min_max(point(x, -7.0), point(x + 3.0, 7.0)),
                    1,
                    color,
                );
            }
        }
        Action::Resume => {
            painter.add(egui::Shape::convex_polygon(
                vec![point(-5.0, -7.0), point(7.0, 0.0), point(-5.0, 7.0)],
                color,
                egui::Stroke::NONE,
            ));
        }
        Action::Cancel => {
            painter.circle_stroke(center, 8.0, stroke);
            line((-3.0, -3.0), (3.0, 3.0));
            line((-3.0, 3.0), (3.0, -3.0));
        }
        Action::Retry => {
            let points = (0..=18)
                .map(|step| {
                    let angle = -1.1 + step as f32 * 5.5 / 18.0;
                    point(angle.cos() * 7.0, angle.sin() * 7.0)
                })
                .collect::<Vec<_>>();
            let tip = points[18];
            let direction = egui::vec2(-4.4_f32.sin(), 4.4_f32.cos());
            let side = egui::vec2(-direction.y, direction.x);
            painter.add(egui::Shape::line(points, stroke));
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip,
                    tip - direction * 4.0 + side * 2.5,
                    tip - direction * 4.0 - side * 2.5,
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        Action::Folder => {
            outline(&[
                (-9.0, 7.0),
                (-9.0, -6.0),
                (-3.0, -6.0),
                (0.0, -3.0),
                (8.0, -3.0),
                (8.0, 0.0),
            ]);
            outline(&[
                (-9.0, 7.0),
                (-6.0, 0.0),
                (10.0, 0.0),
                (7.0, 7.0),
                (-9.0, 7.0),
            ]);
        }
        Action::Copy => {
            outline(&[
                (-3.0, -4.0),
                (-3.0, -8.0),
                (-9.0, -8.0),
                (-9.0, 3.0),
                (-5.0, 3.0),
            ]);
            rect((-5.0, -4.0), (8.0, 9.0));
        }
        Action::Remove => {
            rect((-8.0, -9.0), (5.0, 8.0));
            line((-5.0, -5.0), (2.0, -5.0));
            line((-5.0, -1.0), (2.0, -1.0));
            painter.circle_filled(point(5.0, 5.0), 4.5, theme::RAISED);
            painter.circle_stroke(point(5.0, 5.0), 4.5, stroke);
            line((2.5, 5.0), (7.5, 5.0));
        }
        Action::Delete => {
            line((-8.0, -5.0), (8.0, -5.0));
            outline(&[(-3.0, -5.0), (-3.0, -8.0), (3.0, -8.0), (3.0, -5.0)]);
            outline(&[(-6.0, -2.0), (-5.0, 8.0), (5.0, 8.0), (6.0, -2.0)]);
            line((-2.0, -1.0), (-2.0, 5.0));
            line((2.0, -1.0), (2.0, 5.0));
        }
    }
}
