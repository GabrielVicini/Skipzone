//! The Antennas dialog.
//!
//! There is no antenna model yet: the link budget is between isotropic ends, so
//! this dialog says so rather than offering controls that would not reach the
//! solver. It states exactly what would change once patterns exist.

use egui::{Context, RichText, Ui, vec2};

use crate::state::{Session, UiState};
use crate::ui::theme::WARN;
use crate::ui::widgets::{card, data_grid, hint, kv, section};

pub fn show(ctx: &Context, session: &Session, ui_state: &mut UiState) {
    let tx_power_w = session.inputs.tx_power_w;
    super::chrome::dialog(
        ctx,
        "antennas_dialog",
        "Antennas",
        &mut ui_state.modals.antennas,
        vec2(460.0, 380.0),
        |ui| body(ui, tx_power_w),
    );
}

fn body(ui: &mut Ui, tx_power_w: f64) {
    ui.add_space(4.0);
    ui.colored_label(
        WARN,
        RichText::new("No antenna model is implemented.").strong(),
    );
    hint(
        ui,
        "The link budget currently reports BASIC transmission loss between isotropic \
         ends: free-space spreading over the ray path, plus ionospheric absorption, plus \
         Fresnel ground-reflection loss. No gain, no pattern, no take-off angle weighting.",
    );

    section(ui, "What the solver assumes today");
    card(ui, |ui| {
        data_grid(ui, "antenna_assumptions", 2, |ui| {
            kv(ui, "TX antenna", "isotropic, 0 dBi".to_string());
            kv(ui, "RX antenna", "isotropic, 0 dBi".to_string());
            kv(ui, "Feedline loss", "not modelled".to_string());
            kv(ui, "TX power", format!("{tx_power_w:.0} W at the antenna"));
        });
    });

    section(ui, "What a pattern would change");
    hint(
        ui,
        "Each traced hop already records its launch and arrival elevation, so a gain \
         pattern would weight each solution by G(elevation) at both ends and shift which \
         mode wins - not just scale every result equally. Until that exists, treat mode \
         ranking as geometry and absorption only.",
    );
}
