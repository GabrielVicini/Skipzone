//! Coverage Maps > Area coverage: the grid controls, the run readout and the
//! colour key for the tiles drawn on the map.
//!
//! The map itself is the output - this dialog only says what to compute and
//! reports how the run is going. Nothing here calculates anything; it reads
//! `session.coverage` and emits an [`Action`].

use egui::{Button, Color32, Context, ProgressBar, RichText, Ui, vec2};

use crate::coverage::{CoverageCell, MAX_POINTS, MIN_RANGE_KM};
use crate::solve::mode_label;
use crate::state::{Busy, Session, UiState};
use crate::ui::actions::Action;
use crate::ui::theme::{self, ACCENT, MUTED, WARN};
use crate::ui::widgets::{card, data_grid, hint, kv, section};

/// Grid sizes past this take long enough that the operator should be told
/// before starting rather than after. A full solve measures at a few seconds of
/// CPU, so even across every core a grid this size runs for many minutes.
const LARGE_GRID: usize = 600;

pub fn show(ctx: &Context, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let mut open = ui_state.modals.coverage;
    let result = super::chrome::dialog(
        ctx,
        "coverage_dialog",
        "Area coverage",
        &mut open,
        vec2(460.0, 620.0),
        |ui| body(ui, session, ui_state),
    );
    ui_state.modals.coverage = open;
    result.flatten()
}

fn body(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let mut action = None;

    ui.label(
        RichText::new(
            "One transmitter, a grid of receiver positions. Every grid point is a full \
             point-to-point solve at the tuned frequency - the same ray tracing, link \
             budget, antenna pattern, noise floor and ground detection RUN TRACE uses - \
             and is coloured by the SNR that solve produced.",
        )
        .color(MUTED),
    );

    ui.add_space(8.0);
    section(ui, "Grid");
    let points = grid_controls(ui, session);

    ui.add_space(8.0);
    action = action.or(run_controls(ui, session, points));

    ui.add_space(8.0);
    progress(ui, session);

    ui.add_space(8.0);
    section(ui, "Display");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Tile opacity").small().color(MUTED));
        ui.add(egui::Slider::new(&mut ui_state.coverage_alpha, 40..=255).show_value(false));
    });
    hint(
        ui,
        "Opacity is a drawing control only - it changes how much basemap shows through, \
         never a computed value.",
    );

    ui.add_space(8.0);
    legend(ui);

    if !session.coverage.cells.is_empty() {
        ui.add_space(8.0);
        summary(ui, &session.coverage.cells);
    }

    action
}

/// The resolution and extent controls. Returns how many grid points the current
/// settings would actually solve.
fn grid_controls(ui: &mut Ui, session: &mut Session) -> usize {
    let cfg = &mut session.coverage_config;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Resolution").small().color(MUTED));
        ui.add(
            egui::DragValue::new(&mut cfg.points_per_deg)
                .speed(0.02)
                .range(0.05..=4.0)
                .suffix(" pts/deg"),
        )
        .on_hover_text(
            "Grid points per degree, in both latitude and longitude. This is a real \
             resolution: raising it runs more solves. Nothing is interpolated or smoothed \
             between points, so a coarse grid looks blocky - that is an honest picture of \
             how much was computed.",
        );
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new("Extent").small().color(MUTED));
        ui.add(
            egui::DragValue::new(&mut cfg.half_span_deg)
                .speed(0.5)
                .range(1.0..=180.0)
                .suffix(" deg"),
        )
        .on_hover_text("Half-width of the box centred on the transmitter.");
    });

    let cfg = session.coverage_config;
    let points = cfg.grid(session.inputs.tx_lat, session.inputs.tx_lon).len();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!(
                "{points} grid point(s), {:.2} deg apart",
                cfg.step_deg()
            ))
            .small()
            .color(MUTED),
        );
    });
    // The point cap can shrink the box well below what Extent asks for, and an
    // unannounced 17-degree run in place of a requested 180 just looks like the
    // map failed to fill in.
    let effective = cfg.effective_half_span_deg();
    if effective < cfg.half_span_deg - 1e-9 {
        ui.label(
            RichText::new(format!(
                "Capped at {effective:.1} deg by the {} point limit - lower the resolution to \
                 reach {:.0} deg.",
                crate::coverage::MAX_POINTS,
                cfg.half_span_deg
            ))
            .small()
            .color(MUTED),
        );
    }
    if points >= LARGE_GRID {
        ui.label(
            RichText::new(format!(
                "\u{26A0} {points} full solves. Each one costs about what RUN TRACE costs; \
                 this will run for a while. CANCEL keeps whatever has been drawn."
            ))
            .small()
            .color(WARN),
        );
    }
    hint(
        ui,
        &format!(
            "Points within {MIN_RANGE_KM:.0} km of the transmitter are skipped (no hop \
             geometry to home), and a run is capped at {MAX_POINTS} points."
        ),
    );
    points
}

fn run_controls(ui: &mut Ui, session: &Session, points: usize) -> Option<Action> {
    let mut action = None;
    let running = matches!(session.busy, Busy::Covering { .. });
    let busy_elsewhere = session.is_busy() && !running;

    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !running && !busy_elsewhere && points > 0,
                Button::new(RichText::new("Run coverage").strong()),
            )
            .on_hover_text("Runs off the UI thread; the map fills in as each point finishes.")
            .clicked()
        {
            action = Some(Action::RunCoverage);
        }
        if running
            && ui
                .button(RichText::new("Cancel").strong())
                .on_hover_text("Stop the remaining calculations. Everything already drawn stays.")
                .clicked()
        {
            action = Some(Action::CancelCoverage);
        }
        // Available whenever there is anything on the map to clear, whether the
        // run finished normally or was cancelled. Not offered mid-run, where
        // clearing would only be undone by the next point streaming in.
        if !running
            && session.coverage.has_tiles()
            && ui
                .button("Reset")
                .on_hover_text("Clear the coverage tiles from the map.")
                .clicked()
        {
            action = Some(Action::ResetCoverage);
        }
    });
    action
}

