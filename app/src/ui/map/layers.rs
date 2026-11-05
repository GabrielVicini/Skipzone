//! Everything drawn on top of the basemap, in the order it is drawn.
//!
//! Plain functions over a painter and a [`Proj`]; none of them call the engine.
//!
//! # Clipping
//!
//! Every layer clips to [`Proj::world_rect`], so nothing is painted beyond the
//! poles. Longitude is not clipped but wraps: each layer draws the world at
//! -360, 0 and +360 degrees so a feature straddling the antimeridian is drawn
//! whole.

use egui::epaint::{Vertex, WHITE_UV};
use egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::coverage::{CoverageCell, LAT_LIMIT_DEG};
use crate::solve::Solution;
use crate::ui::theme;

use super::basemap;
use super::projection::{Proj, unwrap_run};

/// The three copies of the world a layer must consider. The view can never be
/// wider than one world (the camera forbids it), so a feature is visible in at
/// most two of these and the rest are culled on their bounding box.
const WORLD_SHIFTS: [f64; 3] = [-360.0, 0.0, 360.0];

// Graticule

const GRATICULE: Color32 = Color32::from_rgba_premultiplied(0x3A, 0x46, 0x50, 0x90);
const GRATICULE_MAJOR: Color32 = Color32::from_rgba_premultiplied(0x54, 0x64, 0x72, 0xB0);
const GRATICULE_LABEL: Color32 = Color32::from_rgb(0x7A, 0x8A, 0x96);

/// Candidate graticule spacings, coarsest first. The first one that puts its
/// lines at least [`MIN_GRID_PX`] apart on screen wins, so the lattice thins out
/// as you zoom out instead of turning into a solid block.
const GRID_STEPS: [f64; 8] = [60.0, 30.0, 15.0, 10.0, 5.0, 2.0, 1.0, 0.5];
const MIN_GRID_PX: f64 = 70.0;

/// The lat/lon lattice, with degree labels down the left edge and along the top.
///
/// Under this projection the graticule is a square grid at every latitude, so
/// it gives the eye a ruler for a path's shape.
pub fn graticule(painter: &Painter, proj: &Proj) {
    let step = GRID_STEPS
        .iter()
        .copied()
        .find(|s| s * proj.pixels_per_degree() >= MIN_GRID_PX)
        .unwrap_or(0.5);

    let world = proj.world_rect();
    let view = proj.rect().intersect(world);
    if !view.is_positive() {
        return;
    }
    let (south, north, west, east) = proj.visible_bounds();

    // Parallels.
    let mut lat = (south / step).ceil() * step;
    while lat <= north + 1e-9 {
        if lat.abs() <= 90.0 {
            let y = proj.project(lat, proj.camera().lon).y;
            let color = if lat.abs() < 1e-9 {
                GRATICULE_MAJOR
            } else {
                GRATICULE
            };
            painter.line_segment(
                [pos2(view.left(), y), pos2(view.right(), y)],
                Stroke::new(1.0, color),
            );
            painter.text(
                pos2(view.left() + 4.0, y - 2.0),
                Align2::LEFT_BOTTOM,
                format_lat(lat),
                FontId::monospace(10.0),
                GRATICULE_LABEL,
            );
        }
        lat += step;
    }

    // Meridians. Walked on the unwrapped scale so the run crosses the
    // antimeridian without special-casing it.
    let mut lon = (west / step).ceil() * step;
    while lon <= east + 1e-9 {
        let x = proj.project_unwrapped(0.0, lon).x;
        let wrapped = super::projection::wrap_lon(lon);
        let color = if wrapped.abs() < 1e-9 || (wrapped.abs() - 180.0).abs() < 1e-9 {
            GRATICULE_MAJOR
        } else {
            GRATICULE
        };
        painter.line_segment(
            [pos2(x, view.top()), pos2(x, view.bottom())],
            Stroke::new(1.0, color),
        );
        painter.text(
            pos2(x + 3.0, view.top() + 2.0),
            Align2::LEFT_TOP,
            format_lon(wrapped),
            FontId::monospace(10.0),
            GRATICULE_LABEL,
        );
        lon += step;
    }
}

