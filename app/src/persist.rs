//! What the app remembers between runs, and the one place it is read and
//! written.
//!
//! [`Saved`] lists every stored field by name: the model settings, the display
//! preferences, both antennas, the coverage grid and the last session (the
//! stations, their notation, the frequency, the mode and the power). The UTC
//! date and time are left out, so a run still starts at "now". Results and
//! open dialogs are left out too.
//!
//! The blob is RON under [`KEY`] in eframe's storage. It carries a
//! [`VERSION`]: a blob from another version, or one that does not decode,
//! is ignored and the app starts from its defaults.

use eframe::Storage;
use serde::{Deserialize, Serialize};
use skipzone_iono::fof2::Fof2Backend;
use skipzone_predict::antenna::AntennaConfig;
use skipzone_predict::coverage::CoverageConfig;
use skipzone_predict::noise::{NoiseEnvironment, OperatingMode};
use skipzone_predict::scenario::{GroundType, Inputs};

use crate::state::{AppState, Display};
use crate::ui::scratch::{LocationMode, ViewScratch};

/// The storage key the settings live under.
pub const KEY: &str = "skipzone";

/// Bump this whenever [`Saved`] changes shape. Older blobs are then dropped.
pub const VERSION: u32 = 1;

/// Everything the Settings dialog edits, apart from the mode and power, which
/// the session block holds.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct ModelSettings {
    pub fof2_backend: Fof2Backend,
    pub ssn: f64,
    pub hmf2_km: f64,
    pub scale_height_km: f64,
    pub es_enabled: bool,
    pub es_manual: bool,
    pub foes_mhz: f64,
    pub es_probability: f64,
    pub collision_manual: bool,
    pub nu0_per_s: f64,
    pub nu_ref_alt_km: f64,
    pub nu_scale_height_km: f64,
    pub noise_env: NoiseEnvironment,
    pub bandwidth_hz: f64,
    pub snr_threshold_db: f64,
    pub use_field: bool,
    pub igrf_epoch: f64,
    pub ground_type: GroundType,
    pub ground_land_fallback: GroundType,
    pub max_hops: u32,
    pub domain_top_km: f64,
}

/// The path the operator was last looking at.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Session {
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
    pub tx_notation: LocationMode,
    pub rx_notation: LocationMode,
    pub freq_mhz: f64,
    pub op_mode: OperatingMode,
    pub tx_power_w: f64,
}

/// The stored blob.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    pub model: ModelSettings,
    pub display: Display,
    pub tx_antenna: AntennaConfig,
    pub rx_antenna: AntennaConfig,
    pub coverage: CoverageConfig,
    pub session: Session,
}

/// Just the version, read first so a blob from another version is dropped
/// without trying to decode the rest.
#[derive(Deserialize)]
struct Header {
    version: u32,
}

impl Saved {
    /// The persisted part of the current state.
    #[must_use]
    pub fn capture(state: &AppState, scratch: &ViewScratch) -> Self {
        let s = &state.inputs.scenario;
        Self {
            version: VERSION,
            model: ModelSettings {
                fof2_backend: s.fof2_backend,
                ssn: s.ssn,
                hmf2_km: s.hmf2_km,
                scale_height_km: s.scale_height_km,
                es_enabled: s.es_enabled,
                es_manual: s.es_manual,
                foes_mhz: s.foes_mhz,
                es_probability: s.es_probability,
                collision_manual: s.collision_manual,
                nu0_per_s: s.nu0_per_s,
                nu_ref_alt_km: s.nu_ref_alt_km,
                nu_scale_height_km: s.nu_scale_height_km,
                noise_env: s.noise_env,
                bandwidth_hz: s.bandwidth_hz,
                snr_threshold_db: s.snr_threshold_db,
                use_field: s.use_field,
                igrf_epoch: s.igrf_epoch,
                ground_type: s.ground_type,
                ground_land_fallback: s.ground_land_fallback,
                max_hops: s.max_hops,
                domain_top_km: s.domain_top_km,
            },
            display: state.inputs.display,
            tx_antenna: s.tx_antenna,
            rx_antenna: s.rx_antenna,
            coverage: state.inputs.coverage,
            session: Session {
                tx_lat: s.tx_lat,
                tx_lon: s.tx_lon,
                rx_lat: s.rx_lat,
                rx_lon: s.rx_lon,
                tx_notation: scratch.tx.mode,
                rx_notation: scratch.rx.mode,
                freq_mhz: s.freq_mhz,
                op_mode: s.op_mode,
                tx_power_w: s.tx_power_w,
            },
        }
    }

