//! Application state: one [`AppState`] owned by the app.
//!
//! * [`AppInputs`] - what the operator chose: the scenario, the coverage grid
//!   and the display preferences.
//! * [`Results`] - what the background jobs computed from them, and what they
//!   are doing now.
//! * [`UiState`] - open dialogs, map placement and camera, the selected mode.
//!
//! Views only read this. They report what the operator asked for as
//! [`crate::ui::actions::Action`]s, and [`crate::update::update`] is the one
//! place that changes it. Text buffers and other per-frame view memory live in
//! [`crate::ui::scratch::ViewScratch`] instead.

mod results;
mod ui;

use skipzone_iono::clock::{self, CivilDate};
use skipzone_predict::coverage::CoverageConfig;
use skipzone_predict::scenario::Inputs;
use skipzone_predict::solve::Solution;

pub use results::{
    AntennaPatterns, Busy, CoverageResults, JobErrors, PatternKey, Results, SolveResults,
    SweepResults,
};
pub use ui::{MapState, Modal, Modals, Selection, UiState};

/// One end of the path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Station {
    Tx,
    Rx,
}

/// Display preferences: they change how results are drawn, never a computed
/// value.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct Display {
    /// Draw the live day/night terminator shading on the map.
    pub show_terminator: bool,
    /// Debug view for coastline auto-detection: draw the land and lake polygon
    /// outlines the classifier tests against, so a hop's verdict can be checked
    /// by eye.
    pub show_coastlines: bool,
    /// Fill opacity of the area coverage tiles, 0-255.
    pub coverage_alpha: u8,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            show_terminator: true,
            show_coastlines: false,
            coverage_alpha: 0xC0,
        }
    }
}

/// Everything the operator chose.
pub struct AppInputs {
    /// The scenario and every model setting, as the solver takes them.
    pub scenario: Inputs,
    /// Grid settings for the area coverage map. A run parameter rather than
    /// part of the scenario, so it is kept out of `Inputs`.
    pub coverage: CoverageConfig,
    pub display: Display,
}

impl AppInputs {
    #[must_use]
    pub fn date(&self) -> CivilDate {
        CivilDate::new(
            self.scenario.year,
            self.scenario.month,
            self.scenario.day_of_month,
        )
    }
}

pub struct AppState {
    pub inputs: AppInputs,
    pub results: Results,
    pub ui: UiState,
}

impl AppState {
    /// Start from the default scenario, but on today's UTC date and time: the
    /// operator almost always wants "now", and `Inputs::default()` stays a
    /// fixed, test-reproducible scenario rather than a moving target.
    #[must_use]
    pub fn new() -> Self {
        let (today, hours) = clock::utc_now();
        Self::with_scenario(Inputs {
            year: today.year,
            month: today.month,
            day_of_month: today.day,
            utc_hours: hours,
            ..Inputs::default()
        })
    }

    /// Start from a given scenario, with nothing computed yet.
    #[must_use]
    pub fn with_scenario(scenario: Inputs) -> Self {
        Self {
            inputs: AppInputs {
                scenario,
                coverage: CoverageConfig::default(),
                display: Display::default(),
            },
            results: Results::default(),
            ui: UiState::default(),
        }
    }

    /// The mode currently selected for the detail readouts.
    #[must_use]
    pub fn selected_solution(&self) -> Option<&Solution> {
        let i = self.ui.selection.selected?;
        self.results.main.solutions().get(i)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
