//! The mode legend and the selected mode's totals plus per-hop breakdown.

use egui::{CollapsingHeader, RichText, Ui};

use crate::noise::PathState;
use crate::solve::{Solution, SolveOutcome, mode_label};
use crate::ui::theme::{self, FAIL, OK, WARN};
use crate::ui::widgets::{data_grid, head_cells, hint, kv, num, sub_head, wide_table};

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
                ui.colored_label(FAIL, "No path found.");
                return;
            }
            for (i, sol) in out.solutions.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    if let Some(v) = visible.get_mut(i) {
                        ui.checkbox(v, "");
                    }
                    ui.colored_label(theme::solution_color(i), "\u{25A0}");
                    let label = RichText::new(format!(
                        "{}-mode, {} hop(s), {:.0} km group, SNR {:.1} dB{}",
                        mode_label(sol.mode),
                        sol.hops,
                        sol.total_group_km,
                        sol.link.snr_db,
                        if sol.link.state() == PathState::Usable {
                            ""
                        } else {
                            " (below threshold)"
                        },
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
                    "TOTAL propagation loss",
                    format!("{:.1} dB", sol.total_system_loss_db),
                );

                sub_head(ui, "ANTENNA GAIN");
                kv(
                    ui,
                    &format!("TX at {:.1} deg launch", sol.tx_elev_deg),
                    format!("{:+.2} dBi", sol.tx_gain_dbi),
                );
                kv(
                    ui,
                    &format!("RX at {:.1} deg arrival", sol.rx_elev_deg),
                    format!("{:+.2} dBi", sol.rx_gain_dbi),
                );
                kv(ui, "TOTAL gain", format!("{:+.2} dB", sol.total_gain_db));

                let lb = &sol.link;
                sub_head(ui, "RECEIVED SIGNAL vs NOISE");
                kv(ui, "TX power", format!("{:.1} dBm", lb.tx_power_dbm));
                kv(ui, "Received power", format!("{:.1} dBm", lb.rx_power_dbm));
                kv(
                    ui,
                    "  atmospheric Fa",
                    format!("{:.1} dB", lb.noise.atmospheric_db),
                );
                kv(
                    ui,
                    "  man-made Fa",
                    format!("{:.1} dB", lb.noise.man_made_db),
                );
                kv(
                    ui,
                    "  galactic Fa",
                    format!("{:.1} dB", lb.noise.galactic_db),
                );
                kv(
                    ui,
                    "Total Fa (power sum)",
                    format!("{:.1} dB above kT0b", lb.noise.total_fa_db),
                );
                kv(
                    ui,
                    &format!("Noise floor ({:.0} Hz)", lb.noise.bandwidth_hz),
                    format!("{:.1} dBm", lb.noise.power_dbm),
                );
                kv(ui, "SNR", format!("{:.1} dB", lb.snr_db));
                kv(ui, "SNR threshold", format!("{:.1} dB", lb.threshold_db));
                kv(ui, "Margin", format!("{:+.1} dB", lb.margin_db()));
            });
            let verdict = sol.link.state();
            ui.colored_label(
                if verdict == PathState::Usable {
                    OK
                } else {
                    WARN
                },
                RichText::new(verdict.label()).strong().small(),
            );
            hint(
                ui,
                "Basic transmission loss = free-space spreading (over the ray path) + \
                 ionospheric absorption + Fresnel ground-reflection loss. Excludes antenna \
                 gains and any statistical excess-loss term, so it sits a few dB below a \
                 full VOACAP path loss.",
            );
            hint(
                ui,
                "Received power = TX power - TOTAL system loss (isotropic ends, no antenna \
                 gain). Noise floor = Fa + 10 log10(bandwidth) - 204 dBW, ITU-R P.372-9 \
                 eq. (6); man-made and galactic Fa are P.372-9 Table 1. The ATMOSPHERIC term \
                 is an approximation of this app's own, NOT P.372 map data - its trends with \
                 frequency, day/night and season are meaningful, its absolute level is \
                 indicative only. Noise is evaluated at the receiver's latitude and local \
                 day/night.",
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
                data_grid(ui, "hops", 19, |ui| {
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
                            "refl lat",
                            "refl lon",
                            "ground",
                            "gnd dB",
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
                        // The reflection point and the surface chosen there.
                        // Blank on the final hop, which arrives at the receiver
                        // and never bounces.
                        match hop.ground_label {
                            Some(label) => {
                                num(ui, format!("{:.2}", hop.end_lat_lon.0));
                                num(ui, format!("{:.2}", hop.end_lat_lon.1));
                                num(ui, label.to_string());
                                num(ui, format!("{:.2}", hop.ground_loss_db));
                            }
                            None => {
                                for _ in 0..4 {
                                    num(ui, "-".to_string());
                                }
                            }
                        }
                        num(ui, hop.steps.to_string());
                        num(ui, format!("{:.2e}", hop.hamiltonian_drift));
                        num(ui, hop.outcome.to_string());
                        ui.end_row();
                    }
                });
            });

            // Auto-detected surfaces have to justify themselves: one line per
            // bounce saying what was picked and why, so the classification can
            // be checked against the map overlay rather than trusted.
            let reasons: Vec<String> = sol
                .hop_details
                .iter()
                .filter_map(|h| {
                    let reason = h.ground_reason.as_ref()?;
                    Some(format!(
                        "hop {}: {} - {reason}",
                        h.index,
                        h.ground_label.unwrap_or("?"),
                    ))
                })
                .collect();
            if !reasons.is_empty() {
                ui.add_space(4.0);
                sub_head(ui, "GROUND AUTO-DETECT (COASTLINE)");
                for line in &reasons {
                    ui.label(RichText::new(line).small());
                }
                hint(
                    ui,
                    "Tested against the Natural Earth 1:50m land and lakes polygons. \
                     Turn on the coastline debug overlay in Settings > Surface to see \
                     those polygons drawn under the reflection dots.",
                );
            }
            hint(
                ui,
                "apex X is (fp/f)^2 at the turning point, from the engine's apex \
                 record; solver health per hop is in the drift/steps totals above.",
            );
        });
}
