//! Bottom-right: framing and display options for the map itself.

use egui::{RichText, Ui};

use crate::state::AppState;
use crate::ui::actions::{Action, InputChange, edited};
use crate::ui::theme::{self, MUTED};

pub fn overlay(ui: &mut Ui, state: &AppState, out: &mut Vec<Action>) {
    let zoom = state.ui.map.camera.zoom_level();
    let show_terminator = state.inputs.display.show_terminator;
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(on) = edited(show_terminator, |on| {
                ui.checkbox(on, RichText::new("Night").small());
            }) {
                out.push(Action::SetInput(InputChange::ShowTerminator(on)));
            }
            ui.separator();
            if ui
                .button("Fit path")
                .on_hover_text("Frame TX and RX")
                .clicked()
            {
                out.push(Action::FitPath);
            }
            if ui.button("\u{2212}").clicked() {
                out.push(Action::ZoomOut);
            }
            if ui.button("+").clicked() {
                out.push(Action::ZoomIn);
            }
            ui.label(
                RichText::new(format!("z{zoom:.1}"))
                    .small()
                    .monospace()
                    .color(MUTED),
            );
        });
    });
}
