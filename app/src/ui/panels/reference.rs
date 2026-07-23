//! The great-circle reference panel: distance, bearings, and solve wall time.

use egui::{CollapsingHeader, Ui};

use crate::solve::SolveOutcome;
use crate::ui::widgets::{data_grid, kv};

pub fn reference_panel(ui: &mut Ui, out: &SolveOutcome) {
    CollapsingHeader::new("Great-circle reference")
        .default_open(true)
        .show(ui, |ui| {
            data_grid(ui, "gc", 2, |ui| {
                kv(ui, "Distance", format!("{:.1} km", out.great_circle_km));
                kv(ui, "Bearing TX->RX", format!("{:.2} deg", out.bearing_deg));
                kv(
                    ui,
                    "Bearing RX->TX",
                    format!("{:.2} deg", out.reverse_bearing_deg),
                );
                kv(ui, "Solve wall time", format!("{:.0} ms", out.elapsed_ms));
            });
        });
}
