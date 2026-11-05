//! Point-to-point application for the Skipzone HF ray tracing engine.
//!
//! This crate calls the engine's public API (`ChapmanLayer`,
//! `QuasiParabolicLayer`, `Igrf`, `ExponentialCollisions`, `Tracer`, `Homing`)
//! and renders the results. The engine is a separate workspace member so the
//! app's dependency tree never reaches it.
//!
//! It is a library with a thin `main.rs` so that the headless validation
//! harnesses in `src/bin/` run the same model and solver code as the GUI.
//!
//! The ionospheric model layer (Chapman layers, foF2 maps, sporadic E, solar
//! geometry, time and grid conversions) lives in the `skipzone-iono` crate and
//! only ever feeds the engine tracer through the standard `ElectronDensity`
//! trait.
//!
//! Headless prediction (scenario, solve, link budget, noise, antennas,
//! sweep, coverage, calibration anchors and the land mask) lives in the
//! `skipzone-predict` crate.
//!
//! Layered by concern, with each layer depending only on the ones above it:
//!
//! | layer       | modules                                    | knows about     |
//! |-------------|--------------------------------------------|-----------------|
//! | state       | `state`                                    | `skipzone-predict` |
//! | view        | `ui`, `app`                                | the state       |
//!
//! Nothing in `ui` computes a physical quantity, and nothing below `state`
//! mentions egui.

pub mod app;
/// A saved, reproducible WSPR corpus for calibration. Harness-only.
pub mod corpus;
/// Two-way fixed-effects calibration against measured WSPR spots. Harness-only.
pub mod fit;
/// Outbound HTTP, used only by the validation harnesses.
pub mod net;
/// Observed solar indices, fetched rather than assumed.
pub mod spaceweather;
pub mod state;
pub mod ui;
pub mod wspr;
pub mod wspr_report;
/// Live WSPR spot retrieval.
pub mod wsprlive;
