//! Editable state for one station's position.
//!
//! The authoritative position always lives in [`crate::scenario::Inputs`] as a
//! latitude/longitude pair - the map, the solver and the terminator all read it
//! from there. This type holds only what typing needs: the text the operator
//! has entered so far and which of the two notations they are entering it in.
//! Half-typed text stays in the buffer without disturbing the scenario, and a
//! map click can rewrite the buffers at any time.

use crate::grid;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LocationMode {
    /// Maidenhead locator, e.g. `IO91wm`.
    Grid,
    /// Decimal degrees.
    LatLon,
}

impl LocationMode {
    pub const ALL: [Self; 2] = [Self::Grid, Self::LatLon];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::LatLon => "Lat/Lon",
        }
    }
}

/// Text buffers for one station, plus the notation being used.
pub struct LocationEntry {
    pub mode: LocationMode,
    pub grid_text: String,
    pub lat_text: String,
    pub lon_text: String,
}

impl LocationEntry {
    #[must_use]
    pub fn new(lat_deg: f64, lon_deg: f64) -> Self {
        let mut entry = Self {
            mode: LocationMode::Grid,
            grid_text: String::new(),
            lat_text: String::new(),
            lon_text: String::new(),
        };
        entry.refresh(lat_deg, lon_deg);
        entry
    }

    /// Rewrite every buffer from authoritative coordinates. Called whenever the
    /// position changed somewhere else (a map click, or the other notation).
    pub fn refresh(&mut self, lat_deg: f64, lon_deg: f64) {
        self.grid_text = grid::encode(lat_deg, lon_deg, grid::ENCODED_PAIRS);
        self.lat_text = format!("{lat_deg:.4}");
        self.lon_text = format!("{lon_deg:.4}");
    }

    /// Coordinates implied by the grid buffer, if it is a valid locator.
    #[must_use]
    pub fn parsed_grid(&self) -> Option<(f64, f64)> {
        grid::decode(&self.grid_text)
    }

    /// Coordinates implied by the two decimal-degree buffers, if both parse and
    /// are in range.
    #[must_use]
    pub fn parsed_lat_lon(&self) -> Option<(f64, f64)> {
        let lat: f64 = self.lat_text.trim().parse().ok()?;
        let lon: f64 = self.lon_text.trim().parse().ok()?;
        ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
    }

    /// Is the text currently on screen valid in the active notation? Drives the
    /// red-text warning; an invalid buffer simply leaves the scenario alone.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        match self.mode {
            LocationMode::Grid => self.parsed_grid().is_some(),
            LocationMode::LatLon => self.parsed_lat_lon().is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_fills_both_notations_consistently() {
        let entry = LocationEntry::new(51.50, -0.13);
        assert_eq!(entry.grid_text, "IO91wm");
        assert_eq!(entry.lat_text, "51.5000");
        assert_eq!(entry.lon_text, "-0.1300");
        let (lat, lon) = entry.parsed_lat_lon().expect("its own output parses");
        assert!((lat - 51.5).abs() < 1e-9 && (lon + 0.13).abs() < 1e-9);
        // The grid is coarser, but must name the same place.
        let (glat, glon) = entry.parsed_grid().expect("its own output parses");
        assert!((glat - 51.5).abs() < 0.05 && (glon + 0.13).abs() < 0.1);
    }

    #[test]
    fn invalid_text_is_reported_not_clamped() {
        let mut entry = LocationEntry::new(0.0, 0.0);
        entry.grid_text = "ZZ99".to_string();
        assert!(entry.parsed_grid().is_none());
        assert!(!entry.is_valid());

        entry.mode = LocationMode::LatLon;
        entry.lat_text = "95".to_string();
        assert!(entry.parsed_lat_lon().is_none(), "latitude past the pole");
        entry.lat_text = "45".to_string();
        entry.lon_text = "-200".to_string();
        assert!(entry.parsed_lat_lon().is_none(), "longitude out of range");
        entry.lon_text = "-73.5".to_string();
        assert_eq!(entry.parsed_lat_lon(), Some((45.0, -73.5)));
        assert!(entry.is_valid());
    }
}
