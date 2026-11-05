//! Headless UI test: settings saved by one run show up when the next starts.

use std::collections::HashMap;

use eframe::Storage;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use skipzone_app::app::SkipzoneApp;
use skipzone_app::jobs::Jobs;
use skipzone_app::persist::{self, Saved};
use skipzone_app::state::AppState;
use skipzone_app::ui::scratch::{LocationMode, ViewScratch};
use skipzone_predict::scenario::Inputs;

#[derive(Default)]
struct Memory(HashMap<String, String>);

impl Storage for Memory {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

#[test]
fn saved_settings_appear_on_startup() {
    // What the previous run left behind: 7.074 MHz, the night shading off and
    // the transmitter typed in decimal degrees.
    let mut previous = AppState::with_scenario(Inputs::default());
    previous.inputs.scenario.freq_mhz = 7.074;
    previous.inputs.display.show_terminator = false;
    let mut scratch = ViewScratch::without_basemap(&previous);
    scratch.tx.mode = LocationMode::LatLon;
    let mut storage = Memory::default();
    persist::save(&mut storage, &previous, &scratch);
    assert!(Saved::decode(&storage.get_string(persist::KEY).unwrap()).is_some());

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .build_eframe(|cc| SkipzoneApp::restoring(cc, Jobs::inline(|| {}), Some(&storage)));
    harness.run();

    let frequency = harness
        .query_all_by_role(Role::SpinButton)
        .find(|n| n.value().is_some_and(|v| v.ends_with("MHz")))
        .expect("the overlay frequency field");
    let shown = frequency.value().unwrap_or_default();
    assert!(shown.starts_with("7.07"), "frequency field shows {shown:?}");

    let night = harness.get_by_role_and_label(Role::CheckBox, "Night");
    assert_eq!(night.accesskit_node().toggled(), Some(Toggled::False));

    // The TX row offers the lat/lon fields, the RX row the grid field.
    let tx_lat = format!("{:.4}", Inputs::default().tx_lat);
    let typed_lat = harness
        .query_all_by_role(Role::TextInput)
        .filter(|n| n.value().as_deref() == Some(tx_lat.as_str()))
        .count();
    assert_eq!(typed_lat, 1, "TX has a latitude field showing {tx_lat}");
}