    /// Put the saved fields back. The date, time and every other input keep
    /// their current values.
    pub fn restore(&self, state: &mut AppState, scratch: &mut ViewScratch) {
        let m = &self.model;
        let n = &self.session;
        let s = &mut state.inputs.scenario;
        *s = Inputs {
            fof2_backend: m.fof2_backend,
            ssn: m.ssn,
            hmf2_km: m.hmf2_km,
            scale_height_km: m.scale_height_km,
            es_enabled: m.es_enabled,
            es_manual: m.es_manual,
            foes_mhz: m.foes_mhz,
            es_probability: m.es_probability,
            collision_manual: m.collision_manual,
            nu0_per_s: m.nu0_per_s,
            nu_ref_alt_km: m.nu_ref_alt_km,
            nu_scale_height_km: m.nu_scale_height_km,
            noise_env: m.noise_env,
            bandwidth_hz: m.bandwidth_hz,
            snr_threshold_db: m.snr_threshold_db,
            use_field: m.use_field,
            igrf_epoch: m.igrf_epoch,
            ground_type: m.ground_type,
            ground_land_fallback: m.ground_land_fallback,
            max_hops: m.max_hops,
            domain_top_km: m.domain_top_km,
            tx_antenna: self.tx_antenna,
            rx_antenna: self.rx_antenna,
            tx_lat: n.tx_lat,
            tx_lon: n.tx_lon,
            rx_lat: n.rx_lat,
            rx_lon: n.rx_lon,
            freq_mhz: n.freq_mhz,
            // Set directly: going through the mode preset would overwrite the
            // saved bandwidth and threshold.
            op_mode: n.op_mode,
            tx_power_w: n.tx_power_w,
            ..s.clone()
        };
        state.inputs.display = self.display;
        state.inputs.coverage = self.coverage;
        scratch.tx.mode = n.tx_notation;
        scratch.rx.mode = n.rx_notation;
        scratch.tx.refresh(n.tx_lat, n.tx_lon);
        scratch.rx.refresh(n.rx_lat, n.rx_lon);
    }

    /// The blob as stored.
    #[must_use]
    pub fn encode(&self) -> Option<String> {
        ron::ser::to_string(self).ok()
    }

    /// Read a stored blob. `None` for another version or anything that does
    /// not decode.
    #[must_use]
    pub fn decode(text: &str) -> Option<Self> {
        let version = match ron::from_str::<Header>(text) {
            Ok(header) => header.version,
            Err(err) => return note(&format!("cannot read the saved settings: {err}")),
        };
        if version != VERSION {
            return note(&format!(
                "saved settings are version {version}, expected {VERSION}"
            ));
        }
        match ron::from_str(text) {
            Ok(saved) => Some(saved),
            Err(err) => note(&format!("cannot read the saved settings: {err}")),
        }
    }
}

/// Write the persisted part of the state to `storage`.
pub fn save(storage: &mut dyn Storage, state: &AppState, scratch: &ViewScratch) {
    if let Some(text) = Saved::capture(state, scratch).encode() {
        storage.set_string(KEY, text);
    }
}

/// Restore what `storage` holds, if it holds anything usable. Otherwise
/// leave the defaults alone.
pub fn load(storage: &dyn Storage, state: &mut AppState, scratch: &mut ViewScratch) {
    if let Some(saved) = storage.get_string(KEY).as_deref().and_then(Saved::decode) {
        saved.restore(state, scratch);
    }
}

