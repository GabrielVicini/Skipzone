//! What the interface can be asked to do, in one enum.
//!
//! Menu items, overlay buttons and dialog buttons all produce an [`Action`]
//! rather than mutating state where they are drawn. That keeps a command's
//! meaning in one place - "Calculate" always means *dispatch a solve and show
//! the trace* whether it was invoked from the menu or from the dialog - and
//! leaves the widgets themselves free of side effects. [`crate::update`]
//! applies them.
//!
//! Every edit to an input is one [`InputChange`], carried by
//! [`Action::SetInput`]. A view edits a copy of the current value and emits
//! the change only when the copy differs, so an untouched control emits
//! nothing.

use crate::state::{AppInputs, Station};

/// A change to one input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputChange {
    /// Move a station to a typed or clicked position.
    Position {
        station: Station,
        lat: f64,
        lon: f64,
    },
}

impl InputChange {
    pub fn apply(self, inputs: &mut AppInputs) {
        let s = &mut inputs.scenario;
        match self {
            Self::Position {
                station: Station::Tx,
                lat,
                lon,
            } => (s.tx_lat, s.tx_lon) = (lat, lon),
            Self::Position {
                station: Station::Rx,
                lat,
                lon,
            } => (s.rx_lat, s.rx_lon) = (lat, lon),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Change one input.
    SetInput(InputChange),
    /// Move the station [`crate::state::UiState::place`] names to a map click.
    PlaceStation {
        lat: f64,
        lon: f64,
    },
    /// Choose which station a map click moves.
    SetPlaceMode(Station),
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
    /// Start the area coverage grid.
    RunCoverage,
    /// Stop the running grid, keeping every tile computed so far.
    CancelCoverage,
    /// Clear the coverage tiles off the map.
    ResetCoverage,
    ShowCoverage,
    ShowSettings,
    ShowAntennas,
    ShowAbout,
}
