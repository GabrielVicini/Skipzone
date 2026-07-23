//! A small self-contained line chart with a hover crosshair.
//!
//! Written rather than pulled in as a dependency because what the sweep needs
//! is specific: series that legitimately contain non-finite values (no path
//! means an SNR of negative infinity, which must be a GAP in the line, not a
//! spike to the bottom of the axis), horizontal reference rules for thresholds,
//! and vertical markers for the tuned and best frequencies.
//!
//! The widget is stateless: [`Chart::show`] returns the x value under the
//! pointer, and the caller decides what to display for it. That keeps the
//! readout logic (which knows about frequencies, modes and SNR) out of the
//! drawing code, which knows only about numbers.

use egui::{Align2, Color32, FontId, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::ui::theme::MUTED;

const AXIS_LEFT: f32 = 52.0;
const AXIS_BOTTOM: f32 = 18.0;
const PLOT_BG: Color32 = Color32::from_rgb(0x16, 0x1A, 0x20);
const GRID: Color32 = Color32::from_gray(0x2E);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SeriesKind {
    Line,
    Dots,
}

pub struct Series<'a> {
    pub color: Color32,
    /// `(x, y)` samples, assumed sorted by x. Non-finite y values are treated
    /// as gaps.
    pub points: &'a [(f64, f64)],
    pub kind: SeriesKind,
}

impl<'a> Series<'a> {
    #[must_use]
    pub fn line(color: Color32, points: &'a [(f64, f64)]) -> Self {
        Self {
            color,
            points,
            kind: SeriesKind::Line,
        }
    }

    /// One dot per sample, for showing where the sweep actually evaluated.
    #[must_use]
    pub fn dots(color: Color32, points: &'a [(f64, f64)]) -> Self {
        Self {
            color,
            points,
            kind: SeriesKind::Dots,
        }
    }
}

/// A horizontal reference line, e.g. the SNR threshold.
pub struct Rule<'a> {
    pub y: f64,
    pub color: Color32,
    pub label: &'a str,
}

/// A vertical marker, e.g. the tuned or best frequency.
pub struct Marker<'a> {
    pub x: f64,
    pub color: Color32,
    pub label: &'a str,
}

pub struct Chart<'a> {
    height: f32,
    unit: &'a str,
    x_range: (f64, f64),
    series: Vec<Series<'a>>,
    rules: Vec<Rule<'a>>,
    markers: Vec<Marker<'a>>,
}

impl<'a> Chart<'a> {
    #[must_use]
    pub fn new(x_range: (f64, f64)) -> Self {
        Self {
            height: 150.0,
            unit: "",
            x_range,
            series: Vec::new(),
            rules: Vec::new(),
            markers: Vec::new(),
        }
    }

    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Unit suffix for the y-axis tick labels.
    #[must_use]
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    #[must_use]
    pub fn series(mut self, series: Series<'a>) -> Self {
        self.series.push(series);
        self
    }

    #[must_use]
    pub fn rule(mut self, y: f64, color: Color32, label: &'a str) -> Self {
        self.rules.push(Rule { y, color, label });
        self
    }

    #[must_use]
    pub fn marker(mut self, x: f64, color: Color32, label: &'a str) -> Self {
        self.markers.push(Marker { x, color, label });
        self
    }

    /// Draw the chart. Returns the x value under the pointer while hovering.
    pub fn show(self, ui: &mut Ui) -> Option<f64> {
        let width = ui.available_width().max(120.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height), Sense::hover());
        let plot = Rect::from_min_max(
            pos2(rect.left() + AXIS_LEFT, rect.top()),
            pos2(rect.right(), rect.bottom() - AXIS_BOTTOM),
        );
        if plot.width() < 4.0 || plot.height() < 4.0 {
            return None;
        }

        let (y_lo, y_hi) = self.y_bounds();
        let painter = ui.painter_at(rect);
        painter.rect_filled(plot, 4.0, PLOT_BG);

        let x_of = |x: f64| {
            let t = (x - self.x_range.0) / (self.x_range.1 - self.x_range.0).max(1e-12);
            #[allow(clippy::cast_possible_truncation)]
            let t = t.clamp(0.0, 1.0) as f32;
            plot.left() + plot.width() * t
        };
        let y_of = |y: f64| {
            let t = (y - y_lo) / (y_hi - y_lo).max(1e-12);
            #[allow(clippy::cast_possible_truncation)]
            let t = t.clamp(0.0, 1.0) as f32;
            plot.bottom() - plot.height() * t
        };

        // Horizontal grid and y-axis labels.
        for y in ticks(y_lo, y_hi, 4) {
            let py = y_of(y);
            painter.line_segment(
                [pos2(plot.left(), py), pos2(plot.right(), py)],
                Stroke::new(1.0, GRID),
            );
            painter.text(
                pos2(plot.left() - 6.0, py),
                Align2::RIGHT_CENTER,
                format!("{}{}", format_tick(y), self.unit),
                FontId::proportional(9.0),
                MUTED,
            );
        }
        // Vertical grid and x-axis labels.
        for x in ticks(self.x_range.0, self.x_range.1, 6) {
            let px = x_of(x);
            painter.line_segment(
                [pos2(px, plot.top()), pos2(px, plot.bottom())],
                Stroke::new(1.0, GRID.gamma_multiply(0.6)),
            );
            painter.text(
                pos2(px, plot.bottom() + 3.0),
                Align2::CENTER_TOP,
                format_tick(x),
                FontId::proportional(9.0),
                MUTED,
            );
        }

