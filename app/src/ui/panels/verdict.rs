//! The headline verdict chip.
//!
//! This is the label the whole judgment layer exists to get right: it reports
//! the geometry and the received-signal judgment as the two separate facts they
//! are, rather than claiming audibility the moment a ray closes.

use egui::{CornerRadius, Margin, RichText, Stroke, Ui};

use crate::noise::PathState;
use crate::solve::SolveOutcome;
use crate::ui::theme::{self, FAIL};

pub fn verdict_chip(ui: &mut Ui, out: &SolveOutcome) {
    let best = out
        .solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db));
    let (colour, text) = match best {
        None => (FAIL, "NO PATH FOUND at this frequency".to_string()),
        Some(s) if s.link.state() == PathState::Usable => (
            theme::state_color(PathState::Usable),
            format!(
                "USABLE - path found, SNR {:.1} dB ({:+.1} dB over threshold), {} mode(s)",
                s.link.snr_db,
                s.link.margin_db(),
                out.solutions.len(),
            ),
        ),
        Some(s) => (
            theme::state_color(PathState::BelowThreshold),
            format!(
                "PATH FOUND, BELOW THRESHOLD - geometry closes, SNR {:.1} dB is {:.1} dB \
                 short; {} mode(s)",
                s.link.snr_db,
                -s.link.margin_db(),
                out.solutions.len(),
            ),
        ),
    };
    egui::Frame::NONE
        .fill(colour.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, colour))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.colored_label(colour, RichText::new(text).strong());
        });
}
