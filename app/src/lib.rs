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
//! Layered by concern, with each layer depending only on the ones above it:
//!
//! | layer       | modules                                    | knows about     |
//! |-------------|--------------------------------------------|-----------------|
//! | conversion  | `coastline`                                | nothing else    |
//! | model       | `scenario`, `noise`                        | the engine, `skipzone-iono` |
//! | computation | `compute`, `solve`, `sweep`, `coverage`    | the model       |
//! | state       | `state`                                    | the computation |
//! | view        | `ui`, `app`                                | the state       |
//!
//! Nothing in `ui` computes a physical quantity, and nothing below `state`
//! mentions egui.

pub mod antenna;
pub mod app;
pub mod calib;
pub mod coastline;
pub mod compute;
/// A saved, reproducible WSPR corpus for calibration. Harness-only.
pub mod corpus;
pub mod coverage;
/// Two-way fixed-effects calibration against measured WSPR spots. Harness-only.
pub mod fit;
/// Outbound HTTP, used only by the validation harnesses.
pub mod net;
pub mod noise;
pub mod scenario;
pub mod solve;
/// Observed solar indices, fetched rather than assumed.
pub mod spaceweather;
pub mod state;
pub mod sweep;
pub mod ui;
pub mod wspr;
pub mod wspr_report;
/// Live WSPR spot retrieval.
pub mod wsprlive;
