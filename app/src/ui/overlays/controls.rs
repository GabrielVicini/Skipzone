//! Right edge: the per-run station settings, floating over the map.
//!
//! These are the controls an operator touches between runs - what frequency,
//! what mode, how much power - as opposed to the modelling assumptions, which
//! live behind Settings.

use egui::{ComboBox, DragValue, RichText, Ui};

use crate::state::AppState;
use crate::ui::actions::Action;
use crate::ui::theme::{self, MUTED};
use skipzone_predict::noise::{OperatingMode, dbm_from_watts};

/// Transmitter power presets, watts: QRP through the common legal limits.
const POWER_LEVELS_W: [f64; 10] = [
    1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 1000.0, 1500.0,
];

const CONTROL_WIDTH: f32 = 176.0;

pub fn overlay(ui: &mut Ui, state: &mut AppState, out: &mut Vec<Action>) {
    ui.vertical(|ui| {
        ui.set_width(CONTROL_WIDTH);

        theme::floating_frame().show(ui, |ui| {
            ui.set_width(CONTROL_WIDTH - 20.0);
            frequency(ui, state);
            ui.add_space(6.0);
            mode(ui, state);
            ui.add_space(6.0);
            power(ui, state);
        });

        ui.add_space(6.0);
        theme::floating_frame().show(ui, |ui| {
            ui.set_width(CONTROL_WIDTH - 20.0);
            if ui
                .add_sized([ui.available_width(), 24.0], egui::Button::new("Antennas"))
                .clicked()
            {
                out.push(Action::ShowAntennas);
            }
            ui.add_space(4.0);
            if ui
                .add_sized([ui.available_width(), 24.0], egui::Button::new("Settings"))
                .clicked()
            {
                out.push(Action::ShowSettings);
            }
        });
    });
}

fn frequency(ui: &mut Ui, state: &mut AppState) {
    ui.label(RichText::new("Frequency").small().color(MUTED));
    ui.add_sized(
        [ui.available_width(), 20.0],
        DragValue::new(&mut state.inputs.scenario.freq_mhz)
            .speed(0.05)
            .range(0.5..=60.0)
            .suffix(" MHz"),
    );
}

fn mode(ui: &mut Ui, state: &mut AppState) {
    ui.label(RichText::new("Mode").small().color(MUTED));
    ComboBox::from_id_salt("mode_select")
        .width(ui.available_width())
        .selected_text(state.inputs.scenario.op_mode.label())
        .show_ui(ui, |ui| {
            for m in OperatingMode::ALL {
                // The preset seeds the bandwidth and threshold; both stay
                // editable in Settings, so the verdict is never decided by a
                // constant baked into the code.
                if ui
                    .selectable_value(&mut state.inputs.scenario.op_mode, m, m.label())
                    .clicked()
                {
                    let (bandwidth_hz, threshold_db) = m.defaults();
                    state.inputs.scenario.bandwidth_hz = bandwidth_hz;
                    state.inputs.scenario.snr_threshold_db = threshold_db;
                }
            }
        });
}

fn power(ui: &mut Ui, state: &mut AppState) {
    ui.label(RichText::new("Power").small().color(MUTED));
    ComboBox::from_id_salt("power_select")
        .width(ui.available_width())
        .selected_text(format_power(state.inputs.scenario.tx_power_w))
        .show_ui(ui, |ui| {
            for watts in POWER_LEVELS_W {
                ui.selectable_value(
                    &mut state.inputs.scenario.tx_power_w,
                    watts,
                    format_power(watts),
                );
            }
        });
}

fn format_power(watts: f64) -> String {
    format!("{watts:.0} W  ({:.0} dBm)", dbm_from_watts(watts))
}
