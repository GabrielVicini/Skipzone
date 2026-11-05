//! Layout assembly: the one function that says where everything goes.
//!
//! Reading it top to bottom is the layout - solid header, then the map filling
//! the rest of the window, then the floating overlays and any open dialogs on
//! top. Every piece reports what the operator asked for; the actions are applied
//! here, in the order they were emitted, after everything has been drawn.

use egui::Ui;

use crate::state::{Session, UiState};
use skipzone_predict::scenario::PlaceMode;

use super::map::MapView;
use super::{actions, header, modals, overlays, theme, trace_panel};

pub fn draw(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState, map: &mut MapView) {
    theme::apply_scale(ui, &mut ui_state.styled_for_width);
    let ctx = ui.ctx().clone();

    let mut actions = Vec::new();
    header::draw(ui, session, ui_state, &mut actions);

    // The trace panel docks to the right, taking its width off the map before
    // the map is laid out - so opening it pushes the map and the floating
    // controls aside rather than covering them.
    trace_panel::draw(ui, session, ui_state, &mut actions);

    // The map is the centrepiece: it takes every pixel the header and the
    // trace panel left.
    if let Some((lat, lon)) = map.draw(ui, session, ui_state) {
        place(session, ui_state, lat, lon);
    }

    overlays::draw(&ctx, session, ui_state, map, &mut actions);
    modals::draw(&ctx, session, ui_state, &mut actions);

    if !actions.is_empty() {
        // Applied after everything was drawn, so this frame showed the old
        // state. Draw once more to show the new one.
        ctx.request_repaint();
    }
    actions::apply_all(actions, session, ui_state, map);
}

/// Move whichever station the operator is placing to a clicked position, and
/// re-fill that station's entry buffers so both notations follow immediately.
fn place(session: &mut Session, ui_state: &mut UiState, lat: f64, lon: f64) {
    match ui_state.place {
        PlaceMode::Tx => {
            session.inputs.tx_lat = lat;
            session.inputs.tx_lon = lon;
            ui_state.tx.refresh(lat, lon);
        }
        PlaceMode::Rx => {
            session.inputs.rx_lat = lat;
            session.inputs.rx_lon = lon;
            ui_state.rx.refresh(lat, lon);
        }
    }
}