fn format_lat(lat: f64) -> String {
    match lat.partial_cmp(&0.0) {
        Some(std::cmp::Ordering::Greater) => format!("{lat:.0}N"),
        Some(std::cmp::Ordering::Less) => format!("{:.0}S", -lat),
        _ => "0".to_string(),
    }
}

fn format_lon(lon: f64) -> String {
    match lon.partial_cmp(&0.0) {
        Some(std::cmp::Ordering::Greater) => format!("{lon:.0}E"),
        Some(std::cmp::Ordering::Less) => format!("{:.0}W", -lon),
        _ => "0".to_string(),
    }
}

// Day/night

/// Width of the twilight ramp, degrees of solar elevation below the horizon
/// (astronomical twilight). Peak night tint alpha (0-255); kept light so the
/// basemap and ray paths stay readable underneath.
const TWILIGHT_DEG: f64 = 18.0;
const NIGHT_ALPHA_MAX: f64 = 110.0;

/// Night tint as a function of solar elevation: 0 in daylight, ramping with a
/// smoothstep across the twilight band to the deep-night maximum.
fn night_alpha(elev_deg: f64) -> u8 {
    let t = ((-elev_deg) / TWILIGHT_DEG).clamp(0.0, 1.0);
    let s = t * t * (3.0 - 2.0 * t); // smoothstep for a soft terminator
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let a = (s * NIGHT_ALPHA_MAX).round() as u8;
    a
}

