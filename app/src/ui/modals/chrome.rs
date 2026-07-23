//! Shared dialog chrome: title bar, close button, scrolling body.
//!
//! Every dialog in the app goes through [`dialog`], so they all close the same
//! ways (the ✕, clicking the backdrop, or Escape) and all scroll rather than
//! growing past the window.

use egui::{Align, Context, Id, Layout, Modal, RichText, ScrollArea, Ui, Vec2};

use crate::ui::theme;

/// Show a modal dialog while `open` is true. Returns the body's return value on
/// the frames it was drawn.
pub fn dialog<R>(
    ctx: &Context,
    id: &str,
    title: &str,
    open: &mut bool,
    max_size: Vec2,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    if !*open {
        return None;
    }
    let mut close_clicked = false;
    let modal = Modal::new(Id::new(id))
        .frame(theme::dialog_frame())
        .show(ctx, |ui| {
            ui.set_max_width(max_size.x);
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).heading());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close_clicked = ui.button(RichText::new("\u{2715}").strong()).clicked();
                });
            });
            ui.separator();
            ScrollArea::vertical()
                .max_height(max_size.y)
                .auto_shrink([false, true])
                .show(ui, content)
                .inner
        });

    if close_clicked || modal.should_close() {
        *open = false;
    }
    Some(modal.inner)
}
