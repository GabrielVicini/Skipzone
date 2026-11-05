//! The map itself: the full-screen centrepiece of the interface.
//!
//! The camera lives in [`crate::state::MapState`] and the rasterised basemap in
//! [`crate::ui::scratch::ViewScratch`]; everything else is read from the
//! state. Pan, zoom and fit come back as [`Action::SetCamera`], and a click as
//! [`Action::PlaceStation`], so the "which station does a click move?" rule
//! lives in the update rather than in the drawing code.
//!
//! Nothing here is fetched. The basemap is built once from the bundled coastline
//! polygons (see [`super::basemap`]) and everything else is painted from values
//! already in memory, so a frame never depends on a network round trip and the
//! map is never blank waiting for one.

use egui::{CentralPanel, Color32, Frame, Mesh, Rect, Sense, TextureHandle, Ui, pos2};

use crate::state::AppState;
use crate::ui::actions::Action;
use crate::ui::scratch::ViewScratch;
use crate::ui::theme;
use skipzone_iono::solar;
use skipzone_predict::scenario::Inputs;

use super::basemap;
use super::layers;
use super::projection::{Camera, Proj};

/// Multiplier per press of the zoom buttons, and per notch of the scroll wheel.
pub const ZOOM_STEP: f64 = 1.6;
/// Fraction of the viewport the fitted path is allowed to fill, leaving room for
/// the floating controls in the corners.
const FIT_MARGIN: f64 = 0.62;

/// Draw the map filling the central panel. Pushes the camera this frame
/// settled on when it moved, and a click as [`Action::PlaceStation`].
pub fn draw(ui: &mut Ui, state: &AppState, scratch: &ViewScratch, out: &mut Vec<Action>) {
    CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if rect.width() < 2.0 || rect.height() < 2.0 {
            return;
        }

        let map = &state.ui.map;
        // Hold the fit until there is a real viewport to fit into. A window
        // that is minimised, or in its first frames, reports a rect small
        // enough that the computed scale is meaningless, and fitting against
        // it leaves the map zoomed somewhere arbitrary once the window opens
        // for real.
        let fitted = map.needs_fit && rect.width() > 200.0 && rect.height() > 200.0;
        let mut camera = if fitted {
            fit(&state.inputs.scenario, rect)
        } else {
            map.camera
        };
        camera = apply_input(camera, map.pending_zoom, &response, rect, ui).clamped(rect);
        // Painted with the new camera now; the state catches up after the
        // frame. A still map emits nothing.
        if fitted || camera != map.camera || map.pending_zoom != 1.0 {
            out.push(Action::SetCamera { camera, fitted });
        }

        let proj = Proj::new(rect, camera);
        paint(ui, &proj, state, scratch.basemap.as_ref());

        if !response.clicked() {
            return;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        let (lat, lon) = proj.unproject(pointer);
        // A click above the north pole or below the south one is not a click
        // on the map, and must not move a station. Longitude cannot miss: it
        // wraps, so every column of pixels inside the world is a real
        // meridian.
        if lat.is_finite() && lat.abs() <= 90.0 {
            out.push(Action::PlaceStation { lat, lon });
        }
    });
}

/// Pan by dragging, zoom by wheel or pinch about the pointer, plus whatever
/// the zoom buttons asked for since the last frame.
fn apply_input(
    camera: Camera,
    pending_zoom: f64,
    response: &egui::Response,
    rect: Rect,
    ui: &Ui,
) -> Camera {
    let mut camera = camera;
    if response.dragged() {
        camera = Proj::new(rect, camera).panned(response.drag_delta());
    }

    let (scroll, pinch, pointer) = ui.input(|i| {
        (
            i.smooth_scroll_delta.y,
            i.zoom_delta(),
            i.pointer.hover_pos(),
        )
    });
    let anchor = pointer.unwrap_or_else(|| rect.center());
    let mut factor = f64::from(pinch);
    if response.hovered() && scroll.abs() > 0.0 {
        factor *= ZOOM_STEP.powf(f64::from(scroll) / 120.0);
    }
    factor *= pending_zoom;
    if (factor - 1.0).abs() > 1e-6 {
        camera = Proj::new(rect, camera).zoomed_about(factor, anchor);
    }
    camera
}

