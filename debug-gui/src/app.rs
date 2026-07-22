//! Application shell: layout, map interaction, and the run trigger.

use eframe::{App, CreationContext, Frame};
use egui::{CentralPanel, Color32, Panel, RichText, ScrollArea, Ui};
use walkers::{HttpTiles, Map, MapMemory, Projector, lat_lon, sources::OpenStreetMap};

use crate::mapview::PathPlugin;
use crate::panels;
use crate::scenario::{self, Assumptions, Inputs, PlaceMode, ProfileRow};
use crate::solve::{self, Solution, SolveOutcome};

pub struct DebugApp {
    tiles: HttpTiles,
    map_memory: MapMemory,
    inputs: Inputs,
    place: PlaceMode,
    outcome: Option<SolveOutcome>,
    assumptions: Option<Assumptions>,
    profile: Vec<ProfileRow>,
    visible: Vec<bool>,
    selected: Option<usize>,
    build_error: Option<String>,
    /// Set when the view should be re-fitted to the TX/RX span on the next
    /// frame, once the map's on-screen size is known.
    needs_fit: bool,
}

impl DebugApp {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let inputs = Inputs::default();
        Self {
            // Live OpenStreetMap tiles are fine for personal, low-volume use.
            // If this tool ever sees sustained or multi-user traffic, OSM's
            // tile usage policy requires switching to a local tile cache or a
            // self-hosted tile server rather than hammering tile.osm.org.
            tiles: HttpTiles::new(OpenStreetMap, cc.egui_ctx.clone()),
            map_memory: MapMemory::default(),
            inputs,
            place: PlaceMode::Tx,
            outcome: None,
            assumptions: None,
            profile: Vec::new(),
            visible: Vec::new(),
            selected: None,
            build_error: None,
            needs_fit: true,
        }
    }

    /// Web-Mercator zoom level that fits the TX/RX span in `size` pixels.
    /// The world is 256 * 2^z pixels wide for 360 deg of longitude; latitude
    /// is treated linearly over 180 deg, which is only approximate near the
    /// poles but is plenty for framing a path. 1.4 leaves a margin.
    fn fit_zoom(&self, size: egui::Vec2) -> f64 {
        let raw_dlon = (self.inputs.tx_lon - self.inputs.rx_lon).abs();
        let dlon = if raw_dlon > 180.0 {
            360.0 - raw_dlon
        } else {
            raw_dlon
        }
        .max(0.05);
        let dlat = (self.inputs.tx_lat - self.inputs.rx_lat).abs().max(0.05);
        let w = f64::from(size.x.max(200.0));
        let h = f64::from(size.y.max(200.0));
        let z_lon = (w * 360.0 / (256.0 * dlon * 1.4)).log2();
        let z_lat = (h * 180.0 / (256.0 * dlat * 1.4)).log2();
        z_lon.min(z_lat).clamp(1.0, 12.0)
    }

    fn map_center(&self) -> (f64, f64) {
        (
            f64::midpoint(self.inputs.tx_lat, self.inputs.rx_lat),
            f64::midpoint(self.inputs.tx_lon, self.inputs.rx_lon),
        )
    }

    fn run_solve(&mut self) {
        self.build_error = None;
        let a = scenario::resolve(&self.inputs);
        match scenario::build_models(&self.inputs, &a) {
            Ok(models) => {
                self.profile = scenario::sample_profile(&models, &a);
                let out = solve::solve(&self.inputs, &a, &models);
                self.visible = vec![true; out.solutions.len()];
                self.selected = (!out.solutions.is_empty()).then_some(0);
                self.outcome = Some(out);
            }
            Err(e) => {
                self.build_error = Some(e);
                self.outcome = None;
                self.profile.clear();
                self.visible.clear();
                self.selected = None;
            }
        }
        self.assumptions = Some(a);
    }
}

