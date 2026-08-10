//! The map itself: the full-screen centrepiece of the interface.
//!
//! [`MapView`] owns only what must persist between frames - the camera and the
//! rasterised basemap - and reads everything else from the session. It reports a
//! click back as a position and lets the caller decide what it means, so the
//! "which station does a click move?" rule lives with the rest of the UI state
//! rather than in the drawing code.
//!
//! Nothing here is fetched. The basemap is built once from the bundled coastline
//! polygons (see [`super::basemap`]) and everything else is painted from values
//! already in memory, so a frame never depends on a network round trip and the
//! map is never blank waiting for one.

use egui::{CentralPanel, Color32, Frame, Mesh, Rect, Sense, TextureHandle, Ui, pos2};

use crate::solar;
use crate::state::{Session, UiState};

use super::basemap;
use super::layers;
use super::projection::{Camera, Proj};

/// Multiplier per press of the zoom buttons, and per notch of the scroll wheel.
const ZOOM_STEP: f64 = 1.6;
/// Fraction of the viewport the fitted path is allowed to fill, leaving room for
/// the floating controls in the corners.
const FIT_MARGIN: f64 = 0.62;

pub struct MapView {
    camera: Camera,
    /// The rasterised world, built once. `None` only if the coastline data could
    /// not be read, in which case the map draws as open ocean rather than
    /// failing - the geometry is still correct without land under it.
    basemap: Option<TextureHandle>,
    /// Re-frame the path on the next draw (startup, or after "Fit path").
    needs_fit: bool,
    /// Pending scale change from the zoom buttons, applied once the viewport
    /// size is known so it can be clamped against it.
    pending_zoom: f64,
}

