//! Solar geometry for the path midpoint: declination, hour angle, and solar
//! zenith angle.
//!
//! Formulas used, and how much each is worth trusting:
//!
//! * **Declination** — Cooper's equation (P. I. Cooper, "The absorption of
//!   solar radiation in solar stills", *Solar Energy* 12(3), 1969):
//!   `delta = 23.45 deg * sin(360 deg * (284 + n) / 365)`, with `n` the day of
//!   year. This is the standard low-precision approximation used throughout
//!   the solar-engineering literature; it is accurate to a few tenths of a
//!   degree, which is far below the accuracy of anything else in this tool.
//! * **Hour angle** — `H = 15 deg/h * (LST - 12 h)`, exact by the definition
//!   of mean solar time, with `LST = UTC + longitude/15`.
//! * **Zenith angle** — `cos(chi) = sin(phi) sin(delta) + cos(phi) cos(delta) cos(H)`,
//!   the spherical law of cosines applied to the observer/pole/sun triangle.
//!   Exact given delta and H.
//!
//! Deliberately omitted: the equation of time (apparent minus mean solar
//! time, up to about +/-16 minutes, i.e. +/-4 deg of hour angle) and
//! atmospheric refraction. Both are small next to the coarse ionospheric
//! climatology this feeds, and adding them would imply a precision the rest
//! of the model does not have.

/// Cumulative days before the start of each month, non-leap year. Leap years
/// are not modelled: one day of offset moves the declination by under 0.4 deg.
const DAYS_BEFORE_MONTH: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

#[derive(Clone, Copy)]
pub struct SolarGeometry {
    pub day_of_year: u32,
    pub declination_deg: f64,
    pub local_solar_time_h: f64,
    pub hour_angle_deg: f64,
    pub zenith_angle_deg: f64,
    /// Solar elevation above the horizon, `90 - chi`.
    pub elevation_deg: f64,
}

impl SolarGeometry {
    /// True when the sun is above the horizon at this point and time.
    pub fn is_day(&self) -> bool {
        self.zenith_angle_deg < 90.0
    }
}

/// Day of year for a month (1..=12) and day of month (1..=31), non-leap.
pub fn day_of_year(month: u32, day_of_month: u32) -> u32 {
    let m = month.clamp(1, 12) as usize - 1;
    DAYS_BEFORE_MONTH[m] + day_of_month.clamp(1, 31)
}

/// Solar geometry at a geographic point for a date and UTC time.
pub fn solar_geometry(
    lat_deg: f64,
    lon_deg: f64,
    month: u32,
    day_of_month: u32,
    utc_hours: f64,
) -> SolarGeometry {
    let n = day_of_year(month, day_of_month);
    // Cooper 1969.
    let decl_deg = 23.45 * (360.0 * (284.0 + f64::from(n)) / 365.0).to_radians().sin();

    let lst = (utc_hours + lon_deg / 15.0).rem_euclid(24.0);
    let hour_angle_deg = 15.0 * (lst - 12.0);

    let (phi, delta, h) = (
        lat_deg.to_radians(),
        decl_deg.to_radians(),
        hour_angle_deg.to_radians(),
    );
    let cos_chi = (phi.sin() * delta.sin() + phi.cos() * delta.cos() * h.cos()).clamp(-1.0, 1.0);
    let chi_deg = cos_chi.acos().to_degrees();

    SolarGeometry {
        day_of_year: n,
        declination_deg: decl_deg,
        local_solar_time_h: lst,
        hour_angle_deg,
        zenith_angle_deg: chi_deg,
        elevation_deg: 90.0 - chi_deg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_of_year_boundaries() {
        assert_eq!(day_of_year(1, 1), 1);
        assert_eq!(day_of_year(2, 1), 32);
        assert_eq!(day_of_year(3, 1), 60);
        assert_eq!(day_of_year(12, 31), 365);
    }

    /// Equinox declination is near zero; solstices near +/-23.45 deg.
    #[test]
    fn declination_matches_known_dates() {
        // Cooper's form gives exactly zero at n = 81 (about 22 March).
        let eq = solar_geometry(0.0, 0.0, 3, 22, 12.0);
        assert!(
            eq.declination_deg.abs() < 0.5,
            "equinox decl {}",
            eq.declination_deg
        );
        let jun = solar_geometry(0.0, 0.0, 6, 21, 12.0);
        assert!(
            (jun.declination_deg - 23.45).abs() < 0.5,
            "June solstice decl {}",
            jun.declination_deg
        );
        let dec = solar_geometry(0.0, 0.0, 12, 21, 12.0);
        assert!(
            (dec.declination_deg + 23.45).abs() < 0.6,
            "December solstice decl {}",
            dec.declination_deg
        );
    }

    /// At the equator on an equinox at local noon the sun is overhead.
    #[test]
    fn overhead_sun_at_equator_equinox_noon() {
        let g = solar_geometry(0.0, 0.0, 3, 22, 12.0);
        assert!(g.zenith_angle_deg < 1.0, "chi {}", g.zenith_angle_deg);
        assert!(g.is_day());
    }

    /// Local midnight is always night; the antipodal longitude at the same
    /// instant is day.
    #[test]
    fn midnight_is_night_and_longitude_shifts_it() {
        let midnight = solar_geometry(40.0, 0.0, 6, 21, 0.0);
        assert!(!midnight.is_day(), "chi {}", midnight.zenith_angle_deg);
        let noon = solar_geometry(40.0, 180.0, 6, 21, 0.0);
        assert!(noon.is_day(), "chi {}", noon.zenith_angle_deg);
    }

    /// Hour angle is zero at local solar noon regardless of longitude.
    #[test]
    fn hour_angle_zero_at_local_noon() {
        for lon in [-150.0_f64, -60.0, 0.0, 75.0, 160.0] {
            let utc = (12.0 - lon / 15.0).rem_euclid(24.0);
            let g = solar_geometry(20.0, lon, 4, 15, utc);
            assert!(
                g.hour_angle_deg.abs() < 1e-9,
                "lon {lon}: hour angle {}",
                g.hour_angle_deg
            );
        }
    }

    /// Winter at high northern latitude is darker than summer at the same
    /// hour: a basic monotonicity the seasonal term must satisfy.
    #[test]
    fn winter_is_darker_than_summer_at_high_latitude() {
        let summer = solar_geometry(55.0, 0.0, 6, 21, 12.0);
        let winter = solar_geometry(55.0, 0.0, 12, 21, 12.0);
        assert!(
            summer.zenith_angle_deg < winter.zenith_angle_deg,
            "summer chi {} not less than winter chi {}",
            summer.zenith_angle_deg,
            winter.zenith_angle_deg
        );
    }
}
