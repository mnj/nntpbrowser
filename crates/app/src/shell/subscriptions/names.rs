//! One metadata pass per picker rebuild; paint only visible names.
use super::{egui, theme, ListedGroup};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) struct NameLayout {
    prefix_end: usize,
    indent: f32,
}

impl NameLayout {
    fn new(name: &str) -> Self {
        Self {
            prefix_end: name.rfind('.').map_or(0, |index| index + 1),
            indent: (name.matches('.').count().saturating_sub(1).min(4) * 9) as f32,
        }
    }
}

fn cache_id() -> egui::Id {
    egui::Id::new("subscribe-name-layouts")
}

pub(super) fn rebuild(ctx: &egui::Context, groups: &[ListedGroup], indexes: &[usize]) {
    let layouts: Arc<[NameLayout]> = indexes
        .iter()
        .map(|&index| NameLayout::new(&groups[index].name))
        .collect();
    ctx.data_mut(|data| data.insert_temp(cache_id(), layouts));
}

pub(super) fn cached(ctx: &egui::Context) -> Arc<[NameLayout]> {
    ctx.data_mut(|data| data.get_temp(cache_id()).unwrap_or_default())
}

pub(super) fn paint(ui: &egui::Ui, rect: egui::Rect, name: &str, layout: Option<NameLayout>) {
    let layout = layout.unwrap_or_else(|| NameLayout::new(name));
    let clip = rect.shrink2(egui::vec2(2.0, 0.0)).intersect(ui.clip_rect());
    let left = rect.left() + 6.0 + layout.indent;
    let (prefix, leaf) = name.split_at(layout.prefix_end);
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in [(prefix, theme::DIM), (leaf, theme::TEXT)] {
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
        egui::pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        theme::TEXT,
    );
}