fn progress(ui: &mut Ui, session: &Session) {
    match session.busy {
        Busy::Covering {
            done,
            total,
            threads,
        } => {
            #[allow(clippy::cast_precision_loss)]
            let fraction = if total > 0 {
                (done as f32 / total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.add(
                ProgressBar::new(fraction)
                    .fill(ACCENT.gamma_multiply(0.7))
                    .text(RichText::new(format!("{done} / {total} points")).small()),
            );
            ui.label(
                RichText::new(format!(
                    "running on {threads} thread(s) - the coverage grid takes every core, \
                     unlike the frequency sweep"
                ))
                .small()
                .color(MUTED),
            );
        }
        _ if session.coverage.cancelled => {
            ui.label(
                RichText::new(format!(
                    "Cancelled with {} point(s) computed. They stay on the map until Reset.",
                    session.coverage.cells.len()
                ))
                .small()
                .color(WARN),
            );
        }
        _ if session.coverage.has_tiles() => {
            ui.label(
                RichText::new(format!(
                    "{} point(s) on the map.",
                    session.coverage.cells.len()
                ))
                .small()
                .color(MUTED),
            );
        }
        _ => {}
    }
}

/// The colour key, drawn from the same breakpoint table the map paints with, so
/// the two can never disagree.
fn legend(ui: &mut Ui) {
    section(ui, "SNR colour key");
    let height = 18.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let stops = theme::COVERAGE_STOPS;
    let (lo, hi) = (stops[0].0, stops[stops.len() - 1].0);
    // One screen column per pixel, each painted with the ramp's value at that
    // pixel: this is a legend for a continuous scale, not a tile grid.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = rect.width().max(1.0) as usize;
    for i in 0..columns {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f64 / columns.max(1) as f64;
        let x = rect.left() + rect.width() * (t as f32);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), vec2(2.0, height)),
            0.0,
            theme::coverage_color(lo + t * (hi - lo)),
        );
    }
    ui.horizontal(|ui| {
        for (db, _) in stops {
            ui.label(
                RichText::new(format!("{db:+.0}"))
                    .small()
                    .monospace()
                    .color(MUTED),
            );
            ui.add_space(2.0);
        }
        ui.label(RichText::new("dB SNR").small().color(MUTED));
    });
    for (colour, caption) in [
        (
            theme::COVERAGE_ES_ONLY,
            "sporadic E only - no deterministic path, but an Es opening reaches here. Faded by \
             how likely that opening is; the ramp above still sets the hue's brightness",
        ),
        (
            theme::COVERAGE_NO_PATH,
            "no path found - nothing arrives at all, by any mode. Distinct from both a weak \
             signal and from an Es-only position",
        ),
    ] {
        ui.horizontal(|ui| {
            let (r, g, b, _) = colour.to_tuple();
            ui.colored_label(
                Color32::from_rgb(r, g, b),
                RichText::new("\u{25A0}").strong(),
            );
            ui.label(RichText::new(caption).small().color(MUTED));
        });
    }
}

/// What the tiles currently on the map add up to.
fn summary(ui: &mut Ui, cells: &[CoverageCell]) {
    section(ui, "Computed tiles");
    let with_path = cells.iter().filter(|c| c.found_path()).count();
    let deterministic = cells.iter().filter(|c| c.has_deterministic_path()).count();
    let es_only = cells.iter().filter(|c| c.es_only()).count();
    let usable = cells
        .iter()
        .filter(|c| c.found_path() && c.margin_db >= 0.0)
        .count();
    let best = cells
        .iter()
        .filter(|c| c.found_path())
        .max_by(|a, b| a.snr_db.total_cmp(&b.snr_db));

    card(ui, |ui| {
        data_grid(ui, "coverage_summary", 2, |ui| {
            kv(ui, "Points computed", cells.len().to_string());
            kv(ui, "With a path", with_path.to_string());
            kv(ui, "  deterministic (F2 / E)", deterministic.to_string());
            kv(ui, "  sporadic E only", es_only.to_string());
            kv(ui, "Clearing the threshold", usable.to_string());
            if let Some(b) = best {
                // The full link budget at the strongest tile, so the colour on
                // the map can be checked against the numbers that produced it.
                kv(
                    ui,
                    "Strongest tile",
                    format!("{:.2}, {:.2} ({:.0} km)", b.lat, b.lon, b.range_km),
                );
                kv(
                    ui,
                    "  mode / hops",
                    format!("{} / {}", b.mode.map_or("?", mode_label), b.hops),
                );
                kv(ui, "  received power", format!("{:.1} dBm", b.rx_power_dbm));
                kv(ui, "  noise floor", format!("{:.1} dBm", b.noise_dbm));
                kv(ui, "  SNR", format!("{:.1} dB", b.snr_db));
                kv(
                    ui,
                    "  margin vs threshold",
                    format!("{:+.1} dB", b.margin_db),
                );
            }
        });
    });
}