/// Night-side shading that tracks the sun, plus a marker at the subsolar point.
///
/// Purely a drawing overlay: it reads the subsolar point the app derived from
/// the UTC/date inputs and makes no engine calls, so it moves live as the time
/// control changes.
///
/// The mesh is built only over the intersection of the viewport with the
/// world, so no shading is drawn at latitudes beyond the poles.
pub fn terminator(painter: &Painter, proj: &Proj, decl_deg: f64, sub_lon_deg: f64) {
    let area = proj.rect().intersect(proj.world_rect());
    if area.width() < 1.0 || area.height() < 1.0 {
        return;
    }

    // Size the grid from how much geography a pixel spans, so the twilight ramp
    // never collapses onto one cell and show as hard facets. Zoomed in the band
    // is hundreds of pixels wide and this relaxes back to a coarse grid.
    let deg_per_px = 1.0 / proj.pixels_per_degree();
    #[allow(clippy::cast_possible_truncation)]
    let band_px = (TWILIGHT_DEG / deg_per_px) as f32;
    let cell_px = (band_px / 8.0).clamp(4.0, 22.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cols = ((area.width() / cell_px).ceil() as usize).clamp(8, 200);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rows = ((area.height() / cell_px).ceil() as usize).clamp(8, 200);

    let (sin_d, cos_d) = decl_deg.to_radians().sin_cos();
    let sub = sub_lon_deg.to_radians();

    let mut mesh = Mesh::default();
    for iy in 0..=rows {
        for ix in 0..=cols {
            #[allow(clippy::cast_precision_loss)]
            let px = area.left() + area.width() * ix as f32 / cols as f32;
            #[allow(clippy::cast_precision_loss)]
            let py = area.top() + area.height() * iy as f32 / rows as f32;
            let (lat_deg, lon_deg) = proj.unproject(pos2(px, py));
            let (lat, lon) = (
                lat_deg.clamp(-90.0, 90.0).to_radians(),
                lon_deg.to_radians(),
            );
            // Solar elevation: sin(elev) = cos(chi).
            let cos_chi =
                (lat.sin() * sin_d + lat.cos() * cos_d * (lon - sub).cos()).clamp(-1.0, 1.0);
            let elev = cos_chi.asin().to_degrees();
            let color = Color32::from_rgba_unmultiplied(4, 8, 22, night_alpha(elev));
            mesh.vertices.push(Vertex {
                pos: pos2(px, py),
                uv: WHITE_UV,
                color,
            });
        }
    }
    let stride = u32::try_from(cols + 1).unwrap_or(u32::MAX);
    for iy in 0..u32::try_from(rows).unwrap_or(0) {
        for ix in 0..u32::try_from(cols).unwrap_or(0) {
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
    let sun = proj.project(decl_deg, sub_lon_deg);
    if area.contains(sun) {
        painter.circle_filled(sun, 6.0, Color32::from_rgb(0xFF, 0xD1, 0x54));
        painter.circle_stroke(
            sun,
            6.0,
            Stroke::new(1.5, Color32::from_rgb(0xB8, 0x86, 0x00)),
        );
    }
}

// Coastline

/// Colours for the debug overlay's two layers: land outlines and lake outlines.
/// Chosen to read over both the basemap and the night shading without competing
/// with the per-solution path colours.
const LAND_OUTLINE: Color32 = Color32::from_rgb(0xFF, 0x6D, 0x00);
const LAKE_OUTLINE: Color32 = Color32::from_rgb(0x00, 0xB8, 0xD4);

/// Smallest on-screen extent, in pixels, a ring is worth stroking at.
const MIN_RING_PX: f64 = 2.5;

/// Smallest step, in pixels, between two kept vertices of a stroked ring.
/// Anything finer than this cannot be seen and is dropped before tessellation.
const MIN_SEGMENT_PX: f32 = 1.5;

/// The shoreline, stroked from the full-detail rings over the raster basemap.
///
/// Two jobs in one pass. Normally it is the quiet [`basemap::COAST`] hairline
/// that keeps the coast crisp when the raster has been magnified past its
/// resolution. With `debug` set it becomes the coastline auto-detection overlay:
/// the land and lake rings in their own colours, so a hop's "sea water" or
/// "fresh water" verdict can be checked against the geometry that produced it.
pub fn coastline(painter: &Painter, proj: &Proj, debug: bool) {
    let Ok(coast) = crate::coastline::get() else {
        return;
    };
    let (land_color, lake_color, width) = if debug {
        (LAND_OUTLINE, LAKE_OUTLINE, 1.0)
    } else {
        (basemap::COAST, basemap::COAST, 1.0)
    };
    let view = proj.rect().intersect(proj.world_rect());
    draw_rings(
        painter,
        proj,
        view,
        coast.land_outlines(),
        land_color,
        width,
    );
    draw_rings(
        painter,
        proj,
        view,
        coast.lake_outlines(),
        lake_color,
        width,
    );
}

fn draw_rings(
    painter: &Painter,
    proj: &Proj,
    view: Rect,
    shapes: &[crate::coastline::OutlineShape],
    color: Color32,
    width: f32,
) {
    let stroke = Stroke::new(width, color);
    let ppd = proj.pixels_per_degree();
    // Strokes on the very edge still have to be drawn, so cull against a
    // slightly larger rectangle than the one that will clip them.
    let pad = view.expand(4.0);

    let mut lons: Vec<f64> = Vec::new();
    let mut points: Vec<Pos2> = Vec::new();
    let mut run: Vec<Pos2> = Vec::new();

    for ring in shapes.iter().flatten() {
        let (min_lon, min_lat, max_lon, max_lat) = ring.bounds;
        // A ring smaller than a couple of pixels is not a shoreline, it is a
        // speck - and there are on the order of a thousand of them in the data.
        // The raster underneath has already drawn whatever of it survives at
        // this scale, so stroking it adds nothing but cost and visual noise.
        let span_px = (max_lon - min_lon).max(max_lat - min_lat) * ppd;
        if span_px < MIN_RING_PX {
            continue;
        }

        // The ring's longitude span on the continuous scale near the camera
        // (see `Proj::span_near_camera`). Unwrapping the two edges separately
        // would collapse a full-width ring such as Africa-Eurasia, whose bounds
        // run -180 to 180, to zero width and cull it.
        let (west, east) = proj.span_near_camera(min_lon, max_lon);
        let bbox = Rect::from_two_pos(
            proj.project_unwrapped(min_lat, west),
            proj.project_unwrapped(max_lat, east),
        );

        // Project once and reuse: a world copy is the same points shifted along
        // x, so the second and third copies cost an addition rather than a
        // reprojection.
        let mut projected = false;
        for shift in WORLD_SHIFTS {
            #[allow(clippy::cast_possible_truncation)]
            let offset = vec2((shift * ppd) as f32, 0.0);
            if !bbox.translate(offset).intersects(pad) {
                continue;
            }
            if !projected {
                projected = true;
                unwrap_run(ring.points.iter().map(|&(lon, _)| lon), &mut lons);
                points.clear();
                for (&(_, lat), &lon) in ring.points.iter().zip(&lons) {
                    let p = proj.project_unwrapped(lat, lon);
                    // Level of detail: a vertex that lands on the same pixel as
                    // the last one kept cannot change the picture, and a
                    // continent zoomed out to the whole world has thousands of
                    // them. This caps the work at the screen's resolution
                    // instead of the data's.
                    let keep = points.last().is_none_or(|q: &Pos2| {
                        (q.x - p.x).abs() + (q.y - p.y).abs() >= MIN_SEGMENT_PX
                    });
                    if keep {
                        points.push(p);
                    }
                }
            }
            if points.len() < 2 {
                break;
            }
            stroke_clipped(painter, pad, &points, offset, stroke, &mut run);
        }
    }
}

/// Stroke a polyline, dropping the parts of it that are off screen.
///
/// A continent's ring is thousands of segments long and, zoomed in, three of
/// them are visible. Handing the whole ring to the tessellator makes it walk and
/// feather every one of those segments each frame. This walks the ring once,
/// cheaply and hands over only the visible runs, which keeps the map
/// responsive.
fn stroke_clipped(
    painter: &Painter,
    view: Rect,
    points: &[Pos2],
    offset: egui::Vec2,
    stroke: Stroke,
    run: &mut Vec<Pos2>,
) {
    run.clear();
    for w in points.windows(2) {
        let (a, b) = (w[0] + offset, w[1] + offset);
        if Rect::from_two_pos(a, b).intersects(view) {
            if run.is_empty() {
                run.push(a);
            }
            run.push(b);
        } else if run.len() >= 2 {
            painter.add(Shape::line(std::mem::take(run), stroke));
        } else {
            run.clear();
        }
    }
    if run.len() >= 2 {
        painter.add(Shape::line(std::mem::take(run), stroke));
    }
}

// Area coverage

/// Area coverage tiles: one filled rectangle per *computed* grid point.
///
/// No interpolation or smoothing: each rectangle covers the grid cell its solve
/// stands for, so a coarse resolution looks blocky.
///
/// Under this projection a cell is the same size everywhere. It draws whole
/// even across the antimeridian, because it is offered at all three world
/// positions.
pub fn coverage(painter: &Painter, proj: &Proj, cells: &[CoverageCell], alpha: u8) {
    if cells.is_empty() {
        return;
    }
    let view = proj.rect().intersect(proj.world_rect());
    if !view.is_positive() {
        return;
    }
    let cam_lon = proj.camera().lon;
    for cell in cells {
        let half = 0.5 * cell.step_deg;
        // A row is only computed if its centre is inside the latitude limit, so
        // the outermost rows still reach half a step past it. Clamp the drawn
        // edge to the pole: the tile was computed and is shown, but not past
        // the world.
        let (lat_s, lat_n) = (
            (cell.lat - half).max(-LAT_LIMIT_DEG),
            (cell.lat + half).min(LAT_LIMIT_DEG),
        );
        if lat_n <= lat_s {
            continue;
        }
        // Three answers, three hues. A deterministic path takes the SNR ramp at
        // full saturation; a position only sporadic E can reach takes the Es
        // hue, faded by how likely that opening is; nothing at all stays grey.
        let base = if cell.found_path() {
            if cell.es_only() {
                theme::coverage_es_color(cell.snr_db, cell.probability)
            } else {
                theme::coverage_color(cell.snr_db)
            }
        } else {
            theme::COVERAGE_NO_PATH
        };
        let color = base.gamma_multiply(f32::from(alpha) / 255.0);

        // The cell's longitude on the same continuous scale as the camera, so
        // its two edges stay on the same side of each other.
        let centre = cam_lon + super::projection::wrap_lon(cell.lon - cam_lon);
        for shift in WORLD_SHIFTS {
            let rect = Rect::from_two_pos(
                proj.project_unwrapped(lat_s, centre - half + shift),
                proj.project_unwrapped(lat_n, centre + half + shift),
            );
            if !rect.intersects(view) {
                continue;
            }
            // Expanded by half a pixel so adjacent cells meet exactly and the
            // grid reads as tiles rather than as a dotted lattice with seams.
            // That changes no value - only whether neighbours touch.
            painter.rect_filled(rect.expand(0.5).intersect(view), 0.0, color);
        }
    }
}

// Ray paths

/// Every hop of every visible solution, then the two stations on top.
pub fn paths(
    painter: &Painter,
    proj: &Proj,
    solutions: &[Solution],
    visible: &[bool],
    selected: Option<usize>,
    tx: (f64, f64),
    rx: (f64, f64),
) {
    let view = proj.rect().intersect(proj.world_rect());
    let pad = view.expand(4.0);
    let ppd = proj.pixels_per_degree();
    let mut lons: Vec<f64> = Vec::new();
    let mut points: Vec<Pos2> = Vec::new();
    let mut run: Vec<Pos2> = Vec::new();

    for (i, sol) in solutions.iter().enumerate() {
        if !visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        let color = theme::solution_color(i);
        let width = if selected == Some(i) { 3.5 } else { 2.0 };

        for hop in &sol.hop_details {
            // Ground-track polyline. Unwrapped first, so a hop running east past
            // the antimeridian carries on east instead of being cut - the old
            // code simply dropped any segment that jumped, leaving a gap in the
            // path exactly where it crossed the date line.
            unwrap_run(hop.polyline.iter().map(|&(_, lon)| lon), &mut lons);
            points.clear();
            points.extend(
                hop.polyline
                    .iter()
                    .zip(&lons)
                    .map(|(&(lat, _), &lon)| proj.project_unwrapped(lat, lon)),
            );
            if points.len() < 2 {
                continue;
            }
            let bbox = Rect::from_points(&points);
            for shift in WORLD_SHIFTS {
                #[allow(clippy::cast_possible_truncation)]
                let offset = vec2((shift * ppd) as f32, 0.0);
                if !bbox.translate(offset).intersects(pad) {
                    continue;
                }
                stroke_clipped(
                    painter,
                    pad,
                    &points,
                    offset,
                    Stroke::new(width, color),
                    &mut run,
                );
            }
            // Apex: hollow ring. Ground reflection / arrival: filled dot.
            let apex = proj.project(hop.apex_lat_lon.0, hop.apex_lat_lon.1);
            painter.circle_stroke(apex, 6.0, Stroke::new(2.0, color));
            let end = proj.project(hop.end_lat_lon.0, hop.end_lat_lon.1);
            painter.circle_filled(end, 4.5, color);
            painter.circle_stroke(end, 4.5, Stroke::new(1.0, Color32::BLACK));
        }
    }

    station(painter, proj, tx, "TX", Color32::from_rgb(0xD0, 0x21, 0x1C));
    station(painter, proj, rx, "RX", Color32::from_rgb(0x14, 0x65, 0xC0));
}

fn station(painter: &Painter, proj: &Proj, at: (f64, f64), label: &str, color: Color32) {
    let p = proj.project(at.0, at.1);
    painter.circle_filled(p, 7.0, color);
    painter.circle_stroke(p, 7.0, Stroke::new(2.0, Color32::WHITE));
    painter.text(
        p + vec2(11.0, -11.0),
        Align2::LEFT_BOTTOM,
        label,
        FontId::proportional(14.0),
        color,
    );
}
