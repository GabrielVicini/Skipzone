//! The eframe application: owns the three pieces of state and hands them to
//! the UI shell each frame. All layout lives in [`crate::ui::shell`].

use eframe::{App, CreationContext, Frame};
use egui::Ui;

use crate::jobs::Jobs;
use crate::state::{Session, UiState};
use crate::ui::map::MapView;
use crate::ui::shell;

pub struct SkipzoneApp {
    /// The scenario, its results, and the background jobs.
    session: Session,
    /// What the interface remembers between frames.
    ui_state: UiState,
    /// Tile cache and pan/zoom memory for the map.
    map: MapView,
}

impl SkipzoneApp {
    #[must_use]
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        // A response from the worker arrives between frames, so it asks for one.
        Self::with_jobs(cc, Jobs::spawn(move || ctx.request_repaint()))
    }

    /// Build the app around a given job service, so a test can pass
    /// [`Jobs::inline`].
    #[must_use]
    pub fn with_jobs(cc: &CreationContext<'_>, jobs: Jobs) -> Self {
        let session = Session::new(jobs);
        let ui_state = UiState::new(&session);
        Self {
            session,
            ui_state,
            map: MapView::new(&cc.egui_ctx),
        }
    }

    /// The scenario and its results, for tests to inspect.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl App for SkipzoneApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        self.session.pump();
        shell::draw(ui, &mut self.session, &mut self.ui_state, &mut self.map);
    }
}
