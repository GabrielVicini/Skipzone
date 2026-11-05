//! The TX and RX rows: each station's position, typed as either a Maidenhead
//! grid square or decimal degrees.
//!
//! The scenario's latitude/longitude is authoritative. A row emits a new
//! position only when what has been typed parses, and rewrites its own
//! buffers from the scenario whenever the operator is not typing - which is
//! what lets a map click move a station and have both notations follow.

use egui::{Color32, RichText, Ui};

use crate::state::{AppState, Station};
use crate::ui::actions::{Action, InputChange};
use crate::ui::scratch::{LocationEntry, LocationMode, ViewScratch};
use crate::ui::theme::{self, MUTED};
use crate::ui::widgets::{self, fields};

/// TX above RX, both on the solid header background.
pub fn rows(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    let inputs = &state.inputs.scenario;
    let mut place = state.ui.place;
    for (station, label, colour, entry, current) in [
        (
            Station::Tx,
            "TX",
            theme::TX_COLOR,
            &mut scratch.tx,
            (inputs.tx_lat, inputs.tx_lon),
        ),
        (
            Station::Rx,
            "RX",
            theme::RX_COLOR,
            &mut scratch.rx,
            (inputs.rx_lat, inputs.rx_lon),
        ),
    ] {
        if let Some((lat, lon)) = row(ui, label, colour, station, &mut place, entry, current) {
            out.push(Action::SetInput(InputChange::Position {
                station,
                lat,
                lon,
            }));
        }
    }
    if place != state.ui.place {
        out.push(Action::SetPlaceMode(place));
    }
}

/// One station row. Returns a new position when the operator typed a valid one.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    label: &str,
    colour: Color32,
    this: Station,
    place: &mut Station,
    entry: &mut LocationEntry,
    current: (f64, f64),
) -> Option<(f64, f64)> {
    let mut committed = None;
    ui.horizontal(|ui| {
        // The badge doubles as the "map clicks place this station" selector.
        ui.selectable_value(place, this, RichText::new(label).color(colour).strong())
            .on_hover_text(format!("Map clicks place {label}"));

        for mode in LocationMode::ALL {
            ui.selectable_value(&mut entry.mode, mode, RichText::new(mode.label()).small());
        }

        let valid = entry.is_valid();
        let typing = match entry.mode {
            LocationMode::Grid => {
                widgets::caption(ui, "grid");
                let response = fields::text(ui, &mut entry.grid_text, 84.0, valid);
                if response.changed() {
                    committed = entry.parsed_grid();
                }
                ui.label(
                    RichText::new(format!("{:.4}, {:.4}", current.0, current.1))
                        .small()
                        .monospace()
                        .color(MUTED),
                );
                response.has_focus()
            }
            LocationMode::LatLon => {
                widgets::caption(ui, "lat");
                let lat_response = fields::text(ui, &mut entry.lat_text, 62.0, valid);
                widgets::caption(ui, "lon");
                let lon_response = fields::text(ui, &mut entry.lon_text, 62.0, valid);
                if lat_response.changed() || lon_response.changed() {
                    committed = entry.parsed_lat_lon();
                }
                ui.label(
                    RichText::new(&entry.grid_text)
                        .small()
                        .monospace()
                        .color(MUTED),
                );
                lat_response.has_focus() || lon_response.has_focus()
            }
        };

        // Follow the authoritative position whenever nothing is being typed.
        if !typing {
            let (lat, lon) = committed.unwrap_or(current);
            entry.refresh(lat, lon);
        }
    });
    committed
}
