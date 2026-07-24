//! What the interface can be asked to do, in one enum.
//!
//! Menu items, overlay buttons and dialog buttons all produce an [`Action`]
//! rather than mutating state where they are drawn. That keeps a command's
//! meaning in one place - "Calculate" always means *dispatch a solve and show
//! the trace* whether it was invoked from the menu or from the dialog - and
//! leaves the widgets themselves free of side effects.

use crate::state::{Session, UiState};

use super::map::MapView;

#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    /// Run the point-to-point solve and show the trace readout.
    Calculate,
    /// Run the frequency sweep and show its charts.
    BestFrequency,
    /// Re-frame the map on the current path.
    FitPath,
    ZoomIn,
    ZoomOut,
    /// Adopt a frequency found by the sweep as the tuned frequency.
    TuneTo(f64),
    ShowSettings,
    ShowAntennas,
    ShowAbout,
}

pub fn apply(action: Action, session: &mut Session, ui_state: &mut UiState, map: &mut MapView) {
    match action {
        Action::Calculate => {
            // Re-running simply replaces what the panel is showing; the panel
            // itself stays where the operator sized it.
            session.calculate();
            ui_state.trace_open = true;
        }
        Action::BestFrequency => {
            session.find_best_frequency();
            ui_state.modals.best_freq = true;
        }
        Action::FitPath => map.request_fit(),
        Action::ZoomIn => map.zoom_in(),
        Action::ZoomOut => map.zoom_out(),
        Action::TuneTo(freq_mhz) => session.inputs.freq_mhz = freq_mhz,
        Action::ShowSettings => ui_state.modals.settings = true,
        Action::ShowAntennas => ui_state.modals.antennas = true,
        Action::ShowAbout => ui_state.modals.about = true,
    }
}
