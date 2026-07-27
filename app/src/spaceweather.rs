//! Observed solar indices, fetched rather than assumed.
//!
//! The ionospheric model is driven by a sunspot number, and until now every
//! headless run took one from the command line or fell back to a default. For a
//! validation harness that is a hole: an SSN guessed 40 points wrong moves foF2
//! enough to change which paths close, so a scoring run against real spots would
//! be measuring the guess as much as the model. This module gets the real
//! number for the day being scored, and records WHERE it came from so the report
//! can say so.
//!
//! Two sources, both official and both plain text:
//!
//! * **SIDC/SILSO EISN** - the estimated international sunspot number, issued
//!   daily. This is the right source for a date in the last few weeks, which is
//!   what a live run scores. It is provisional: the value for a given day is
//!   revised as more stations report, and the file says how many did.
//! * **NOAA SWPC observed solar cycle indices** - monthly means, definitive but
//!   only published after the month ends. The right source for anything older.
//!
//! Neither is interpolated into a model or a table of anything: the number is
//! read for the date asked about, and if it cannot be had, that is reported
//! instead of being papered over with a default.

use std::fmt;

use crate::net::{self, NetError};

/// SIDC/SILSO daily estimated international sunspot number.
const EISN_URL: &str = "https://www.sidc.be/SILSO/DATA/EISN/EISN_current.csv";
/// NOAA SWPC monthly observed indices, back to 1749.
const SWPC_MONTHLY_URL: &str =
    "https://services.swpc.noaa.gov/json/solar-cycle/observed-solar-cycle-indices.json";

/// Where a sunspot number came from, carried with the value so a report can
/// never present a fetched number and a fallback as though they were the same
/// kind of thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SsnSource {
    /// SIDC daily estimated SN, with the number of stations behind it.
    SidcDaily { stations: u32 },
    /// NOAA SWPC monthly observed SSN for `YYYY-MM`.
    SwpcMonthly { month: String },
    /// Supplied by the operator; nothing was fetched.
    Operator,
}

impl fmt::Display for SsnSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SidcDaily { stations } => {
                write!(f, "SIDC daily estimate ({stations} stations reporting)")
            }
            Self::SwpcMonthly { month } => write!(f, "NOAA SWPC observed monthly mean for {month}"),
            Self::Operator => write!(f, "supplied on the command line"),
        }
    }
}

/// An observed sunspot number and its provenance.
#[derive(Clone, Debug)]
pub struct Ssn {
    pub value: f64,
    pub source: SsnSource,
    /// The date the value describes, `(year, month, day)`; the day is 0 for a
    /// monthly mean, which describes no single day.
    pub as_of: (i32, u32, u32),
    /// Standard deviation across reporting stations, where the source gives
    /// one. A daily estimate with a wide spread is a weaker input than a narrow
    /// one, and the report says so rather than hiding it.
    pub stdev: Option<f64>,
}

impl fmt::Display for Ssn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SSN {:.1}", self.value)?;
        if let Some(sd) = self.stdev {
            write!(f, " +/- {sd:.1}")?;
        }
        let (y, m, d) = self.as_of;
        if d == 0 {
            write!(f, " ({y:04}-{m:02}, {})", self.source)
        } else {
            write!(f, " ({y:04}-{m:02}-{d:02}, {})", self.source)
        }
    }
}

/// The best available sunspot number for a date.
///
/// Tries the daily estimate first and falls back to the monthly mean, which is
/// the order of preference by recency: SIDC publishes the current month day by
/// day, NOAA publishes months once they are complete. Whichever answers, the
/// value carries its source.
///
/// # Errors
/// Both sources failing is an error rather than a silent default. A validation
/// run against the wrong SSN is worse than one that refuses to start.
pub fn ssn_for(year: i32, month: u32, day: u32) -> Result<Ssn, NetError> {
    let daily = fetch_eisn().and_then(|rows| {
        pick_daily(&rows, year, month, day).ok_or_else(|| {
            NetError::Data(format!(
                "SIDC daily file has no entry for {year:04}-{month:02}-{day:02}"
            ))
        })
    });
    match daily {
        Ok(s) => Ok(s),
        Err(daily_err) => match fetch_monthly(year, month) {
            Ok(s) => Ok(s),
            Err(monthly_err) => Err(NetError::Data(format!(
                "no sunspot number for {year:04}-{month:02}-{day:02}: \
                 daily source said [{daily_err}]; monthly source said [{monthly_err}]"
            ))),
        },
    }
}

/// One parsed row of the SIDC EISN file.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EisnRow {
    year: i32,
    month: u32,
    day: u32,
    sn: f64,
    stdev: f64,
    stations: u32,
}

fn fetch_eisn() -> Result<Vec<EisnRow>, NetError> {
    Ok(parse_eisn(&net::get_text(EISN_URL)?))
}

/// Parse the SIDC EISN CSV.
///
/// Comma separated, one line per day:
/// `year, month, day, decimal year, SN, stdev, n_calculated, n_available`.
/// A day the estimate has not been computed for carries `SN = -1`, which is a
/// no-value marker and is dropped rather than being read as a sunspot number.
fn parse_eisn(text: &str) -> Vec<EisnRow> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() < 7 {
            continue;
        }
        let (Ok(year), Ok(month), Ok(day)) = (
            f[0].parse::<i32>(),
            f[1].parse::<u32>(),
            f[2].parse::<u32>(),
        ) else {
            continue;
        };
        let (Ok(sn), Ok(stdev), Ok(stations)) = (
            f[4].parse::<f64>(),
            f[5].parse::<f64>(),
            f[6].parse::<u32>(),
        ) else {
            continue;
        };
        if sn < 0.0 {
            continue; // the file's marker for "not computed yet"
        }
        out.push(EisnRow {
            year,
            month,
            day,
            sn,
            stdev,
            stations,
        });
    }
    out
}

