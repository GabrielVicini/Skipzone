//! Small egui building blocks shared by the panels: section headers, key/value
//! rows, framed cards, and the striped data grids. Keeping them in one place
//! lets each panel file stay focused on the content it renders.

use egui::{
    CornerRadius, DragValue, Frame, Grid, Layout, Margin, RichText, ScrollArea, Stroke, Ui,
};

use super::theme::MUTED;

pub(crate) fn hint(ui: &mut Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(text).small().color(MUTED));
}

pub(crate) fn section(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).strong());
    ui.add_space(2.0);
}

pub(crate) fn sub_head(ui: &mut Ui, text: &str) {
    ui.label("");
    ui.label(RichText::new(text).small().strong().color(MUTED));
    ui.end_row();
}

pub(crate) fn kv(ui: &mut Ui, k: &str, v: String) {
    ui.label(RichText::new(k).small());
    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(v).monospace().small());
    });
    ui.end_row();
}

pub(crate) fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::NONE
        .inner_margin(Margin::symmetric(8, 6))
        .corner_radius(CornerRadius::same(6))
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .show(ui, add)
        .inner
}

pub(crate) fn data_grid<R>(
    ui: &mut Ui,
    id: &str,
    cols: usize,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    Grid::new(id)
        .num_columns(cols)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, add)
        .inner
}

/// Wide fixed-column tables get their own horizontal scroll so they never
/// force the whole panel wider than the window.
pub(crate) fn wide_table<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, add)
        .inner
}

pub(crate) fn head_cells(ui: &mut Ui, headers: &[&str]) {
    for h in headers {
        ui.label(RichText::new(*h).strong().small().color(MUTED));
    }
    ui.end_row();
}

pub(crate) fn num(ui: &mut Ui, v: String) {
    ui.label(RichText::new(v).monospace().small());
}

pub(crate) fn labelled_drag(ui: &mut Ui, label: &str, drag: DragValue<'_>) {
    ui.label(RichText::new(label).small());
    ui.add(drag);
    ui.end_row();
}
