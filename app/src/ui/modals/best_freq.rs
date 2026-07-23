//! Point-to-Point > Best FREQ: the frequency sweep, presented as charts.
//!
//! The sweep's raw per-frequency log is exact but unreadable at a glance, so
//! this dialog draws it three ways - what the receiver hears against its
//! threshold, what arrives against the noise it has to beat, and where the loss
//! went - over the same frequency axis. Hovering any chart pins one frequency
//! and reports every computed metric for it underneath, so the three views can
//! be read as one answer rather than three pictures.

use egui::{Button, Color32, Context, RichText, Ui, vec2};

use crate::noise::PathState;
use crate::solve::mode_label;
use crate::state::{Session, UiState};
use crate::sweep::{SWEEP_MAX_MHZ, SWEEP_MIN_MHZ, SweepBest, SweepPoint};
use crate::ui::actions::Action;
use crate::ui::theme::{self, ACCENT, MUTED};
use crate::ui::widgets::chart::{Chart, Series};
use crate::ui::widgets::{band, card, data_grid, hint, kv, section};

/// Colours used consistently across the three charts for the same quantity.
const SNR_COLOR: Color32 = Color32::from_rgb(0x6E, 0xC8, 0xF2);
const POWER_COLOR: Color32 = Color32::from_rgb(0x8E, 0xD0, 0x81);
const NOISE_COLOR: Color32 = Color32::from_rgb(0xF2, 0x9A, 0x76);
const LOSS_COLOR: Color32 = Color32::from_rgb(0xC0, 0xA6, 0xF0);
const ABSORPTION_COLOR: Color32 = Color32::from_rgb(0xF2, 0xD3, 0x5C);

pub fn show(ctx: &Context, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let busy = session.is_busy();
    let tuned_mhz = session.inputs.freq_mhz;
    let threshold_db = session.inputs.snr_threshold_db;
    let points = session.sweep.sorted();
    let best = session.sweep.best;

    super::chrome::dialog(
        ctx,
        "best_freq_dialog",
        "Best frequency",
        &mut ui_state.modals.best_freq,
        vec2(720.0, 660.0),
        |ui| {
            body(
                ui,
                &points,
                best,
                Params {
                    busy,
                    tuned_mhz,
                    threshold_db,
                },
            )
        },
    )
    .flatten()
}

/// Scalars the charts need that are not part of the sweep result.
#[derive(Clone, Copy)]
struct Params {
    busy: bool,
    tuned_mhz: f64,
    threshold_db: f64,
}

fn body(
    ui: &mut Ui,
    points: &[SweepPoint],
    best: Option<SweepBest>,
    params: Params,
) -> Option<Action> {
    let mut action = controls(ui, best, params);

    if points.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "No sweep has run yet. Find best frequency scans 2-30 MHz at the current \
                 TX/RX, date, time and scenario, and reports the frequency with the \
                 strongest SNR at the receiver.",
            )
            .color(MUTED),
        );
        return action;
    }

    if let Some(best) = best {
        ui.add_space(6.0);
        let colour = theme::state_color(best.point.state);
        ui.colored_label(colour, RichText::new(verdict_text(best)).strong());
    }

    ui.add_space(8.0);
    band::band(ui, points, params.tuned_mhz, best.map(|b| b.point));
    band::legend(ui);

    let hovered = charts(ui, points, best, params);
    readout(ui, points, hovered, params);

    if action.is_none()
        && let Some(freq) = hovered
        && ui
            .button(format!("Tune to {:.2} MHz", nearest(points, freq).freq_mhz))
            .clicked()
    {
        action = Some(Action::TuneTo(nearest(points, freq).freq_mhz));
    }

    action
}

fn controls(ui: &mut Ui, best: Option<SweepBest>, params: Params) -> Option<Action> {
    let mut action = None;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                !params.busy,
                Button::new(RichText::new("Find best frequency").strong()),
            )
            .on_hover_text(
                "Sweep 2-30 MHz at the current TX/RX and scenario. Runs off the UI thread; \
                 the current solution stays.",
            )
            .clicked()
        {
            action = Some(Action::BestFrequency);
        }
        if params.busy {
            ui.add(egui::Spinner::new().size(14.0));
        }
        if let Some(best) = best
            && ui
                .button(format!("Tune to best ({:.2} MHz)", best.point.freq_mhz))
                .clicked()
        {
            action = Some(Action::TuneTo(best.point.freq_mhz));
        }
    });
    action
}

