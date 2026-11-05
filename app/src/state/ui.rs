//! The interface state that belongs to the model: which dialogs are open,
//! which station a map click moves, which solution is selected, and where the
//! map camera is.
//!
//! Text being typed, the open menu and last frame's layout sizes are not here.
//! They belong to the views and live in [`crate::ui::scratch::ViewScratch`].

use super::Station;
use crate::ui::map::projection::Camera;

/// The dialog windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modal {
    /// Point-to-Point > Best FREQ: the sweep charts.
    BestFreq,
    /// Coverage Maps > Area coverage: the grid controls and legend.
    Coverage,
    /// Help > About.
    About,
    Settings,
    Antennas,
}

/// Which dialogs are open. They are ordinary windows, not modals: several can
/// be open at once and the rest of the interface stays live behind them.
#[derive(Default)]
pub struct Modals {
    pub best_freq: bool,
    pub coverage: bool,
    pub about: bool,
    pub settings: bool,
    pub antennas: bool,
}

impl Modals {
    #[must_use]
    pub fn is_open(&self, modal: Modal) -> bool {
        match modal {
            Modal::BestFreq => self.best_freq,
            Modal::Coverage => self.coverage,
            Modal::About => self.about,
            Modal::Settings => self.settings,
            Modal::Antennas => self.antennas,
        }
    }

    pub fn set(&mut self, modal: Modal, open: bool) {
        let flag = match modal {
            Modal::BestFreq => &mut self.best_freq,
            Modal::Coverage => &mut self.coverage,
            Modal::About => &mut self.about,
            Modal::Settings => &mut self.settings,
            Modal::Antennas => &mut self.antennas,
        };
        *flag = open;
    }
}

/// Which of the solved modes the operator is looking at.
#[derive(Default)]
pub struct Selection {
    /// Per-solution draw flag, parallel to the solve's `solutions`.
    pub visible: Vec<bool>,
    /// Index of the mode shown in detail.
    pub selected: Option<usize>,
}

/// The map camera, plus framing requests waiting for a viewport to apply to.
pub struct MapState {
    pub camera: Camera,
    /// Re-frame the path on the next draw (startup, or after "Fit path").
    pub needs_fit: bool,
    /// Pending scale change from the zoom buttons, applied once the viewport
    /// size is known so it can be clamped against it.
    pub pending_zoom: f64,
}

impl Default for MapState {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            needs_fit: true,
            pending_zoom: 1.0,
        }
    }
}

pub struct UiState {
    pub modals: Modals,
    /// Point-to-Point > Calculate: the trace readout, shown as a docked side
    /// panel rather than a window so it never covers the map.
    pub trace_open: bool,
    /// Which station a map click moves.
    pub place: Station,
    pub selection: Selection,
    pub map: MapState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            modals: Modals::default(),
            trace_open: false,
            place: Station::Tx,
            selection: Selection::default(),
            map: MapState::default(),
        }
    }
}
