//! The TX and RX rows: each station's position, typed as either a Maidenhead
//! grid square or decimal degrees.
//!
//! The scenario's latitude/longitude is authoritative. A row writes to it only
//! when what has been typed parses, and rewrites its own buffers from it
//! whenever the operator is not typing - which is what lets a map click move a
//! station and have both notations follow.

use egui::{Color32, RichText, Ui};

use crate::scenario::PlaceMode;
use crate::state::{LocationEntry, LocationMode, Session, UiState};
use crate::ui::theme::MUTED;
use crate::ui::widgets::fields;

/// TX above RX, both on the solid header background.
pub fn rows(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) {
    let tx = (session.inputs.tx_lat, session.inputs.tx_lon);
    if let Some((lat, lon)) = row(
        ui,
        "TX",
        Color32::from_rgb(0xD0, 0x21, 0x1C),
        PlaceMode::Tx,
        &mut ui_state.place,
        &mut ui_state.tx,
        tx,
    ) {
        session.inputs.tx_lat = lat;
        session.inputs.tx_lon = lon;
    }

    let rx = (session.inputs.rx_lat, session.inputs.rx_lon);
    if let Some((lat, lon)) = row(
        ui,
        "RX",
        Color32::from_rgb(0x14, 0x65, 0xC0),
        PlaceMode::Rx,
        &mut ui_state.place,
        &mut ui_state.rx,
        rx,
    ) {
        session.inputs.rx_lat = lat;
        session.inputs.rx_lon = lon;
    }
}

/// One station row. Returns a new position when the operator typed a valid one.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    label: &str,
    colour: Color32,
    this: PlaceMode,
    place: &mut PlaceMode,
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
                fields::caption(ui, "grid");
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
                fields::caption(ui, "lat");
                let lat_response = fields::text(ui, &mut entry.lat_text, 62.0, valid);
                fields::caption(ui, "lon");
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
