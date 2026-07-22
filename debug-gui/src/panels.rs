//! All the debug readouts. Deliberately dense and complete rather than tidy:
//! this is an instrument panel, not a product screen.

use egui::{CollapsingHeader, Color32, DragValue, Grid, RichText, Ui};

use crate::mapview::PALETTE;
use crate::scenario::{Assumptions, Inputs, PlaceMode, ProfileRow};
use crate::solve::{Solution, SolveOutcome, mode_label};

fn kv(ui: &mut Ui, k: &str, v: String) {
    ui.label(k);
    ui.label(RichText::new(v).monospace());
    ui.end_row();
}

/// Left panel: everything the operator can change. Returns true if Run was hit.
pub fn inputs_panel(ui: &mut Ui, inputs: &mut Inputs, place: &mut PlaceMode) -> bool {
    let mut run = false;
    ui.heading("Inputs");

    ui.horizontal(|ui| {
        ui.label("Map click sets:");
        ui.selectable_value(place, PlaceMode::Tx, "TX");
        ui.selectable_value(place, PlaceMode::Rx, "RX");
    });
    ui.separator();

    Grid::new("endpoints").num_columns(3).show(ui, |ui| {
        ui.label("TX lat/lon");
        ui.add(DragValue::new(&mut inputs.tx_lat).speed(0.05).range(-89.9..=89.9));
        ui.add(DragValue::new(&mut inputs.tx_lon).speed(0.05).range(-180.0..=180.0));
        ui.end_row();
        ui.label("RX lat/lon");
        ui.add(DragValue::new(&mut inputs.rx_lat).speed(0.05).range(-89.9..=89.9));
        ui.add(DragValue::new(&mut inputs.rx_lon).speed(0.05).range(-180.0..=180.0));
        ui.end_row();
    });

    ui.separator();
    Grid::new("signal").num_columns(2).show(ui, |ui| {
        ui.label("Frequency");
        ui.add(DragValue::new(&mut inputs.freq_mhz).speed(0.1).range(0.5..=60.0).suffix(" MHz"));
        ui.end_row();
        ui.label("Time (UTC)");
        ui.add(DragValue::new(&mut inputs.utc_hours).speed(0.25).range(0.0..=24.0).suffix(" h"));
        ui.end_row();
        ui.label("Month");
        ui.add(DragValue::new(&mut inputs.month).range(1..=12));
        ui.end_row();
        ui.label("Max hops");
        ui.add(DragValue::new(&mut inputs.max_hops).range(1..=8));
        ui.end_row();
    });

    ui.separator();
    ui.label(RichText::new("Ionosphere").strong());
    ui.checkbox(&mut inputs.solar_high, "Solar maximum (else minimum)");

    let mut fof2_on = inputs.fof2_override.is_some();
    if ui.checkbox(&mut fof2_on, "Override foF2").changed() {
        inputs.fof2_override = fof2_on.then_some(7.0);
    }
    if let Some(v) = inputs.fof2_override.as_mut() {
        ui.add(DragValue::new(v).speed(0.1).range(0.5..=30.0).suffix(" MHz"));
    }

    let mut hmf2_on = inputs.hmf2_override.is_some();
    if ui.checkbox(&mut hmf2_on, "Override hmF2").changed() {
        inputs.hmf2_override = hmf2_on.then_some(300.0);
    }
    if let Some(v) = inputs.hmf2_override.as_mut() {
        ui.add(DragValue::new(v).speed(1.0).range(90.0..=600.0).suffix(" km"));
    }

    Grid::new("iono2").num_columns(2).show(ui, |ui| {
        ui.label("Chapman scale H");
        ui.add(DragValue::new(&mut inputs.scale_height_km).speed(1.0).range(5.0..=200.0).suffix(" km"));
        ui.end_row();
        ui.label("Domain top");
        ui.add(DragValue::new(&mut inputs.domain_top_km).speed(10.0).range(400.0..=2000.0).suffix(" km"));
        ui.end_row();
    });

    ui.separator();
    ui.label(RichText::new("Collisions (drives absorption)").strong());
    Grid::new("coll").num_columns(2).show(ui, |ui| {
        ui.label("nu at ref alt");
        ui.add(DragValue::new(&mut inputs.nu0_per_s).speed(1e3).range(0.0..=1e8).suffix(" /s"));
        ui.end_row();
        ui.label("ref altitude");
        ui.add(DragValue::new(&mut inputs.nu_ref_alt_km).speed(1.0).range(50.0..=300.0).suffix(" km"));
        ui.end_row();
        ui.label("nu scale H");
        ui.add(DragValue::new(&mut inputs.nu_scale_height_km).speed(1.0).range(2.0..=100.0).suffix(" km"));
        ui.end_row();
    });
    ui.label(
        RichText::new(
            "The engine ships no default collision magnitude; these are your \
             numbers and are used as-is.",
        )
        .small()
        .color(Color32::GRAY),
    );

    ui.separator();
    ui.label(RichText::new("Magnetic field").strong());
    ui.checkbox(&mut inputs.use_field, "Use IGRF-14 (off = zero field, O/X degenerate)");
    if inputs.use_field {
        ui.add(DragValue::new(&mut inputs.igrf_epoch).speed(0.1).range(1900.0..=2030.0).prefix("epoch "));
    }

    ui.separator();
    if ui.button(RichText::new("RUN TRACE").strong()).clicked() {
        run = true;
    }
    ui.label(
        RichText::new("Solving is synchronous; a failed path runs a sweep and may take a second.")
            .small()
            .color(Color32::GRAY),
    );
    run
}

