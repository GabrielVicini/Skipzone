//! The eframe application: owns the three pieces of state and hands them to
//! the UI shell each frame. All layout lives in [`crate::ui::shell`].

use eframe::{App, CreationContext, Frame};
use egui::Ui;

use crate::state::{Session, UiState};
use crate::ui::map::MapView;
use crate::ui::shell;

pub struct SkipzoneApp {
    /// The scenario, its results, and the background solver.
    session: Session,
    /// What the interface remembers between frames.
    ui_state: UiState,
    /// Tile cache and pan/zoom memory for the map.
    map: MapView,
}

impl SkipzoneApp {
    #[must_use]
    pub fn new(cc: &CreationContext<'_>) -> Self {
        let session = Session::new(cc.egui_ctx.clone());
        let ui_state = UiState::new(&session);
        Self {
            session,
            ui_state,
            map: MapView::new(&cc.egui_ctx),
        }
    }
}

impl App for SkipzoneApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        self.session.pump();
        shell::draw(ui, &mut self.session, &mut self.ui_state, &mut self.map);
    }
}
