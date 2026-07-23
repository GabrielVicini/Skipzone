//! Reusable UI pieces, none of which know anything about the scenario they are
//! displaying. Every one takes plain data or a `&mut` to state owned by
//! [`crate::state`], so the same control can appear in the header, in a
//! floating overlay or inside a dialog without change.

pub mod band;
pub mod calendar;
pub mod chart;
pub mod fields;
pub mod layout;
pub mod menu;

pub use layout::{
    card, data_grid, head_cells, hint, kv, labelled_drag, num, section, sub_head, wide_table,
};
