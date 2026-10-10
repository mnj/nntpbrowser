//! Dark theme. One accent, loose spacing, 12 px cards.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};

pub const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x1a);
pub const SURFACE: Color32 = Color32::from_rgb(0x1c, 0x1f, 0x27);
pub const RAISED: Color32 = Color32::from_rgb(0x26, 0x2a, 0x34);
pub const HAIRLINE: Color32 = Color32::from_rgb(0x34, 0x38, 0x46);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xef);
pub const DIM: Color32 = Color32::from_rgb(0x9a, 0xa3, 0xb2);
pub const ACCENT: Color32 = Color32::from_rgb(0x3d, 0xbe, 0xa5);
pub const WARN: Color32 = Color32::from_rgb(0xe6, 0xb3, 0x5a);
pub const DANGER: Color32 = Color32::from_rgb(0xe1, 0x5d, 0x5d);
pub const INK: Color32 = Color32::from_rgb(0x12, 0x14, 0x1a);
pub const SELECT: Color32 = Color32::from_rgb(0x17, 0x33, 0x2e);
pub const HOVER: Color32 = Color32::from_rgb(0x22, 0x26, 0x30);

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "SourceSans".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SourceSans3-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "SourceSansSemibold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SourceSans3-Semibold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "JetBrainsMono".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/JetBrainsMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "SourceSans".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "JetBrainsMono".into());
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["SourceSansSemibold".into()],
    );
    ctx.set_fonts(fonts);

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(DIM);
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = DANGER;
    visuals.faint_bg_color = RAISED;
    visuals.extreme_bg_color = BG;
    visuals.code_bg_color = RAISED;
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, HAIRLINE);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(12);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(10);
        widget.bg_stroke = Stroke::new(1.0, HAIRLINE);
        widget.fg_stroke.color = TEXT;
    }
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x32, 0x37, 0x44);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x32, 0x37, 0x44);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke.color = INK;
    visuals.widgets.open.bg_fill = RAISED;
    ctx.set_visuals(visuals);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.menu_margin = egui::Margin::same(8);
    });
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, HAIRLINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(12))
}

pub fn side() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .inner_margin(egui::Margin::symmetric(12, 10))
}

pub fn bar() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .inner_margin(egui::Margin::symmetric(12, 8))
}

pub fn content() -> egui::Frame {
    egui::Frame::new()
        .fill(BG)
        .inner_margin(egui::Margin::symmetric(14, 10))
}

pub fn heading(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .family(FontFamily::Name("semibold".into()))
        .size(22.0)
        .color(TEXT)
}

pub fn dim(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(DIM).size(14.0)
}
