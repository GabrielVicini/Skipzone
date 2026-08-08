//! Civil date/time arithmetic and the system UTC clock.
//!
//! The app needs three things the standard library does not give directly: the
//! current UTC date, a calendar (days in a month, day of week) for the date
//! picker, and text parsing/formatting for the editable time and date fields.
//! Rather than take a date-time dependency for that, the proleptic Gregorian
//! conversion is implemented here from its closed form.
//!
//! `days_from_civil` / `civil_from_days` are Howard Hinnant's `chrono`-compatible
//! algorithms (<http://howardhinnant.github.io/date_algorithms.html>, public
//! domain), which shift the year to start in March so that the leap day falls at
//! the end of a 146097-day / 400-year era. They are exact for the whole range of
//! `i64` days and are pinned against known dates by the tests below.
//!
//! Nothing here feeds the physics: [`crate::solar`] derives declination from the
//! day of year alone and deliberately ignores leap years, which is far below the
//! accuracy of the ionospheric climatology it drives. The year exists so the
//! operator sees a real date.

use std::time::{SystemTime, UNIX_EPOCH};

/// Days in each month of a non-leap year, indexed by `month - 1`.
const MONTH_LENGTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Weekday initials, Monday first (matching [`weekday_index`]).
pub const WEEKDAY_INITIALS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

/// A calendar date in the proleptic Gregorian calendar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CivilDate {
    pub year: i32,
    /// 1..=12
    pub month: u32,
    /// 1..=31
    pub day: u32,
}

impl CivilDate {
    #[must_use]
    pub fn new(year: i32, month: u32, day: u32) -> Self {
        let month = month.clamp(1, 12);
        Self {
            year,
            month,
            day: day.clamp(1, days_in_month(year, month)),
        }
    }
}

#[must_use]
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[must_use]
pub fn days_in_month(year: i32, month: u32) -> u32 {
    let m = month.clamp(1, 12);
    if m == 2 && is_leap_year(year) {
        29
    } else {
        MONTH_LENGTHS[m as usize - 1]
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant).
#[must_use]
pub fn days_from_civil(date: CivilDate) -> i64 {
    let (y, m, d) = (
        i64::from(date.year),
        i64::from(date.month),
        i64::from(date.day),
    );
    // Shift the year to start in March, so the leap day is last.
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn civil_from_days(days: i64) -> CivilDate {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    CivilDate {
        year: (y + i64::from(m <= 2)) as i32,
        month: m as u32,
        day: d as u32,
    }
}

/// Day of the week as 0 = Monday .. 6 = Sunday. 1970-01-01 was a Thursday, so
/// the epoch lands on index 3.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn weekday_index(date: CivilDate) -> usize {
    (days_from_civil(date) + 3).rem_euclid(7) as usize
}

/// Current UTC date and time-of-day in hours. Falls back to the epoch if the
/// system clock is set before 1970 (which must not panic the app).
#[must_use]
pub fn utc_now() -> (CivilDate, f64) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let days = (secs / 86_400.0).floor();
    let rem = secs - days * 86_400.0;
    #[allow(clippy::cast_possible_truncation)]
    let date = civil_from_days(days as i64);
    (date, rem / 3600.0)
}

/// `YYYY-MM-DD`.
#[must_use]
pub fn format_date(date: CivilDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

/// Parse `YYYY-MM-DD`, rejecting out-of-range months and days rather than
/// silently clamping - a half-typed date must not move the map's terminator.
#[must_use]
pub fn parse_date(text: &str) -> Option<CivilDate> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.trim().parse().ok()?;
    let month: u32 = parts.next()?.trim().parse().ok()?;
    let day: u32 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    (1..=days_in_month(year, month))
        .contains(&day)
        .then_some(CivilDate { year, month, day })
}

/// Hours-of-day as `HH:MM`. 24.0 wraps to `00:00`, and rounding that lands on
/// 60 minutes carries into the hour so `23:59:40` never prints as `23:60`.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn format_hours(hours: f64) -> String {
    let total_minutes = (hours.rem_euclid(24.0) * 60.0).round() as u32;
    let (h, m) = (total_minutes / 60 % 24, total_minutes % 60);
    format!("{h:02}:{m:02}")
}