pub fn assumptions_panel(ui: &mut Ui, a: &Assumptions) {
    CollapsingHeader::new("Assumed values (all of them)")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("assume").num_columns(2).striped(true).show(ui, |ui| {
                kv(ui, "foF2", format!("{:.2} MHz", a.fof2_mhz));
                kv(ui, "  source", a.fof2_source.clone());
                kv(ui, "hmF2", format!("{:.0} km", a.hmf2_km));
                kv(ui, "  source", a.hmf2_source.clone());
                kv(ui, "Chapman scale H", format!("{:.0} km", a.scale_height_km));
                kv(ui, "NmF2", format!("{:.4e} m^-3", a.nm_per_m3));
                kv(ui, "Path midpoint", format!("{:.2}, {:.2}", a.midpoint_lat, a.midpoint_lon));
                kv(
                    ui,
                    "Midpoint LST",
                    format!("{:.2} h ({})", a.lst_hours, if a.is_day { "day" } else { "night" }),
                );
                kv(ui, "Season", a.season.label().to_string());
                kv(ui, "Collision nu0", format!("{:.3e} /s", a.nu0_per_s));
                kv(ui, "  at altitude", format!("{:.0} km", a.nu_ref_alt_km));
                kv(ui, "  scale height", format!("{:.0} km", a.nu_scale_height_km));
                kv(ui, "Field model", a.field_desc.clone());
                kv(ui, "Ground radius", format!("{:.1} km", a.r_ground_m / 1e3));
                kv(ui, "Domain top", format!("{:.1} km", a.r_top_m / 1e3));
            });
        });
}

pub fn reference_panel(ui: &mut Ui, out: &SolveOutcome) {
    CollapsingHeader::new("Great-circle reference")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("gc").num_columns(2).striped(true).show(ui, |ui| {
                kv(ui, "Distance", format!("{:.1} km", out.great_circle_km));
                kv(ui, "Bearing TX->RX", format!("{:.2} deg", out.bearing_deg));
                kv(ui, "Bearing RX->TX", format!("{:.2} deg", out.reverse_bearing_deg));
                kv(ui, "Solve wall time", format!("{:.0} ms", out.elapsed_ms));
            });
        });
}

/// Legend + per-solution visibility and selection.
pub fn legend_panel(
    ui: &mut Ui,
    out: &SolveOutcome,
    visible: &mut [bool],
    selected: &mut Option<usize>,
) {
    CollapsingHeader::new(format!("Modes found ({})", out.solutions.len()))
        .default_open(true)
        .show(ui, |ui| {
            if out.solutions.is_empty() {
                ui.colored_label(Color32::from_rgb(0xD0, 0x21, 0x1C), "No path connects.");
                return;
            }
            for (i, sol) in out.solutions.iter().enumerate() {
                ui.horizontal(|ui| {
                    if let Some(v) = visible.get_mut(i) {
                        ui.checkbox(v, "");
                    }
                    ui.colored_label(PALETTE[i % PALETTE.len()], "\u{25A0}");
                    let label = format!(
                        "{}-mode, {} hop(s), {:.0} km group, {:.2} dB",
                        mode_label(sol.mode),
                        sol.hops,
                        sol.total_group_km,
                        sol.total_absorption_db
                    );
                    if ui.selectable_label(*selected == Some(i), label).clicked() {
                        *selected = Some(i);
                    }
                });
            }
        });
}

