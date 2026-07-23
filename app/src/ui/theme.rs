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

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(lerp(a[0], b[0]), lerp(a[1], b[1]), lerp(a[2], b[2]))
}
