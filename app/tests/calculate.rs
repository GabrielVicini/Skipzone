//! Headless UI test: drive the real app through egui_kittest, from the menu
//! to a solved path in the trace panel.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use skipzone_app::app::SkipzoneApp;
use skipzone_app::jobs::Jobs;
use skipzone_predict::scenario::Inputs;

#[test]
fn calculate_from_the_menu_shows_the_modes_found() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_eframe(|cc| {
            let ctx = cc.egui_ctx.clone();
            // Inline jobs: the solve runs inside the frame that dispatches it,
            // so the test needs no waiting on another thread.
            SkipzoneApp::with_jobs(cc, Jobs::inline(move || ctx.request_repaint()))
        });
    // A fixed scenario: the app otherwise starts at today's date and time.
    harness.state_mut().state_mut().inputs.scenario = Inputs::default();
    harness.run();

    harness.get_by_label("Point-to-Point").hover();
    harness.run();

    // The trace panel is closed, so the menu item is the only Calculate
    // button. Once the panel opens it has a Calculate button of its own.
    let menu_items = harness.query_all_by_role_and_label(Role::Button, "Calculate");
    assert_eq!(menu_items.count(), 1);
    harness
        .get_by_role_and_label(Role::Button, "Calculate")
        .click();

    // Inline jobs queue the response during the click's frame; the next
    // frames apply it and draw the panel.
    for _ in 0..10 {
        harness.run();
        if harness.state().state().results.main.outcome.is_some() {
            break;
        }
    }
    assert!(
        !harness.state().state().results.is_busy(),
        "the solve finished"
    );
    harness.run();

    let heading = harness.get_by_label_contains("Modes found (");
    let label = heading.accesskit_node().label().unwrap_or_default();
    let n: usize = label
        .trim_start_matches("Modes found (")
        .trim_end_matches(')')
        .parse()
        .unwrap_or_else(|_| panic!("unexpected heading {label:?}"));
    assert!(
        n >= 1,
        "the default scenario finds at least one mode, got {label:?}"
    );
    assert_eq!(
        n,
        harness.state().state().results.main.solutions().len(),
        "the panel shows the solve's mode count"
    );
}
