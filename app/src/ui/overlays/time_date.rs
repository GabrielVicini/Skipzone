//! Bottom-left: the UTC time slider and the date field, floating over the map.
//!
//! Both are editable two ways round - drag the slider or type the time, click
//! the calendar or type the date - and both drive the terminator shading live,
//! without needing a re-solve.

use egui::{Popup, RectAlign, RichText, Slider, Ui};

use crate::state::AppState;
use crate::ui::scratch::ViewScratch;
use crate::ui::theme::{self, MUTED};
use crate::ui::widgets::{calendar, fields};
use skipzone_iono::clock;

pub fn overlay(ui: &mut Ui, state: &mut AppState, scratch: &mut ViewScratch) {
    ui.vertical(|ui| {
        time_control(ui, state, scratch);
        ui.add_space(6.0);
        date_control(ui, state, scratch);
    });
}

/// `HH:MM` on the left, the slider filling the rest: one control, two ways to
/// set the same value.
fn time_control(ui: &mut Ui, state: &mut AppState, scratch: &mut ViewScratch) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let valid = clock::parse_hours(&scratch.time_text).is_some();
            let response = fields::text(ui, &mut scratch.time_text, 48.0, valid);
            if response.changed()
                && let Some(hours) = clock::parse_hours(&scratch.time_text)
            {
                state.inputs.scenario.utc_hours = hours;
            }
            ui.label(RichText::new("UTC").small().color(MUTED));

            let mut hours = state.inputs.scenario.utc_hours;
            let slider = ui.add(
                Slider::new(&mut hours, 0.0..=24.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
            );
            if slider.changed() {
                state.inputs.scenario.utc_hours = hours;
            }
            // The text follows the value whenever it is not being typed into.
            if !response.has_focus() {
                scratch.time_text = clock::format_hours(state.inputs.scenario.utc_hours);
            }
        });
    });
}

/// `YYYY-MM-DD` with a calendar button beside it.
fn date_control(ui: &mut Ui, state: &mut AppState, scratch: &mut ViewScratch) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let date = state.inputs.date();
            let valid = clock::parse_date(&scratch.date_text).is_some();
            let response = fields::text(ui, &mut scratch.date_text, 92.0, valid);
            if response.changed()
                && let Some(typed) = clock::parse_date(&scratch.date_text)
            {
                set_date(state, typed);
                scratch.calendar.show_month_of(typed);
            }
            if !response.has_focus() {
                scratch.date_text = clock::format_date(state.inputs.date());
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

            if let Some(picked) = picked {
                set_date(state, picked);
                scratch.date_text = clock::format_date(picked);
                scratch.calendar.open = false;
            }
        });
    });
}

fn set_date(state: &mut AppState, date: clock::CivilDate) {
    let inputs = &mut state.inputs.scenario;
    inputs.year = date.year;
    inputs.month = date.month;
    inputs.day_of_month = date.day;
}
