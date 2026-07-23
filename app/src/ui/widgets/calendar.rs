//! Month-grid date picker, shown from the calendar button beside the date
//! field. Built on [`crate::clock`]'s civil-date arithmetic, so the month
//! lengths, leap years and weekday alignment all come from one place.

use egui::{Align, Button, Color32, Grid, Layout, RichText, Ui};

use crate::clock::{self, CivilDate, MONTH_NAMES, WEEKDAY_INITIALS};
use crate::state::CalendarState;
use crate::ui::theme::{ACCENT, MUTED};

/// Draw the picker. Returns the date if one was clicked.
pub fn picker(ui: &mut Ui, cal: &mut CalendarState, selected: CivilDate) -> Option<CivilDate> {
    let mut picked = None;

    ui.horizontal(|ui| {
        if ui
            .add(Button::new("\u{2039}").min_size([22.0, 20.0].into()))
            .clicked()
        {
            cal.step_month(-1);
        }
        let title = format!(
            "{} {}",
            MONTH_NAMES[(cal.month.clamp(1, 12) - 1) as usize],
            cal.year
        );
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_sized(
                [140.0, 20.0],
                egui::Label::new(RichText::new(title).strong()),
            );
        });
        if ui
            .add(Button::new("\u{203A}").min_size([22.0, 20.0].into()))
            .clicked()
        {
            cal.step_month(1);
        }
    });
    ui.add_space(2.0);

    let first = CivilDate::new(cal.year, cal.month, 1);
    let lead = clock::weekday_index(first);
    let days = clock::days_in_month(cal.year, cal.month);

    Grid::new("calendar_days")
        .num_columns(7)
        .spacing([2.0, 2.0])
        .show(ui, |ui| {
            for initial in WEEKDAY_INITIALS {
                ui.label(RichText::new(initial).small().color(MUTED));
            }
            ui.end_row();

            for _ in 0..lead {
                ui.label("");
            }
            let mut column = lead;
            for day in 1..=days {
                let date = CivilDate::new(cal.year, cal.month, day);
                let is_selected = date == selected;
                let mut button = Button::new(RichText::new(day.to_string()).small())
                    .min_size([24.0, 20.0].into());
                button = if is_selected {
                    button.fill(ACCENT.gamma_multiply(0.35))
                } else {
                    button.fill(Color32::TRANSPARENT)
                };
                if ui.add(button).clicked() {
                    picked = Some(date);
                }
                column += 1;
                if column.is_multiple_of(7) {
                    ui.end_row();
                }
            }
        });

    ui.add_space(2.0);
    if ui.button("Today (UTC)").clicked() {
        let (today, _) = clock::utc_now();
        cal.show_month_of(today);
        picked = Some(today);
    }

    picked
}
