//! View scratch: what the views remember between frames that is neither an
//! input nor a result.
//!
//! Views read [`crate::state::AppState`] and never write it, so anything a
//! widget has to keep across frames lives here instead: half-typed text, the
//! open menu, calendar paging, last frame's layout sizes and the basemap
//! texture. It is an explicit struct rather than egui temp memory so it stays
//! visible and testable. Nothing here feeds the solver: a buffer reaches the
//! scenario only through an action.

mod location;

use egui::TextureHandle;
use skipzone_iono::clock::{self, CivilDate};

use crate::state::AppState;
use crate::ui::map::basemap;

pub use location::{LocationEntry, LocationMode};

/// The three top-bar menus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Menu {
    PointToPoint,
    CoverageMaps,
    Help,
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

pub struct ViewScratch {
    /// Entry buffers and notation for each station.
    pub tx: LocationEntry,
    pub rx: LocationEntry,
    /// Editable `HH:MM` buffer for the UTC time control.
    pub time_text: String,
    /// Editable `YYYY-MM-DD` buffer for the date control.
    pub date_text: String,
    pub calendar: CalendarState,
    /// Which top-bar menu is open, if any.
    pub menu: Option<Menu>,
    /// Width the trace panel took last frame, or 0 when it is closed. The
    /// floating overlays inset their right edge by it so the panel pushes them
    /// aside instead of covering them.
    pub right_inset: f32,
    /// Height of the solid header, measured last frame. The floating overlays
    /// use it to start just below the bar without hard-coding its size.
    pub header_height: f32,
    /// Window width the text styles were last scaled for.
    pub styled_for_width: f32,
    /// The rasterised world, built once. A render cache: `None` only if the
    /// coastline data could not be read, and then the map draws as open ocean.
    pub basemap: Option<TextureHandle>,
}

impl ViewScratch {
    /// Buffers filled from `state`, and the basemap built on `ctx`.
    #[must_use]
    pub fn new(ctx: &egui::Context, state: &AppState) -> Self {
        let mut scratch = Self::without_basemap(state);
        scratch.basemap = basemap::load(ctx);
        scratch
    }

    /// Buffers filled from `state`, with no basemap. For tests that do not
    /// draw the map.
    #[must_use]
    pub fn without_basemap(state: &AppState) -> Self {
        let inputs = &state.inputs.scenario;
        let date = state.inputs.date();
        Self {
            tx: LocationEntry::new(inputs.tx_lat, inputs.tx_lon),
            rx: LocationEntry::new(inputs.rx_lat, inputs.rx_lon),
            time_text: clock::format_hours(inputs.utc_hours),
            date_text: clock::format_date(date),
            calendar: CalendarState::new(date),
            menu: None,
            right_inset: 0.0,
            header_height: 0.0,
            styled_for_width: 0.0,
            basemap: None,
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
