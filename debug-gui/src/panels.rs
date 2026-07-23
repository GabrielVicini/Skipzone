//! All the debug readouts. Deliberately dense and complete rather than tidy:
//! this is an instrument panel, not a product screen.

use egui::{
    Align2, CollapsingHeader, Color32, ComboBox, CornerRadius, DragValue, FontId, Frame, Grid,
    Layout, Margin, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2,
};

use crate::mapview::PALETTE;
use crate::scenario::{Assumptions, GroundType, Inputs, PlaceMode, ProfileRow};
use crate::solve::{Solution, SolveOutcome, mode_label};
use crate::sweep::{SWEEP_MAX_MHZ, SWEEP_MIN_MHZ, SweepBest, SweepPoint};

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
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
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
        Grid::new("iono")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Sunspot number",
                    DragValue::new(&mut inputs.ssn)
                        .speed(1.0)
                        .range(0.0..=300.0),
                );
                labelled_drag(
                    ui,
                    "hmF2 (peak height)",
                    DragValue::new(&mut inputs.hmf2_km)
                        .speed(1.0)
                        .range(90.0..=600.0)
                        .suffix(" km"),
                );
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
        hint(
            ui,
            &format!(
                "foF2 = {:.2} MHz, derived from SSN (NmF2 linear in SSN, coarse midlat \
                 anchor - not a path/season/time prediction). hmF2 and scale H are yours \
                 directly; there is no climatology table underneath.",
                crate::scenario::fof2_from_ssn(inputs.ssn)
            ),
        );
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

    section(ui, "Surface (ground reflections)");
    card(ui, |ui| {
        ComboBox::from_id_salt("ground_type")
            .selected_text(inputs.ground_type.label())
            .show_ui(ui, |ui| {
                for g in GroundType::ALL {
                    ui.selectable_value(&mut inputs.ground_type, g, g.label());
                }
            });
        let (eps_r, sigma) = inputs.ground_type.constants();
        hint(
            ui,
            &format!(
                "Surface at the intermediate ground bounces, used for the Fresnel \
                 reflection loss in the link budget. eps_r = {eps_r:.0}, sigma = {sigma} S/m \
                 (ITU-R P.527 / P.368 HF-band values). One choice approximates the whole \
                 path - there is no coastline database here.",
            ),
        );
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
                        "producing at midpoint (varies along path)".to_string()
                    } else {
                        "night at midpoint (still active on any sunlit part)".to_string()
                    },
                );
                kv(
                    ui,
                    "  midpoint peak Ne",
                    format!("{:.4e} m^-3", a.d_region_peak_ne),
                );
                kv(
                    ui,
                    "  midpoint peak alt",
                    format!("{:.2} km", a.d_region_peak_alt_km),
                );
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

                sub_head(ui, "LINK BUDGET (BASIC TRANSMISSION LOSS)");
                kv(
                    ui,
                    "Free-space spreading",
                    format!("{:.1} dB", sol.free_space_loss_db),
                );
                kv(
                    ui,
                    "Ionospheric absorption",
                    format!("{:.3} dB", sol.total_absorption_db),
                );
                kv(
                    ui,
                    &format!("Ground reflections ({}x)", sol.num_ground_reflections),
                    format!("{:.2} dB", sol.ground_reflection_loss_db),
                );
                kv(
                    ui,
                    "TOTAL system loss",
                    format!("{:.1} dB", sol.total_system_loss_db),
                );
            });
            hint(
                ui,
                "Basic transmission loss = free-space spreading (over the ray path) + \
                 ionospheric absorption + Fresnel ground-reflection loss. Excludes antenna \
                 gains and any statistical excess-loss term, so it sits a few dB below a \
                 full VOACAP path loss.",
            );
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

/// Green (best) -> amber -> red (worst) ramp for a badness in [0, 1].
fn grad_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8, u: f32| -> u8 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (f32::from(a) + (f32::from(b) - f32::from(a)) * u).round() as u8
        }
    };
    let mix = |a: [u8; 3], b: [u8; 3], u: f32| {
        Color32::from_rgb(
            lerp(a[0], b[0], u),
            lerp(a[1], b[1], u),
            lerp(a[2], b[2], u),
        )
    };
    let green = [0x2E, 0x9D, 0x4F];
    let amber = [0xE9, 0xC4, 0x4A];
    let red = [0xC8, 0x3A, 0x1C];
    if t < 0.5 {
        mix(green, amber, t / 0.5)
    } else {
        mix(amber, red, (t - 0.5) / 0.5)
    }
}

