use egui::{Context, RichText, Ui, vec2};

use crate::state::{AppState, Modal};
use crate::ui::actions::Action;
use crate::ui::theme::MUTED;
use crate::ui::widgets::hint;

pub fn show(ctx: &Context, state: &AppState, out: &mut Vec<Action>) {
    super::chrome::dialog(
        ctx,
        Modal::About,
        state.ui.modals.about,
        "About Skipzone",
        vec2(420.0, 380.0),
        out,
        |ui, _| body(ui),
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
        "Skipzone is a three-dimensional ionospheric ray tracer \
         for HF point-to-point prediction: it traces real rays through a magnetised, \
         collisional plasma and reports what arrives at the receiver.",
    );
    ui.add_space(6.0);
    ui.label(
        "Copyright \u{00A9} 2026 Gabriel Vicini. Licensed under the MIT License. \
         See LICENSE for the full text.",
    );
    ui.add_space(10.0);
    hint(
        ui,
        "Map tiles \u{00A9} OpenStreetMap contributors. Magnetic field: IGRF-14. Radio \
         noise: ITU-R P.372-9 (atmospheric term approximated - see the trace readout).",
    );
}
