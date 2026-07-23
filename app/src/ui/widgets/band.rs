//! The frequency-sweep band strip: one bar per tried frequency, coloured by the
//! three-state verdict, with the tuned and best frequencies marked.
//!
//! Drawn from the cached sweep points, so it redraws every frame without
//! re-running any solve. It is the overview companion to the detailed charts in
//! [`crate::ui::widgets::chart`]: this answers "where in the band is it good?"
//! at a glance, the charts answer "by how much?".

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::sweep::{SWEEP_MAX_MHZ, SWEEP_MIN_MHZ, SweepPoint};
use crate::ui::theme::{self, ACCENT, MUTED};

/// Draw the band. `points` need not be sorted.
pub fn band(ui: &mut Ui, points: &[SweepPoint], tuned_mhz: f64, best: Option<SweepPoint>) {
    let height = 40.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(0x1E));

    let span = (SWEEP_MAX_MHZ - SWEEP_MIN_MHZ).max(1e-9);
    #[allow(clippy::cast_possible_truncation)]
    let x_of =
        |f: f64| rect.left() + rect.width() * ((f - SWEEP_MIN_MHZ) / span).clamp(0.0, 1.0) as f32;

    // Sampling is non-uniform (a coarse pass plus a dense cluster near the
    // best), so each point is drawn as a bar spanning halfway to its sorted
    // neighbours. Unswept frequencies stay background - which is exactly where
    // the search decided not to look.
    let mut sorted: Vec<&SweepPoint> = points.iter().collect();
    sorted.sort_by(|a, b| a.freq_mhz.total_cmp(&b.freq_mhz));
    for (i, p) in sorted.iter().enumerate() {
        let x = x_of(p.freq_mhz);
        let prev_x = i.checked_sub(1).map(|j| x_of(sorted[j].freq_mhz));
        let next_x = sorted.get(i + 1).map(|q| x_of(q.freq_mhz));
        // Span halfway to each neighbour; at an end, mirror the near gap (or a
        // small default when this is the only point so far).
        let left = match (prev_x, next_x) {
            (Some(px), _) => f32::midpoint(x, px),
            (None, Some(nx)) => x - 0.5 * (nx - x),
            (None, None) => x - 3.0,
        };
        let right = match (next_x, prev_x) {
            (Some(nx), _) => f32::midpoint(x, nx),
            (None, Some(px)) => x + 0.5 * (x - px),
            (None, None) => x + 3.0,
        };
        painter.rect_filled(
            Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom())),
            0.0,
            theme::state_shade(p.state, p.badness()),
        );
    }

    if let Some(best) = best {
        let x = x_of(best.freq_mhz);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(2.5, ACCENT),
        );
    }
    let x = x_of(tuned_mhz);
    painter.line_segment(
        [pos2(x, rect.top()), pos2(x, rect.bottom())],
        Stroke::new(1.5, Color32::WHITE),
    );

    // Hovering a bar gives that frequency's full readout - the same line the
    // sweep logs, so the two can never disagree.
    if let Some(pos) = response.hover_pos()
        && let Some(nearest) = sorted.iter().min_by(|a, b| {
            (x_of(a.freq_mhz) - pos.x)
                .abs()
                .total_cmp(&(x_of(b.freq_mhz) - pos.x).abs())
        })
    {
        response.clone().on_hover_text(nearest.debug_line());
    }

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

/// Swatch legend for the three verdict states, plus the marker key.
pub fn legend(ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        for (colour, text) in theme::state_legend() {
            ui.colored_label(colour, egui::RichText::new("\u{25A0}").small());
            ui.label(egui::RichText::new(text).small().color(MUTED));
        }
        ui.label(
            egui::RichText::new("| white = tuned, cyan = best")
                .small()
                .color(MUTED),
        );
    });
}
