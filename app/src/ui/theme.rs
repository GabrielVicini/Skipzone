//! The one place colours, spacing and container chrome are defined.
//!
//! Two container styles carry the whole layout: the solid [`header_frame`] the
//! menus and station rows sit on, and the translucent [`floating_frame`] every
//! control that floats over the map wears. Nothing else paints its own
//! background, so "does this sit on the map or above it?" is answered by which
//! frame a widget is wrapped in.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Frame, Margin, Shadow, Stroke, Style, TextStyle, Ui,
};

use crate::noise::PathState;

// --- Verdict palette -----------------------------------------------------

pub const OK: Color32 = Color32::from_rgb(0x3F, 0xC7, 0x66);
pub const WARN: Color32 = Color32::from_rgb(0xF2, 0xB5, 0x3C);
pub const BAD: Color32 = Color32::from_rgb(0xE8, 0x6A, 0x3C);
pub const FAIL: Color32 = Color32::from_rgb(0xE6, 0x39, 0x46);
pub const MUTED: Color32 = Color32::from_gray(0x92);
pub const ACCENT: Color32 = Color32::from_rgb(0x4C, 0xC9, 0xF0);

// --- Surfaces ------------------------------------------------------------

/// The solid bar behind the menus and the TX/RX rows.
const HEADER_FILL: Color32 = Color32::from_rgb(0x14, 0x18, 0x1D);
/// Fill of a control floating over the map: dark enough to stay legible over
/// bright coastline tiles, translucent enough that the map reads through.
const FLOAT_FILL: Color32 = Color32::from_rgba_premultiplied(0x11, 0x15, 0x1A, 0xE0);
const HAIRLINE: Color32 = Color32::from_gray(0x3A);

/// Frame for the solid top bar.
pub fn header_frame() -> Frame {
    Frame::NONE
        .fill(HEADER_FILL)
        .inner_margin(Margin {
            left: 10,
            right: 10,
            top: 4,
            bottom: 6,
        })
        .stroke(Stroke::NONE)
}

/// Frame for a group of controls floating over the map.
pub fn floating_frame() -> Frame {
    Frame::NONE
        .fill(FLOAT_FILL)
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(1.0, HAIRLINE))
        .shadow(Shadow {
            offset: [0, 3],
            blur: 10,
            spread: 0,
            color: Color32::from_black_alpha(0x50),
        })
}

/// Frame for a bordered block inside a dialog. No fill: dialogs already have
/// one, and stacking two surfaces just muddies them.
pub fn card_frame(ui: &Ui) -> Frame {
    Frame::NONE
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
}

/// Frame for a modal dialog.
pub fn dialog_frame() -> Frame {
    Frame::NONE
        .fill(Color32::from_rgb(0x17, 0x1B, 0x21))
        .inner_margin(Margin::same(12))
        .corner_radius(CornerRadius::same(10))
        .stroke(Stroke::new(1.0, HAIRLINE))
        .shadow(Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(0x80),
        })
}

/// Frame for a dropdown or picker popup.
pub fn popup_frame() -> Frame {
    Frame::NONE
        .fill(Color32::from_rgb(0x1A, 0x1F, 0x26))
        .inner_margin(Margin::symmetric(6, 6))
        .corner_radius(CornerRadius::same(8))
        .stroke(Stroke::new(1.0, HAIRLINE))
        .shadow(Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(0x70),
        })
}

// --- Text and spacing ----------------------------------------------------

/// Text and spacing scale with the window width, so the same readouts stay
/// legible on a laptop without looking cramped on a large display. Re-applied
/// only when the width has moved meaningfully, since restyling every frame
/// would thrash egui's font atlas.
pub fn apply_scale(ui: &mut Ui, styled_for_width: &mut f32) {
    let width = ui.available_width();
    if (width - *styled_for_width).abs() < 40.0 {
        return;
    }
    *styled_for_width = width;
    let scale = (width / 1440.0).clamp(0.86, 1.18);
    scale_style(ui.style_mut(), scale);
}

