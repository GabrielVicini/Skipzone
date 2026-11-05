//! Point-to-Point > Calculate: the full trace readout, docked to the right.
//!
//! A side panel rather than a window: the trace is long, is read alongside the
//! map, and is referred back to while changing inputs - so it pushes the map
//! and the floating controls aside instead of covering them. It is resizable by
//! its inner edge and closes with the close button; running Calculate again
//! replaces its contents.
//!
//! Every readout in it comes from [`crate::ui::panels`]: the verdict, the
//! great-circle reference, the modes found, the selected mode's totals and
//! per-hop breakdown, near-misses, engine errors, the assumptions the model was
//! built from, and the sampled vertical profile.

use egui::{Align, Button, Layout, Panel, RichText, ScrollArea, Ui};

use crate::state::AppState;
use crate::ui::actions::Action;
use crate::ui::panels;
use crate::ui::scratch::ViewScratch;
use crate::ui::theme::{self, BAD, MUTED};

const DEFAULT_WIDTH: f32 = 430.0;
const MIN_WIDTH: f32 = 320.0;
const MAX_WIDTH: f32 = 900.0;

/// Draw the panel when it is open, and record the width it took so the floating
/// overlays can inset by it. Pushes any action its buttons produced.
pub fn draw(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    if !state.ui.trace_open {
        scratch.right_inset = 0.0;
        return;
    }

    let mut close = false;

    // The window's right edge, read before the panel takes its bite out of the
    // layout, so the width the panel occupies can be measured against it.
    let right_edge = ui.max_rect().right();

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
                .show(ui, |ui| body(ui, state, out));
        });

    // Measured from the panel's left edge to the window's right edge rather
    // than from the panel's reported width, which egui grows to cover
    // overflowing content and can run off the window. The overlays need the
    // strip the panel covers.
    scratch.right_inset = (right_edge - panel.response.rect.left()).clamp(0.0, right_edge);
    if close {
        out.push(Action::CloseTrace);
        scratch.right_inset = 0.0;
    }
}

fn body(ui: &mut Ui, state: &AppState, actions: &mut Vec<Action>) {
    let busy = state.results.is_busy();
    let error = state.results.errors.main.as_deref();

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, Button::new(RichText::new("Calculate").strong()))
            .on_hover_text("Trace the path again at the current inputs")
            .clicked()
        {
            actions.push(Action::Calculate);
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
        if let Some(out) = &state.results.main.outcome {
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

    let solve = &state.results.main;
    let Some(out) = &solve.outcome else {
        if error.is_none() {
            ui.label(RichText::new("Press Calculate to trace the path.").color(MUTED));
        }
        return;
    };

    panels::verdict_chip(ui, out);
    ui.add_space(6.0);
    panels::reference_panel(ui, out);
    panels::legend_panel(ui, out, &state.ui.selection, actions);
    if let Some(sol) = state.selected_solution() {
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
}
