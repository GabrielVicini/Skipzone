//! Bottom-right: framing and display options for the map itself.

use egui::{RichText, Ui};

use crate::state::UiState;
use crate::ui::actions::Action;
use crate::ui::theme::{self, MUTED};

pub fn overlay(ui: &mut Ui, ui_state: &mut UiState, zoom: f64, out: &mut Vec<Action>) {
    theme::floating_frame().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut ui_state.show_terminator,
                RichText::new("Night").small(),
            );
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