fn scale_style(style: &mut Style, scale: f32) {
    let (mono, prop) = (FontFamily::Monospace, FontFamily::Proportional);
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0 * scale, prop.clone())),
        (TextStyle::Body, FontId::new(13.0 * scale, prop.clone())),
        (TextStyle::Button, FontId::new(13.0 * scale, prop.clone())),
        (TextStyle::Small, FontId::new(11.0 * scale, prop)),
        (TextStyle::Monospace, FontId::new(11.5 * scale, mono)),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(6.0 * scale, 4.0 * scale);
    style.spacing.button_padding = egui::vec2(7.0 * scale, 4.0 * scale);
    style.spacing.indent = 14.0 * scale;
    style.spacing.interact_size.y = 20.0 * scale;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
}

// --- Solution and verdict colours ----------------------------------------

/// Palette for distinguishing solutions on the map and in the legend; the index
/// wraps, so any number of modes can be drawn.
pub const SOLUTION_PALETTE: [Color32; 8] = [
    Color32::from_rgb(0xE6, 0x39, 0x46),
    Color32::from_rgb(0x2A, 0x9D, 0x8F),
    Color32::from_rgb(0xE9, 0xC4, 0x6A),
    Color32::from_rgb(0x8E, 0x7D, 0xBE),
    Color32::from_rgb(0xF4, 0xA2, 0x61),
    Color32::from_rgb(0x4C, 0xC9, 0xF0),
    Color32::from_rgb(0x90, 0xBE, 0x6D),
    Color32::from_rgb(0xFF, 0x6F, 0xB5),
];

#[must_use]
pub fn solution_color(index: usize) -> Color32 {
    SOLUTION_PALETTE[index % SOLUTION_PALETTE.len()]
}

/// The headline colour for a verdict.
#[must_use]
pub fn state_color(state: PathState) -> Color32 {
    match state {
        PathState::Usable => OK,
        PathState::BelowThreshold => WARN,
        PathState::NoPath => FAIL,
    }
}

/// Colour band per verdict state, shaded WITHIN the band by `badness`
/// (0 = best, 1 = worst):
///   * usable          - green, darkening as the SNR margin shrinks
///   * below threshold - yellow, darkening as the shortfall grows
///   * no path         - red, darkening as the near-miss grows
///
/// The three bands are deliberately different HUES, not points on one ramp:
/// "geometry closes but nobody can hear it" is a different kind of answer from
/// "there is no path", and no chart should blend them into each other.
#[must_use]
pub fn state_shade(state: PathState, badness: f32) -> Color32 {
    match state {
        PathState::Usable => mix([0x3F, 0xC7, 0x66], [0x1B, 0x63, 0x33], badness),
        PathState::BelowThreshold => mix([0xF2, 0xD3, 0x5C], [0x8A, 0x6E, 0x14], badness),
        PathState::NoPath => mix([0xD8, 0x53, 0x35], [0x6B, 0x1A, 0x0C], badness),
    }
}

/// Swatch and caption per verdict state, for chart legends.
#[must_use]
pub fn state_legend() -> [(Color32, &'static str); 3] {
    [
        (OK, "usable (SNR clears threshold)"),
        (WARN, "path found, below threshold"),
        (FAIL, "no path"),
    ]
}

// --- Area coverage gradient ----------------------------------------------

/// SNR-to-colour breakpoints for the area coverage map, in the VOACAP idiom:
/// blues for weak, greens for moderate, yellow/orange for strong, red for very
/// strong. Between two stops the colour is interpolated linearly in RGB; below
/// the first and above the last it saturates.
///
/// This is a colour ramp over ONE computed number. It never interpolates
/// between grid points - each tile is painted the colour of its own solve.
pub const COVERAGE_STOPS: [(f64, [u8; 3]); 7] = [
    (-10.0, [0x0D, 0x1B, 0x4A]), // deep navy: nothing usable
    (0.0, [0x1F, 0x4F, 0xD8]),   // blue: signal equals the noise
    (10.0, [0x00, 0xB7, 0xD8]),  // cyan
    (20.0, [0x2F, 0xBF, 0x4F]),  // green: comfortably readable
    (30.0, [0xE8, 0xDA, 0x2A]),  // yellow
    (40.0, [0xF0, 0x8A, 0x22]),  // orange
    (50.0, [0xD8, 0x23, 0x1C]),  // red: very strong
];

/// Colour for a grid point where ray tracing found no path at all. Grey, and
/// deliberately outside the ramp: "nothing arrives" is a different kind of
/// answer from "something weak arrives", and the two must not blend.
pub const COVERAGE_NO_PATH: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x6E);

