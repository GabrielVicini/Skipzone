//! The Skipzone desktop application: an egui front end for point-to-point HF
//! path prediction on a map.
//!
//! It depends on `skipzone-predict` for scenarios and the solve, and on
//! `skipzone-iono` for the date, grid and solar conversions it displays. It
//! never depends on `skipzone-lab`. The entry type is [`app::SkipzoneApp`],
//! which `main.rs` hands to eframe.
//!
//! Nothing in `ui` computes a physical quantity, and nothing in `state`
//! mentions egui.

pub mod app;
pub mod state;
pub mod ui;
