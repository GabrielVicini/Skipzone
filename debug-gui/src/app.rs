//! Application shell: layout, map interaction, and the run trigger.

use eframe::{App, CreationContext, Frame};
use egui::{
    CentralPanel, Color32, CornerRadius, FontFamily, FontId, Margin, Panel, RichText, ScrollArea,
    Stroke, TextStyle, Ui,
};
use walkers::{HttpTiles, Map, MapMemory, Projector, lat_lon, sources::OpenStreetMap};

use crate::mapview::PathPlugin;
use crate::panels::{self, BAD, FAIL, MUTED, OK};
use crate::scenario::{self, Assumptions, Inputs, PlaceMode, ProfileRow};
use crate::solve::{self, Solution, SolveOutcome};

const INPUTS_MIN: f32 = 240.0;
const INPUTS_MAX: f32 = 420.0;
const DEBUG_MIN: f32 = 280.0;
const DEBUG_MAX: f32 = 760.0;
const MAP_MIN: f32 = 260.0;

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
    needs_fit: bool,
    styled_for_width: f32,
}

impl DebugApp {
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let inputs = Inputs::default();
        Self {
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
            styled_for_width: 0.0,
        }
    }

    /// Text and spacing scale from the window width, so the same readouts stay
    /// legible on a laptop and don't look cramped on a large display.
    fn apply_scale(&mut self, ui: &mut Ui, width: f32) {
        if (width - self.styled_for_width).abs() < 40.0 {
            return;
        }
        self.styled_for_width = width;
        let scale = (width / 1440.0).clamp(0.86, 1.18);

        let style = ui.style_mut();
        let mono = FontFamily::Monospace;
        let prop = FontFamily::Proportional;
        style.text_styles = [
            (TextStyle::Heading,   FontId::new(17.0 * scale, prop.clone())),
            (TextStyle::Body,      FontId::new(13.0 * scale, prop.clone())),
            (TextStyle::Button,    FontId::new(13.0 * scale, prop.clone())),
            (TextStyle::Small,     FontId::new(11.0 * scale, prop)),
            (TextStyle::Monospace, FontId::new(11.5 * scale, mono)),
        ].into();

        style.spacing.item_spacing     = egui::vec2(6.0 * scale, 4.0 * scale);
        style.spacing.button_padding   = egui::vec2(7.0 * scale, 4.0 * scale);
        style.spacing.indent           = 14.0 * scale;
        style.spacing.interact_size.y  = 20.0 * scale;
        style.visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0, Color32::from_gray(0x3A));
    }

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

    fn status_chip(ui: &mut Ui, out: &SolveOutcome) {
        let (colour, text) = if out.solutions.is_empty() {
            (FAIL, "NO PATH CONNECTS at this frequency".to_string())
        } else {
            (
                OK,
                format!("CONNECTS - {} mode(s) found", out.solutions.len()),
            )
        };
        egui::Frame::NONE
            .fill(colour.gamma_multiply(0.18))
            .stroke(Stroke::new(1.0, colour))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.colored_label(colour, RichText::new(text).strong());
            });
    }
}

impl App for DebugApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        let total_w = ui.available_width();
        self.apply_scale(ui, total_w);

        let mut run = false;

        // Panel widths track the window so neither side crowds out the map on a
        // small screen, and both stay draggable within sane bounds.
        let inputs_w = (total_w * 0.20).clamp(INPUTS_MIN, INPUTS_MAX);
        let debug_w = (total_w * 0.32).clamp(DEBUG_MIN, DEBUG_MAX);
        let debug_max = (total_w - inputs_w - MAP_MIN).clamp(DEBUG_MIN, DEBUG_MAX);
        let debug_w = debug_w.min(debug_max);

        Panel::left("inputs_panel")
            .resizable(true)
            .default_size(inputs_w)
            .size_range(INPUTS_MIN..=INPUTS_MAX)
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        run = panels::inputs_panel(ui, &mut self.inputs, &mut self.place);
                    });
            });

        Panel::right("debug_panel")
            .resizable(true)
            .default_size(debug_w)
            .size_range(DEBUG_MIN..=debug_max.max(DEBUG_MIN))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading("Debug output");
                        ui.add_space(4.0);

                        if let Some(e) = &self.build_error {
                            ui.colored_label(
                                BAD,
                                RichText::new(format!("model build failed: {e}"))
                                    .monospace()
                                    .small(),
                            );
                        }
                        if self.outcome.is_none() && self.build_error.is_none() {
                            ui.label(RichText::new("Press RUN TRACE.").color(MUTED));
                        }

                        if let Some(out) = &self.outcome {
                            Self::status_chip(ui, out);
                            ui.add_space(6.0);
                            panels::reference_panel(ui, out);
                            panels::legend_panel(
                                ui,
                                out,
                                &mut self.visible,
                                &mut self.selected,
                            );
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
                        ui.add_space(8.0);
                    });
            });

        CentralPanel::default().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "Click to place: {}",
                        match self.place {
                            PlaceMode::Tx => "TX",
                            PlaceMode::Rx => "RX",
                        }
                    ))
                        .strong(),
                );
                ui.separator();
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
                        .color(MUTED),
                );
            });
            ui.add_space(2.0);

            let center = self.map_center();
            let center_pos = lat_lon(center.0, center.1);

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