//! Read-only snapshots for comparing group-name layouts.
use super::{egui, theme, Browser};
mod data;
mod dialog;
mod rows;
mod tree;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Flat,
    Indent,
    Muted,
    Tags,
    Columns,
    Tree,
    Sections,
    Bands,
}
const STYLES: [Style; 8] = [
    Style::Flat,
    Style::Indent,
    Style::Muted,
    Style::Tags,
    Style::Columns,
    Style::Tree,
    Style::Sections,
    Style::Bands,
];

impl Style {
    fn title(self) -> &'static str {
        match self {
            Self::Flat => "Flat full names",
            Self::Indent => "Indent + muted prefix",
            Self::Muted => "Muted prefix",
            Self::Tags => "Namespace tags",
            Self::Columns => "Namespace / name columns",
            Self::Tree => "Folder tree",
            Self::Sections => "Namespace sections",
            Self::Bands => "Family bands",
        }
    }
    fn caption(self) -> &'static str {
        match self {
            Self::Flat => "Every name stays intact. Rows follow the selected global sort.",
            Self::Indent => "Tiny spacing and a quiet namespace bring names forward. Global sort stays intact.",
            Self::Muted => "A quiet namespace brings the final name forward. Global sort stays intact.",
            Self::Tags => "Namespace tags separate context from the final name. Globally sorted rows.",
            Self::Columns => "Name sorts the full group name; Articles sorts the article count.",
            Self::Tree => "Folders stay alphabetical; leaf groups follow the selected sort within each folder.",
            Self::Sections => "Namespaces stay alphabetical; groups follow the selected sort within each section.",
            Self::Bands => "Color marks each namespace family. Rows retain the selected global sort.",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Name,
    Articles,
}
#[derive(Clone)]
struct State {
    style: Style,
    sort: Sort,
    descending: bool,
    groups: Vec<data::Group>,
    source: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            style: Style::Flat,
            sort: Sort::Name,
            descending: false,
            groups: data::fake(),
            source: "Design preview · sample groups".into(),
        }
    }
}
fn state_id() -> egui::Id {
    egui::Id::new("group-layout-comparison-state")
}

impl Browser {
    pub(super) fn open_group_layouts(&mut self, ctx: &egui::Context) {
        let previous = ctx.data_mut(|data| data.get_temp::<State>(state_id()).unwrap_or_default());
        let mut state = State {
            style: previous.style,
            sort: previous.sort,
            descending: previous.descending,
            ..State::default()
        };
        if let Some((server_id, sample)) = self.picker_server.and_then(|id| {
            self.samples
                .get(&id)
                .filter(|sample| sample.done)
                .map(|sample| (id, sample))
        }) {
            let needle = self.picker_filter.trim().to_ascii_lowercase();
            let mut total = 0usize;
            state.groups.clear();
            for group in &sample.groups {
                if !needle.is_empty() && !group.name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                total += 1;
                if state.groups.len() < 200 {
                    state.groups.push(data::Group {
                        name: group.name.clone(),
                        articles: group.posts(),
                    });
                }
            }
            let server = self
                .servers
                .iter()
                .find(|server| server.id == server_id)
                .map(|server| server.name.as_str())
                .unwrap_or("Loaded server");
            state.source = if total == 0 {
                format!("Design preview · {server} · No matching groups")
            } else if total > 200 {
                format!(
                    "Design preview · {server} · Previewing first 200 of {total} matching groups"
                )
            } else {
                format!("Design preview · {server} · {total} matching groups")
            };
            if !needle.is_empty() {
                state
                    .source
                    .push_str(&format!(" · filter ‘{}’", self.picker_filter.trim()));
            }
        }
        ctx.data_mut(|data| data.insert_temp(state_id(), state));
        self.group_layouts_open = true;
    }

    pub(super) fn group_layouts_window(&mut self, ctx: &egui::Context) {
        dialog::show(self, ctx);
    }
}

fn sorted(state: &State) -> Vec<data::Group> {
    let mut groups = state.groups.clone();
    sort_groups(&mut groups, state);
    groups
}
fn sort_groups(groups: &mut [data::Group], state: &State) {
    groups.sort_by(|a, b| {
        let order = match state.sort {
            Sort::Name => compare_names(&a.name, &b.name),
            Sort::Articles => a
                .articles
                .cmp(&b.articles)
                .then(compare_names(&a.name, &b.name)),
        };
        if state.descending {
            order.reverse()
        } else {
            order
        }
    });
}

fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_ascii_lowercase()
        .cmp(&b.to_ascii_lowercase())
        .then(a.cmp(b))
}

#[cfg(feature = "ui-preview")]
pub(super) fn preview_sort(ctx: &egui::Context, articles: bool, descending: bool) {
    ctx.data_mut(|data| {
        let mut state = data.get_temp::<State>(state_id()).unwrap_or_default();
        state.sort = if articles { Sort::Articles } else { Sort::Name };
        state.descending = descending;
        data.insert_temp(state_id(), state);
    });
}

#[cfg(feature = "ui-preview")]
pub(super) fn preview_style(ctx: &egui::Context, key: &str) {
    let style = match key.to_ascii_lowercase().as_str() {
        "indent" => Style::Indent,
        "muted" => Style::Muted,
        "tags" => Style::Tags,
        "columns" => Style::Columns,
        "tree" => Style::Tree,
        "sections" => Style::Sections,
        "bands" => Style::Bands,
        _ => Style::Flat,
    };
    ctx.data_mut(|data| {
        let mut state = data.get_temp::<State>(state_id()).unwrap_or_default();
        state.style = style;
        data.insert_temp(state_id(), state);
    });
}
