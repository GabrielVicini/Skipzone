//! The map itself: the full-screen centrepiece of the interface.
//!
//! [`MapView`] owns only what must persist between frames - the tile cache and
//! the pan/zoom memory - and reads everything else from the session. It reports
//! a click back as a position and lets the caller decide what it means, so the
//! "which station does a click move?" rule lives with the rest of the UI state
//! rather than in the drawing code.

use egui::{CentralPanel, Frame, Ui};
use walkers::{HttpTiles, Map, MapMemory, Projector, lat_lon, sources::OpenStreetMap};

use crate::solar;
use crate::state::{Session, UiState};

use super::plugins::{CoastlinePlugin, CoveragePlugin, PathPlugin, TerminatorPlugin};

pub struct MapView {
    tiles: HttpTiles,
    memory: MapMemory,
    /// Re-frame the path on the next draw (startup, or after "Fit path").
    needs_fit: bool,
}

impl MapView {
    #[must_use]
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            tiles: HttpTiles::new(OpenStreetMap, ctx.clone()),
            memory: MapMemory::default(),
            needs_fit: true,
        }
    }

    pub fn request_fit(&mut self) {
        self.needs_fit = true;
    }

    pub fn zoom_in(&mut self) {
        let _ = self.memory.zoom_in();
    }

    pub fn zoom_out(&mut self) {
        let _ = self.memory.zoom_out();
    }

    #[must_use]
    pub fn zoom(&self) -> f64 {
        self.memory.zoom()
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
                let inputs = &session.inputs;
                let centre = lat_lon(
                    f64::midpoint(inputs.tx_lat, inputs.rx_lat),
                    f64::midpoint(inputs.tx_lon, inputs.rx_lon),
                );

                if self.needs_fit {
                    let zoom = fit_zoom(session, ui.available_size());
                    let _ = self.memory.set_zoom(zoom);
                    self.memory.center_at(centre);
                    self.needs_fit = false;
                }

                let mut map =
                    Map::new(Some(&mut self.tiles), &mut self.memory, centre).zoom_with_ctrl(false);

                // Terminator shading is added first so it sits under the ray
                // paths. It reads the subsolar point derived from the current
                // UTC/date, so it tracks the time control live without a
                // re-solve.
                if ui_state.show_terminator {
                    let (decl_deg, sub_lon_deg) =
                        solar::subsolar_point(inputs.month, inputs.day_of_month, inputs.utc_hours);
                    map = map.with_plugin(TerminatorPlugin {
                        decl_deg,
                        sub_lon_deg,
                    });
                }

                // Coverage tiles sit above the night shading (they are the
                // answer being read) but below the coastlines and ray paths,
                // which have to stay legible on top of them.
                if !session.coverage.cells.is_empty() {
                    map = map.with_plugin(CoveragePlugin {
                        cells: &session.coverage.cells,
                        alpha: ui_state.coverage_alpha,
                    });
                }

                // Coastline debug outlines sit above the shading but below the
                // ray paths: they are there to be compared against the
                // reflection dots, so the dots must stay on top.
                if ui_state.show_coastlines {
                    map = map.with_plugin(CoastlinePlugin);
                }

                let response = ui.add(map.with_plugin(PathPlugin {
                    solutions: session.solve.solutions(),
                    visible: &session.solve.visible,
                    selected: session.solve.selected,
                    tx: (inputs.tx_lat, inputs.tx_lon),
                    rx: (inputs.rx_lat, inputs.rx_lon),
                }));

                if !response.clicked() {
                    return None;
                }
                let pointer = response.interact_pointer_pos()?;
                let projector = Projector::new(response.rect, &self.memory, centre);
                let position = projector.unproject(pointer.to_vec2());
                let (lat, lon) = (position.y(), position.x());
                // A click that is not on the map does not move the station.
                //
                // Both axes are rejected outright rather than folded back onto
                // the map. Walkers does not repeat the world horizontally - the
                // void either side of the sheet is void, not another copy of it
                // - so wrapping a click there to a legal longitude lands the
                // marker at a place the operator never clicked, which reads as
                // the station jumping around at random. Clamping to the edge
                // has the same flaw in the other direction. Past the Mercator
                // latitude cut-off there is no basemap at all.
                let limit = crate::coverage::LAT_LIMIT_DEG;
                let on_map = lat.is_finite()
                    && lon.is_finite()
                    && lat.abs() <= limit
                    && (-180.0..=180.0).contains(&lon);
                on_map.then_some((lat, lon))
            })
            .inner
    }
}

/// Zoom level that fits the whole TX-RX path in `size` pixels, with margin.
/// Web-Mercator tiles are 256 px at 360 degrees of longitude for zoom 0, so the
/// zoom that fits `dlon` degrees into `w` pixels is `log2(w * 360 / (256 dlon))`;
/// the same holds for latitude with the 180-degree span, and the tighter of the
/// two wins.
fn fit_zoom(session: &Session, size: egui::Vec2) -> f64 {
    let inputs = &session.inputs;
    let raw_dlon = (inputs.tx_lon - inputs.rx_lon).abs();
    let dlon = if raw_dlon > 180.0 {
        360.0 - raw_dlon
    } else {
        raw_dlon
    }
    .max(0.05);
    let dlat = (inputs.tx_lat - inputs.rx_lat).abs().max(0.05);
    let w = f64::from(size.x.max(200.0));
    let h = f64::from(size.y.max(200.0));
    let z_lon = (w * 360.0 / (256.0 * dlon * 1.4)).log2();
    let z_lat = (h * 180.0 / (256.0 * dlat * 1.4)).log2();
    z_lon.min(z_lat).clamp(1.0, 12.0)
}
