//! Expandable namespace hierarchy with stable path identities.
use super::{
    data::{self, Group},
    egui, rows, theme, State,
};
use std::collections::BTreeMap;

#[derive(Default)]
struct Node {
    group: Option<Group>,
    children: BTreeMap<String, Node>,
}

pub(super) fn paint(ui: &mut egui::Ui, groups: &[Group], state: &State) {
    let mut root = Node::default();
    for group in groups {
        let mut node = &mut root;
        for component in group.name.split('.') {
            node = node.children.entry(component.to_owned()).or_default();
        }
        node.group = Some(group.clone());
    }
    children(ui, &root, "", state);
}

fn children(ui: &mut egui::Ui, node: &Node, path: &str, state: &State) {
    let mut entries = node.children.iter().collect::<Vec<_>>();
    entries.sort_by(|(a_name, a), (b_name, b)| {
        match (a.children.is_empty(), b.children.is_empty()) {
            (false, false) => super::compare_names(a_name, b_name),
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            (true, true) => {
                let order = match state.sort {
                    super::Sort::Name => super::compare_names(a_name, b_name),
                    super::Sort::Articles => a
                        .group
                        .as_ref()
                        .map_or(0, |g| g.articles)
                        .cmp(&b.group.as_ref().map_or(0, |g| g.articles))
                        .then(super::compare_names(a_name, b_name)),
                };
                if state.descending {
                    order.reverse()
                } else {
                    order
                }
            }
        }
    });
    for (name, node) in entries {
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        if node.children.is_empty() {
            if let Some(group) = &node.group {
                rows::paint_leaf(ui, group);
            }
        } else {
            let count = node
                .group
                .as_ref()
                .map(|group| format!("  ·  {} articles", data::count(group.articles)))
                .unwrap_or_default();
            let mut label = egui::text::LayoutJob::default();
            label.append(
                &format!("{name}{count}"),
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(13.0),
                    color: theme::TEXT,
                    ..Default::default()
                },
            );
            label.wrap.max_width = (ui.available_width() - 40.0).max(1.0);
            label.wrap.max_rows = 1;
            label.wrap.break_anywhere = true;
            egui::CollapsingHeader::new(label)
                .id_salt((&path, "preview-tree"))
                .default_open(true)
                .show(ui, |ui| {
                    ui.spacing_mut().indent = 13.0;
                    children(ui, node, &path, state);
                })
                .header_response
                .on_hover_text(&path);
        }
    }
}
