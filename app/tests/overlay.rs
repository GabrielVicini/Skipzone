//! Headless UI test: an overlay edit reaches the inputs through an action.
//!
//! The overlay draws from `&AppState`, so the only way its edit can reach the
//! inputs is the action it emits.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, Node};
use skipzone_app::app::SkipzoneApp;
use skipzone_app::jobs::Jobs;
use skipzone_predict::scenario::Inputs;

#[test]
fn typing_a_frequency_into_the_overlay_tunes_the_scenario() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_eframe(|cc| SkipzoneApp::with_jobs(cc, Jobs::inline(|| {})));
    harness.state_mut().state_mut().inputs.scenario = Inputs::default();
    harness.run();
    let before = harness.state().state().inputs.scenario.clone();
    assert!((before.freq_mhz - 14.1).abs() < 1e-9, "default is 14.1 MHz");

    // A click on a drag value starts text editing.
    frequency_field(&harness).click();
    harness.run();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    harness.run();
    // Typed text goes to whichever field has focus: the one just clicked.
    harness.event(egui::Event::Text("7.074".to_string()));
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();

    let after = &harness.state().state().inputs.scenario;
    assert!(
        (after.freq_mhz - 7.074).abs() < 1e-9,
        "tuned to {} MHz",
        after.freq_mhz
    );
    // Nothing else in the scenario moved.
    let expected = Inputs {
        freq_mhz: after.freq_mhz,
        ..before
    };
    assert_eq!(*after, expected);
    assert!(
        frequency_field(&harness)
            .value()
            .is_some_and(|v| v.starts_with("7.07"))
    );
}

/// The overlay's frequency field: the only spin button showing MHz while every
/// dialog is closed.
fn frequency_field<'a>(harness: &'a Harness<'_, SkipzoneApp>) -> Node<'a> {
    harness
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.value().is_some_and(|v| v.ends_with("MHz")))
        .expect("the overlay frequency field")
}
