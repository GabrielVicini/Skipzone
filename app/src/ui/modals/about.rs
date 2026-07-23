//! Help > About. Placeholder copy, pending real release text.

use egui::{Context, RichText, Ui, vec2};

use crate::state::UiState;
use crate::ui::theme::MUTED;
use crate::ui::widgets::hint;

pub fn show(ctx: &Context, ui_state: &mut UiState) {
    super::chrome::dialog(
        ctx,
        "about_dialog",
        "About Skipzone",
        &mut ui_state.modals.about,
        vec2(420.0, 380.0),
        body,
    );
}

fn body(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.label(RichText::new("Skipzone").heading());
    ui.label(
        RichText::new(concat!("version ", env!("CARGO_PKG_VERSION")))
            .small()
            .color(MUTED),
    );
    ui.add_space(8.0);
    ui.label(
        "Example placeholder text. Skipzone is a three-dimensional ionospheric ray tracer \
         for HF point-to-point prediction: it traces real rays through a magnetised, \
         collisional plasma and reports what arrives at the receiver.",
    );
    ui.add_space(6.0);
    ui.label(
        "Replace this text with the real about copy - credits, licence, and the data \
         sources the shipped build uses.",
    );
    ui.add_space(10.0);
    hint(
        ui,
        "Map tiles \u{00A9} OpenStreetMap contributors. Magnetic field: IGRF-14. Radio \
         noise: ITU-R P.372-9 (atmospheric term approximated - see the trace readout).",
    );
}
