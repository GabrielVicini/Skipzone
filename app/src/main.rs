//! Point-to-point application for the Skipzone HF ray tracing engine.
//!
//! This crate calls the engine's public API (`ChapmanLayer`, `Igrf`,
//! `ExponentialCollisions`, `Tracer`, `Homing`) and renders the results. The
//! engine crate is untouched by design, and lives in a separate workspace
//! member so the app's dependency tree never reaches it.
//!
//! The one exception to "no physics here" is `dregion`: the day/night-aware
//! D-region absorbing layer (Chapman grazing function). Twilight ionisation
//! couples electron production to solar geometry, which is a scenario concern,
//! not a reusable engine primitive; it is derived and validated the same way
//! (docs/derivations/chapman-grazing.md) and only ever feeds the engine tracer
//! through the standard `ElectronDensity` trait.
//!
//! Layered by concern, with each layer depending only on the ones above it:
//!
//! | layer       | modules                              | knows about        |
//! |-------------|--------------------------------------|--------------------|
//! | conversion  | `clock`, `grid`, `solar`, `coastline`| nothing else       |
//! | model       | `scenario`, `dregion`, `noise`       | the engine         |
//! | computation | `compute`, `solve`, `sweep`          | the model          |
//! | state       | `state`                              | the computation    |
//! | view        | `ui`, `app`                          | the state          |
//!
//! Nothing in `ui` computes a physical quantity, and nothing below `state`
//! mentions egui.

mod antenna;
mod app;
mod clock;
mod coastline;
mod compute;
mod dregion;
mod grid;
mod noise;
mod scenario;
mod solar;
mod solve;
mod state;
mod sweep;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1700.0, 980.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Skipzone - point-to-point HF ray tracing",
        options,
        Box::new(|cc| Ok(Box::new(app::SkipzoneApp::new(cc)))),
    )
}