        // Reference rules, drawn under the data.
        for rule in &self.rules {
            if !(y_lo..=y_hi).contains(&rule.y) {
                continue;
            }
            let py = y_of(rule.y);
            dashed_line(
                &painter,
                pos2(plot.left(), py),
                pos2(plot.right(), py),
                rule.color,
            );
            painter.text(
                pos2(plot.right() - 4.0, py - 2.0),
                Align2::RIGHT_BOTTOM,
                rule.label,
                FontId::proportional(9.0),
                rule.color,
            );
        }

        // Vertical markers.
        for marker in &self.markers {
            if !(self.x_range.0..=self.x_range.1).contains(&marker.x) {
                continue;
            }
            let px = x_of(marker.x);
            painter.line_segment(
                [pos2(px, plot.top()), pos2(px, plot.bottom())],
                Stroke::new(1.5, marker.color.gamma_multiply(0.8)),
            );
            painter.text(
                pos2(px + 3.0, plot.top() + 2.0),
                Align2::LEFT_TOP,
                marker.label,
                FontId::proportional(9.0),
                marker.color,
            );
        }

        // Data. A non-finite sample breaks the polyline rather than clamping,
        // so "no path here" reads as a gap instead of a plunge to the floor.
        for series in &self.series {
            let mut run: Vec<egui::Pos2> = Vec::new();
            for &(x, y) in series.points {
                if y.is_finite() {
                    run.push(pos2(x_of(x), y_of(y)));
                } else {
                    flush(&painter, &mut run, series);
                }
            }
            flush(&painter, &mut run, series);
        }

        self.hover(ui, &response, plot)
    }

    /// Crosshair and the x value under the pointer.
    fn hover(&self, ui: &Ui, response: &Response, plot: Rect) -> Option<f64> {
        let pos = response.hover_pos().filter(|p| plot.contains(*p))?;
        let painter = ui.painter_at(plot);
        painter.line_segment(
            [pos2(pos.x, plot.top()), pos2(pos.x, plot.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(0x60)),
        );
        let t = f64::from((pos.x - plot.left()) / plot.width().max(1.0));
        Some(self.x_range.0 + t * (self.x_range.1 - self.x_range.0))
    }

    /// Y bounds over every finite sample and every rule, padded so the extremes
    /// are not drawn on the frame.
    fn y_bounds(&self) -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        let mut consider = |v: f64| {
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        };
        for series in &self.series {
            for &(_, y) in series.points {
                consider(y);
            }
        }
        for rule in &self.rules {
            consider(rule.y);
        }
        if !lo.is_finite() || !hi.is_finite() {
            return (0.0, 1.0);
        }
        let pad = ((hi - lo) * 0.08).max(0.5);
        (lo - pad, hi + pad)
    }
}

fn flush(painter: &egui::Painter, run: &mut Vec<egui::Pos2>, series: &Series<'_>) {
    match series.kind {
        SeriesKind::Line if run.len() >= 2 => {
            painter.add(egui::Shape::line(
                std::mem::take(run),
                Stroke::new(1.8, series.color),
            ));
        }
        _ => {
            for p in run.iter() {
                painter.circle_filled(*p, 2.0, series.color);
            }
            run.clear();
        }
    }
}

fn dashed_line(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2, color: Color32) {
    painter.add(egui::Shape::dashed_line(
        &[from, to],
        Stroke::new(1.0, color.gamma_multiply(0.9)),
        5.0,
        4.0,
    ));
}

/// "Nice" tick positions: a 1/2/5 x 10^n step covering `[lo, hi]` with roughly
/// `target` divisions.
fn ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let span = hi - lo;
    #[allow(clippy::cast_precision_loss)]
    let raw = span / target.max(1) as f64;
    if !raw.is_finite() || raw <= 0.0 {
        return Vec::new();
    }
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalised = raw / magnitude;
    let step = magnitude
        * if normalised <= 1.5 {
            1.0
        } else if normalised <= 3.0 {
            2.0
        } else if normalised <= 7.0 {
            5.0
        } else {
            10.0
        };
    let first = (lo / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = first;
    while v <= hi + 1e-9 && out.len() < 64 {
        out.push(v);
        v += step;
    }
    out
}

fn format_tick(v: f64) -> String {
    if v.abs() >= 100.0 || (v - v.round()).abs() < 1e-9 {
        format!("{v:.0}")
    } else if v.abs() >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_round_numbers_inside_the_range() {
        let t = ticks(0.0, 100.0, 4);
        assert_eq!(t.first().copied(), Some(0.0));
        assert!(t.last().copied().unwrap() <= 100.0);
        assert!(
            t.windows(2).all(|w| (w[1] - w[0] - 20.0).abs() < 1e-9),
            "{t:?}"
        );

        // A range that does not start on a step still produces round values.
        let t = ticks(2.0, 30.0, 6);
        assert!(t.iter().all(|v| (v / 5.0).fract().abs() < 1e-9), "{t:?}");
        assert!(t.iter().all(|v| (2.0..=30.0).contains(v)));

        // Degenerate ranges must not hang or produce garbage.
        assert!(ticks(1.0, 1.0, 4).len() <= 1);
        assert!(ticks(f64::NAN, 1.0, 4).is_empty());
    }
}
