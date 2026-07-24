//! Shared dialog chrome: a draggable, resizable window with a scrolling body.
//!
//! These are deliberately NOT modal. An operator reads the sweep charts while
//! moving TX on the map, or keeps Settings open while re-running - so the rest
//! of the interface stays live and clickable behind an open dialog, and several
//! can be open at once. The window's own title bar carries the ✕ and the drag
//! handle; the bottom-right corner resizes it.

use egui::{Context, Id, ScrollArea, Ui, Vec2, Window};

use crate::ui::theme;

/// Show a dialog window while `open` is true. Returns the body's return value
/// on the frames it was drawn (`None` when closed or collapsed).
pub fn dialog<R>(
    ctx: &Context,
    id: &str,
    title: &str,
    open: &mut bool,
    default_size: Vec2,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    Window::new(title)
        .id(Id::new(id))
        .open(open)
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
                .show(ui, content)
                .inner
        })
        .and_then(|response| response.inner)
}
