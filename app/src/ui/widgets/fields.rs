//! Text entry that reports whether what has been typed is usable.
//!
//! Every free-text field in this app (grid squares, coordinates, times, dates)
//! follows the same rule: the buffer is the operator's, and the scenario only
//! changes when the buffer parses. Invalid text is coloured rather than
//! rejected, so a half-typed value can be finished.

use egui::{Response, TextEdit, Ui};

use crate::ui::theme::FAIL;

/// Single-line entry, red while its contents do not parse.
pub fn text(ui: &mut Ui, buffer: &mut String, width: f32, valid: bool) -> Response {
    let mut edit = TextEdit::singleline(buffer)
        .desired_width(width)
        .margin(egui::vec2(6.0, 3.0));
    if !valid {
        edit = edit.text_color(FAIL);
    }
    ui.add(edit)
}
