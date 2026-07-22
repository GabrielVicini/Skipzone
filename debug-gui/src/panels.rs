//! All the debug readouts. Deliberately dense and complete rather than tidy:
//! this is an instrument panel, not a product screen.

use egui::{
    CollapsingHeader, Color32, CornerRadius, DragValue, Frame, Grid, Layout, Margin, RichText,
    ScrollArea, Stroke, Ui,
};

use crate::mapview::PALETTE;
use crate::scenario::{Assumptions, Inputs, PlaceMode, ProfileRow};
use crate::solve::{Solution, SolveOutcome, mode_label};

pub const OK: Color32 = Color32::from_rgb(0x1B, 0x7F, 0x3B);
pub const WARN: Color32 = Color32::from_rgb(0xC8, 0x7A, 0x00);
pub const BAD: Color32 = Color32::from_rgb(0xC8, 0x3A, 0x1C);
pub const FAIL: Color32 = Color32::from_rgb(0xD0, 0x21, 0x1C);
pub const MUTED: Color32 = Color32::from_gray(0x88);

fn hint(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).small().color(MUTED));
}

fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}

fn sub_head(ui: &mut Ui, text: &str) {
    ui.label("");
    ui.label(RichText::new(text).small().strong().color(MUTED));
    ui.end_row();
}

fn kv(ui: &mut Ui, k: &str, v: String) {
    ui.label(RichText::new(k).small());
    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(v).monospace().small());
    });
    ui.end_row();
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .show(ui, add)
        .inner
}

fn data_grid<R>(ui: &mut Ui, id: &str, cols: usize, add: impl FnOnce(&mut Ui) -> R) -> R {
    Grid::new(id)
        .num_columns(cols)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, add)
        .inner
}

/// Wide fixed-column tables get their own horizontal scroll so they never
/// force the whole panel wider than the window.
fn wide_table<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, add)
        .inner
}

fn head_cells(ui: &mut Ui, headers: &[&str]) {
    for h in headers {
        ui.label(RichText::new(*h).strong().small().color(MUTED));
    }
    ui.end_row();
}

fn num(ui: &mut Ui, v: String) {
    ui.label(RichText::new(v).monospace().small());
}

fn labelled_drag(ui: &mut Ui, label: &str, drag: DragValue<'_>) {
    ui.label(RichText::new(label).small());
    ui.add(drag);
    ui.end_row();
}

