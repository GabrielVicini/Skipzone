//! The Antennas dialog: pick the antenna at each end and see its pattern.
//!
//! Every control here reaches the solver. The chosen pattern is evaluated at
//! the launch elevation of the first hop and the arrival elevation of the last,
//! and both gains enter the link budget - see [`crate::antenna`] and
//! [`crate::solve`]. Nothing in this file computes a gain; it selects a model
//! and renders the curve that model produces.

use egui::{Color32, ComboBox, Context, DragValue, Grid, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::antenna::{AntennaConfig, AntennaKind, GainCurve};
use crate::scenario::Inputs;
use crate::state::{Session, UiState};
use crate::ui::theme::WARN;
use crate::ui::widgets::{card, data_grid, hint, kv, labelled_drag, section};

pub fn show(ctx: &Context, session: &mut Session, ui_state: &mut UiState) {
    let inputs = &mut session.inputs;
    super::chrome::dialog(
        ctx,
        "antennas_dialog",
        "Antennas",
        &mut ui_state.modals.antennas,
        vec2(560.0, 640.0),
        |ui| body(ui, inputs),
    );
}

fn body(ui: &mut Ui, inputs: &mut Inputs) {
    let f_hz = inputs.freq_mhz * 1e6;
    let ground = inputs.ground_type.as_antenna_ground();

    end(
        ui,
        "Transmitting antenna",
        "tx",
        &mut inputs.tx_antenna,
        inputs.freq_mhz,
    );
    end(
        ui,
        "Receiving antenna",
        "rx",
        &mut inputs.rx_antenna,
        inputs.freq_mhz,
    );

    let tx = inputs.tx_antenna.curve(ground, f_hz);
    let rx = inputs.rx_antenna.curve(ground, f_hz);

    section(ui, "Pattern at the current frequency");
    card(ui, |ui| {
        plot(ui, &tx, &rx);
        hint(
            ui,
            &format!(
                "Gain against take-off angle at {:.3} MHz over {}. Blue = TX, orange = RX. \
                 The solver reads each curve at the angle its ray actually uses, so a \
                 pattern changes which mode wins, not just the overall level.",
                inputs.freq_mhz,
                inputs.ground_type.label()
            ),
        );
    });

    section(ui, "What the solver will use");
    card(ui, |ui| {
        data_grid(ui, "antenna_summary", 2, |ui| {
            let (tg, ta) = tx.peak();
            let (rg, ra) = rx.peak();
            kv(ui, "TX", tx.label().to_string());
            kv(ui, "TX peak gain", format!("{tg:.2} dBi at {ta:.1} deg"));
            kv(ui, "RX", rx.label().to_string());
            kv(ui, "RX peak gain", format!("{rg:.2} dBi at {ra:.1} deg"));
            kv(
                ui,
                "TX power",
                format!("{:.0} W at the antenna", inputs.tx_power_w),
            );
            kv(ui, "Feedline loss", "not modelled".to_string());
        });
    });

    section(ui, "Where these numbers come from");
    card(ui, |ui| {
        for (who, cfg) in [("TX", inputs.tx_antenna), ("RX", inputs.rx_antenna)] {
            ui.label(RichText::new(format!("{who}: {}", cfg.kind.label())).strong());
            hint(ui, cfg.build(ground).provenance());
            ui.add_space(4.0);
        }
    });

    ui.colored_label(WARN, RichText::new("Not modelled at either end").strong());
    hint(
        ui,
        "Feedline loss, conductor and matching loss (beyond the EFHW transformer term), \
         radial / ground-screen loss under a vertical, terrain, and nearby structures. \
         Gain is reported in the antenna's best azimuth - the array is assumed aimed at \
         the path - so a wire broadside to the wrong bearing will do worse than this says.",
    );
}

fn end(ui: &mut Ui, title: &str, id: &str, cfg: &mut AntennaConfig, freq_mhz: f64) {
    section(ui, title);
    card(ui, |ui| {
        ComboBox::from_id_salt(format!("antenna_kind_{id}"))
            .selected_text(cfg.kind.label())
            .show_ui(ui, |ui| {
                for kind in AntennaKind::ALL {
                    ui.selectable_value(&mut cfg.kind, kind, kind.label());
                }
            });

        Grid::new(format!("antenna_params_{id}"))
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                if cfg.kind.uses_height() {
                    let label = if cfg.kind == AntennaKind::VerticalMonopole {
                        "Base height above ground"
                    } else {
                        "Height above ground"
                    };
                    labelled_drag(
                        ui,
                        label,
                        DragValue::new(&mut cfg.height_m)
                            .speed(0.5)
                            .range(0.0..=200.0)
                            .suffix(" m"),
                    );
                }
                if cfg.kind.uses_design_freq() {
                    labelled_drag(
                        ui,
                        "Cut for (design freq)",
                        DragValue::new(&mut cfg.efhw_design_mhz)
                            .speed(0.1)
                            .range(1.0..=30.0)
                            .suffix(" MHz"),
                    );
                }
            });

        match cfg.kind {
            AntennaKind::Isotropic => hint(
                ui,
                "0 dBi at every angle. Physically unrealisable, and kept only as the \
                 baseline: select it at both ends to reproduce the numbers this app \
                 produced before it had antenna patterns.",
            ),
            AntennaKind::HorizontalDipole => {
                let lam = 299.792_458 / freq_mhz;
                hint(
                    ui,
                    &format!(
                        "Resonant at whatever frequency is in use: {:.1} m tip to tip at \
                         {freq_mhz:.3} MHz, and {:.2} wavelengths up at this height.",
                        lam / 2.0,
                        cfg.height_m / lam
                    ),
                );
            }
            AntennaKind::VerticalMonopole => hint(
                ui,
                "Quarter-wave worked against ground, resonant at the operating frequency. \
                 Leave the base height at 0 for the ordinary ground-mounted case. A \
                 non-zero height models the earth reflection of an elevated vertical but \
                 NOT its elevated radial screen, which is a materially different antenna.",
            ),
            AntennaKind::Efhw => {
                let n = (freq_mhz / cfg.efhw_design_mhz).round().max(1.0);
                let lam0 = 299.792_458 / cfg.efhw_design_mhz;
                let note = if freq_mhz < cfg.efhw_design_mhz * 0.9 {
                    " BELOW its design band: the wire is electrically short, the standing \
                     wave assumption fails, and the half-wave pattern shown is optimistic."
                } else {
                    ""
                };
                hint(
                    ui,
                    &format!(
                        "A fixed {:.1} m wire, half-wave at {:.3} MHz. At {freq_mhz:.3} MHz \
                         it is {n:.0} half-wave(s) long, which is what shapes the pattern.{note}",
                        lam0 / 2.0,
                        cfg.efhw_design_mhz
                    ),
                );
            }
        }
    });
}

