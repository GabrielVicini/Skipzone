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
use skipzone_predict::noise::OperatingMode;

use crate::state::{AppInputs, Station};

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
    /// Run the point-to-point solve and show the trace readout.
    Calculate,
    /// Run the frequency sweep and show its charts.
    BestFrequency,
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
    ShowCoverage,
    ShowSettings,
    ShowAntennas,
    ShowAbout,
}
