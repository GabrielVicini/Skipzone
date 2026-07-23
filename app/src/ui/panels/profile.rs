//! The vertical-profile panel: the electron density, collision, and field
//! quantities actually sampled along the vertical at the path midpoint.

use egui::{CollapsingHeader, Ui};

use crate::scenario::ProfileRow;
use crate::ui::widgets::{data_grid, head_cells, num, wide_table};

pub fn profile_panel(ui: &mut Ui, rows: &[ProfileRow]) {
    CollapsingHeader::new("Vertical profile actually used (at path midpoint)")
        .default_open(false)
        .show(ui, |ui| {
            wide_table(ui, "prof_scroll", |ui| {
                data_grid(ui, "prof", 7, |ui| {
                    head_cells(
                        ui,
                        &["alt km", "Ne m^-3", "fp MHz", "nu /s", "X", "Z", "|B| uT"],
                    );
                    for r in rows {
                        num(ui, format!("{:.0}", r.alt_km));
                        num(ui, format!("{:.3e}", r.ne_per_m3));
                        num(ui, format!("{:.3}", r.plasma_mhz));
                        num(ui, format!("{:.3e}", r.nu_per_s));
                        num(ui, format!("{:.4}", r.x));
                        num(ui, format!("{:.3e}", r.z));
                        num(
                            ui,
                            r.b_microtesla
                                .map_or("-".to_string(), |b| format!("{b:.2}")),
                        );
                        ui.end_row();
                    }
                });
            });
        });
}
