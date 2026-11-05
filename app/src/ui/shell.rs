//! Layout assembly: the one function that says where everything goes.
//!
//! Reading it top to bottom is the layout - solid header, then the map filling
//! the rest of the window, then the floating overlays and any open dialogs on
//! top. Every piece reports what the operator asked for as actions, returned
//! in the order they were emitted for the app to apply after the frame.

use egui::Ui;

use crate::state::AppState;
use crate::ui::actions::Action;
use crate::ui::scratch::ViewScratch;

use super::{header, map, modals, overlays, theme, trace_panel};

pub fn draw(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch) -> Vec<Action> {
    theme::apply_scale(ui, &mut scratch.styled_for_width);
    let ctx = ui.ctx().clone();

    let mut actions = Vec::new();
    header::draw(ui, state, scratch, &mut actions);

    // The trace panel docks to the right, taking its width off the map before
    // the map is laid out - so opening it pushes the map and the floating
    // controls aside rather than covering them.
    trace_panel::draw(ui, state, scratch, &mut actions);

    // The map is the centrepiece: it takes every pixel the header and the
    // trace panel left.
    map::view::draw(ui, state, scratch, &mut actions);

    overlays::draw(&ctx, state, scratch, &mut actions);
    modals::draw(&ctx, state, &mut actions);
    actions
}
