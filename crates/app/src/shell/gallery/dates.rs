//! UTC calendar ranges, applied before paging the cached article catalog.
use super::super::*;
use chrono::NaiveDate;

const DAY: i64 = 86_400;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Preset {
    #[default]
    All,
    Today,
    Week,
    Month,
    Custom,
}

impl Preset {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All dates",
            Self::Today => "Today",
            Self::Week => "Last 7 days",
            Self::Month => "Last 30 days",
            Self::Custom => "Custom range",
        }
    }
}

#[derive(Default)]
pub(in crate::shell) struct DateFilter {
    preset: Preset,
    from_text: String,
    until_text: String,
    from: Option<i64>,
    until: Option<i64>,
    error: Option<String>,
}

impl DateFilter {
    pub(in crate::shell) fn bounds(&self) -> (Option<i64>, Option<i64>) {
        (self.from, self.until)
    }

    pub(in crate::shell) fn matches(&self, posted_at: i64) -> bool {
        (self.from.is_none() && self.until.is_none())
            || (posted_at > 0
                && self.from.is_none_or(|from| posted_at >= from)
                && self.until.is_none_or(|until| posted_at <= until))
    }

    pub(in crate::shell) fn show(&mut self, ui: &mut egui::Ui, now: i64) -> bool {
        let before = self.bounds();
        let old_preset = self.preset;
        ui.horizontal_wrapped(|ui| {
            dense(ui);
            ui.label(theme::dim("Posted"));
            egui::ComboBox::from_id_salt("gallery-date-preset")
                .width(138.0)
                .selected_text(self.preset.label())
                .show_ui(ui, |ui| {
                    for preset in [
                        Preset::All,
                        Preset::Today,
                        Preset::Week,
                        Preset::Month,
                        Preset::Custom,
                    ] {
                        ui.selectable_value(&mut self.preset, preset, preset.label());
                    }
                });
            if self.preset != old_preset {
                self.apply_preset(now);
            }
            if ui
                .add_enabled(
                    self.preset != Preset::All || self.error.is_some(),
                    egui::Button::new("Clear dates"),
                )
                .clicked()
            {
                self.clear();
            }
            ui.label(theme::dim("UTC"));
        });
        if self.preset == Preset::Custom {
            let mut apply = false;
            ui.horizontal_wrapped(|ui| {
                dense(ui);
                ui.label(theme::dim("From"));
                let from = ui.add(
                    egui::TextEdit::singleline(&mut self.from_text)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(112.0),
                );
                ui.label(theme::dim("To"));
                let until = ui.add(
                    egui::TextEdit::singleline(&mut self.until_text)
                        .hint_text("YYYY-MM-DD")
                        .desired_width(112.0),
                );
                apply = ui.button("Apply dates").clicked()
                    || ((from.lost_focus() || until.lost_focus())
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)));
                ui.label(theme::dim("End date included"));
            });
            if apply {
                self.apply_custom();
            }
        }
        if let Some(error) = &self.error {
            ui.label(egui::RichText::new(error).size(13.0).color(theme::DANGER));
        }
        before != self.bounds()
    }

    fn apply_preset(&mut self, now: i64) {
        self.error = None;
        let today = now.div_euclid(DAY) * DAY;
        match self.preset {
            Preset::All => self.clear(),
            Preset::Today | Preset::Week | Preset::Month => {
                let days = match self.preset {
                    Preset::Week => 7,
                    Preset::Month => 30,
                    _ => 1,
                };
                self.from = Some(today - (days - 1) * DAY);
                self.until = Some(today + DAY - 1);
                self.from_text = day_label(self.from.unwrap());
                self.until_text = day_label(self.until.unwrap());
            }
            Preset::Custom => {}
        }
    }

    fn clear(&mut self) {
        self.preset = Preset::All;
        self.from = None;
        self.until = None;
        self.from_text.clear();
        self.until_text.clear();
        self.error = None;
    }

    fn apply_custom(&mut self) {
        let result = (|| {
            let from = parse_day(&self.from_text)
                .map_err(|_| "Enter a valid start date as YYYY-MM-DD.")?;
            let until = parse_day(&self.until_text)
                .map_err(|_| "Enter a valid end date as YYYY-MM-DD.")?
                .map(|day| day + DAY - 1);
            if matches!((from, until), (Some(from), Some(until)) if from > until) {
                return Err("Start date must be before or equal to end date.");
            }
            Ok((from, until))
        })();
        match result {
            Ok((from, until)) => {
                self.from = from;
                self.until = until;
                self.error = None;
            }
            Err(error) => self.error = Some(error.into()),
        }
    }
}

fn parse_day(text: &str) -> Result<Option<i64>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.len() != 10
        || text.as_bytes()[4] != b'-'
        || text.as_bytes()[7] != b'-'
        || !text
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return Err(());
    }
    let day = NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| ())?;
    Ok(Some(
        day.and_hms_opt(0, 0, 0).ok_or(())?.and_utc().timestamp(),
    ))
}

fn day_label(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
