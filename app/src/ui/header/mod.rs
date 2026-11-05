//! The solid top bar: menus and status on the first row, the TX and RX entry
//! rows beneath. This is the only opaque chrome in the layout - everything else
//! floats over the map.

mod menus;
mod stations;
mod status;

use egui::{Align, Layout, Panel, Ui};

use crate::state::AppState;
use crate::ui::actions::Action;
use crate::ui::scratch::ViewScratch;
use crate::ui::theme;

/// Draw the header and push any menu selection to `out`. Records the bar's height in
/// `scratch` so the floating overlays can start just below it.
pub fn draw(ui: &mut Ui, state: &mut AppState, scratch: &mut ViewScratch, out: &mut Vec<Action>) {
    let panel = Panel::top("header")
        .resizable(false)
        .frame(theme::header_frame())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                menus::menu_bar(ui, scratch, out);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    status::status(ui, state);
                });
            });
            ui.add_space(2.0);
            stations::rows(ui, state, scratch);
        });

    scratch.header_height = panel.response.rect.height();
}
