//! Binary entry point for the Skipzone point-to-point application. Everything
//! it drives lives in the library root next to it (`lib.rs`).

use skipzone_app::app;

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
