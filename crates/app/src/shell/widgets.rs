use super::*;

pub(super) fn server_from_form(form: &Form) -> Option<NewServer> {
    let port = form.port.trim().parse::<u16>().ok()?;
    if form.name.trim().is_empty() || form.host.trim().is_empty() {
        return None;
    }
    Some(NewServer {
        name: form.name.trim().to_string(),
        host: form.host.trim().to_string(),
        port,
        tls: form.tls,
        starttls: form.starttls,
        username: blank(&form.username),
        password: blank(&form.password),
        priority: form.priority.trim().parse().unwrap_or(1),
        max_connections: form.connections.trim().parse::<i64>().unwrap_or(8).max(1),
        pipeline_depth: form.pipeline.trim().parse::<i64>().unwrap_or(8).max(1),
        fill: form.fill,
    })
}

pub(super) fn form_from(server: &ServerRow) -> Form {
    Form {
        name: server.name.clone(),
        host: server.host.clone(),
        port: server.port.to_string(),
        tls: server.tls,
        starttls: server.starttls,
        username: server.username.clone().unwrap_or_default(),
        password: server.password.clone().unwrap_or_default(),
        priority: server.priority.to_string(),
        connections: server.max_connections.to_string(),
        pipeline: server.pipeline_depth.to_string(),
        fill: server.fill,
        enabled: server.enabled,
    }
}

pub(super) fn blank(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn dense(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
    ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
}

pub(super) fn nav_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let response = ui.add(
        egui::Button::new(egui::RichText::new(label).size(16.0).color(if selected {
            theme::TEXT
        } else {
            theme::DIM
        }))
        .fill(if selected {
            theme::RAISED
        } else {
            Color32::TRANSPARENT
        })
        .stroke(egui::Stroke::NONE)
        .min_size(egui::vec2(82.0, 38.0)),
    );
    if selected {
        ui.painter().hline(
            response.rect.x_range().shrink(12.0),
            response.rect.bottom() - 2.0,
            egui::Stroke::new(2.0, theme::ACCENT),
        );
    }
    response.clicked()
}

pub(super) fn segment(ui: &mut egui::Ui, label: &str, on: bool) -> bool {
    let text = if on {
        egui::RichText::new(label).color(theme::INK).size(14.0)
    } else {
        egui::RichText::new(label).color(theme::TEXT).size(14.0)
    };
    let fill = if on { theme::ACCENT } else { theme::RAISED };
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(egui::vec2(0.0, 28.0)),
    )
    .clicked()
}

pub(super) fn primary(ui: &mut egui::Ui, label: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(theme::INK).size(14.0))
            .fill(theme::ACCENT)
            .corner_radius(egui::CornerRadius::same(8)),
    )
    .clicked()
}

pub(super) fn glyph(post: &PostRow) -> &'static str {
    if post.has_image {
        "IMG"
    } else if post.has_video {
        "VIDEO"
    } else if post.has_par2 {
        "PAR"
    } else {
        "FILE"
    }
}

pub(super) fn status_of(jobs: &[JobRow], post: &PostRow) -> (&'static str, Color32) {
    if let Some(job) = jobs.iter().rev().find(|job| job.post_id == Some(post.id)) {
        return (state_label(&job.state), state_color(&job.state));
    }
    if post.incomplete {
        ("partial", theme::WARN)
    } else {
        ("new", theme::ACCENT)
    }
}

pub(super) fn state_label(state: &str) -> &'static str {
    match state {
        "queued" => "queued",
        "fetching" => "fetching",
        "decoding" => "decoding",
        "repair" => "repair",
        "extract" => "extract",
        "paused" => "paused",
        "failed" => "failed",
        "done" => "done",
        "cancelled" => "cancelled",
        _ => "queued",
    }
}

pub(super) fn state_color(state: &str) -> Color32 {
    match state {
        "failed" | "cancelled" => theme::DANGER,
        "paused" | "repair" | "extract" | "recovery_headers" | "recovery_fetching" => theme::WARN,
        "done" => theme::ACCENT,
        _ => theme::DIM,
    }
}

pub(super) fn is_active(state: &str) -> bool {
    matches!(
        state,
        "queued"
            | "fetching"
            | "decoding"
            | "repair"
            | "extract"
            | "recovery_headers"
            | "recovery_fetching"
    )
}

pub(super) fn trim(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub(super) fn fitted(size: egui::Vec2, max: egui::Vec2) -> egui::Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return max;
    }
    let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
    size * scale
}

pub(super) fn columns_for(width: f32) -> Columns {
    let date = if width >= 800.0 { 132.0 } else { 105.0 };
    let size = if width >= 640.0 { 72.0 } else { 64.0 };
    let files = if width >= 640.0 { 52.0 } else { 36.0 };
    let parts = if width >= 640.0 { 56.0 } else { 40.0 };
    let poster = if width >= 800.0 {
        120.0
    } else if width >= 640.0 {
        80.0
    } else {
        0.0
    };
    let status = if width >= 800.0 { 78.0 } else { 60.0 };
    let title = (width - date - size - files - parts - poster - status - 40.0).max(40.0);
    Columns {
        date,
        title,
        size,
        files,
        parts,
        poster,
        status,
    }
}

pub(super) fn paint_cells(
    ui: &egui::Ui,
    mut x: f32,
    rect: egui::Rect,
    cells: &[(f32, &str, Color32, bool)],
) {
    for (width, text, color, mono) in cells {
        if *width <= 0.0 {
            continue;
        }
        let cell =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(*width, rect.height()));
        let font = if *mono {
            FontId::monospace(12.5)
        } else {
            FontId::proportional(13.5)
        };
        let shown = fit_chars(text, *width, if *mono { 7.4 } else { 7.0 });
        clip_text(ui, cell.shrink2(egui::vec2(2.0, 0.0)), &shown, *color, font);
        x += *width;
    }
}

pub(super) fn fit_chars(text: &str, width: f32, px: f32) -> String {
    let max = ((width - 8.0) / px).floor().max(1.0) as usize;
    trim(text, max)
}

pub(super) fn clip_text(ui: &egui::Ui, rect: egui::Rect, text: &str, color: Color32, font: FontId) {
    ui.painter().with_clip_rect(rect).text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

pub(super) fn paint_check(ui: &egui::Ui, rect: egui::Rect, on: bool, hover: bool) {
    let stroke = if on || hover {
        theme::ACCENT
    } else {
        theme::HAIRLINE
    };
    ui.painter().rect_stroke(
        rect,
        egui::CornerRadius::same(4),
        egui::Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    if on {
        ui.painter()
            .rect_filled(rect.shrink(4.0), egui::CornerRadius::same(2), theme::ACCENT);
    }
}

pub(super) fn open_system(path: &Path) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