/// Parse `HH:MM` (or `HH:MM:SS`, or a bare hour) into hours of day.
#[must_use]
pub fn parse_hours(text: &str) -> Option<f64> {
    let text = text.trim();
    let mut parts = text.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = match parts.next() {
        Some(s) => s.trim().parse().ok()?,
        None => 0.0,
    };
    let s: f64 = match parts.next() {
        Some(s) => s.trim().parse().ok()?,
        None => 0.0,
    };
    if parts.next().is_some()
        || !(0.0..=24.0).contains(&h)
        || !(0.0..60.0).contains(&m)
        || !(0.0..60.0).contains(&s)
    {
        return None;
    }
    Some((h + m / 60.0 + s / 3600.0).min(24.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two conversions are inverse over a long span that crosses leap
    /// years, century non-leap years, and the 400-year era boundary.
    #[test]
    fn civil_and_days_round_trip() {
        let mut day = days_from_civil(CivilDate::new(1899, 1, 1));
        let end = days_from_civil(CivilDate::new(2101, 1, 1));
        while day < end {
            let date = civil_from_days(day);
            assert_eq!(days_from_civil(date), day, "round trip broke at {date:?}");
            assert!((1..=12).contains(&date.month));
            assert!((1..=days_in_month(date.year, date.month)).contains(&date.day));
            day += 1;
        }
    }

    /// Anchors that pin the epoch offset and the era arithmetic.
    #[test]
    fn known_dates_match() {
        assert_eq!(days_from_civil(CivilDate::new(1970, 1, 1)), 0);
        assert_eq!(days_from_civil(CivilDate::new(1969, 12, 31)), -1);
        assert_eq!(days_from_civil(CivilDate::new(2000, 3, 1)), 11_017);
        assert_eq!(civil_from_days(0), CivilDate::new(1970, 1, 1));
        assert_eq!(civil_from_days(19_000), CivilDate::new(2022, 1, 8));
        // 1970-01-01 was a Thursday; 2026-07-23 is a Thursday too.
        assert_eq!(weekday_index(CivilDate::new(1970, 1, 1)), 3);
        assert_eq!(weekday_index(CivilDate::new(2026, 7, 23)), 3);
        assert_eq!(weekday_index(CivilDate::new(2026, 7, 20)), 0);
    }

    /// The Gregorian leap rule, at the three cases that distinguish it.
    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(is_leap_year(2024) && days_in_month(2024, 2) == 29);
        assert!(!is_leap_year(1900) && days_in_month(1900, 2) == 28);
        assert!(is_leap_year(2000) && days_in_month(2000, 2) == 29);
        assert_eq!(days_in_month(2026, 4), 30);
    }

    #[test]
    fn date_text_round_trips_and_rejects_nonsense() {
        let d = CivilDate::new(2026, 7, 23);
        assert_eq!(format_date(d), "2026-07-23");
        assert_eq!(parse_date("2026-07-23"), Some(d));
        assert_eq!(parse_date(" 2026-7-3 "), Some(CivilDate::new(2026, 7, 3)));
        assert_eq!(parse_date("2026-02-30"), None, "February has no 30th");
        assert_eq!(parse_date("2024-02-29"), Some(CivilDate::new(2024, 2, 29)));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-07"), None);
        assert_eq!(parse_date("not a date"), None);
    }

    #[test]
    fn time_text_round_trips_and_rejects_nonsense() {
        assert_eq!(format_hours(0.0), "00:00");
        assert_eq!(format_hours(13.5), "13:30");
        assert_eq!(format_hours(24.0), "00:00");
        // 23:59:40 rounds to 60 minutes: it must carry, not print "23:60".
        assert_eq!(format_hours(23.0 + 59.0 / 60.0 + 40.0 / 3600.0), "00:00");
        assert_eq!(parse_hours("13:30"), Some(13.5));
        assert_eq!(parse_hours("7"), Some(7.0));
        assert_eq!(parse_hours("25:00"), None);
        assert_eq!(parse_hours("12:60"), None);
        assert_eq!(parse_hours("half past"), None);
        let h = 18.25;
        assert_eq!(parse_hours(&format_hours(h)), Some(h));
    }

    /// The clock reads a plausible present-day date: a smoke test that the
    /// epoch conversion is applied to real system time correctly.
    #[test]
    fn system_clock_reads_a_sane_date() {
        let (date, hours) = utc_now();
        assert!((2020..2200).contains(&date.year), "{date:?}");
        assert!((0.0..24.0).contains(&hours));
    }
}