/// Left panel: everything the operator can change. Returns true if Run was hit.
pub fn inputs_panel(ui: &mut Ui, inputs: &mut Inputs, place: &mut PlaceMode) -> bool {
    let mut run = false;
    ui.heading("Inputs");
    ui.add_space(4.0);

    if ui
        .add_sized(
            [ui.available_width(), 30.0],
            egui::Button::new(RichText::new("RUN TRACE").strong()),
        )
        .clicked()
    {
        run = true;
    }
    hint(
        ui,
        "Solving is synchronous; a failed path runs a sweep and may take a second.",
    );

    ui.add_space(8.0);
    ui.separator();

    section(ui, "Endpoints");
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Map click sets").small());
            ui.selectable_value(place, PlaceMode::Tx, "TX");
            ui.selectable_value(place, PlaceMode::Rx, "RX");
        });
        ui.add_space(4.0);
        Grid::new("endpoints")
            .num_columns(3)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("TX lat/lon").small());
                ui.add(
                    DragValue::new(&mut inputs.tx_lat)
                        .speed(0.05)
                        .range(-89.9..=89.9),
                );
                ui.add(
                    DragValue::new(&mut inputs.tx_lon)
                        .speed(0.05)
                        .range(-180.0..=180.0),
                );
                ui.end_row();
                ui.label(RichText::new("RX lat/lon").small());
                ui.add(
                    DragValue::new(&mut inputs.rx_lat)
                        .speed(0.05)
                        .range(-89.9..=89.9),
                );
                ui.add(
                    DragValue::new(&mut inputs.rx_lon)
                        .speed(0.05)
                        .range(-180.0..=180.0),
                );
                ui.end_row();
            });
    });

    section(ui, "Signal");
    card(ui, |ui| {
        Grid::new("signal")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Frequency",
                    DragValue::new(&mut inputs.freq_mhz)
                        .speed(0.1)
                        .range(0.5..=60.0)
                        .suffix(" MHz"),
                );
                labelled_drag(
                    ui,
                    "Time (UTC)",
                    DragValue::new(&mut inputs.utc_hours)
                        .speed(0.25)
                        .range(0.0..=24.0)
                        .suffix(" h"),
                );
                labelled_drag(ui, "Month", DragValue::new(&mut inputs.month).range(1..=12));
                labelled_drag(
                    ui,
                    "Day of month",
                    DragValue::new(&mut inputs.day_of_month).range(1..=31),
                );
                labelled_drag(
                    ui,
                    "Max hops",
                    DragValue::new(&mut inputs.max_hops).range(1..=8),
                );
            });
    });

    section(ui, "Ionosphere");
    card(ui, |ui| {
        ui.checkbox(&mut inputs.solar_high, "Solar maximum (else minimum)");

        let mut fof2_on = inputs.fof2_override.is_some();
        if ui.checkbox(&mut fof2_on, "Override foF2").changed() {
            inputs.fof2_override = fof2_on.then_some(7.0);
        }
        if let Some(v) = inputs.fof2_override.as_mut() {
            ui.add(
                DragValue::new(v)
                    .speed(0.1)
                    .range(0.5..=30.0)
                    .suffix(" MHz"),
            );
        }

        let mut hmf2_on = inputs.hmf2_override.is_some();
        if ui.checkbox(&mut hmf2_on, "Override hmF2").changed() {
            inputs.hmf2_override = hmf2_on.then_some(300.0);
        }
        if let Some(v) = inputs.hmf2_override.as_mut() {
            ui.add(
                DragValue::new(v)
                    .speed(1.0)
                    .range(90.0..=600.0)
                    .suffix(" km"),
            );
        }

        ui.add_space(4.0);
        Grid::new("iono2")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Chapman scale H",
                    DragValue::new(&mut inputs.scale_height_km)
                        .speed(1.0)
                        .range(5.0..=200.0)
                        .suffix(" km"),
                );
                labelled_drag(
                    ui,
                    "Domain top",
                    DragValue::new(&mut inputs.domain_top_km)
                        .speed(10.0)
                        .range(400.0..=2000.0)
                        .suffix(" km"),
                );
            });
    });

    section(ui, "Absorption inputs");
    card(ui, |ui| {
        hint(
            ui,
            "By default the D-region electron density is derived from the solar \
             zenith angle at the path midpoint (alpha-Chapman, Nm x sqrt(cos chi)), \
             and nu comes from a fixed neutral-atmosphere profile. nu itself is NOT \
             a function of zenith angle - it follows neutral density.",
        );
        ui.add_space(4.0);
        ui.checkbox(
            &mut inputs.collision_manual,
            "Override collision profile manually",
        );
        if inputs.collision_manual {
            ui.add_space(4.0);
            Grid::new("coll")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    labelled_drag(
                        ui,
                        "nu at ref alt",
                        DragValue::new(&mut inputs.nu0_per_s)
                            .speed(1e3)
                            .range(0.0..=1e9)
                            .suffix(" /s"),
                    );
                    labelled_drag(
                        ui,
                        "ref altitude",
                        DragValue::new(&mut inputs.nu_ref_alt_km)
                            .speed(1.0)
                            .range(50.0..=300.0)
                            .suffix(" km"),
                    );
                    labelled_drag(
                        ui,
                        "nu scale H",
                        DragValue::new(&mut inputs.nu_scale_height_km)
                            .speed(0.5)
                            .range(2.0..=100.0)
                            .suffix(" km"),
                    );
                });
        }
    });

    section(ui, "Magnetic field");
    card(ui, |ui| {
        ui.checkbox(
            &mut inputs.use_field,
            "Use IGRF-14 (off = zero field, O/X degenerate)",
        );
        if inputs.use_field {
            ui.add_space(4.0);
            ui.add(
                DragValue::new(&mut inputs.igrf_epoch)
                    .speed(0.1)
                    .range(1900.0..=2030.0)
                    .prefix("epoch "),
            );
        }
    });

    ui.add_space(8.0);
    run
}

