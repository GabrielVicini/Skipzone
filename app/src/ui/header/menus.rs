//! The three top-bar menus.

use egui::Ui;

use crate::state::{AppState, Busy, Modal};
use crate::ui::actions::Action;
use crate::ui::scratch::{Menu, ViewScratch};
use crate::ui::widgets::menu;

pub fn menu_bar(ui: &mut Ui, state: &AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    let open = &mut scratch.menu;
    // A job cannot be restarted while it runs. Starting the other kind is
    // allowed: it supersedes the running job.
    let busy = state.results.busy;
    let solving = busy == Busy::Solving;
    let sweeping = matches!(busy, Busy::Sweeping { .. });

    out.extend(menu::dropdown(
        ui,
        open,
        Menu::PointToPoint,
        "Point-to-Point",
        |ui| {
            if menu::item_enabled(ui, "Calculate", !solving, "Already calculating") {
                return Some(Action::Calculate);
            }
            if menu::item_enabled(ui, "Best FREQ", !sweeping, "Already sweeping") {
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
        |ui| menu::item(ui, "Area coverage").then_some(Action::OpenModal(Modal::Coverage)),
    ));

    out.extend(menu::dropdown(ui, open, Menu::Help, "Help", |ui| {
        menu::item(ui, "About").then_some(Action::OpenModal(Modal::About))
    }));
}
