//! View state: everything the interface remembers between frames that is not
//! part of the scenario or its results.
//!
//! Kept beside [`super::Session`] rather than inside the widgets so that any
//! part of the UI can open a dialog or read the entry buffers without threading
//! flags through call chains. It holds no egui types: the widgets take these
//! fields by reference and are otherwise stateless.

use crate::clock::{self, CivilDate};
use crate::scenario::PlaceMode;

use super::location::LocationEntry;

/// The three top-bar menus.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    PointToPoint,
    CoverageMaps,
    Help,
}

/// Which dialogs are open. They are ordinary windows, not modals: several can
/// be open at once and the rest of the interface stays live behind them.
#[derive(Default)]
pub struct Modals {
    /// Point-to-Point > Best FREQ: the sweep charts.
    pub best_freq: bool,
    /// Help > About.
    pub about: bool,
    pub settings: bool,
    pub antennas: bool,
}

/// Month shown by the date picker, independent of the selected date so that
/// paging through months does not change the scenario.
pub struct CalendarState {
    pub open: bool,
    pub year: i32,
    pub month: u32,
}

impl CalendarState {
    fn new(date: CivilDate) -> Self {
        Self {
            open: false,
            year: date.year,
            month: date.month,
        }
    }

    /// Point the picker at a date without opening it.
    pub fn show_month_of(&mut self, date: CivilDate) {
        self.year = date.year;
        self.month = date.month;
    }

    /// Step by whole months, carrying the year.
    pub fn step_month(&mut self, delta: i32) {
        let zero_based = i32::try_from(self.month).unwrap_or(1) - 1 + delta;
        self.year += zero_based.div_euclid(12);
        self.month = u32::try_from(zero_based.rem_euclid(12) + 1).unwrap_or(1);
    }
}

pub struct UiState {
    pub tx: LocationEntry,
    pub rx: LocationEntry,
    /// Which station a map click moves.
    pub place: PlaceMode,
    /// Editable `HH:MM` buffer for the UTC time control.
    pub time_text: String,
    /// Editable `YYYY-MM-DD` buffer for the date control.
    pub date_text: String,
    pub calendar: CalendarState,
    /// Which top-bar menu is open, if any.
    pub menu: Option<Menu>,
    pub modals: Modals,
    /// Point-to-Point > Calculate: the trace readout, shown as a docked side
    /// panel rather than a window so it never covers the map.
    pub trace_open: bool,
    /// Width the trace panel took last frame, or 0 when it is closed. The
    /// floating overlays inset their right edge by it so the panel pushes them
    /// aside instead of covering them.
    pub right_inset: f32,
    /// Draw the live day/night terminator shading on the map.
    pub show_terminator: bool,
    /// Window width the text styles were last scaled for.
    pub styled_for_width: f32,
    /// Height of the solid header, measured last frame. The floating overlays
    /// use it to start just below the bar without hard-coding its size.
    pub header_height: f32,
}

impl UiState {
    #[must_use]
    pub fn new(session: &super::Session) -> Self {
        let inputs = &session.inputs;
        let date = session.date();
        Self {
            tx: LocationEntry::new(inputs.tx_lat, inputs.tx_lon),
            rx: LocationEntry::new(inputs.rx_lat, inputs.rx_lon),
            place: PlaceMode::Tx,
            time_text: clock::format_hours(inputs.utc_hours),
            date_text: clock::format_date(date),
            calendar: CalendarState::new(date),
            menu: None,
            modals: Modals::default(),
            trace_open: false,
            right_inset: 0.0,
            show_terminator: true,
            styled_for_width: 0.0,
            header_height: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_months_carries_the_year_in_both_directions() {
        let mut cal = CalendarState::new(CivilDate::new(2026, 1, 15));
        cal.step_month(-1);
        assert_eq!((cal.year, cal.month), (2025, 12));
        cal.step_month(1);
        assert_eq!((cal.year, cal.month), (2026, 1));
        cal.step_month(11);
        assert_eq!((cal.year, cal.month), (2026, 12));
        cal.step_month(1);
        assert_eq!((cal.year, cal.month), (2027, 1));
        cal.step_month(-14);
        assert_eq!((cal.year, cal.month), (2025, 11));
    }
}
