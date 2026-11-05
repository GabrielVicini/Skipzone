//! Image snapshots of the key screens, rendered with wgpu.
//!
//! Built only with `--features snapshots`. Each test pins the clock and the
//! scenario, so the night shading and the time label are the same every run.
//! References live in `tests/snapshots/`. To update them after a deliberate
//! change, run with `UPDATE_SNAPSHOTS=1` and commit the new images.

use eframe::egui_wgpu::RendererOptions;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions};
use skipzone_app::app::SkipzoneApp;
use skipzone_app::jobs::Jobs;
use skipzone_app::state::{Clock, Modal};
use skipzone_iono::clock::CivilDate;
use skipzone_predict::scenario::Inputs;

/// The default scenario's own date and hour.
fn pinned_clock() -> Clock {
    let s = Inputs::default();
    Clock::Fixed(CivilDate::new(s.year, s.month, s.day_of_month), s.utc_hours)
}

/// The app on a fixed clock, inline jobs and no saved settings.
fn harness() -> Harness<'static, SkipzoneApp> {
    // The predictable preset filters textures in the shader and clamps at the
    // edge, which ignores the basemap's repeat wrap. The hardware sampler
    // keeps the wrap, and the threshold absorbs its small differences.
    let render = RendererOptions {
        predictable_texture_filtering: false,
        ..RendererOptions::PREDICTABLE
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .with_render_options(render)
        .wgpu()
        .build_eframe(|cc| {
            let ctx = cc.egui_ctx.clone();
            let jobs = Jobs::inline(move || ctx.request_repaint());
            SkipzoneApp::build(cc, jobs, None, pinned_clock())
        });
    harness.run();
    harness
}

/// Small rasterisation differences between drivers pass. A changed label or
/// a moved widget does not.
fn options() -> SnapshotOptions {
    SnapshotOptions::new().threshold(0.6).max_failed_pixels(100)
}

#[test]
fn main_window() {
    let mut harness = harness();
    harness.snapshot_options("main_window", &options());
}

#[test]
fn point_to_point_menu() {
    let mut harness = harness();
    harness.get_by_label("Point-to-Point").hover();
    harness.run();
    harness.snapshot_options("point_to_point_menu", &options());
}

#[test]
fn settings_dialog() {
    let mut harness = harness();
    harness
        .state_mut()
        .state_mut()
        .ui
        .modals
        .set(Modal::Settings, true);
    harness.run();
    harness.snapshot_options("settings_dialog", &options());
}

#[test]
fn trace_panel_with_a_result() {
    let mut harness = harness();
    harness.get_by_label("Point-to-Point").hover();
    harness.run();
    harness
        .get_by_role_and_label(Role::Button, "Calculate")
        .click();
    for _ in 0..10 {
        harness.run();
        if harness.state().state().results.main.outcome.is_some() {
            break;
        }
    }
    // The wall time differs every run; pin it.
    if let Some(out) = harness
        .state_mut()
        .state_mut()
        .results
        .main
        .outcome
        .as_mut()
    {
        out.elapsed_ms = 0.0;
    }
    // Move the pointer off the menu so it closes.
    harness.hover_at(egui::pos2(640.0, 600.0));
    harness.run();
    harness.snapshot_options("trace_panel", &options());
}
