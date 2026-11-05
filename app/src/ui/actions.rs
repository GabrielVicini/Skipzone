//! What the interface can be asked to do, in one enum.
//!
//! Menu items, overlay buttons and dialog buttons all produce an [`Action`]
//! rather than mutating state where they are drawn. That keeps a command's
//! meaning in one place - "Calculate" always means *dispatch a solve and show
//! the trace* whether it was invoked from the menu or from the dialog - and
//! leaves the widgets themselves free of side effects. [`crate::update`]
//! applies them.
//!
//! Every edit to an input is one [`InputChange`], carried by
//! [`Action::SetInput`]. A view edits a copy of the current value and emits
//! the change only when the copy differs, so an untouched control emits
//! nothing.

use skipzone_iono::clock::CivilDate;
use skipzone_iono::fof2::Fof2Backend;
use skipzone_predict::antenna::AntennaConfig;
use skipzone_predict::coverage::CoverageConfig;
use skipzone_predict::noise::{NoiseEnvironment, OperatingMode};
use skipzone_predict::scenario::GroundType;

use crate::state::{AppInputs, Modal, Station};
use crate::ui::map::projection::Camera;

/// Let `edit` change a copy of `value`, and return the copy if it changed.
pub fn edited<T: Copy + PartialEq>(value: T, edit: impl FnOnce(&mut T)) -> Option<T> {
    let mut copy = value;
    edit(&mut copy);
    (copy != value).then_some(copy)
}

/// A change to one input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputChange {
    /// Move a station to a typed or clicked position.
    Position {
        station: Station,
        lat: f64,
        lon: f64,
    },
    /// The tuned frequency, MHz.
    Freq(f64),
    /// The operating mode. Also seeds the bandwidth and SNR threshold from
    /// the mode's preset; both stay editable in Settings.
    OpMode(OperatingMode),
    TxPower(f64),
    UtcHours(f64),
    Date(CivilDate),
    ShowTerminator(bool),
    // The Settings dialog, one variant per field it edits.
    Fof2Backend(Fof2Backend),
    Ssn(f64),
    Hmf2Km(f64),
    ScaleHeightKm(f64),
    EsEnabled(bool),
    EsManual(bool),
    FoesMhz(f64),
    EsProbability(f64),
    CollisionManual(bool),
    Nu0PerS(f64),
    NuRefAltKm(f64),
    NuScaleHeightKm(f64),
    NoiseEnv(NoiseEnvironment),
    BandwidthHz(f64),
    SnrThresholdDb(f64),
    UseField(bool),
    IgrfEpoch(f64),
    Ground(GroundType),
    GroundLandFallback(GroundType),
    MaxHops(u32),
    DomainTopKm(f64),
    ShowCoastlines(bool),
    /// The coverage grid's resolution and extent.
    CoverageGrid(CoverageConfig),
    /// Fill opacity of the coverage tiles.
    CoverageAlpha(u8),
    /// One end's antenna, from the Antennas dialog.
    Antenna {
        station: Station,
        config: AntennaConfig,
    },
}

impl InputChange {
    pub fn apply(self, inputs: &mut AppInputs) {
        let s = &mut inputs.scenario;
        match self {
            Self::Position { station, lat, lon } => match station {
                Station::Tx => (s.tx_lat, s.tx_lon) = (lat, lon),
                Station::Rx => (s.rx_lat, s.rx_lon) = (lat, lon),
            },
            Self::Freq(mhz) => s.freq_mhz = mhz,
            Self::OpMode(mode) => {
                s.op_mode = mode;
                (s.bandwidth_hz, s.snr_threshold_db) = mode.defaults();
            }
            Self::TxPower(watts) => s.tx_power_w = watts,
            Self::UtcHours(hours) => s.utc_hours = hours,
            Self::Date(date) => {
                (s.year, s.month, s.day_of_month) = (date.year, date.month, date.day);
            }
            Self::ShowTerminator(on) => inputs.display.show_terminator = on,
            Self::Fof2Backend(b) => s.fof2_backend = b,
            Self::Ssn(v) => s.ssn = v,
            Self::Hmf2Km(v) => s.hmf2_km = v,
            Self::ScaleHeightKm(v) => s.scale_height_km = v,
            Self::EsEnabled(on) => s.es_enabled = on,
            Self::EsManual(on) => s.es_manual = on,
            Self::FoesMhz(v) => s.foes_mhz = v,
            Self::EsProbability(v) => s.es_probability = v,
            Self::CollisionManual(on) => s.collision_manual = on,
            Self::Nu0PerS(v) => s.nu0_per_s = v,
            Self::NuRefAltKm(v) => s.nu_ref_alt_km = v,
            Self::NuScaleHeightKm(v) => s.nu_scale_height_km = v,
            Self::NoiseEnv(env) => s.noise_env = env,
            Self::BandwidthHz(v) => s.bandwidth_hz = v,
            Self::SnrThresholdDb(v) => s.snr_threshold_db = v,
            Self::UseField(on) => s.use_field = on,
            Self::IgrfEpoch(v) => s.igrf_epoch = v,
            Self::Ground(g) => s.ground_type = g,
            Self::GroundLandFallback(g) => s.ground_land_fallback = g,
            Self::MaxHops(n) => s.max_hops = n,
            Self::DomainTopKm(v) => s.domain_top_km = v,
            Self::ShowCoastlines(on) => inputs.display.show_coastlines = on,
            Self::CoverageGrid(config) => inputs.coverage = config,
            Self::CoverageAlpha(alpha) => inputs.display.coverage_alpha = alpha,
            Self::Antenna { station, config } => match station {
                Station::Tx => s.tx_antenna = config,
                Station::Rx => s.rx_antenna = config,
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Change one input.
    SetInput(InputChange),
    /// Move the station [`crate::state::UiState::place`] names to a map click.
    PlaceStation {
        lat: f64,
        lon: f64,
    },
    /// Choose which station a map click moves.
    SetPlaceMode(Station),
    /// Show or hide one solved mode on the map.
    SetSolutionVisible {
        index: usize,
        visible: bool,
    },
    /// Pick the mode the trace panel details.
    SelectSolution(usize),
    /// The trace panel's close button.
    CloseTrace,
    /// Run the point-to-point solve and show the trace readout.
    Calculate,
    /// Run the frequency sweep and show its charts.
    BestFrequency,
    /// The camera the map settled on this frame after pan, zoom or fit.
    /// Consumes the pending zoom, and the pending fit when `fitted`.
    SetCamera {
        camera: Camera,
        fitted: bool,
    },
    /// Re-frame the map on the current path.
    FitPath,
    ZoomIn,
    ZoomOut,
    /// Set the date to today, UTC.
    SetDateToday,
    /// Start the area coverage grid.
    RunCoverage,
    /// Stop the running grid, keeping every tile computed so far.
    CancelCoverage,
    /// Clear the coverage tiles off the map.
    ResetCoverage,
    OpenModal(Modal),
    /// A dialog's close button.
    CloseModal(Modal),
}
