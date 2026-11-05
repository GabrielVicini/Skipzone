//! Dialog windows. Each is one file and one entry point; `chrome::dialog`
//! gives them all the same draggable, resizable window with a scrolling body.
//! They are not modal - the map, the menus and the overlays stay live behind
//! them. The trace readout is not here: it is a docked side panel
//! ([`crate::ui::trace_panel`]) because it is read alongside the map.

mod about;
mod antennas;
mod best_freq;
mod chrome;
mod coverage;
mod settings;

use egui::Context;

use crate::state::AppState;
use crate::ui::actions::Action;

/// Draw whichever dialogs are open, pushing what they produced to `out`.
pub fn draw(ctx: &Context, state: &mut AppState, out: &mut Vec<Action>) {
    best_freq::show(ctx, state, out);
    coverage::show(ctx, state, out);
    settings::show(ctx, state);
    antennas::show(ctx, state);
    about::show(ctx, state);
}
