//! The solid top bar: menus and status on the first row, the TX and RX entry
//! rows beneath. This is the only opaque chrome in the layout - everything else
//! floats over the map.

mod menus;
mod stations;
mod status;

use egui::{Align, Layout, Panel, Ui};

use crate::state::{Session, UiState};
use crate::ui::actions::Action;
use crate::ui::theme;

/// Draw the header and report any menu selection. Records the bar's height in
/// `ui_state` so the floating overlays can start just below it.
pub fn draw(ui: &mut Ui, session: &mut Session, ui_state: &mut UiState) -> Option<Action> {
    let panel = Panel::top("header")
        .resizable(false)
        .frame(theme::header_frame())
        .show(ui, |ui| {
            let action = ui
                .horizontal(|ui| {
                    let action = menus::menu_bar(ui, ui_state);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        status::status(ui, session);
                    });
                    action
                })
                .inner;
            ui.add_space(2.0);
            stations::rows(ui, session, ui_state);
            action
        });

    ui_state.header_height = panel.response.rect.height();
    panel.inner
}
