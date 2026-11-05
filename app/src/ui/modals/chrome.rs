//! Shared dialog chrome: a draggable, resizable window with a scrolling body.
//!
//! These are not modal: an operator reads the sweep charts while moving TX on
//! the map, or keeps Settings open while re-running, so the rest of the
//! interface stays live and several dialogs can be open at once. The title bar
//! carries the close button and the drag handle; the bottom-right corner
//! resizes it.

use egui::{Context, Id, ScrollArea, Ui, Vec2, Window};

use crate::state::Modal;
use crate::ui::actions::Action;
use crate::ui::theme;

/// Show `modal`'s window while `open`. The body gets the action sink, and the
/// close button emits [`Action::CloseModal`].
pub fn dialog(
    ctx: &Context,
    modal: Modal,
    open: bool,
    title: &str,
    default_size: Vec2,
    out: &mut Vec<Action>,
    content: impl FnOnce(&mut Ui, &mut Vec<Action>),
) {
    let mut open = open;
    if !open {
        return;
    }
    Window::new(title)
        .id(Id::new(id(modal)))
        .open(&mut open)
        .resizable(true)
        .collapsible(true)
        .scroll([false, false])
        .default_size(default_size)
        .min_size([320.0, 200.0])
        .frame(theme::dialog_frame())
        .show(ctx, |ui| {
            // The body scrolls inside whatever size the window has been dragged
            // to, so resizing reveals more of the content rather than clipping.
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| content(ui, out));
        });
    if !open {
        out.push(Action::CloseModal(modal));
    }
}

/// The window's egui id, which is where egui keeps its position and size.
fn id(modal: Modal) -> &'static str {
    match modal {
        Modal::BestFreq => "best_freq_dialog",
        Modal::Coverage => "coverage_dialog",
        Modal::About => "about_dialog",
        Modal::Settings => "settings_dialog",
        Modal::Antennas => "antennas_dialog",
    }
}
