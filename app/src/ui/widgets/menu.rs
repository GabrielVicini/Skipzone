//! Hover-opening dropdown menus for the top bar.
//!
//! egui's own menus open on click; these open as soon as the pointer touches
//! the button and, once one is open, sliding sideways moves straight to the
//! next - the behaviour a desktop menu bar has. Which menu is open lives in the
//! caller's state (see [`crate::state::Menu`]), so the widget stays stateless
//! and any menu can be closed from anywhere.

use egui::{Align, Button, Color32, Layout, Popup, RectAlign, RichText, Ui};

use crate::ui::theme;

/// One menu-bar entry. `items` draws the dropdown contents and returns `Some`
/// when the operator picked something, which closes the menu and hands the
/// choice back.
///
/// `open` is the shared "which menu is open" slot; `id` identifies this one.
pub fn dropdown<Id, R>(
    ui: &mut Ui,
    open: &mut Option<Id>,
    id: Id,
    label: &str,
    items: impl FnOnce(&mut Ui) -> Option<R>,
) -> Option<R>
where
    Id: Copy + PartialEq,
{
    let is_open = *open == Some(id);
    let fill = if is_open {
        Color32::from_white_alpha(0x18)
    } else {
        Color32::TRANSPARENT
    };
    let response = ui.add(Button::new(RichText::new(label).strong()).fill(fill));

    // Hovering the button opens this menu (and so closes any other).
    if response.hovered() {
        *open = Some(id);
    }
    // A click on an already-open menu closes it, which is how a click-first
    // operator dismisses one without moving the pointer away.
    if response.clicked() && is_open {
        *open = None;
        return None;
    }

    let popup = Popup::from_response(&response)
        .open(is_open)
        .align(RectAlign::BOTTOM_START)
        .gap(4.0)
        .frame(theme::popup_frame())
        .width(190.0)
        .show(|ui| {
            ui.with_layout(Layout::top_down_justified(Align::Min), items)
                .inner
        });

    let popup = popup?;
    // Leaving both the button and the dropdown closes the menu.
    if !response.contains_pointer() && !popup.response.contains_pointer() {
        *open = None;
    }
    if popup.inner.is_some() {
        *open = None;
    }
    popup.inner
}

/// One selectable line inside a dropdown. Returns true when clicked.
pub fn item(ui: &mut Ui, label: &str) -> bool {
    ui.add(Button::new(label).fill(Color32::TRANSPARENT))
        .clicked()
}

/// A dropdown line that is shown but cannot be chosen (a placeholder), with the
/// reason as its hover text.
pub fn disabled_item(ui: &mut Ui, label: &str, why: &str) {
    ui.add_enabled(false, Button::new(label).fill(Color32::TRANSPARENT))
        .on_disabled_hover_text(why);
}