/// One-line verdict for the best-frequency search.
#[must_use]
pub fn sweep_verdict_text(best: SweepBest) -> String {
    let p = best.point;
    if p.connects {
        format!(
            "Best: {:.2} MHz - {}-mode, {} hop(s), {:.2} dB absorption",
            p.freq_mhz,
            p.mode.map_or("?", mode_label),
            p.hops,
            p.absorption_db
        )
    } else {
        format!(
            "No frequency connects in {SWEEP_MIN_MHZ:.0}-{SWEEP_MAX_MHZ:.0} MHz. Closest: \
             {:.2} MHz, near-miss {:.0} km ({} hop(s))",
            p.freq_mhz, p.miss_km, p.hops
        )
    }
}

/// The live frequency-sweep band: one coloured bar per tried frequency, green
/// (connects, low absorption) through amber to red (no connection / large
/// miss), with the current and best frequencies marked. Drawn from the cache,
/// so it redraws every frame without re-running any solve.
pub fn sweep_chart(
    ui: &mut Ui,
    points: &[SweepPoint],
    current_freq: f64,
    best: Option<SweepPoint>,
) {
    let width = ui.available_width();
    let height = 54.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(0x1E));

    let span = (SWEEP_MAX_MHZ - SWEEP_MIN_MHZ).max(1e-9);
    #[allow(clippy::cast_possible_truncation)]
    let x_of =
        |f: f64| rect.left() + rect.width() * ((f - SWEEP_MIN_MHZ) / span).clamp(0.0, 1.0) as f32;

    // Sampling is non-uniform (a coarse pass plus a dense cluster near the
    // best), so draw each point as a bar spanning halfway to its sorted
    // neighbours. Unswept frequencies stay background - which is exactly where
    // the early-stop decided not to look.
    let mut sorted: Vec<&SweepPoint> = points.iter().collect();
    sorted.sort_by(|a, b| a.freq_mhz.total_cmp(&b.freq_mhz));
    for (i, p) in sorted.iter().enumerate() {
        let x = x_of(p.freq_mhz);
        let prev_x = i.checked_sub(1).map(|j| x_of(sorted[j].freq_mhz));
        let next_x = sorted.get(i + 1).map(|q| x_of(q.freq_mhz));
        // Span halfway to each neighbour; at an end, mirror the near gap (or a
        // small default when this is the only point so far).
        let left = match (prev_x, next_x) {
            (Some(px), _) => 0.5 * (x + px),
            (None, Some(nx)) => x - 0.5 * (nx - x),
            (None, None) => x - 3.0,
        };
        let right = match (next_x, prev_x) {
            (Some(nx), _) => 0.5 * (x + nx),
            (None, Some(px)) => x + 0.5 * (x - px),
            (None, None) => x + 3.0,
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())),
            0.0,
            grad_color(p.badness()),
        );
    }

    // Best frequency (cyan) and current tuned frequency (white) markers.
    if let Some(b) = best {
        let xb = x_of(b.freq_mhz);
        painter.line_segment(
            [pos2(xb, rect.top()), pos2(xb, rect.bottom())],
            Stroke::new(2.5, Color32::from_rgb(0x4C, 0xC9, 0xF0)),
        );
    }
    let xc = x_of(current_freq);
    painter.line_segment(
        [pos2(xc, rect.top()), pos2(xc, rect.bottom())],
        Stroke::new(1.5, Color32::WHITE),
    );

    // Frequency axis labels along the bottom edge.
    for f in [SWEEP_MIN_MHZ, 10.0, 20.0, SWEEP_MAX_MHZ] {
        painter.text(
            pos2(x_of(f), rect.bottom() - 1.0),
            Align2::CENTER_BOTTOM,
            format!("{f:.0}"),
            FontId::proportional(9.0),
            Color32::from_gray(0xE0),
        );
    }
}
