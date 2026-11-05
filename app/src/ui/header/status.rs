//! The calculating indicator and progress bar, top right.

use egui::{ProgressBar, RichText, Spinner, Ui};

use crate::state::Session;
use crate::ui::theme::{ACCENT, FAIL, MUTED};

/// Right-aligned status readout. Silent when idle and nothing has failed, so
/// the corner of the map stays clear during normal work.
pub fn status(ui: &mut Ui, session: &Session) {
    let errors = &session.errors;
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

    let Some(label) = session.busy.label() else {
        return;
    };
    if let Some(fraction) = session.busy.fraction() {
        ui.add(
            ProgressBar::new(fraction)
                .desired_width(180.0)
                .fill(ACCENT.gamma_multiply(0.7))
                .text(RichText::new(label).small()),
        );
    } else {
        ui.label(RichText::new(label).small().color(MUTED));
    }
    ui.add(Spinner::new().size(14.0));
}