pub fn solution_panel(ui: &mut Ui, sol: &Solution) {
    CollapsingHeader::new("Selected mode - totals")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("soltot").num_columns(2).striped(true).show(ui, |ui| {
                kv(ui, "Mode", format!("{} ({:?})", mode_label(sol.mode), sol.mode));
                kv(ui, "Hops", sol.hops.to_string());
                kv(ui, "Total ground range", format!("{:.2} km", sol.total_ground_km));
                kv(ui, "Total group path", format!("{:.2} km", sol.total_group_km));
                kv(ui, "Total phase path", format!("{:.2} km", sol.total_phase_km));
                kv(ui, "Total arc length", format!("{:.2} km", sol.total_arc_km));
                kv(ui, "Group delay", format!("{:.3} ms", sol.group_delay_ms));
                kv(ui, "Absorption", format!("{:.3} dB", sol.total_absorption_db));
                kv(ui, "Homing miss (1 hop)", format!("{:.2} m", sol.homing_miss_m));
                kv(ui, "Terminal miss vs RX", format!("{:.3} km", sol.terminal_miss_km));
                kv(ui, "Max |H| drift", format!("{:.3e}", sol.max_hamiltonian_drift));
                kv(ui, "Total solver steps", sol.total_steps.to_string());
            });
            if let Some(note) = &sol.note {
                ui.colored_label(Color32::from_rgb(0xC8, 0x7A, 0x00), format!("note: {note}"));
            }
            if sol.hops > 1 {
                ui.label(
                    RichText::new(
                        "Multi-hop: launch angle homed on one hop of 1/N the arc, then \
                         propagated N hops by specular ground reflection. Terminal miss \
                         shows the equal-hop assumption's error.",
                    )
                    .small()
                    .color(Color32::GRAY),
                );
            }
        });

    CollapsingHeader::new("Selected mode - per hop")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("hops").num_columns(15).striped(true).show(ui, |ui| {
                for h in [
                    "hop", "launch el", "launch az", "arr el", "arr az", "apex km", "apex X",
                    "range km", "group km", "phase km", "arc km", "abs dB", "steps", "|H| drift",
                    "outcome",
                ] {
                    ui.label(RichText::new(h).strong().small());
                }
                ui.end_row();
                for hop in &sol.hop_details {
                    ui.label(hop.index.to_string());
                    ui.label(format!("{:.3}", hop.launch_elev_deg));
                    ui.label(format!("{:.3}", hop.launch_az_deg));
                    ui.label(format!("{:.3}", hop.arrival_elev_deg));
                    ui.label(format!("{:.3}", hop.arrival_az_deg));
                    ui.label(format!("{:.2}", hop.apex_alt_km));
                    ui.label(format!("{:.4}", hop.apex_x));
                    ui.label(format!("{:.2}", hop.ground_range_km));
                    ui.label(format!("{:.2}", hop.group_km));
                    ui.label(format!("{:.2}", hop.phase_km));
                    ui.label(format!("{:.2}", hop.arc_km));
                    ui.label(format!("{:.4}", hop.absorption_db));
                    ui.label(hop.steps.to_string());
                    ui.label(format!("{:.2e}", hop.hamiltonian_drift));
                    ui.label(hop.outcome);
                    ui.end_row();
                }
            });
            ui.label(
                RichText::new(
                    "apex X is (fp/f)^2 at the turning point, from the engine's apex \
                     record; solver health per hop is in the drift/steps totals above.",
                )
                .small()
                .color(Color32::GRAY),
            );
        });
}

pub fn profile_panel(ui: &mut Ui, rows: &[ProfileRow]) {
    CollapsingHeader::new("Vertical profile actually used (at path midpoint)")
        .default_open(false)
        .show(ui, |ui| {
            Grid::new("prof").num_columns(7).striped(true).show(ui, |ui| {
                for h in ["alt km", "Ne m^-3", "fp MHz", "nu /s", "X", "Z", "|B| uT"] {
                    ui.label(RichText::new(h).strong().small());
                }
                ui.end_row();
                for r in rows {
                    ui.label(format!("{:.0}", r.alt_km));
                    ui.label(format!("{:.3e}", r.ne_per_m3));
                    ui.label(format!("{:.3}", r.plasma_mhz));
                    ui.label(format!("{:.3e}", r.nu_per_s));
                    ui.label(format!("{:.4}", r.x));
                    ui.label(format!("{:.3e}", r.z));
                    ui.label(r.b_microtesla.map_or("-".to_string(), |b| format!("{b:.2}")));
                    ui.end_row();
                }
            });
        });
}

pub fn near_miss_panel(ui: &mut Ui, out: &SolveOutcome) {
    if out.near_misses.is_empty() {
        return;
    }
    CollapsingHeader::new("Closest near-misses (elevation sweep)")
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Nothing homed, so each hop count was swept in elevation and the \
                     closest landing recorded.",
                )
                .small()
                .color(Color32::GRAY),
            );
            Grid::new("nm").num_columns(7).striped(true).show(ui, |ui| {
                for h in [
                    "mode", "hops", "elev deg", "landed km", "target km", "miss km", "note",
                ] {
                    ui.label(RichText::new(h).strong().small());
                }
                ui.end_row();
                for nm in &out.near_misses {
                    ui.label(mode_label(nm.mode));
                    ui.label(nm.hops.to_string());
                    ui.label(format!("{:.2}", nm.elevation_deg));
                    ui.label(format!("{:.1}", nm.landed_range_km));
                    ui.label(format!("{:.1}", nm.target_range_km));
                    ui.label(format!("{:.1}", nm.miss_km));
                    ui.label(&nm.note);
                    ui.end_row();
                }
            });
        });
}

pub fn errors_panel(ui: &mut Ui, out: &SolveOutcome) {
    CollapsingHeader::new(format!("Engine errors ({})", out.errors.len()))
        .default_open(true)
        .show(ui, |ui| {
            if out.errors.is_empty() {
                ui.label("none");
                return;
            }
            for e in &out.errors {
                ui.colored_label(Color32::from_rgb(0xC8, 0x3A, 0x1C), RichText::new(e).monospace());
            }
        });
}