impl MapView {
    #[must_use]
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            camera: Camera::default(),
            basemap: basemap::load(ctx),
            needs_fit: true,
            pending_zoom: 1.0,
        }
    }

    pub fn request_fit(&mut self) {
        self.needs_fit = true;
    }

    pub fn zoom_in(&mut self) {
        self.pending_zoom *= ZOOM_STEP;
    }

    pub fn zoom_out(&mut self) {
        self.pending_zoom /= ZOOM_STEP;
    }

    #[must_use]
    pub fn zoom(&self) -> f64 {
        self.camera.zoom_level()
    }

    /// Draw the map filling the central panel. Returns the `(lat, lon)` of a
    /// click on it, if there was one.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        ui_state: &UiState,
    ) -> Option<(f64, f64)> {
        CentralPanel::default()
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                if rect.width() < 2.0 || rect.height() < 2.0 {
                    return None;
                }

                // Hold the fit until there is a real viewport to fit into. A
                // window that is minimised, or in its first frames, reports a
                // rect small enough that the computed scale is meaningless, and
                // fitting against it leaves the map zoomed somewhere arbitrary
                // once the window opens for real.
                if self.needs_fit && rect.width() > 200.0 && rect.height() > 200.0 {
                    self.camera = fit(session, rect);
                    self.needs_fit = false;
                }
                self.apply_input(&response, rect, ui);
                self.camera = self.camera.clamped(rect);

                let proj = Proj::new(rect, self.camera);
                self.paint(ui, &proj, session, ui_state);

                if !response.clicked() {
                    return None;
                }
                let pointer = response.interact_pointer_pos()?;
                let (lat, lon) = proj.unproject(pointer);
                // A click above the north pole or below the south one is not a
                // click on the map, and must not move a station. Longitude
                // cannot miss: it wraps, so every column of pixels inside the
                // world is a real meridian.
                (lat.is_finite() && lat.abs() <= 90.0).then_some((lat, lon))
            })
            .inner
    }

    /// Pan by dragging, zoom by wheel or pinch about the pointer, plus whatever
    /// the zoom buttons asked for since the last frame.
    fn apply_input(&mut self, response: &egui::Response, rect: Rect, ui: &Ui) {
        let proj = Proj::new(rect, self.camera);

        if response.dragged() {
            self.camera = proj.panned(response.drag_delta());
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
        factor *= self.pending_zoom;
        self.pending_zoom = 1.0;
        if (factor - 1.0).abs() > 1e-6 {
            let proj = Proj::new(rect, self.camera);
            self.camera = proj.zoomed_about(factor, anchor);
        }
    }

    /// One frame of map, bottom to top. The order is the argument: shading under
    /// the answers, answers under the geography that locates them, stations last.
    fn paint(&self, ui: &Ui, proj: &Proj, session: &Session, ui_state: &UiState) {
        let painter = ui.painter_at(proj.rect());
        let inputs = &session.inputs;

        // The void first, so the world is drawn onto a surface that is visibly
        // not-the-world rather than onto whatever was there before.
        painter.rect_filled(proj.rect(), 0.0, basemap::VOID);

        let world = proj.world_rect();
        if let Some(tex) = &self.basemap {
            // One quad for the whole world. Its left edge is at the camera's
            // longitude minus 180, which is a different meridian every time the
            // map is panned - so the texture coordinate has to be the one for
            // *that* meridian, and is then allowed to run past 1 and wrap. `u`
            // fixed at 0..1 is what slid the raster out from under the vectors.
            let u0 = basemap::u_of_lon(proj.camera().lon - 180.0);
            let mut mesh = Mesh::with_texture(tex.id());
            mesh.add_rect_with_uv(
                world,
                Rect::from_min_max(pos2(u0, 0.0), pos2(u0 + 1.0, 1.0)),
                Color32::WHITE,
            );
            painter.add(mesh);
        } else {
            painter.rect_filled(world, 0.0, basemap::OCEAN);
        }
        layers::coastline(&painter, proj, false);
        layers::graticule(&painter, proj);

        // Terminator shading sits under the ray paths. It reads the subsolar
        // point derived from the current UTC/date, so it tracks the time control
        // live without a re-solve.
        if ui_state.show_terminator {
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
            &session.coverage.cells,
            ui_state.coverage_alpha,
        );

        // The coastline debug overlay is redrawn in its own colours above the
        // tiles: it is there to be compared against the reflection dots, so the
        // dots must stay on top of it and it must stay on top of the fill.
        if ui_state.show_coastlines {
            layers::coastline(&painter, proj, true);
        }

        layers::paths(
            &painter,
            proj,
            session.solve.solutions(),
            &session.solve.visible,
            session.solve.selected,
            (inputs.tx_lat, inputs.tx_lon),
            (inputs.rx_lat, inputs.rx_lon),
        );
    }
}

/// The camera that frames the whole TX-RX path in `rect`, with margin.
///
/// Both spans are measured the short way round the world, so a path over the
/// Pacific frames across the antimeridian instead of zooming out to show the
/// long way. The tighter of the two axes wins.
fn fit(session: &Session, rect: Rect) -> Camera {
    let inputs = &session.inputs;
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

    /// A fitted view must contain both stations. Checked on a path that crosses
    /// the antimeridian, which is the case the old Mercator fit got wrong by
    /// framing the long way round.
    #[test]
    fn fit_frames_both_stations_across_the_antimeridian() {
        let mut session = Session::new(egui::Context::default());
        session.inputs.tx_lat = 35.0;
        session.inputs.tx_lon = 139.0; // Tokyo
        session.inputs.rx_lat = 37.0;
        session.inputs.rx_lon = -122.0; // San Francisco

        let cam = fit(&session, rect());
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
        let mut session = Session::new(egui::Context::default());
        session.inputs.tx_lat = 51.5;
        session.inputs.tx_lon = -0.13;
        session.inputs.rx_lat = 51.6;
        session.inputs.rx_lon = -0.12;
        let cam = fit(&session, rect());
        assert!(cam.ppd >= Camera::min_ppd(rect()));

        session.inputs.rx_lat = -33.9;
        session.inputs.rx_lon = 151.2;
        let cam = fit(&session, rect());
        assert!(cam.ppd >= Camera::min_ppd(rect()));
        let world = Proj::new(rect(), cam).world_rect();
        assert!(world.width() >= rect().width() - 0.5);
    }
}
