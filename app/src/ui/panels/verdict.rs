//! The headline verdict chip, and the per-layer breakdown under it.
//!
//! This is the label the whole judgment layer exists to get right. It reports
//! three separate facts rather than one, because they really are separate:
//!
//! 1. whether the geometry closes,
//! 2. whether the signal clears the threshold,
//! 3. whether the layer carrying it is reliably there at all.
//!
//! The third is why the deterministic verdict and the sporadic-E one are drawn
//! apart. A path that needs an Es sheet is not "usable" in the same sense as an
//! F2 path; folding the two into one yes/no is what let a position inside the F2
//! skip zone read as though nothing could ever reach it.

use egui::{CornerRadius, Margin, RichText, Stroke, Ui};

use crate::noise::PathState;
use crate::solve::{LayerStatus, SolveOutcome};
use crate::ui::theme::{self, FAIL, MUTED, WARN};

pub fn verdict_chip(ui: &mut Ui, out: &SolveOutcome) {
    let best = out
        .solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db));
    let best_es = out
        .es_solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db));

    let (colour, text) = match best {
        Some(s) if s.link.state() == PathState::Usable => (
            theme::state_color(PathState::Usable),
            format!(
                "USABLE - {} - SNR {:.1} dB ({:+.1} dB over threshold), {} mode(s)",
                s.link.confidence_label(),
                s.link.snr_db,
                s.link.margin_db(),
                out.solutions.len(),
            ),
        ),
        Some(s) => (
            theme::state_color(PathState::BelowThreshold),
            format!(
                "PATH FOUND, BELOW THRESHOLD - {} - SNR {:.1} dB is {:.1} dB \
                 short; {} mode(s)",
                s.link.confidence_label(),
                s.link.snr_db,
                -s.link.margin_db(),
                out.solutions.len(),
            ),
        ),
        // No deterministic path. Whether that means "nothing arrives" depends
        // entirely on what sporadic E did, so say which.
        None => match best_es {
            Some(s) => (
                WARN,
                // TWO different probabilities, and they must not be conflated.
                // `s.probability` is how often the Es sheet EXISTS - a climatological
                // occurrence rate, and the one number here with a defensible level.
                // `confidence_label` is whether the signal clears the threshold GIVEN
                // that it does, and only its ORDERING is validated (see
                // `noise::PREDICTIVE_SPREAD_DB`), so it is stated as a word.
                // Multiplying the two would produce a joint percentage that looks
                // far more precise than either input supports.
                format!(
                    "SPORADIC E ONLY - no F2 or E path at this range. Es gives SNR {:.1} dB, \
                     {} if the sheet is there - and the sheet is present about {:.0} % of \
                     the time. NOT a dead path, but not a reliable one",
                    s.link.snr_db,
                    s.link.confidence_label(),
                    100.0 * s.probability,
                ),
            ),
            None => (FAIL, no_path_headline(out)),
        },
    };

    egui::Frame::NONE
        .fill(colour.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, colour))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.colored_label(colour, RichText::new(text).strong());
        });

    mode_breakdown(ui, out);
}

/// When nothing at all connected, say WHY in the headline rather than leaving
/// "NO PATH" to mean four different things.
fn no_path_headline(out: &SolveOutcome) -> String {
    let f2 = out.mode_reports.first();
    match f2.map(|r| r.status) {
        Some(LayerStatus::NoBracket) => "NO PATH at this frequency - every layer reflects, but \
             none puts a ray at this range: the receiver is inside the skip zone, or past the \
             maximum range"
            .to_string(),
        Some(LayerStatus::Penetrates) => "NO PATH at this frequency - the ray penetrates every \
             layer at every elevation: this frequency is above the MUF for any geometry"
            .to_string(),
        Some(LayerStatus::Failed) => "NO VERDICT - the tracer failed on this scenario. This is a \
             numerical failure, not a statement that nothing arrives; see the errors list"
            .to_string(),
        _ => "NO PATH FOUND at this frequency".to_string(),
    }
}

/// One line per layer: its own SNR and its own status, then the combination.
/// Every layer is listed even when it produced nothing, because "F2 has no
/// solution here" is a result an operator needs to see, not an absence.
fn mode_breakdown(ui: &mut Ui, out: &SolveOutcome) {
    ui.add_space(4.0);
    for r in &out.mode_reports {
        ui.horizontal_wrapped(|ui| {
            let colour = theme::state_color(r.state());
            ui.colored_label(colour, RichText::new("\u{25A0}").strong());
            ui.label(
                RichText::new(format!("{:>2}", r.layer.label()))
                    .monospace()
                    .strong(),
            );
            if r.status == LayerStatus::Solved {
                ui.label(
                    RichText::new(format!(
                        "SNR {:>6.1} dB ({:+.1} dB vs threshold), {} hop(s)",
                        r.best_snr_db,
                        r.margin_db(),
                        r.hops
                    ))
                    .monospace(),
                );
                // Probability is shown only where it is not 1, so a
                // deterministic layer is never decorated with false precision.
                if r.probability < 1.0 {
                    ui.colored_label(
                        WARN,
                        RichText::new(format!("{:.0} % occurrence", 100.0 * r.probability))
                            .strong(),
                    );
                }
            } else {
                ui.label(RichText::new(r.status.label()).color(MUTED));
            }
        });
    }
}
