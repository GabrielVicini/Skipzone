//! The frequency-sweep verdict line and the live band chart drawn beneath the
//! FIND BEST FREQUENCY control.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::noise::PathState;
use crate::solve::mode_label;
use crate::sweep::{SWEEP_MAX_MHZ, SWEEP_MIN_MHZ, SweepBest, SweepPoint};

fn lerp(a: u8, b: u8, u: f32) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * u).round() as u8
    }
}

fn mix(a: [u8; 3], b: [u8; 3], u: f32) -> Color32 {
    let u = u.clamp(0.0, 1.0);
    Color32::from_rgb(lerp(a[0], b[0], u), lerp(a[1], b[1], u), lerp(a[2], b[2], u))
}

/// Colour band per verdict state, shaded within the band by `badness`:
///   * usable            - green, darkening as the SNR margin shrinks
///   * below threshold   - yellow, darkening as the shortfall grows
///   * no path           - red, darkening as the near-miss grows
///
/// The three bands are deliberately different HUES, not points on one ramp:
/// "geometry closes but nobody can hear it" is a different kind of answer from
/// "there is no path", and the chart should not blend them into each other.
fn state_color(p: SweepPoint) -> Color32 {
    let t = p.badness();
    match p.state {
        PathState::Usable => mix([0x3F, 0xC7, 0x66], [0x1B, 0x63, 0x33], t),
        PathState::BelowThreshold => mix([0xF2, 0xD3, 0x5C], [0x8A, 0x6E, 0x14], t),
        PathState::NoPath => mix([0xD8, 0x53, 0x35], [0x6B, 0x1A, 0x0C], t),
    }
}

/// Swatches for the chart legend, in the same order as the three states.
#[must_use]
pub fn state_legend() -> [(Color32, &'static str); 3] {
    [
        (
            Color32::from_rgb(0x3F, 0xC7, 0x66),
            "usable (SNR clears threshold)",
        ),
        (
            Color32::from_rgb(0xF2, 0xD3, 0x5C),
            "path found, below threshold",
        ),
        (Color32::from_rgb(0xD8, 0x53, 0x35), "no path"),
    ]
}

/// One-line verdict for the best-frequency search.
#[must_use]
pub fn sweep_verdict_text(best: SweepBest) -> String {
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

/// The live frequency-sweep band: one bar per tried frequency, coloured by the
/// three-state verdict (green usable / yellow path-found-but-too-weak / red no
/// path), with the current and best frequencies marked. Drawn from the cache,
/// so it redraws every frame without re-running any solve.
pub fn sweep_chart(
    ui: &mut Ui,
    points: &[SweepPoint],
    current_freq: f64,
    best: Option<SweepPoint>,
) {
    let width = ui.available_width();
    let height = 54.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
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
            state_color(**p),
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

    // Hovering a bar gives that frequency's full readout - the same line the
    // sweep logs to stderr, so the two never disagree.
    if let Some(pos) = response.hover_pos()
        && let Some(nearest) = sorted.iter().min_by(|a, b| {
            (x_of(a.freq_mhz) - pos.x)
                .abs()
                .total_cmp(&(x_of(b.freq_mhz) - pos.x).abs())
        })
    {
        response.clone().on_hover_text(nearest.debug_line());
    }

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
