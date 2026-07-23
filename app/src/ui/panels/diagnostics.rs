//! Diagnostic panels shown when a solve struggles: the closest near-misses
//! from the elevation sweep, and any typed engine errors encountered.

use egui::{CollapsingHeader, RichText, Ui};

use crate::solve::{SolveOutcome, mode_label};
use crate::ui::theme::{BAD, MUTED, WARN};
use crate::ui::widgets::{data_grid, head_cells, hint, num, wide_table};

pub fn near_miss_panel(ui: &mut Ui, out: &SolveOutcome) {
    if out.near_misses.is_empty() && out.sweep_notes.is_empty() {
        return;
    }
    CollapsingHeader::new("Closest near-misses (elevation sweep)")
        .default_open(true)
        .show(ui, |ui| {
            hint(
                ui,
                "Nothing homed, so each hop count was swept in elevation and the \
                 closest landing recorded.",
            );
            for note in &out.sweep_notes {
                ui.colored_label(WARN, RichText::new(note).small());
            }
            if out.near_misses.is_empty() {
                return;
            }
            ui.add_space(4.0);
            wide_table(ui, "nm_scroll", |ui| {
                data_grid(ui, "nm", 7, |ui| {
                    head_cells(
                        ui,
                        &[
                            "mode",
                            "hops",
                            "elev deg",
                            "landed km",
                            "target km",
                            "miss km",
                            "note",
                        ],
                    );
                    for nm in &out.near_misses {
                        num(ui, mode_label(nm.mode).to_string());
                        num(ui, nm.hops.to_string());
                        num(ui, format!("{:.2}", nm.elevation_deg));
                        num(ui, format!("{:.1}", nm.landed_range_km));
                        num(ui, format!("{:.1}", nm.target_range_km));
                        num(ui, format!("{:.1}", nm.miss_km));
                        num(ui, nm.note.clone());
                        ui.end_row();
                    }
                });
            });
        });
}

pub fn errors_panel(ui: &mut Ui, out: &SolveOutcome) {
    CollapsingHeader::new(format!("Engine errors ({})", out.errors.len()))
        .default_open(!out.errors.is_empty())
        .show(ui, |ui| {
            if out.errors.is_empty() {
                ui.label(RichText::new("none").small().color(MUTED));
                return;
            }
            for e in &out.errors {
                ui.colored_label(BAD, RichText::new(e).monospace().small());
            }
        });
}
