//! Point-to-Point > Calculate: the full trace readout, docked to the right.
//!
//! A side panel rather than a window: the trace is long, is read alongside the
//! map, and is referred back to while changing inputs - so it pushes the map
//! and the floating controls aside instead of covering them. It is resizable by
//! its inner edge and closes with the ✕; running Calculate again simply
//! replaces its contents with the new solve.
//!
//! Every readout in it comes from [`crate::ui::panels`]: the verdict, the
//! great-circle reference, the modes found, the selected mode's totals and
//! per-hop breakdown, near-misses, engine errors, the assumptions the model was
//! built from, and the vertical profile actually sampled.

use egui::{Align, Button, Layout, Panel, RichText, ScrollArea, Ui};

use crate::state::{Session, SolveResults, UiState};
use crate::ui::actions::Action;
use crate::ui::panels;
use crate::ui::theme::{self, BAD, MUTED};

const DEFAULT_WIDTH: f32 = 430.0;
const MIN_WIDTH: f32 = 320.0;
const MAX_WIDTH: f32 = 900.0;

/// Draw the panel when it is open, and record the width it took so the floating
/// overlays can inset by it. Returns any action its buttons produced.
pub fn draw(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    if !ui_state.trace_open {
        ui_state.right_inset = 0.0;
        return None;
    }

    let busy = session.is_busy();
    let error = session.error.clone();
    let solve = &mut session.solve;
    let mut close = false;

    let panel = Panel::right("trace_panel")
        .resizable(true)
        .default_size(DEFAULT_WIDTH)
        .size_range(MIN_WIDTH..=MAX_WIDTH)
        .frame(theme::header_frame())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Point-to-point trace").heading());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close = ui.button(RichText::new("\u{2715}").strong()).clicked();
                });
            });
            ui.separator();
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| body(ui, solve, busy, error.as_deref()))
                .inner
        });

    ui_state.right_inset = panel.response.rect.width();
    if close {
        ui_state.trace_open = false;
        ui_state.right_inset = 0.0;
    }
    panel.inner
}

fn body(ui: &mut Ui, solve: &mut SolveResults, busy: bool, error: Option<&str>) -> Option<Action> {
    let mut action = None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, Button::new(RichText::new("Calculate").strong()))
            .on_hover_text("Trace the path again at the current inputs")
            .clicked()
        {
            action = Some(Action::Calculate);
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
        if let Some(out) = &solve.outcome {
            ui.label(
                RichText::new(format!("solved in {:.0} ms", out.elapsed_ms))
                    .small()
                    .color(MUTED),
            );
        }
    });
    ui.add_space(6.0);

    if let Some(error) = error {
        ui.colored_label(
            BAD,
            RichText::new(format!("model build failed: {error}"))
                .monospace()
                .small(),
        );
    }

    let Some(out) = &solve.outcome else {
        if error.is_none() {
            ui.label(RichText::new("Press Calculate to trace the path.").color(MUTED));
        }
        return action;
    };

    panels::verdict_chip(ui, out);
    ui.add_space(6.0);
    panels::reference_panel(ui, out);
    panels::legend_panel(ui, out, &mut solve.visible, &mut solve.selected);
    if let Some(sol) = solve.selected_solution() {
        panels::solution_panel(ui, sol);
    }
    panels::near_miss_panel(ui, out);
    panels::errors_panel(ui, out);

    if let Some(assumptions) = &solve.assumptions {
        panels::assumptions_panel(ui, assumptions);
    }
    if !solve.profile.is_empty() {
        panels::profile_panel(ui, &solve.profile);
    }
    ui.add_space(6.0);

    action
}
