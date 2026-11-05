//! Small building blocks shared by every readout: section headers, key/value
//! rows, framed cards and striped data grids.

use egui::{Color32, DragValue, Grid, Layout, ProgressBar, RichText, ScrollArea, Spinner, Ui};

use crate::ui::theme::{self, ACCENT, MUTED};

/// Small muted caption next to a control.
pub fn caption(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text).small().color(MUTED));
}

/// Small muted explanatory text under a control.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    caption(ui, text);
}

/// One legend entry: a glyph in `colour`, then a muted caption.
pub fn swatch(ui: &mut Ui, colour: Color32, glyph: RichText, text: &str) {
    ui.colored_label(colour, glyph);
    caption(ui, text);
}

/// The spinner shown while a job runs.
pub fn busy_spinner(ui: &mut Ui) {
    ui.add(Spinner::new().size(14.0));
}

/// A progress bar in the accent colour, with a small caption inside.
pub fn progress_bar(fraction: f32, text: String) -> ProgressBar {
    ProgressBar::new(fraction)
        .fill(ACCENT.gamma_multiply(0.7))
        .text(RichText::new(text).small())
}

/// Dropdown over `options`. Returns the option clicked this frame, if any,
/// even when it is the current one.
pub fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    width: Option<f32>,
    current: T,
    options: &[T],
    label: impl Fn(T) -> &'static str,
) -> Option<T> {
    let mut combo = egui::ComboBox::from_id_salt(id).selected_text(label(current));
    if let Some(width) = width {
        combo = combo.width(width);
    }
    let mut picked = None;
    combo.show_ui(ui, |ui| {
        for &option in options {
            if ui
                .selectable_label(current == option, label(option))
                .clicked()
            {
                picked = Some(option);
            }
        }
    });
    picked
}

/// Two-column grid of labelled controls, as the dialogs use.
pub fn form_grid<R>(ui: &mut Ui, id: impl egui::AsIdSalt, add: impl FnOnce(&mut Ui) -> R) -> R {
    Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, add)
        .inner
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
