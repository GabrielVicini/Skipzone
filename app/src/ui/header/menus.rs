//! The three top-bar menus.

use egui::Ui;

use crate::state::{Menu, UiState};
use crate::ui::actions::Action;
use crate::ui::widgets::menu;

pub fn menu_bar(ui: &mut Ui, ui_state: &mut UiState) -> Option<Action> {
    let open = &mut ui_state.menu;
    let mut action = None;

    action = action.or(menu::dropdown(
        ui,
        open,
        Menu::PointToPoint,
        "Point-to-Point",
        |ui| {
            if menu::item(ui, "Calculate") {
                return Some(Action::Calculate);
            }
            if menu::item(ui, "Best FREQ") {
                return Some(Action::BestFrequency);
            }
            None
        },
    ));

    action = action.or(menu::dropdown(
        ui,
        open,
        Menu::CoverageMaps,
        "Coverage Maps",
        |ui| menu::item(ui, "Area coverage").then_some(Action::ShowCoverage),
    ));

    action.or(menu::dropdown(ui, open, Menu::Help, "Help", |ui| {
        menu::item(ui, "About").then_some(Action::ShowAbout)
    }))
}
