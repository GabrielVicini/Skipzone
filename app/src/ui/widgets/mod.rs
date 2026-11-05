//! Reusable UI pieces, none of which know anything about the scenario they are
//! displaying. Every one takes plain data or a `&mut` to view memory owned by
//! [`crate::ui::scratch`], so the same control can appear in the header, in a
//! floating overlay or inside a dialog without change.

pub mod band;
pub mod calendar;
pub mod chart;
pub mod fields;
pub mod layout;
pub mod menu;

pub use layout::{
    busy_spinner, caption, card, choice, data_grid, form_grid, head_cells, hint, kv, labelled_drag,
    num, progress_bar, section, sub_head, swatch, wide_table,
};