/// One frame of map, bottom to top. The order is the argument: shading under
/// the answers, answers under the geography that locates them, stations last.
fn paint(ui: &Ui, proj: &Proj, state: &AppState, basemap: Option<&TextureHandle>) {
    let painter = ui.painter_at(proj.rect());
    let inputs = &state.inputs.scenario;
    let display = state.inputs.display;

    // The void first, so the world is drawn onto a surface that is visibly
    // not-the-world rather than onto whatever was there before.
    painter.rect_filled(proj.rect(), 0.0, theme::MAP_VOID);

    let world = proj.world_rect();
    if let Some(tex) = basemap {
        // One quad for the whole world. Its left edge is at the camera's
        // longitude minus 180, which is a different meridian every time the
        // map is panned, so the texture coordinate is the one for that
        // meridian and is allowed to run past 1 and wrap. A fixed 0..1 would
        // misalign the raster with the vectors.
        let u0 = basemap::u_of_lon(proj.camera().lon - 180.0);
        let mut mesh = Mesh::with_texture(tex.id());
        mesh.add_rect_with_uv(
            world,
            Rect::from_min_max(pos2(u0, 0.0), pos2(u0 + 1.0, 1.0)),
            Color32::WHITE,
        );
        painter.add(mesh);
    } else {
        painter.rect_filled(world, 0.0, theme::MAP_OCEAN);
    }
    layers::coastline(&painter, proj, false);
    layers::graticule(&painter, proj);

    // Terminator shading sits under the ray paths. It reads the subsolar
    // point derived from the current UTC/date, so it tracks the time control
    // live without a re-solve.
    if display.show_terminator {
        let (decl_deg, sub_lon_deg) =
            solar::subsolar_point(inputs.month, inputs.day_of_month, inputs.utc_hours);
        layers::terminator(&painter, proj, decl_deg, sub_lon_deg);
    }

    // Coverage tiles sit above the night shading (they are the answer being
    // read) but below the coastline overlay and the ray paths, which have to
    // stay legible on top of them.
    layers::coverage(
        &painter,
        proj,
        &state.results.coverage.cells,
        display.coverage_alpha,
    );

    // The coastline debug overlay is redrawn in its own colours above the
    // tiles: it is there to be compared against the reflection dots, so the
    // dots must stay on top of it and it must stay on top of the fill.
    if display.show_coastlines {
        layers::coastline(&painter, proj, true);
    }

    layers::paths(
        &painter,
        proj,
        state.results.main.solutions(),
        &state.ui.selection.visible,
        state.ui.selection.selected,
        (inputs.tx_lat, inputs.tx_lon),
        (inputs.rx_lat, inputs.rx_lon),
    );
}

/// The camera that frames the whole TX-RX path in `rect`, with margin.
///
/// Both spans are measured the short way round the world, so a path over the
/// Pacific frames across the antimeridian instead of zooming out to show the
/// long way. The tighter of the two axes wins.
fn fit(inputs: &Inputs, rect: Rect) -> Camera {
    let dlon = super::projection::wrap_lon(inputs.rx_lon - inputs.tx_lon);
    let centre_lon = super::projection::wrap_lon(inputs.tx_lon + 0.5 * dlon);
    let centre_lat = f64::midpoint(inputs.tx_lat, inputs.rx_lat);

    let span_lon = dlon.abs().max(0.5);
    let span_lat = (inputs.tx_lat - inputs.rx_lat).abs().max(0.5);
    let ppd_lon = f64::from(rect.width()) * FIT_MARGIN / span_lon;
    let ppd_lat = f64::from(rect.height()) * FIT_MARGIN / span_lat;

    Camera {
        lat: centre_lat,
        lon: centre_lon,
        ppd: ppd_lon.min(ppd_lat),
    }
    .clamped(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))
    }

    /// A fitted view must contain both stations, checked on a path that
    /// crosses the antimeridian so it must not frame the long way round.
    #[test]
    fn fit_frames_both_stations_across_the_antimeridian() {
        let inputs = Inputs {
            tx_lat: 35.0,
            tx_lon: 139.0, // Tokyo
            rx_lat: 37.0,
            rx_lon: -122.0, // San Francisco
            ..Inputs::default()
        };

        let cam = fit(&inputs, rect());
        let proj = Proj::new(rect(), cam);
        for (lat, lon) in [(35.0, 139.0), (37.0, -122.0)] {
            let p = proj.project(lat, lon);
            assert!(rect().contains(p), "({lat}, {lon}) projected to {p:?}");
        }
    }

    /// Fitting a short path must not zoom past the hard limit, and fitting a
    /// world-spanning one must not zoom out past the whole world.
    #[test]
    fn fit_stays_inside_the_camera_limits() {
        let mut inputs = Inputs {
            tx_lat: 51.5,
            tx_lon: -0.13,
            rx_lat: 51.6,
            rx_lon: -0.12,
            ..Inputs::default()
        };
        let cam = fit(&inputs, rect());
        assert!(cam.ppd >= Camera::min_ppd(rect()));

        inputs.rx_lat = -33.9;
        inputs.rx_lon = 151.2;
        let cam = fit(&inputs, rect());
        assert!(cam.ppd >= Camera::min_ppd(rect()));
        let world = Proj::new(rect(), cam).world_rect();
        assert!(world.width() >= rect().width() - 0.5);
    }
}
