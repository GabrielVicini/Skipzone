//! walkers map plugin: draws the TX/RX markers and every hop of every visible
//! solution. Drawing only — no engine calls happen here.

use egui::epaint::{Vertex, WHITE_UV};
use egui::{Align2, Color32, FontId, Mesh, Pos2, Response, Shape, Stroke, Ui, pos2, vec2};
use walkers::{MapMemory, Plugin, Projector, lat_lon};

use crate::coastline::Outline;
use crate::coverage::CoverageCell;
use crate::solve::Solution;
use crate::ui::theme;

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
        // The mesh colours the twilight ramp per-vertex and lets the GPU
        // interpolate. When the whole 18 deg band collapses onto about one
        // grid cell - which is what happens zoomed out, where many degrees of
        // longitude fall on each pixel - that linear interpolation shows as
        // hard facets. So size the grid from how much geography a pixel spans:
        // measure degrees of longitude per pixel off the projector (Mercator x
        // is linear in longitude, so this is stable at any latitude), work out
        // how wide the twilight band is on screen, and aim for several cells
        // across it. Zoomed in the band is hundreds of pixels wide, so this
        // relaxes back to a cheap coarse grid.
        let deg_per_px = {
            let mid_y = rect.center().y;
            let a = projector.unproject(vec2(rect.center().x, mid_y));
            let b = projector.unproject(vec2(rect.center().x + 100.0, mid_y));
            let mut d = (b.x() - a.x()).abs();
            if d > 180.0 {
                d = 360.0 - d; // guard the antimeridian wrap
            }
            (d / 100.0).max(1e-9)
        };
        // Pixels spanned by the twilight band, then the cell size that puts
        // ~8 cells across it. Clamped so we never sample finer than 4 px or
        // coarser than 22 px, and the cell count is bounded so an extreme
        // zoom-out on a large window cannot explode the vertex count.
        #[allow(clippy::cast_possible_truncation)]
        let band_px = (TWILIGHT_DEG / deg_per_px) as f32;
        let cell_px = (band_px / 8.0).clamp(4.0, 22.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cols = ((rect.width() / cell_px).ceil() as usize).clamp(8, 200);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rows = ((rect.height() / cell_px).ceil() as usize).clamp(8, 200);

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

/// Debug overlay for coastline auto-detection: the land and lake polygon
/// outlines exactly as the classifier holds them, so a hop's "sea water" or
/// "fresh water" verdict can be checked against the geometry that produced it.
///
/// Deliberately cheap: it draws the pre-thinned rings from
/// [`crate::coastline`] (about a hundred points each), culls anything whose
/// bounding box is off screen, and strokes outlines only - no fills. It is a
/// sanity check, not a basemap; the tiles underneath are the pretty rendering.
pub struct CoastlinePlugin;

/// Colours for the two layers: land outlines and lake outlines. Chosen to read
/// over both the map tiles and the night shading without competing with the
/// per-solution path colours.
const LAND_OUTLINE: Color32 = Color32::from_rgb(0xFF, 0x6D, 0x00);
const LAKE_OUTLINE: Color32 = Color32::from_rgb(0x00, 0xB8, 0xD4);

fn draw_rings(
    painter: &egui::Painter,
    projector: &Projector,
    rect: egui::Rect,
    rings: &[Outline],
    color: Color32,
) {
    let stroke = Stroke::new(1.0, color);
    let mut points: Vec<Pos2> = Vec::new();
    for ring in rings {
        // Cull on the ring's own geographic bounds first: two projections
        // decide whether the other hundred are worth doing, and at any useful
        // zoom nearly every ring in the world fails here.
        let (min_lon, min_lat, max_lon, max_lat) = ring.bounds;
        let a = screen(projector, min_lat, min_lon);
        let b = screen(projector, max_lat, max_lon);
        if !egui::Rect::from_two_pos(a, b).intersects(rect) {
            continue;
        }
        points.clear();
        points.extend(
            ring.points
                .iter()
                .map(|&(lon, lat)| screen(projector, lat, lon)),
        );
        for w in points.windows(2) {
            // Rings clipped at the antimeridian would otherwise draw a chord
            // straight across the map.
            if (w[1].x - w[0].x).abs() > rect.width() * 4.0 {
                continue;
            }
            painter.line_segment([w[0], w[1]], stroke);
        }
    }
}

impl Plugin for CoastlinePlugin {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &Response,
        projector: &Projector,
        _m: &MapMemory,
    ) {
        let Ok(coast) = crate::coastline::get() else {
            return;
        };
        let painter = ui.painter();
        let rect = response.rect;
        draw_rings(
            painter,
            projector,
            rect,
            coast.land_outlines(),
            LAND_OUTLINE,
        );
        draw_rings(
            painter,
            projector,
            rect,
            coast.lake_outlines(),
            LAKE_OUTLINE,
        );
    }
}

/// Area coverage tiles: one filled rectangle per *computed* grid point.
///
/// There is no interpolation, no smoothing and no fill between points. Each
/// rectangle covers exactly the grid cell its solve stands for, so at a coarse
/// resolution the map is visibly blocky - which is the truth about how much was
/// computed. The cure for blockiness is raising the resolution setting, which
/// runs more solves.
///
/// The plugin simply draws whatever cells exist right now, so a run in progress
/// paints itself in progressively as the worker streams results.
pub struct CoveragePlugin<'a> {
    pub cells: &'a [CoverageCell],
    /// 0-255 fill alpha, so the basemap stays readable underneath.
    pub alpha: u8,
}

impl Plugin for CoveragePlugin<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        response: &Response,
        projector: &Projector,
        _m: &MapMemory,
    ) {
        if self.cells.is_empty() {
            return;
        }
        let painter = ui.painter();
        let view = response.rect;
        for cell in self.cells {
            let half = 0.5 * cell.step_deg;
            let (sw, ne) = (
                screen(projector, cell.lat - half, cell.lon - half),
                screen(projector, cell.lat + half, cell.lon + half),
            );
            // A cell straddling the antimeridian projects to a rectangle the
            // width of the world; skip it rather than smear it across the map.
            if (ne.x - sw.x).abs() > view.width() {
                continue;
            }
            let rect = egui::Rect::from_two_pos(sw, ne);
            if !rect.intersects(view) {
                continue;
            }
            // Three answers, three hues. A deterministic path takes the SNR
            // ramp at full saturation; a position only sporadic E can reach
            // takes the Es hue, faded by how likely that opening is; nothing at
            // all stays grey. Painting the second as the third is what made the
            // dead zone look hard when it was not.
            let base = if !cell.found_path() {
                theme::COVERAGE_NO_PATH
            } else if cell.es_only() {
                theme::coverage_es_color(cell.snr_db, cell.probability)
            } else {
                theme::coverage_color(cell.snr_db)
            };
            // Expand by half a pixel: adjacent cells then meet exactly, so the
            // grid reads as tiles rather than as a dotted lattice with seams.
            // This changes no value - only whether neighbours touch.
            painter.rect_filled(
                rect.expand(0.5),
                0.0,
                base.gamma_multiply(f32::from(self.alpha) / 255.0),
            );
        }
    }
}

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
            let color = theme::solution_color(i);
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
