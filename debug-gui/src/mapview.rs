//! walkers map plugin: draws the TX/RX markers and every hop of every visible
//! solution. Drawing only — no engine calls happen here.

use egui::{Align2, Color32, FontId, Pos2, Response, Stroke, Ui, vec2};
use walkers::{MapMemory, Plugin, Projector, lat_lon};

use crate::solve::Solution;

/// Palette for distinguishing solutions; index wraps.
pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0xE6, 0x39, 0x46),
    Color32::from_rgb(0x2A, 0x9D, 0x8F),
    Color32::from_rgb(0xE9, 0xC4, 0x6A),
    Color32::from_rgb(0x8E, 0x7D, 0xBE),
    Color32::from_rgb(0xF4, 0xA2, 0x61),
    Color32::from_rgb(0x4C, 0xC9, 0xF0),
    Color32::from_rgb(0x90, 0xBE, 0x6D),
    Color32::from_rgb(0xFF, 0x6F, 0xB5),
];

pub struct PathPlugin<'a> {
    pub solutions: &'a [Solution],
    pub visible: &'a [bool],
    pub selected: Option<usize>,
    pub tx: (f64, f64),
    pub rx: (f64, f64),
}

/// Project a (lat, lon) to screen space.
fn screen(projector: &Projector, lat: f64, lon: f64) -> Pos2 {
    projector.project(lat_lon(lat, lon)).to_pos2()
}

impl Plugin for PathPlugin<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        _map_memory: &MapMemory,
    ) {
        let painter = ui.painter();

        for (i, sol) in self.solutions.iter().enumerate() {
            if !self.visible.get(i).copied().unwrap_or(true) {
                continue;
            }
            let color = PALETTE[i % PALETTE.len()];
            let width = if self.selected == Some(i) { 3.5 } else { 2.0 };

            for hop in &sol.hop_details {
                // Ground-track polyline. Segments that jump the antimeridian
                // are skipped so the path does not smear across the map.
                for w in hop.polyline.windows(2) {
                    let ((lat1, lon1), (lat2, lon2)) = (w[0], w[1]);
                    if (lon2 - lon1).abs() > 180.0 {
                        continue;
                    }
                    painter.line_segment(
                        [
                            screen(projector, lat1, lon1),
                            screen(projector, lat2, lon2),
                        ],
                        Stroke::new(width, color),
                    );
                }
                // Apex: hollow ring.
                let apex = screen(projector, hop.apex_lat_lon.0, hop.apex_lat_lon.1);
                painter.circle_stroke(apex, 6.0, Stroke::new(2.0, color));
                // Ground reflection / arrival: filled dot.
                let end = screen(projector, hop.end_lat_lon.0, hop.end_lat_lon.1);
                painter.circle_filled(end, 4.5, color);
                painter.circle_stroke(end, 4.5, Stroke::new(1.0, Color32::BLACK));
            }
        }

        // Endpoints drawn last so they sit on top of the paths.
        let tx = screen(projector, self.tx.0, self.tx.1);
        let rx = screen(projector, self.rx.0, self.rx.1);
        painter.circle_filled(tx, 7.0, Color32::from_rgb(0xD0, 0x21, 0x1C));
        painter.circle_stroke(tx, 7.0, Stroke::new(2.0, Color32::WHITE));
        painter.text(
            tx + vec2(11.0, -11.0),
            Align2::LEFT_BOTTOM,
            "TX",
            FontId::proportional(14.0),
            Color32::from_rgb(0xD0, 0x21, 0x1C),
        );
        painter.circle_filled(rx, 7.0, Color32::from_rgb(0x14, 0x65, 0xC0));
        painter.circle_stroke(rx, 7.0, Stroke::new(2.0, Color32::WHITE));
        painter.text(
            rx + vec2(11.0, -11.0),
            Align2::LEFT_BOTTOM,
            "RX",
            FontId::proportional(14.0),
            Color32::from_rgb(0x14, 0x65, 0xC0),
        );
    }
}