impl App for DebugApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        let mut run = false;

        Panel::left("inputs_panel")
            .default_size(330.0)
            .show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    run = panels::inputs_panel(ui, &mut self.inputs, &mut self.place);
                });
            });

        Panel::right("debug_panel")
            .default_size(620.0)
            .show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Debug output");
                    if let Some(e) = &self.build_error {
                        ui.colored_label(
                            Color32::from_rgb(0xC8, 0x3A, 0x1C),
                            RichText::new(format!("model build failed: {e}")).monospace(),
                        );
                    }
                    if self.outcome.is_none() && self.build_error.is_none() {
                        ui.label("Press RUN TRACE.");
                    }

                    if let Some(out) = &self.outcome {
                        if out.solutions.is_empty() {
                            ui.colored_label(
                                Color32::from_rgb(0xD0, 0x21, 0x1C),
                                RichText::new("NO PATH CONNECTS at this frequency.").strong(),
                            );
                        } else {
                            ui.colored_label(
                                Color32::from_rgb(0x1B, 0x7F, 0x3B),
                                RichText::new(format!(
                                    "CONNECTS: {} mode(s) found.",
                                    out.solutions.len()
                                ))
                                .strong(),
                            );
                        }
                        ui.separator();
                        panels::reference_panel(ui, out);
                        panels::legend_panel(ui, out, &mut self.visible, &mut self.selected);
                        if let Some(sol) = self.selected.and_then(|i| out.solutions.get(i)) {
                            panels::solution_panel(ui, sol);
                        }
                        panels::near_miss_panel(ui, out);
                        panels::errors_panel(ui, out);
                    }

                    if let Some(a) = &self.assumptions {
                        panels::assumptions_panel(ui, a);
                    }
                    if !self.profile.is_empty() {
                        panels::profile_panel(ui, &self.profile);
                    }
                });
            });

        CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!(
                    "Click the map to place: {}",
                    match self.place {
                        PlaceMode::Tx => "TX",
                        PlaceMode::Rx => "RX",
                    }
                )));
                if ui.button("Fit path").clicked() {
                    self.needs_fit = true;
                }
                if ui.button("Recentre").clicked() {
                    self.map_memory.follow_my_position();
                    self.needs_fit = true;
                }
                if ui.button("\u{2212}").clicked() {
                    let _ = self.map_memory.zoom_out();
                }
                if ui.button("+").clicked() {
                    let _ = self.map_memory.zoom_in();
                }
                ui.label(
                    RichText::new(format!("zoom {:.1}", self.map_memory.zoom()))
                        .monospace()
                        .small(),
                );
                ui.label(
                    RichText::new("drag to pan, scroll to zoom")
                        .small()
                        .color(Color32::GRAY),
                );
            });

            let center = self.map_center();
            let center_pos = lat_lon(center.0, center.1);

            // Fit on the first frame (and on request), now that the map's
            // on-screen size is known. Without this the default zoom is
            // street-level, which over a long path shows nothing but ocean.
            if self.needs_fit {
                let z = self.fit_zoom(ui.available_size());
                let _ = self.map_memory.set_zoom(z);
                self.map_memory.center_at(center_pos);
                self.needs_fit = false;
            }
            let empty: &[Solution] = &[];
            let sols = self
                .outcome
                .as_ref()
                .map_or(empty, |o| o.solutions.as_slice());
            let plugin = PathPlugin {
                solutions: sols,
                visible: &self.visible,
                selected: self.selected,
                tx: (self.inputs.tx_lat, self.inputs.tx_lon),
                rx: (self.inputs.rx_lat, self.inputs.rx_lon),
            };
            // zoom_with_ctrl defaults to true in walkers, which makes a bare
            // scroll pan instead of zoom. For a debug tool plain scroll-to-zoom
            // is what people expect; drag still pans.
            let map = Map::new(Some(&mut self.tiles), &mut self.map_memory, center_pos)
                .zoom_with_ctrl(false)
                .with_plugin(plugin);
            let resp = ui.add(map);

            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let projector = Projector::new(resp.rect, &self.map_memory, center_pos);
                    let pos = projector.unproject(p.to_vec2());
                    let (lat, lon) = (pos.y(), pos.x());
                    if lat.is_finite() && lon.is_finite() {
                        match self.place {
                            PlaceMode::Tx => {
                                self.inputs.tx_lat = lat.clamp(-89.9, 89.9);
                                self.inputs.tx_lon = lon;
                            }
                            PlaceMode::Rx => {
                                self.inputs.rx_lat = lat.clamp(-89.9, 89.9);
                                self.inputs.rx_lon = lon;
                            }
                        }
                    }
                }
            }
        });

        if run {
            self.run_solve();
        }
    }
}
