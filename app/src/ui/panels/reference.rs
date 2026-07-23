//! The great-circle reference panel: distance, bearings, and solve wall time.

use egui::{CollapsingHeader, Ui};

use crate::solve::SolveOutcome;
use crate::ui::widgets::{data_grid, kv, sub_head};

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

                // Shown whether or not anything was found: the noise floor is a
                // property of the receiver, so "no path" runs still have one,
                // and it is what any future path would have to beat.
                sub_head(ui, "RECEIVER NOISE FLOOR");
                kv(
                    ui,
                    "External noise Fa",
                    format!("{:.1} dB above kT0b", out.noise.total_fa_db),
                );
                kv(
                    ui,
                    &format!("Floor in {:.0} Hz", out.noise.bandwidth_hz),
                    format!("{:.1} dBm", out.noise.power_dbm),
                );
                kv(
                    ui,
                    "SNR threshold",
                    format!("{:.1} dB", out.snr_threshold_db),
                );
            });
        });
}