/// Gain against elevation, 0-90 deg horizontal, dBi vertical.
fn plot(ui: &mut Ui, tx: &GainCurve, rx: &GainCurve) {
    const HEIGHT: f32 = 190.0;
    const FLOOR_DBI: f64 = -20.0;

    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();

    // Vertical range: whole dB steps around whatever the two curves do, never
    // narrower than the floor, so the shape is readable at any gain.
    let peak = tx.peak().0.max(rx.peak().0).max(0.0).ceil() + 2.0;
    let span = (peak - FLOOR_DBI).max(1.0);

    let x_of = |deg: f64| {
        #[allow(clippy::cast_possible_truncation)]
        let t = (deg / 90.0) as f32;
        rect.left() + t * rect.width()
    };
    let y_of = |dbi: f64| {
        #[allow(clippy::cast_possible_truncation)]
        let t = ((peak - dbi.max(FLOOR_DBI)) / span) as f32;
        rect.top() + t * rect.height()
    };

    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);

    // Elevation gridlines every 15 deg, gain gridline every 5 dB.
    let grid = Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.35));
    for deg in (0..=90).step_by(15) {
        let x = x_of(f64::from(deg));
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
        painter.text(
            pos2(x, rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            format!("{deg}"),
            egui::FontId::proportional(9.0),
            visuals.weak_text_color(),
        );
    }
    let mut db = (peak / 5.0).floor() * 5.0;
    while db > FLOOR_DBI {
        let y = y_of(db);
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
        painter.text(
            pos2(rect.left() + 2.0, y),
            egui::Align2::LEFT_BOTTOM,
            format!("{db:.0}"),
            egui::FontId::proportional(9.0),
            visuals.weak_text_color(),
        );
        db -= 5.0;
    }

    for (curve, color) in [
        (tx, Color32::from_rgb(90, 160, 240)),
        (rx, Color32::from_rgb(240, 150, 70)),
    ] {
        let pts: Vec<_> = curve
            .points()
            .map(|(deg, g)| pos2(x_of(deg), y_of(g)))
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.6, color)));
    }

    painter.text(
        pos2(rect.right() - 4.0, rect.top() + 2.0),
        egui::Align2::RIGHT_TOP,
        "dBi vs take-off angle (deg)",
        egui::FontId::proportional(9.0),
        visuals.weak_text_color(),
    );
}
