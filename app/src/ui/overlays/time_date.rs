//! Bottom-left: the UTC time slider and the date field, floating over the map.
//!
//! Both are editable two ways round - drag the slider or type the time, click
//! the calendar or type the date - and both drive the terminator shading live,
//! without needing a re-solve.

use egui::{Popup, RectAlign, RichText, Slider, Ui};

use crate::clock;
use crate::state::{Session, UiState};
use crate::ui::theme::{self, MUTED};
use crate::ui::widgets::{calendar, fields};

pub fn overlay(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) {
    ui.vertical(|ui| {
        time_control(ui, session, ui_state);
        ui.add_space(6.0);
        date_control(ui, session, ui_state);
    });
}

/// `HH:MM` on the left, the slider filling the rest: one control, two ways to
/// set the same value.
fn time_control(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let valid = clock::parse_hours(&ui_state.time_text).is_some();
            let response = fields::text(ui, &mut ui_state.time_text, 48.0, valid);
            if response.changed()
                && let Some(hours) = clock::parse_hours(&ui_state.time_text)
            {
                session.inputs.utc_hours = hours;
            }
            ui.label(RichText::new("UTC").small().color(MUTED));

            let mut hours = session.inputs.utc_hours;
            let slider = ui.add(
                Slider::new(&mut hours, 0.0..=24.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
            );
            if slider.changed() {
                session.inputs.utc_hours = hours;
            }
            // The text follows the value whenever it is not being typed into.
            if !response.has_focus() {
                ui_state.time_text = clock::format_hours(session.inputs.utc_hours);
            }
        });
    });
}

/// `YYYY-MM-DD` with a calendar button beside it.
fn date_control(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            let date = session.date();
            let valid = clock::parse_date(&ui_state.date_text).is_some();
            let response = fields::text(ui, &mut ui_state.date_text, 92.0, valid);
            if response.changed()
                && let Some(typed) = clock::parse_date(&ui_state.date_text)
            {
                session.set_date(typed);
                ui_state.calendar.show_month_of(typed);
            }
            if !response.has_focus() {
                ui_state.date_text = clock::format_date(session.date());
            }

            let button = ui
                .button(RichText::new("\u{1F4C5}").size(15.0))
                .on_hover_text("Pick a date");
            if button.clicked() {
                ui_state.calendar.open = !ui_state.calendar.open;
                if ui_state.calendar.open {
                    ui_state.calendar.show_month_of(date);
                }
            }

            let mut open = ui_state.calendar.open;
            let picked = Popup::from_response(&button)
                .open_bool(&mut open)
                .align(RectAlign::TOP_START)
                .gap(4.0)
                .frame(theme::popup_frame())
                .show(|ui| calendar::picker(ui, &mut ui_state.calendar, date))
                .and_then(|inner| inner.inner);
            ui_state.calendar.open = open;

            if let Some(picked) = picked {
                session.set_date(picked);
                ui_state.date_text = clock::format_date(picked);
                ui_state.calendar.open = false;
            }
        });
    });
}
