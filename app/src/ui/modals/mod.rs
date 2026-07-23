//! Modal dialogs. Each is one file and one entry point; [`chrome::dialog`]
//! gives them all the same title bar, close behaviour and scrolling body.

mod about;
mod antennas;
mod best_freq;
mod chrome;
mod settings;
mod trace;

use egui::Context;

use crate::state::{Session, UiState};
use crate::ui::actions::Action;

/// Draw whichever dialogs are open. Returns the first action any of them
/// produced.
pub fn draw(ctx: &Context, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let trace = trace::show(ctx, session, ui_state);
    let best = best_freq::show(ctx, session, ui_state);
    settings::show(ctx, session, ui_state);
    antennas::show(ctx, session, ui_state);
    about::show(ctx, ui_state);
    trace.or(best)
}