/// The three charts. Returns the frequency under the pointer, if any.
fn charts(
    ui: &mut Ui,
    points: &[SweepPoint],
    best: Option<SweepBest>,
    params: Params,
) -> Option<f64> {
    let range = (SWEEP_MIN_MHZ, SWEEP_MAX_MHZ);
    let series = |f: fn(&SweepPoint) -> f64| -> Vec<(f64, f64)> {
        points.iter().map(|p| (p.freq_mhz, f(p))).collect()
    };
    let snr = series(|p| p.snr_db);
    let rx_power = series(|p| p.rx_power_dbm);
    let noise = series(|p| p.noise_dbm);
    let loss = series(|p| p.system_loss_db);
    let absorption = series(|p| p.absorption_db);

    let best_mhz = best.map(|b| b.point.freq_mhz);
    let mut hovered = None;

    section(ui, "Signal-to-noise ratio");
    let mut chart = Chart::new(range)
        .height(160.0)
        .unit(" dB")
        .series(Series::line(SNR_COLOR, &snr))
        .series(Series::dots(SNR_COLOR, &snr))
        .rule(params.threshold_db, theme::OK, "threshold")
        .marker(params.tuned_mhz, Color32::WHITE, "tuned");
    if let Some(mhz) = best_mhz {
        chart = chart.marker(mhz, ACCENT, "best");
    }
    hovered = hovered.or(chart.show(ui));
    hint(
        ui,
        "Gaps are frequencies where ray tracing found no path at all - there is no SNR to \
         plot, which is a different statement from a weak one.",
    );

    section(ui, "Received power against the noise floor");
    let mut chart = Chart::new(range)
        .height(150.0)
        .unit(" dBm")
        .series(Series::line(POWER_COLOR, &rx_power))
        .series(Series::line(NOISE_COLOR, &noise))
        .marker(params.tuned_mhz, Color32::WHITE, "tuned");
    if let Some(mhz) = best_mhz {
        chart = chart.marker(mhz, ACCENT, "best");
    }
    hovered = hovered.or(chart.show(ui));
    swatches(
        ui,
        &[
            (POWER_COLOR, "received power"),
            (NOISE_COLOR, "noise floor at the receiver"),
        ],
    );

    section(ui, "Where the loss goes");
    let mut chart = Chart::new(range)
        .height(150.0)
        .unit(" dB")
        .series(Series::line(LOSS_COLOR, &loss))
        .series(Series::line(ABSORPTION_COLOR, &absorption))
        .marker(params.tuned_mhz, Color32::WHITE, "tuned");
    if let Some(mhz) = best_mhz {
        chart = chart.marker(mhz, ACCENT, "best");
    }
    hovered = hovered.or(chart.show(ui));
    swatches(
        ui,
        &[
            (LOSS_COLOR, "total system loss"),
            (ABSORPTION_COLOR, "ionospheric absorption"),
        ],
    );

    hovered
}

/// Every computed metric at the hovered frequency, or at the best one when the
/// pointer is elsewhere - so the panel is never blank.
fn readout(ui: &mut Ui, points: &[SweepPoint], hovered: Option<f64>, params: Params) {
    let Some(freq) = hovered.or_else(|| points.first().map(|_| params.tuned_mhz)) else {
        return;
    };
    let point = nearest(points, freq);

    section(
        ui,
        if hovered.is_some() {
            "At the pointer"
        } else {
            "At the tuned frequency (hover a chart for any other)"
        },
    );
    card(ui, |ui| {
        data_grid(ui, "sweep_readout", 2, |ui| {
            kv(ui, "Frequency", format!("{:.2} MHz", point.freq_mhz));
            kv(ui, "Verdict", point.state.label().to_string());
            kv(
                ui,
                "Mode / hops",
                format!("{} / {}", point.mode.map_or("-", mode_label), point.hops),
            );
            if point.state == PathState::NoPath {
                kv(ui, "Near miss", format!("{:.0} km", point.miss_km));
            } else {
                kv(ui, "SNR", format!("{:.1} dB", point.snr_db));
                kv(
                    ui,
                    "Margin vs threshold",
                    format!("{:+.1} dB", point.margin_db),
                );
                kv(
                    ui,
                    "Received power",
                    format!("{:.1} dBm", point.rx_power_dbm),
                );
                kv(
                    ui,
                    "Total system loss",
                    format!("{:.1} dB", point.system_loss_db),
                );
                kv(ui, "Absorption", format!("{:.2} dB", point.absorption_db));
            }
            kv(ui, "Noise floor", format!("{:.1} dBm", point.noise_dbm));
            kv(
                ui,
                "SNR threshold",
                format!("{:.1} dB", params.threshold_db),
            );
        });
    });
}

fn swatches(ui: &mut Ui, entries: &[(Color32, &str)]) {
    ui.horizontal_wrapped(|ui| {
        for (colour, label) in entries {
            ui.colored_label(*colour, RichText::new("\u{2014}").strong());
            ui.label(RichText::new(*label).small().color(MUTED));
        }
    });
}

/// The swept point closest in frequency to `freq_mhz`. `points` is never empty
/// where this is called.
fn nearest(points: &[SweepPoint], freq_mhz: f64) -> SweepPoint {
    points
        .iter()
        .copied()
        .min_by(|a, b| {
            (a.freq_mhz - freq_mhz)
                .abs()
                .total_cmp(&(b.freq_mhz - freq_mhz).abs())
        })
        .unwrap_or(points[0])
}

/// One-line verdict for the best-frequency search.
#[must_use]
fn verdict_text(best: SweepBest) -> String {
    let p = best.point;
    match p.state {
        PathState::Usable => format!(
            "Best: {:.2} MHz - {}-mode, {} hop(s), SNR {:.1} dB ({:+.1} dB margin)",
            p.freq_mhz,
            p.mode.map_or("?", mode_label),
            p.hops,
            p.snr_db,
            p.margin_db,
        ),
        PathState::BelowThreshold => format!(
            "No frequency is usable in {SWEEP_MIN_MHZ:.0}-{SWEEP_MAX_MHZ:.0} MHz. Best geometry: \
             {:.2} MHz, {}-mode, {} hop(s), SNR {:.1} dB - {:.1} dB short of the threshold",
            p.freq_mhz,
            p.mode.map_or("?", mode_label),
            p.hops,
            p.snr_db,
            -p.margin_db,
        ),
        PathState::NoPath => format!(
            "No path found in {SWEEP_MIN_MHZ:.0}-{SWEEP_MAX_MHZ:.0} MHz. Closest: \
             {:.2} MHz, near-miss {:.0} km ({} hop(s))",
            p.freq_mhz, p.miss_km, p.hops
        ),
    }
}