pub fn assumptions_panel(ui: &mut Ui, a: &Assumptions) {
    CollapsingHeader::new("Assumed values (all of them)")
        .default_open(false)
        .show(ui, |ui| {
            data_grid(ui, "assume", 2, |ui| {
                kv(ui, "foF2", format!("{:.2} MHz", a.fof2_mhz));
                kv(ui, "  source", a.fof2_source.clone());
                kv(ui, "hmF2", format!("{:.0} km", a.hmf2_km));
                kv(ui, "  source", a.hmf2_source.clone());
                kv(
                    ui,
                    "Chapman scale H",
                    format!("{:.0} km", a.scale_height_km),
                );
                kv(ui, "NmF2", format!("{:.4e} m^-3", a.nm_per_m3));
                kv(
                    ui,
                    "Path midpoint",
                    format!("{:.2}, {:.2}", a.midpoint_lat, a.midpoint_lon),
                );
                kv(
                    ui,
                    "Midpoint LST",
                    format!(
                        "{:.2} h ({})",
                        a.lst_hours,
                        if a.is_day { "day" } else { "night" }
                    ),
                );
                kv(ui, "Season", a.season.label().to_string());

                sub_head(ui, "SOLAR GEOMETRY (MIDPOINT)");
                kv(ui, "Day of year", a.solar.day_of_year.to_string());
                kv(
                    ui,
                    "Declination",
                    format!("{:.3} deg", a.solar.declination_deg),
                );
                kv(
                    ui,
                    "Hour angle",
                    format!("{:.3} deg", a.solar.hour_angle_deg),
                );
                kv(
                    ui,
                    "Solar zenith chi",
                    format!("{:.3} deg", a.solar.zenith_angle_deg),
                );
                kv(
                    ui,
                    "Solar elevation",
                    format!("{:.3} deg", a.solar.elevation_deg),
                );

                sub_head(ui, "D REGION (DRIVES ABSORPTION)");
                kv(
                    ui,
                    "D layer",
                    if a.d_region_active {
                        "active".to_string()
                    } else {
                        "omitted (night)".to_string()
                    },
                );
                if a.d_region_active {
                    kv(ui, "  peak Ne", format!("{:.4e} m^-3", a.d_region_peak_ne));
                    kv(
                        ui,
                        "  peak alt",
                        format!("{:.2} km", a.d_region_peak_alt_km),
                    );
                }
                kv(ui, "  basis", a.d_region_source.clone());

                sub_head(ui, "COLLISION FREQUENCY");
                kv(ui, "nu0", format!("{:.3e} /s", a.nu0_per_s));
                kv(ui, "  at altitude", format!("{:.0} km", a.nu_ref_alt_km));
                kv(
                    ui,
                    "  scale height",
                    format!("{:.1} km", a.nu_scale_height_km),
                );
                kv(ui, "  basis", a.collision_source.clone());
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
                ui.colored_label(FAIL, "No path connects.");
                return;
            }
            for (i, sol) in out.solutions.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    if let Some(v) = visible.get_mut(i) {
                        ui.checkbox(v, "");
                    }
                    ui.colored_label(PALETTE[i % PALETTE.len()], "\u{25A0}");
                    let label = RichText::new(format!(
                        "{}-mode, {} hop(s), {:.0} km group, {:.2} dB",
                        mode_label(sol.mode),
                        sol.hops,
                        sol.total_group_km,
                        sol.total_absorption_db
                    ))
                        .small();
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
            data_grid(ui, "soltot", 2, |ui| {
                kv(
                    ui,
                    "Mode",
                    format!("{} ({:?})", mode_label(sol.mode), sol.mode),
                );
                kv(ui, "Hops", sol.hops.to_string());
                kv(
                    ui,
                    "Total ground range",
                    format!("{:.2} km", sol.total_ground_km),
                );
                kv(
                    ui,
                    "Total group path",
                    format!("{:.2} km", sol.total_group_km),
                );
                kv(
                    ui,
                    "Total phase path",
                    format!("{:.2} km", sol.total_phase_km),
                );
                kv(
                    ui,
                    "Total arc length",
                    format!("{:.2} km", sol.total_arc_km),
                );
                kv(ui, "Group delay", format!("{:.3} ms", sol.group_delay_ms));
                kv(
                    ui,
                    "Absorption",
                    format!("{:.3} dB", sol.total_absorption_db),
                );
                kv(
                    ui,
                    "Homing miss (1 hop)",
                    format!("{:.2} m", sol.homing_miss_m),
                );
                kv(
                    ui,
                    "Terminal miss vs RX",
                    format!("{:.3} km", sol.terminal_miss_km),
                );
                kv(
                    ui,
                    "Max |H| drift",
                    format!("{:.3e}", sol.max_hamiltonian_drift),
                );
                kv(ui, "Total solver steps", sol.total_steps.to_string());
            });
            if let Some(note) = &sol.note {
                ui.add_space(4.0);
                ui.colored_label(WARN, RichText::new(format!("note: {note}")).small());
            }
            if sol.hops > 1 {
                hint(
                    ui,
                    "Multi-hop: launch angle homed on one hop of 1/N the arc, then \
                     propagated N hops by specular ground reflection. Terminal miss \
                     shows the equal-hop assumption's error.",
                );
            }
        });

    CollapsingHeader::new("Selected mode - per hop")
        .default_open(true)
        .show(ui, |ui| {
            wide_table(ui, "hops_scroll", |ui| {
                data_grid(ui, "hops", 15, |ui| {
                    head_cells(
                        ui,
                        &[
                            "hop",
                            "launch el",
                            "launch az",
                            "arr el",
                            "arr az",
                            "apex km",
                            "apex X",
                            "range km",
                            "group km",
                            "phase km",
                            "arc km",
                            "abs dB",
                            "steps",
                            "|H| drift",
                            "outcome",
                        ],
                    );
                    for hop in &sol.hop_details {
                        num(ui, hop.index.to_string());
                        num(ui, format!("{:.3}", hop.launch_elev_deg));
                        num(ui, format!("{:.3}", hop.launch_az_deg));
                        num(ui, format!("{:.3}", hop.arrival_elev_deg));
                        num(ui, format!("{:.3}", hop.arrival_az_deg));
                        num(ui, format!("{:.2}", hop.apex_alt_km));
                        num(ui, format!("{:.4}", hop.apex_x));
                        num(ui, format!("{:.2}", hop.ground_range_km));
                        num(ui, format!("{:.2}", hop.group_km));
                        num(ui, format!("{:.2}", hop.phase_km));
                        num(ui, format!("{:.2}", hop.arc_km));
                        num(ui, format!("{:.4}", hop.absorption_db));
                        num(ui, hop.steps.to_string());
                        num(ui, format!("{:.2e}", hop.hamiltonian_drift));
                        num(ui, hop.outcome.to_string());
                        ui.end_row();
                    }
                });
            });
            hint(
                ui,
                "apex X is (fp/f)^2 at the turning point, from the engine's apex \
                 record; solver health per hop is in the drift/steps totals above.",
            );
        });
}

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