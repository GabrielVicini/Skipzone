//! Bottom-left: the UTC time slider and the date field, floating over the map.
//!
//! Both are editable two ways round - drag the slider or type the time, click
//! the calendar or type the date - and both drive the terminator shading live,
//! without needing a re-solve.

use egui::{Popup, RectAlign, RichText, Slider, Ui};

use crate::state::AppState;
use crate::ui::actions::{Action, InputChange};
use crate::ui::scratch::ViewScratch;
use crate::ui::theme::{self, MUTED};
use crate::ui::widgets::calendar::{self, Pick};
use crate::ui::widgets::fields;
use skipzone_iono::clock;

pub fn overlay(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    ui.vertical(|ui| {
        time_control(ui, state, scratch, out);
        ui.add_space(6.0);
        date_control(ui, state, scratch, out);
    });
}

/// `HH:MM` on the left, the slider filling the rest: one control, two ways to
/// set the same value.
fn time_control(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    let current = state.inputs.scenario.utc_hours;
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let valid = clock::parse_hours(&scratch.time_text).is_some();
            let response = fields::text(ui, &mut scratch.time_text, 48.0, valid);
            if response.changed()
                && let Some(hours) = clock::parse_hours(&scratch.time_text)
            {
                out.push(Action::SetInput(InputChange::UtcHours(hours)));
            }
            ui.label(RichText::new("UTC").small().color(MUTED));

            let mut hours = current;
            let slider = ui.add(
                Slider::new(&mut hours, 0.0..=24.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
            );
            if slider.changed() {
                out.push(Action::SetInput(InputChange::UtcHours(hours)));
            }
            // The text follows the value whenever it is not being typed into.
            if !response.has_focus() {
                scratch.time_text = clock::format_hours(current);
            }
        });
    });
}

/// `YYYY-MM-DD` with a calendar button beside it.
fn date_control(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let date = state.inputs.date();
            let valid = clock::parse_date(&scratch.date_text).is_some();
            let response = fields::text(ui, &mut scratch.date_text, 92.0, valid);
            if response.changed()
                && let Some(typed) = clock::parse_date(&scratch.date_text)
            {
                out.push(Action::SetInput(InputChange::Date(typed)));
                scratch.calendar.show_month_of(typed);
            }
            if !response.has_focus() {
                scratch.date_text = clock::format_date(date);
            }

            let button = ui
                .button(RichText::new("\u{1F4C5}").size(15.0))
                .on_hover_text("Pick a date");
            if button.clicked() {
                scratch.calendar.open = !scratch.calendar.open;
                if scratch.calendar.open {
                    scratch.calendar.show_month_of(date);
                }
            }

            let mut open = scratch.calendar.open;
            let picked = Popup::from_response(&button)
                .open_bool(&mut open)
                .align(RectAlign::TOP_START)
                .gap(4.0)
                .frame(theme::popup_frame())
                .show(|ui| calendar::picker(ui, &mut scratch.calendar, date))
                .and_then(|inner| inner.inner);
            scratch.calendar.open = open;

            match picked {
                Some(Pick::Day(day)) => {
                    out.push(Action::SetInput(InputChange::Date(day)));
                    scratch.date_text = clock::format_date(day);
                    scratch.calendar.open = false;
                }
                Some(Pick::Today) => {
                    out.push(Action::SetDateToday);
                    scratch.calendar.open = false;
                }
                None => {}
            }
        });
    });
}
