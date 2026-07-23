//! Small building blocks shared by every readout: section headers, key/value
//! rows, framed cards and striped data grids.

use egui::{DragValue, Grid, Layout, RichText, ScrollArea, Ui};

use crate::ui::theme::{self, MUTED};

/// Small muted explanatory text under a control.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).small().color(MUTED));
}

/// Heading above a group of controls.
pub fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}

/// A sub-heading row inside a two-column data grid.
pub fn sub_head(ui: &mut Ui, text: &str) {
    ui.label("");
    ui.label(RichText::new(text).small().strong().color(MUTED));
    ui.end_row();
}

/// Key on the left, monospaced value right-aligned. Grid row.
pub fn kv(ui: &mut Ui, key: &str, value: String) {
    ui.label(RichText::new(key).small());
    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(value).monospace().small());
    });
    ui.end_row();
}

/// Bordered block grouping related controls.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    theme::card_frame(ui).show(ui, add).inner
}

/// Striped grid for tabular readouts.
pub fn data_grid<R>(ui: &mut Ui, id: &str, cols: usize, add: impl FnOnce(&mut Ui) -> R) -> R {
    Grid::new(id)
        .num_columns(cols)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, add)
        .inner
}

/// Wide fixed-column tables get their own horizontal scroll so they never force
/// their container wider than the window.
pub fn wide_table<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, add)
        .inner
}

/// Header row of a data grid.
pub fn head_cells(ui: &mut Ui, headers: &[&str]) {
    for h in headers {
        ui.label(RichText::new(*h).strong().small().color(MUTED));
    }
    ui.end_row();
}

/// One monospaced cell of a data grid.
pub fn num(ui: &mut Ui, value: String) {
    ui.label(RichText::new(value).monospace().small());
}

/// Label plus drag value, as one row of a two-column grid.
pub fn labelled_drag(ui: &mut Ui, label: &str, drag: DragValue<'_>) {
    ui.label(RichText::new(label).small());
    ui.add(drag);
    ui.end_row();
}
