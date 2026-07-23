//! walkers map plugin: draws the TX/RX markers and every hop of every visible
//! solution. Drawing only — no engine calls happen here.

use egui::epaint::{Vertex, WHITE_UV};
use egui::{Align2, Color32, FontId, Mesh, Pos2, Response, Shape, Stroke, Ui, pos2, vec2};
use walkers::{MapMemory, Plugin, Projector, lat_lon};

use crate::solve::Solution;

/// Night-side shading that tracks the sun. Draws a smooth twilight gradient
/// (day clear, night tinted) over the visible map, plus a marker at the
/// subsolar point. Purely a drawing overlay - it reads the subsolar point the
/// app derived from the UTC/date inputs and makes no engine calls, so it moves
/// live as the Time (UTC) control changes.
pub struct TerminatorPlugin {
    /// Solar declination (subsolar latitude), degrees.
    pub decl_deg: f64,
    /// Subsolar longitude (local solar noon meridian), degrees.
    pub sub_lon_deg: f64,
}

/// Width of the twilight ramp, degrees of solar elevation below the horizon
/// (astronomical twilight). Peak night tint alpha (0-255); kept light so the
/// tiles and ray paths stay readable underneath.
const TWILIGHT_DEG: f64 = 18.0;
const NIGHT_ALPHA_MAX: f64 = 90.0;

/// Night tint as a function of solar elevation: 0 in daylight, ramping with a
/// smoothstep across the twilight band to the deep-night maximum.
fn night_alpha(elev_deg: f64) -> u8 {
    let t = ((-elev_deg) / TWILIGHT_DEG).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t); // smoothstep for a soft terminator
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let a = (s * NIGHT_ALPHA_MAX).round() as u8;
    a
}

impl Plugin for TerminatorPlugin {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &Response,
        projector: &Projector,
        _m: &MapMemory,
    ) {
        let painter = ui.painter();
        let rect = response.rect;
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return;
        }
        // Grid fine enough for a smooth gradient (~every 22 px), bounded so an
        // extreme window size can neither alias the terminator nor explode the
        // vertex count.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cols = ((rect.width() / 22.0).ceil() as usize).clamp(8, 96);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = ((rect.height() / 22.0).ceil() as usize).clamp(8, 96);

        let (sin_d, cos_d) = self.decl_deg.to_radians().sin_cos();
        let sub = self.sub_lon_deg.to_radians();

        let mut mesh = Mesh::default();
        for iy in 0..=rows {
            for ix in 0..=cols {
                #[allow(clippy::cast_precision_loss)]
                let px = rect.left() + rect.width() * ix as f32 / cols as f32;
                #[allow(clippy::cast_precision_loss)]
                let py = rect.top() + rect.height() * iy as f32 / rows as f32;
                let geo = projector.unproject(vec2(px, py));
                let (lat, lon) = (geo.y().to_radians(), geo.x().to_radians());
                // Solar elevation: sin(elev) = cos(chi).
                let cos_chi =
                    (lat.sin() * sin_d + lat.cos() * cos_d * (lon - sub).cos()).clamp(-1.0, 1.0);
                let elev = cos_chi.asin().to_degrees();
                let color = Color32::from_rgba_unmultiplied(8, 12, 34, night_alpha(elev));
                mesh.vertices.push(Vertex {
                    pos: pos2(px, py),
                    uv: WHITE_UV,
                    color,
                });
            }
        }
        let stride = (cols + 1) as u32;
        for iy in 0..rows as u32 {
            for ix in 0..cols as u32 {
                let i0 = iy * stride + ix;
                mesh.indices.extend_from_slice(&[
                    i0,
                    i0 + 1,
                    i0 + stride,
                    i0 + 1,
                    i0 + stride + 1,
                    i0 + stride,
                ]);
            }
        }
        painter.add(Shape::mesh(mesh));

        // Subsolar sun marker, when it falls on screen.
        let sun = screen(projector, self.decl_deg, self.sub_lon_deg);
        if rect.contains(sun) {
            painter.circle_filled(sun, 6.0, Color32::from_rgb(0xFF, 0xD1, 0x54));
            painter.circle_stroke(
                sun,
                6.0,
                Stroke::new(1.5, Color32::from_rgb(0xB8, 0x86, 0x00)),
            );
        }
    }
}

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
                        [screen(projector, lat1, lon1), screen(projector, lat2, lon2)],
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
