//! Point-to-Point > Calculate: the full trace readout.
//!
//! Every diagnostic the solver produces, in one dialog: the verdict, the
//! great-circle reference, the modes found, the selected mode's totals and
//! per-hop breakdown, near-misses, engine errors, the assumptions the model was
//! built from, and the vertical profile actually sampled.

use egui::{Button, Context, RichText, Ui, vec2};

use crate::state::{Session, SolveResults, UiState};
use crate::ui::actions::Action;
use crate::ui::panels;
use crate::ui::theme::{BAD, MUTED};

pub fn show(ctx: &Context, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let busy = session.is_busy();
    let error = session.error.clone();
    let solve = &mut session.solve;
    super::chrome::dialog(
        ctx,
        "trace_dialog",
        "Point-to-point trace",
        &mut ui_state.modals.trace,
        vec2(620.0, 620.0),
        |ui| body(ui, solve, busy, error.as_deref()),
    )
    .flatten()
}

fn body(ui: &mut Ui, solve: &mut SolveResults, busy: bool, error: Option<&str>) -> Option<Action> {
    let mut action = None;

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, Button::new(RichText::new("Calculate").strong()))
            .on_hover_text("Trace the path at the tuned frequency")
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
