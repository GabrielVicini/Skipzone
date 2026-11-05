//! The eframe application: owns the state, the view scratch and the job
//! service, and runs one frame as absorb, draw, update. All layout lives in
//! [`crate::ui::shell`].

use eframe::{App, CreationContext, Frame, Storage};
use egui::Ui;

use crate::jobs::Jobs;
use crate::persist;
use crate::state::AppState;
use crate::ui::scratch::ViewScratch;
use crate::ui::shell;
use crate::update;

pub struct SkipzoneApp {
    /// Inputs, results and the model-side UI state.
    state: AppState,
    /// What the views remember between frames: buffers, layout, the basemap.
    scratch: ViewScratch,
    jobs: Jobs,
}

impl SkipzoneApp {
    #[must_use]
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        // A response from the worker arrives between frames, so it asks for one.
        Self::with_jobs(cc, Jobs::spawn(move || ctx.request_repaint()))
    }

    /// Build the app around a given job service, so a test can pass
    /// [`Jobs::inline`]. Restores the saved settings from `cc.storage`.
    #[must_use]
    pub fn with_jobs(cc: &CreationContext<'_>, jobs: Jobs) -> Self {
        Self::restoring(cc, jobs, cc.storage)
    }

    /// Build the app and restore the saved settings from `storage`, if any.
    /// Tests pass their own storage here.
    #[must_use]
    pub fn restoring(cc: &CreationContext<'_>, jobs: Jobs, storage: Option<&dyn Storage>) -> Self {
        let mut state = AppState::new();
        let mut scratch = ViewScratch::new(&cc.egui_ctx, &state);
        if let Some(storage) = storage {
            persist::load(storage, &mut state, &mut scratch);
        }
        Self {
            state,
            scratch,
            jobs,
        }
    }

    /// The application state, for tests to inspect.
    #[must_use]
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Mutable access for tests, which pin the inputs to a fixed scenario
    /// instead of today's date and time.
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }
}

impl App for SkipzoneApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        update::absorb_all(&mut self.state, &self.jobs);
        let actions = shell::draw(ui, &self.state, &mut self.scratch);
        if !actions.is_empty() {
            // Applied after everything was drawn, so this frame showed the old
            // state. Draw once more to show the new one.
            ui.ctx().request_repaint();
        }
        update::update_all(&mut self.state, actions, &self.jobs);
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        persist::save(storage, &self.state, &self.scratch);
    }

    /// Only the settings in [`persist`] carry over. egui's own memory holds
    /// window positions and other layout, which start fresh each run.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}
