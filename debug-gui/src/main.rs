//! Point-to-point debug harness for the Skipzone HF ray tracing engine.
//!
//! This crate contains no physics. It only calls the engine's public API
//! (`ChapmanLayer`, `Igrf`, `ExponentialCollisions`, `Tracer`, `Homing`) and
//! renders the results. The engine crate is untouched by design, and lives in
//! a separate workspace member so the GUI's dependency tree never reaches it.

mod app;
mod mapview;
mod panels;
mod scenario;
mod solar;
mod solve;

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
