//! The calculating indicator and progress bar, top right.

use egui::{RichText, Ui};

use crate::state::AppState;
use crate::ui::theme::FAIL;
use crate::ui::widgets;

/// Right-aligned status readout. Silent when idle and nothing has failed, so
/// the corner of the map stays clear during normal work.
pub fn status(ui: &mut Ui, state: &AppState) {
    let errors = &state.results.errors;
    for (job, error) in [
        ("trace", &errors.main),
        ("sweep", &errors.sweep),
        ("coverage", &errors.coverage),
    ] {
        if let Some(error) = error {
            ui.label(
                RichText::new(format!("\u{26A0} {job}: {error}"))
                    .small()
                    .color(FAIL),
            )
            .on_hover_text(error);
        }
    }

    let Some(label) = state.results.busy.label() else {
        return;
    };
    if let Some(fraction) = state.results.busy.fraction() {
        ui.add(widgets::progress_bar(fraction, label).desired_width(180.0));
    } else {
        widgets::caption(ui, label);
    }
    widgets::busy_spinner(ui);
}
