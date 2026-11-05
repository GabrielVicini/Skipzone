//! Headless UI test: one frame that emits two actions applies both.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use skipzone_app::app::SkipzoneApp;
use skipzone_app::jobs::Jobs;

#[test]
fn a_frame_with_two_actions_applies_both() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_eframe(|cc| SkipzoneApp::with_jobs(cc, Jobs::inline(|| {})));
    harness.run();
    assert!(harness.state().state().inputs.display.show_terminator);

    // Ask for a fit, so the next frame's map emits a camera, then click the
    // Night box, which the overlay drawn after the map reports in the same
    // frame.
    harness.state_mut().state_mut().ui.map.needs_fit = true;
    harness
        .get_by_role_and_label(Role::CheckBox, "Night")
        .click();
    harness.step();

    let state = harness.state().state();
    assert!(!state.ui.map.needs_fit, "the map's camera was applied");
    assert!(
        !state.inputs.display.show_terminator,
        "the Night toggle was applied"
    );
}