/// A stale or broken blob is not an error: the app starts from defaults. Debug
/// builds say why on stderr.
fn note(why: &str) -> Option<Saved> {
    if cfg!(debug_assertions) {
        eprintln!("skipzone: {why}; starting from the defaults");
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skipzone_predict::antenna::AntennaKind;

    use super::*;

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

    fn fresh() -> (AppState, ViewScratch) {
        let state = AppState::with_scenario(Inputs::default());
        let scratch = ViewScratch::without_basemap(&state);
        (state, scratch)
    }

    /// A state with every persisted field moved off its default.
    fn edited() -> (AppState, ViewScratch) {
        let (mut state, mut scratch) = fresh();
        let s = &mut state.inputs.scenario;
        s.fof2_backend = Fof2Backend::Gridded;
        s.ssn = 120.0;
        s.hmf2_km = 320.0;
        s.scale_height_km = 45.0;
        s.es_enabled = false;
        s.es_manual = true;
        s.foes_mhz = 6.5;
        s.es_probability = 0.3;
        s.collision_manual = true;
        s.nu0_per_s = 4.0e6;
        s.nu_ref_alt_km = 72.0;
        s.nu_scale_height_km = 7.0;
        s.noise_env = NoiseEnvironment::City;
        s.op_mode = OperatingMode::Ft8;
        s.bandwidth_hz = 60.0;
        s.snr_threshold_db = -5.0;
        s.use_field = false;
        s.igrf_epoch = 2025.0;
        s.ground_type = GroundType::SeaWater;
        s.ground_land_fallback = GroundType::DryGround;
        s.max_hops = 2;
        s.domain_top_km = 600.0;
        s.tx_antenna.kind = AntennaKind::Efhw;
        s.tx_antenna.efhw_design_mhz = 3.5;
        s.rx_antenna.kind = AntennaKind::VerticalMonopole;
        s.rx_antenna.height_m = 2.0;
        (s.tx_lat, s.tx_lon) = (45.0, 7.0);
        (s.rx_lat, s.rx_lon) = (-33.9, 151.2);
        s.freq_mhz = 7.074;
        s.tx_power_w = 5.0;
        state.inputs.display = Display {
            show_terminator: false,
            show_coastlines: true,
            coverage_alpha: 0x80,
        };
        state.inputs.coverage = CoverageConfig {
            half_span_deg: 20.0,
            points_per_deg: 1.0,
        };
        scratch.tx.mode = LocationMode::LatLon;
        (state, scratch)
    }

    #[test]
    fn save_then_load_restores_every_persisted_field() {
        let (state, scratch) = edited();
        let mut storage = Memory::default();
        save(&mut storage, &state, &scratch);

        let (mut loaded, mut loaded_scratch) = fresh();
        // Restoring must leave the clock alone.
        loaded.inputs.scenario.utc_hours = 3.5;
        loaded.inputs.scenario.day_of_month = 2;
        load(&storage, &mut loaded, &mut loaded_scratch);

        assert_eq!(
            Saved::capture(&loaded, &loaded_scratch),
            Saved::capture(&state, &scratch)
        );
        let s = &loaded.inputs.scenario;
        assert_eq!((s.utc_hours, s.day_of_month), (3.5, 2));
        // The text buffers follow the restored position.
        assert_eq!(loaded_scratch.tx.lat_text, "45.0000");
        let rx = crate::ui::scratch::LocationEntry::new(-33.9, 151.2);
        assert_eq!(loaded_scratch.rx.grid_text, rx.grid_text);
        // Bandwidth and threshold are the saved ones, not the FT8 preset.
        assert_eq!(s.bandwidth_hz, 60.0);
    }

    #[test]
    fn a_broken_or_old_blob_leaves_the_defaults() {
        let (state, scratch) = edited();
        let good = Saved::capture(&state, &scratch).encode().unwrap();
        let old = good.replacen(&format!("version:{VERSION}"), "version:0", 1);
        assert_ne!(old, good, "the version field is where the test expects");
        let truncated = &good[..good.len() / 2];

        for blob in ["", "not ron at all", truncated, &old, "(version:1)"] {
            let mut storage = Memory::default();
            storage.set_string(KEY, blob.to_string());
            let (mut loaded, mut loaded_scratch) = fresh();
            let (defaults, default_scratch) = fresh();
            load(&storage, &mut loaded, &mut loaded_scratch);
            assert_eq!(
                Saved::capture(&loaded, &loaded_scratch),
                Saved::capture(&defaults, &default_scratch),
                "blob {blob:?}"
            );
        }
    }
}