/// Colour for a grid point the deterministic layers cannot reach, but that a
/// sporadic-E opening can. Purple, a third hue outside both the ramp and the
/// no-path grey.
///
/// This exists because the key used to promise a distinction the solver could
/// not make. A position inside the F2 skip zone was painted the same grey as a
/// position nothing could reach - which is how a several-hundred-km "hard dead
/// zone" appeared around the transmitter on paths that were, in fact, being
/// heard. An Es tile is now its own answer, and its colour carries how likely
/// it is: see [`coverage_es_color`].
pub const COVERAGE_ES_ONLY: Color32 = Color32::from_rgb(0x9B, 0x59, 0xD0);

/// Colour for an Es-only tile at a given SNR and occurrence probability.
///
/// The ramp colour for the SNR, faded towards the background as the occurrence
/// probability falls. The fade is the honesty: a 45 %-likely opening reads
/// strongly, a 5 %-likely one barely reads at all, and neither is confusable
/// with a deterministic path of the same strength - which keeps its full
/// saturation.
#[must_use]
pub fn coverage_es_color(snr_db: f64, probability: f64) -> Color32 {
    let base = coverage_color(snr_db);
    // Never fade to nothing: even an unlikely opening must stay visible as a
    // distinct answer from "no path at all".
    #[allow(clippy::cast_possible_truncation)]
    let weight = (0.35 + 0.65 * probability.clamp(0.0, 1.0)) as f32;
    let (br, bg, bb, _) = base.to_tuple();
    let (er, eg, eb, _) = COVERAGE_ES_ONLY.to_tuple();
    mix([er, eg, eb], [br, bg, bb], weight)
}

/// Colour for one computed SNR, following [`COVERAGE_STOPS`].
#[must_use]
pub fn coverage_color(snr_db: f64) -> Color32 {
    if !snr_db.is_finite() {
        return COVERAGE_NO_PATH;
    }
    let first = COVERAGE_STOPS[0];
    let last = COVERAGE_STOPS[COVERAGE_STOPS.len() - 1];
    if snr_db <= first.0 {
        return Color32::from_rgb(first.1[0], first.1[1], first.1[2]);
    }
    if snr_db >= last.0 {
        return Color32::from_rgb(last.1[0], last.1[1], last.1[2]);
    }
    for w in COVERAGE_STOPS.windows(2) {
        let ((lo_db, lo), (hi_db, hi)) = (w[0], w[1]);
        if snr_db <= hi_db {
            #[allow(clippy::cast_possible_truncation)]
            let t = ((snr_db - lo_db) / (hi_db - lo_db)) as f32;
            return mix(lo, hi, t);
        }
    }
    COVERAGE_NO_PATH
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published breakpoints are what the map actually paints, the ramp
    /// saturates outside them, and a value with no path never lands anywhere on
    /// the ramp.
    #[test]
    fn coverage_ramp_hits_its_stated_breakpoints() {
        for (db, rgb) in COVERAGE_STOPS {
            let (r, g, b, _) = coverage_color(db).to_tuple();
            assert_eq!([r, g, b], rgb, "stop at {db} dB");
        }
        // Saturating, not wrapping, outside the table.
        assert_eq!(coverage_color(-999.0), coverage_color(COVERAGE_STOPS[0].0));
        assert_eq!(coverage_color(999.0), coverage_color(50.0));
        // No path is off the ramp entirely.
        assert_eq!(coverage_color(f64::NEG_INFINITY), COVERAGE_NO_PATH);
        assert!(COVERAGE_STOPS.iter().all(|&(db, rgb)| {
            let (r, g, b, _) = COVERAGE_NO_PATH.to_tuple();
            let _ = db;
            [r, g, b] != rgb
        }));
    }

    /// Between two stops the colour moves monotonically from one to the other,
    /// so a brighter tile really does mean a stronger computed SNR.
    #[test]
    fn coverage_ramp_interpolates_between_stops() {
        let mid = coverage_color(5.0).to_tuple();
        let (lo, hi) = (
            coverage_color(0.0).to_tuple(),
            coverage_color(10.0).to_tuple(),
        );
        for c in 0..3 {
            let (a, m, b) = (
                i32::from([lo.0, lo.1, lo.2][c]),
                i32::from([mid.0, mid.1, mid.2][c]),
                i32::from([hi.0, hi.1, hi.2][c]),
            );
            assert!(m >= a.min(b) && m <= a.max(b), "channel {c}: {a} {m} {b}");
        }
    }
}
