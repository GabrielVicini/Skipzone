//! Point-to-point application for the Skipzone HF ray tracing engine.
//!
//! This crate calls the engine's public API (`ChapmanLayer`,
//! `QuasiParabolicLayer`, `Igrf`, `ExponentialCollisions`, `Tracer`, `Homing`)
//! and renders the results. The engine crate is untouched by design, and lives
//! in a separate workspace member so the app's dependency tree never reaches it.
//!
//! It is a library with a thin `main.rs` on top rather than a plain binary, so
//! that the headless validation harnesses in `src/bin/` drive exactly the same
//! model and solver code the GUI does, instead of a parallel copy of it.
//!
//! The exception to "no physics here" is the ionospheric model layer -
//! `chapman`, `fof2` and `sporadic_e`. Coupling electron production to solar
//! geometry and to a climatology map is a scenario concern, not a reusable
//! engine primitive; it is derived and validated the same way
//! (docs/derivations/chapman-grazing.md) and only ever feeds the engine tracer
//! through the standard `ElectronDensity` trait.
//!
//! Layered by concern, with each layer depending only on the ones above it:
//!
//! | layer       | modules                                    | knows about     |
//! |-------------|--------------------------------------------|-----------------|
//! | conversion  | `clock`, `grid`, `solar`, `coastline`      | nothing else    |
//! | model       | `chapman`, `fof2`, `sporadic_e`, `scenario`, `noise` | the engine |
//! | computation | `compute`, `solve`, `sweep`, `coverage`    | the model       |
//! | state       | `state`                                    | the computation |
//! | view        | `ui`, `app`                                | the state       |
//!
//! Nothing in `ui` computes a physical quantity, and nothing below `state`
//! mentions egui.

pub mod antenna;
pub mod app;
pub mod chapman;
pub mod clock;
pub mod coastline;
pub mod compute;
pub mod coverage;
pub mod fof2;
pub mod grid;
/// Outbound HTTP, used only by the validation harnesses. Not built for the web
/// target, which has no business making these requests.
#[cfg(not(target_arch = "wasm32"))]
pub mod net;
pub mod noise;
pub mod scenario;
pub mod solar;
pub mod solve;
/// Observed solar indices, fetched rather than assumed.
#[cfg(not(target_arch = "wasm32"))]
pub mod spaceweather;
pub mod sporadic_e;
pub mod state;
pub mod sweep;
pub mod ui;
/// Browser entry point. A proof of concept: Windows is the primary target and
/// `main.rs` remains the default build. See the module docs for the two things
/// the browser does differently, both forced by wasm32 having no threads.
#[cfg(target_arch = "wasm32")]
pub mod web;
pub mod wspr;
pub mod wspr_report;
/// Live WSPR spot retrieval.
#[cfg(not(target_arch = "wasm32"))]
pub mod wsprlive;
