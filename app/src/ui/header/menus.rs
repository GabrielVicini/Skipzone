//! The three top-bar menus.

use egui::Ui;

use crate::state::{Menu, UiState};
use crate::ui::actions::Action;
use crate::ui::widgets::menu;

pub fn menu_bar(ui: &mut Ui, ui_state: &mut UiState, out: &mut Vec<Action>) {
    let open = &mut ui_state.menu;

    out.extend(menu::dropdown(
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

    out.extend(menu::dropdown(
        ui,
        open,
        Menu::CoverageMaps,
        "Coverage Maps",
        |ui| menu::item(ui, "Area coverage").then_some(Action::ShowCoverage),
    ));

    out.extend(menu::dropdown(ui, open, Menu::Help, "Help", |ui| {
        menu::item(ui, "About").then_some(Action::ShowAbout)
    }));
}
