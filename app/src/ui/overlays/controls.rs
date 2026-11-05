//! Right edge: the per-run station settings, floating over the map.
//!
//! These are the controls an operator touches between runs - what frequency,
//! what mode, how much power - as opposed to the modelling assumptions, which
//! live behind Settings.

use egui::{ComboBox, DragValue, RichText, Ui};

use crate::state::AppState;
use crate::ui::actions::{Action, InputChange, edited};
use crate::ui::theme::{self, MUTED};
use skipzone_predict::noise::{OperatingMode, dbm_from_watts};

/// Transmitter power presets, watts: QRP through the common legal limits.
const POWER_LEVELS_W: [f64; 10] = [
    1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 1000.0, 1500.0,
];

const CONTROL_WIDTH: f32 = 176.0;

pub fn overlay(ui: &mut Ui, state: &AppState, out: &mut Vec<Action>) {
    let inputs = &state.inputs.scenario;
    ui.vertical(|ui| {
        ui.set_width(CONTROL_WIDTH);

        theme::floating_frame().show(ui, |ui| {
            ui.set_width(CONTROL_WIDTH - 20.0);
            out.extend(frequency(ui, inputs.freq_mhz));
            ui.add_space(6.0);
            out.extend(mode(ui, inputs.op_mode));
            ui.add_space(6.0);
            out.extend(power(ui, inputs.tx_power_w));
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

fn frequency(ui: &mut Ui, freq_mhz: f64) -> Option<Action> {
    ui.label(RichText::new("Frequency").small().color(MUTED));
    let freq = edited(freq_mhz, |f| {
        ui.add_sized(
            [ui.available_width(), 20.0],
            DragValue::new(f)
                .speed(0.05)
                .range(0.5..=60.0)
                .suffix(" MHz"),
        );
    })?;
    Some(Action::SetInput(InputChange::Freq(freq)))
}

fn mode(ui: &mut Ui, op_mode: OperatingMode) -> Option<Action> {
    ui.label(RichText::new("Mode").small().color(MUTED));
    let mut picked = None;
    ComboBox::from_id_salt("mode_select")
        .width(ui.available_width())
        .selected_text(op_mode.label())
        .show_ui(ui, |ui| {
            for m in OperatingMode::ALL {
                // The preset seeds the bandwidth and threshold; both stay
                // editable in Settings, so the verdict is never decided by a
                // constant baked into the code.
                if ui.selectable_label(op_mode == m, m.label()).clicked() {
                    picked = Some(m);
                }
            }
        });
    picked.map(|m| Action::SetInput(InputChange::OpMode(m)))
}

fn power(ui: &mut Ui, tx_power_w: f64) -> Option<Action> {
    ui.label(RichText::new("Power").small().color(MUTED));
    let watts = edited(tx_power_w, |w| {
        ComboBox::from_id_salt("power_select")
            .width(ui.available_width())
            .selected_text(format_power(*w))
            .show_ui(ui, |ui| {
                for watts in POWER_LEVELS_W {
                    ui.selectable_value(w, watts, format_power(watts));
                }
            });
    })?;
    Some(Action::SetInput(InputChange::TxPower(watts)))
}

fn format_power(watts: f64) -> String {
    format!("{watts:.0} W  ({:.0} dBm)", dbm_from_watts(watts))
}
