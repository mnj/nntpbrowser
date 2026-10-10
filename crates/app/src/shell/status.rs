//! Stable statistic columns and the active task's progress/cancel controls.
use super::*;

const HEIGHT: f32 = 32.0;
const GAP: f32 = 8.0;
const CPU_WIDTH: f32 = 104.0;
const RAM_WIDTH: f32 = 132.0;
const SPEED_WIDTH: f32 = 156.0;
const OPEN_WIDTH: f32 = 168.0;
const BUSY_WIDTH: f32 = 88.0;
const ERRORS_WIDTH: f32 = 96.0;

impl Browser {
    pub(super) fn status(&mut self, ui: &mut egui::Ui) {
        let progress = super::queue::AggregateProgress::active(&self.jobs);
        let notice = if self.notice.is_empty() {
            "Ready".into()
        } else {
            self.scrub_text(&self.notice)
        };
        if super::errors::is_error_notice(&notice) {
            self.remember_notice_error(&notice);
        }
        let is_error = super::errors::is_error_notice(&notice)
            || self
                .error_history
                .iter()
                .any(|entry| notice.ends_with(&entry.message));
        let busy: usize = self.network_stats.values().map(|(busy, _, _)| *busy).sum();
        let open: usize = self.network_stats.values().map(|(_, open, _)| *open).sum();
        let bps: f64 = self.network_stats.values().map(|(_, _, bps)| *bps).sum();
        let limit: usize = self
            .servers
            .iter()
            .filter(|server| server.enabled)
            .map(|server| server.max_connections as usize)
            .sum();
        let speed = format!("{}/s", format_bytes(bps.max(0.0) as i64));
        let (cpu, ram) = self.process_stats.values();
        let fixed_width = CPU_WIDTH
            + RAM_WIDTH
            + SPEED_WIDTH
            + OPEN_WIDTH
            + BUSY_WIDTH
            + ERRORS_WIDTH
            + GAP * 5.0;
        let header = self.header_progress.as_ref().map(|headers| {
            let fraction = if headers.total > 0 {
                (headers.completed as f64 / headers.total as f64).clamp(0.0, 1.0) as f32
            } else { 0.0 };
            let title = if self.header_cancel_pending {
                "Cancelling header fetch…".into()
            } else if headers.first == 0 {
                "Connecting for article headers…".into()
            } else {
                format!("Headers {:.0}% · {} received · {} cached", fraction * 100.0,
                    headers.received, headers.stored)
            };
            let detail = format!(
                "Article headers {}–{}\n{} received · {} cached\n{} of {} article-number slots checked · {:.0}s\n{notice}",
                headers.first, headers.last, headers.received, headers.stored,
                headers.completed, headers.total, headers.started.elapsed().as_secs_f32());
            (title, detail, fraction, headers.total == 0 || headers.completed == 0)
        });
        ui.spacing_mut().item_spacing.x = GAP;
        ui.horizontal_centered(|ui| {
            let left_width = (ui.available_width() - fixed_width - GAP).max(0.0);
            let (left, _) = ui.allocate_exact_size(egui::vec2(left_width, HEIGHT), egui::Sense::hover());
            if let Some((title, detail, fraction, animate)) = header {
                let cancel_rect = egui::Rect::from_min_size(
                    egui::pos2(left.right() - HEIGHT, left.top()), egui::vec2(HEIGHT, HEIGHT));
                let bar_rect = egui::Rect::from_min_max(left.min,
                    egui::pos2((cancel_rect.left() - GAP).max(left.left()), left.bottom()));
                task_progress(ui, bar_rect, &title, &detail, fraction, animate, theme::ACCENT);
                if cancel_button(ui, cancel_rect, !self.header_cancel_pending).clicked() {
                    if let Some(group_id) = self.header_task_group.or(self.selected_group) {
                        self.header_cancel_pending = true;
                        self.handle.send(Command::CancelHeaderFetch(group_id));
                    }
                }
            } else if progress.active > 0 {
                task_progress(ui, left, &format!("Downloads · {}", progress.compact),
                    &format!("{}\n{}\n{notice}", progress.detail, progress.stages),
                    progress.fraction.unwrap_or(0.0), progress.animate, progress.color);
            } else {
                clip_text(ui, left, &notice, if is_error { theme::DANGER } else { theme::DIM },
                    FontId::proportional(13.0));
                let response = ui.interact(left, ui.id().with("status-notice"),
                    if is_error { egui::Sense::click() } else { egui::Sense::hover() });
                if is_error && response.clicked() { self.errors_open = true; }
                response.on_hover_text(if is_error {
                    format!("{notice}\nClick to open error history")
                } else { notice.clone() });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if metric(ui, ERRORS_WIDTH, "Errors", &self.error_count.to_string(),
                    if self.error_history.is_empty() { theme::DIM } else { theme::DANGER }, true)
                    .on_hover_text("Open this session’s error history").clicked() {
                    self.errors_open = true;
                }
                metric(ui, BUSY_WIDTH, "Busy", &busy.to_string(), theme::DIM, false)
                    .on_hover_text("Peak connections in use during the last sample (normally 250 ms). Short transfers remain visible.");
                let connections_response = metric(ui, OPEN_WIDTH, "Connections", &format!("{open} / {limit}"), theme::DIM, false);
                #[cfg(feature = "ui-preview")]
                if self.preview_connections_tooltip {
                    egui::Tooltip::for_widget(&connections_response).width(460.0)
                        .show(|ui| self.show_connections_tooltip(ui));
                }
                connections_response.on_hover_ui(|ui| self.show_connections_tooltip(ui));
                let speed_response = metric(ui, SPEED_WIDTH, "Speed", &speed, theme::DIM, false);
                #[cfg(feature = "ui-preview")]
                if self.preview_speed_tooltip {
                    egui::Tooltip::for_widget(&speed_response).width(380.0)
                        .show(|ui| { ui.label(self.traffic.tooltip(&speed)); });
                }
                speed_response.on_hover_text(self.traffic.tooltip(&speed));
                metric(ui, RAM_WIDTH, "RAM", &ram, theme::DIM, false)
                    .on_hover_text("Current resident memory of NNTPBrowser’s own process, including shared pages. Sampled about once per second.");
                metric(ui, CPU_WIDTH, "CPU", &cpu, theme::DIM, false)
                    .on_hover_text("NNTPBrowser’s own process, sampled about once per second. 100% means one fully busy core; multiple busy cores can exceed 100%. Available after the first sampling interval.");
            });
        });
    }
}