fn pick_daily(rows: &[EisnRow], year: i32, month: u32, day: u32) -> Option<Ssn> {
    let r = rows
        .iter()
        .find(|r| r.year == year && r.month == month && r.day == day)?;
    Some(Ssn {
        value: r.sn,
        source: SsnSource::SidcDaily {
            stations: r.stations,
        },
        as_of: (r.year, r.month, r.day),
        stdev: Some(r.stdev),
    })
}

fn fetch_monthly(year: i32, month: u32) -> Result<Ssn, NetError> {
    let body = net::get_text(SWPC_MONTHLY_URL)?;
    let tag = format!("{year:04}-{month:02}");
    monthly_ssn(&body, &tag)
        .ok_or_else(|| NetError::Data(format!("NOAA monthly indices have no entry for {tag}")))
}

/// Pull one month's `ssn` out of the NOAA indices JSON.
///
/// Hand-scanned rather than deserialised: the document is a flat array of flat
/// objects with two fields this needs, and a JSON dependency for that would
/// weigh more than the twenty lines it saves. The scan is deliberately literal -
/// it looks for the exact `"time-tag": "<tag>"` record and then the next `"ssn"`
/// inside it - and returns `None` rather than a guess if the shape changes.
fn parse_monthly(body: &str, tag: &str) -> Option<f64> {
    let needle = format!("\"time-tag\":\"{tag}\"");
    let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let at = compact.find(&needle)?;
    let rest = &compact[at..];
    // Stay inside this record: stop at the object that contains the tag.
    let record = &rest[..rest.find('}').unwrap_or(rest.len())];
    let ssn_at = record.find("\"ssn\":")? + "\"ssn\":".len();
    let value: String = record[ssn_at..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let v = value.parse::<f64>().ok()?;
    // NOAA writes -1 for "not available", which is not a sunspot number.
    (v >= 0.0).then_some(v)
}

/// One month's observed SSN, with its provenance attached.
fn monthly_ssn(body: &str, tag: &str) -> Option<Ssn> {
    let value = parse_monthly(body, tag)?;
    let (y, m) = tag.split_once('-')?;
    Some(Ssn {
        value,
        source: SsnSource::SwpcMonthly {
            month: tag.to_string(),
        },
        as_of: (y.parse().ok()?, m.parse().ok()?, 0),
        stdev: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SIDC format, including its no-value marker, which must never be
    /// read as a sunspot number of -1.
    #[test]
    fn eisn_parses_and_drops_the_no_value_marker() {
        let text = "\
2026, 07, 25, 2026.563, 107,  11.0,  23,  31,
2026, 07, 26, 2026.566, 106,  11.4,  21,  24,
2026, 07, 27, 2026.568, 119,   8.0,  21,  27,
2026, 07, 28, 2026.571,  -1,   0.0,   0,   0,
";
        let rows = parse_eisn(text);
        assert_eq!(rows.len(), 3, "the -1 row must be dropped: {rows:?}");
        assert_eq!(rows[2].day, 27);
        assert!((rows[2].sn - 119.0).abs() < 1e-9);
        assert_eq!(rows[2].stations, 21);

        let picked = pick_daily(&rows, 2026, 7, 27).expect("27 July present");
        assert!((picked.value - 119.0).abs() < 1e-9);
        assert_eq!(picked.source, SsnSource::SidcDaily { stations: 21 });
        assert!(pick_daily(&rows, 2026, 7, 28).is_none(), "marker row");
        assert!(pick_daily(&rows, 2026, 7, 30).is_none(), "absent day");
    }

    /// The NOAA scan must find the right record, not merely the first `ssn` in
    /// the document, and must reject the -1 "not available" marker.
    #[test]
    fn monthly_scan_finds_the_requested_month() {
        let body = r#"[
          {"time-tag": "2026-04", "ssn": 88.1, "smoothed_ssn": -1.0, "f10.7": 120.0},
          {"time-tag": "2026-05", "ssn": 101.5, "smoothed_ssn": -1.0, "f10.7": 125.7},
          {"time-tag": "2026-06", "ssn": 94.4, "smoothed_ssn": -1.0, "f10.7": 138.2}
        ]"#;
        assert!((parse_monthly(body, "2026-05").unwrap() - 101.5).abs() < 1e-9);
        assert!((parse_monthly(body, "2026-06").unwrap() - 94.4).abs() < 1e-9);
        assert!(parse_monthly(body, "2026-07").is_none(), "absent month");

        let s = monthly_ssn(body, "2026-04").expect("April");
        assert_eq!(s.as_of, (2026, 4, 0));
        assert!(matches!(s.source, SsnSource::SwpcMonthly { .. }));

        let unavailable = r#"[{"time-tag": "2026-07", "ssn": -1.0}]"#;
        assert!(
            parse_monthly(unavailable, "2026-07").is_none(),
            "-1 is a marker, not a sunspot number"
        );
    }

    /// The display form has to state the provenance: a report that shows a
    /// number without saying where it came from is the thing this module
    /// exists to prevent.
    #[test]
    fn display_states_provenance() {
        let s = Ssn {
            value: 119.0,
            source: SsnSource::SidcDaily { stations: 21 },
            as_of: (2026, 7, 27),
            stdev: Some(8.0),
        };
        let text = s.to_string();
        assert!(text.contains("119.0"), "{text}");
        assert!(text.contains("8.0"), "{text}");
        assert!(text.contains("2026-07-27"), "{text}");
        assert!(text.contains("SIDC"), "{text}");
    }
}
