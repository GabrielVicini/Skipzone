//! The frequency-sweep verdict line and the live band chart drawn beneath the
//! FIND BEST FREQUENCY control.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::solve::mode_label;
use crate::sweep::{SWEEP_MAX_MHZ, SWEEP_MIN_MHZ, SweepBest, SweepPoint};

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