fn metric(
    ui: &mut egui::Ui,
    width: f32,
    name: &str,
    value: &str,
    color: Color32,
    clickable: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, HEIGHT),
        if clickable {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            if clickable {
                egui::WidgetType::Button
            } else {
                egui::WidgetType::Label
            },
            ui.is_enabled(),
            format!("{name}: {value}"),
        )
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if clickable && (response.hovered() || response.has_focus()) {
        painter.rect_filled(rect, 4.0, theme::RAISED);
    }
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.top() + 2.0),
            egui::pos2(rect.left(), rect.bottom() - 2.0),
        ],
        egui::Stroke::new(1.0, theme::HAIRLINE),
    );
    painter.text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.0),
        theme::DIM,
    );
    painter.text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        value,
        FontId::monospace(14.0),
        color,
    );
    if clickable {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn task_progress(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    title: &str,
    detail: &str,
    fraction: f32,
    animate: bool,
    color: Color32,
) {
    let bar_width = (rect.width() * 0.30).clamp(64.0, 280.0).min(rect.width());
    let bar = egui::Rect::from_center_size(
        egui::pos2(rect.right() - bar_width / 2.0, rect.center().y),
        egui::vec2(bar_width, 8.0),
    );
    let title_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2((bar.left() - GAP).max(rect.left()), rect.bottom()),
    );
    clip_text(
        ui,
        title_rect,
        title,
        theme::DIM,
        FontId::proportional(13.0),
    );
    ui.put(
        bar,
        egui::ProgressBar::new(fraction)
            .animate(animate)
            .fill(color)
            .desired_width(bar.width())
            .desired_height(8.0),
    )
    .on_hover_text(detail);
    ui.interact(
        title_rect,
        ui.id().with("status-task-detail"),
        egui::Sense::hover(),
    )
    .on_hover_text(detail);
}

fn cancel_button(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool) -> egui::Response {
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ui.put(
                rect,
                egui::Button::new("")
                    .min_size(egui::vec2(HEIGHT, HEIGHT))
                    .fill(theme::RAISED)
                    .stroke(egui::Stroke::new(1.0, theme::HAIRLINE))
                    .corner_radius(6),
            )
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            enabled,
            "Cancel article header fetch",
        )
    });
    let painter = ui.painter();
    let center = response.rect.center();
    let color = if enabled {
        theme::DIM
    } else {
        theme::DIM.gamma_multiply(0.4)
    };
    painter.circle_stroke(center, 9.0, egui::Stroke::new(1.5, color));
    for direction in [-1.0, 1.0] {
        painter.line_segment(
            [
                center + egui::vec2(-4.0, -4.0 * direction),
                center + egui::vec2(4.0, 4.0 * direction),
            ],
            egui::Stroke::new(1.5, color),
        );
    }
    response
        .on_hover_text("Cancel article header fetch\nAlready cached headers are kept.")
        .on_disabled_hover_text("Cancelling article header fetch…")
}
