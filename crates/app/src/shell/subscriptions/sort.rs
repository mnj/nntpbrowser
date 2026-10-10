//! Sort the filtered index, keeping server discovery rows and checkbox identity stable.
use super::*;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::shell) enum Column {
    Subscribed,
    #[default]
    Name,
    Articles,
    Posting,
}

#[derive(Default)]
pub(in crate::shell) struct PickerSort {
    column: Column,
    descending: bool,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Subscribed(bool),
    Name(String),
    Articles(u64),
    Posting(&'static str),
}

impl PickerSort {
    #[cfg(feature = "ui-preview")]
    pub(in crate::shell) fn preview_column(&mut self, column: Column) {
        self.column = column;
        self.descending = false;
    }

    pub(super) fn choose(&mut self, column: Column) {
        if self.column == column {
            self.descending = !self.descending;
        } else {
            self.column = column;
            self.descending = false;
        }
    }

    pub(super) fn heading(&self, column: Column, title: &str) -> String {
        if column == self.column {
            format!("{title} {}", if self.descending { "▾" } else { "▴" })
        } else {
            title.into()
        }
    }

    pub(super) fn apply(
        &self,
        indexes: &mut [usize],
        groups: &[ListedGroup],
        subscribed: &HashSet<String>,
    ) {
        indexes.sort_by_cached_key(|index| {
            let group = &groups[*index];
            let key = match self.column {
                Column::Subscribed => Key::Subscribed(subscribed.contains(&group.name)),
                Column::Name => Key::Name(group.name.to_ascii_lowercase()),
                Column::Articles => Key::Articles(group.posts()),
                Column::Posting => Key::Posting(posting_label(group.status)),
            };
            (key, group.name.to_ascii_lowercase(), group.name.clone())
        });
        if self.descending {
            indexes.reverse();
        }
    }
}

pub(super) fn posting_label(status: char) -> &'static str {
    match status {
        'y' => "open",
        'n' => "closed",
        'm' => "moderated",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
