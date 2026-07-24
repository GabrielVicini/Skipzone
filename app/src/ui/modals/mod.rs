//! Dialog windows. Each is one file and one entry point; [`chrome::dialog`]
//! gives them all the same draggable, resizable window with a scrolling body.
//! They are not modal - the map, the menus and the overlays stay live behind
//! them. The trace readout is not here: it is a docked side panel
//! ([`crate::ui::trace_panel`]) because it is read alongside the map.

mod about;
mod antennas;
mod best_freq;
mod chrome;
mod settings;

use egui::Context;

use crate::state::{Session, UiState};
use crate::ui::actions::Action;

/// Draw whichever dialogs are open. Returns the first action any of them
/// produced.
pub fn draw(ctx: &Context, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let best = best_freq::show(ctx, session, ui_state);
    settings::show(ctx, session, ui_state);
    antennas::show(ctx, session, ui_state);
    about::show(ctx, ui_state);
    best
}
