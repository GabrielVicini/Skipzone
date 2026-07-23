//! Point-to-point debug harness for the Skipzone HF ray tracing engine.
//!
//! This crate calls the engine's public API (`ChapmanLayer`, `Igrf`,
//! `ExponentialCollisions`, `Tracer`, `Homing`) and renders the results. The
//! engine crate is untouched by design, and lives in a separate workspace
//! member so the GUI's dependency tree never reaches it.
//!
//! The one exception to "no physics here" is `dregion`: the day/night-aware
//! D-region absorbing layer (Chapman grazing function). Twilight ionisation
//! couples electron production to solar geometry, which is a scenario concern,
//! not a reusable engine primitive; it is derived and validated the same way
//! (docs/derivations/chapman-grazing.md) and only ever feeds the engine tracer
//! through the standard `ElectronDensity` trait.

mod app;
mod dregion;
mod mapview;
mod panels;
mod scenario;
mod solar;
mod solve;
mod sweep;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1700.0, 980.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Skipzone - point-to-point ray trace debug",
        options,
        Box::new(|cc| Ok(Box::new(app::DebugApp::new(cc)))),
    )
}
