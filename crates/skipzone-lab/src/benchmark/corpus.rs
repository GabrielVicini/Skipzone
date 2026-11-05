//! Loading the frozen corpora into one row type.
//!
//! Every path observation, whatever its source, becomes an [`Obs`]: a path,
//! a time, a frequency, and either a decode (with or without a measured SNR)
//! or a constructed non-decode. Ionosonde soundings become [`Sounding`]s.
//! The formats are the ones docs/notes/accuracy-roadmap.md §2.2 describes.
//!
//! Rows that cannot be used are counted and reported, never silently
//! dropped, in the same spirit as `corpus::read_positives`.

use std::collections::{BTreeMap, BTreeSet};

use skipzone::geo::central_angle;
use skipzone::units::Radians;
use skipzone_iono::grid;
use skipzone_iono::solar;
use skipzone_predict::noise::OperatingMode;
use skipzone_predict::scenario::{EARTH_RADIUS_M, Inputs, destination_point, ground_point};

use crate::corpus::{self as wspr_corpus, Negative};
use crate::wspr::WsprSpot;

use super::manifest::Entry;

/// Which corpus family a row came from. It decides the receiver settings the
/// row is predicted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Wspr,
    Rbn,
    Beacon,
}

impl Source {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Wspr => "wspr",
            Self::Rbn => "rbn",
            Self::Beacon => "beacon",
        }
    }
}

/// UTC timestamp as `(year, month, day, hour, minute)`, the layout
/// [`WsprSpot`] uses.
pub type Stamp = (i32, u32, u32, u32, u32);

/// One path observation: a decode, or a path that was listening and heard
/// nothing.
#[derive(Clone, Debug)]
pub struct Obs {
    /// Stable key within the corpus, `<file>#<row>`. Before and after runs are
    /// paired on it.
    pub id: String,
    pub source: Source,
    pub tx_call: String,
    pub tx_grid: String,
    pub rx_call: String,
    pub rx_grid: String,
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
    pub freq_mhz: f64,
    pub timestamp: Stamp,
    /// Transmitter power, dBm. Assumed for RBN (see [`ASSUMED_RBN_DBM`]).
    pub tx_dbm: f64,
    /// Daily sunspot number for this row's date.
    pub ssn: f64,
    /// Measured SNR, for a decode that reported one.
    pub measured_db: Option<f64>,
    pub decoded: bool,
}

/// RBN spots carry no power. 100 W is the common station power and is what
/// the app assumes by default; the difference per station goes into its
/// station effect for the SNR figures.
pub const ASSUMED_RBN_DBM: f64 = 50.0;

/// NCDXF beacons send their callsign at 100 W.
pub const NCDXF_CALLSIGN_DBM: f64 = 50.0;

/// Path geometry that does not depend on any model: the cuts and the region
/// split are taken from this, so every model is cut the same way.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub range_km: f64,
    pub mid_lat: f64,
    pub mid_lon: f64,
    /// Sun below the horizon at the path midpoint.
    pub midpoint_night: bool,
}

/// Great-circle range, km, as the solver computes it.
#[must_use]
pub fn great_circle_km(tx_lat: f64, tx_lon: f64, rx_lat: f64, rx_lon: f64) -> f64 {
    let a = ground_point(tx_lat, tx_lon);
    let b = ground_point(rx_lat, rx_lon);
    central_angle(&a, &b).get() * EARTH_RADIUS_M / 1000.0
}

impl Obs {
    /// UTC hour as a fraction.
    #[must_use]
    pub fn utc_hours(&self) -> f64 {
        f64::from(self.timestamp.3) + f64::from(self.timestamp.4) / 60.0
    }

    /// `(year, month, day)`.
    #[must_use]
    pub fn date(&self) -> (i32, u32, u32) {
        (self.timestamp.0, self.timestamp.1, self.timestamp.2)
    }

    /// This row as a [`WsprSpot`], so WSPR rows reach the solver through
    /// [`WsprSpot::inputs_for`] exactly as `wspr_calibrate` sends them.
    #[must_use]
    pub fn as_spot(&self) -> WsprSpot {
        WsprSpot {
            timestamp: self.timestamp,
            tx_call: self.tx_call.clone(),
            freq_mhz: self.freq_mhz,
            snr_db: self.measured_db.unwrap_or(f64::NAN),
            tx_grid: self.tx_grid.clone(),
            tx_dbm: self.tx_dbm,
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
            reported_km: f64::NAN,
            tx_lat: self.tx_lat,
            tx_lon: self.tx_lon,
            rx_lat: self.rx_lat,
            rx_lon: self.rx_lon,
        }
    }

    /// Scenario inputs for this row.
    ///
    /// WSPR rows take the `wspr_calibrate` path (`solving::solve_one`): the
    /// spot's own inputs with the day's SSN. RBN and beacon rows are CW, so
    /// they take the app's CW bandwidth and threshold. The skimmer's own
    /// bandwidth differs, but that is a constant per skimmer and goes into its
    /// station effect (docs/notes/accuracy-roadmap.md §2.2.2).
    #[must_use]
    pub fn inputs(&self, base: &Inputs) -> Inputs {
        match self.source {
            Source::Wspr => Inputs {
                ssn: self.ssn,
                ..self.as_spot().inputs_for(base)
            },
            Source::Rbn | Source::Beacon => {
                let (bandwidth_hz, snr_threshold_db) = OperatingMode::Cw.defaults();
                let spot = self.as_spot();
                Inputs {
                    tx_lat: self.tx_lat,
                    tx_lon: self.tx_lon,
                    rx_lat: self.rx_lat,
                    rx_lon: self.rx_lon,
                    freq_mhz: self.freq_mhz,
                    utc_hours: self.utc_hours(),
                    year: self.timestamp.0,
                    month: self.timestamp.1,
                    day_of_month: self.timestamp.2,
                    tx_power_w: spot.tx_power_w(),
                    bandwidth_hz,
                    snr_threshold_db,
                    op_mode: OperatingMode::Cw,
                    ssn: self.ssn,
                    ..base.clone()
                }
            }
        }
    }

    /// Model-free geometry of this path.
    #[must_use]
    pub fn geometry(&self) -> Geometry {
        path_geometry(
            (self.tx_lat, self.tx_lon),
            (self.rx_lat, self.rx_lon),
            self.timestamp,
        )
    }
}

/// Range, midpoint and midpoint day/night, computed the way
/// `scenario::resolve` does.
#[must_use]
pub fn path_geometry(tx: (f64, f64), rx: (f64, f64), t: Stamp) -> Geometry {
    let a = ground_point(tx.0, tx.1);
    let b = ground_point(rx.0, rx.1);
    let arc = central_angle(&a, &b);
    let brng = skipzone::geo::bearing(&a, &b);
    let mid = destination_point(&a, brng, Radians::new(0.5 * arc.get()));
    let (mid_lat, mid_lon) = skipzone_predict::scenario::to_lat_lon(&mid);
    let utc = f64::from(t.3) + f64::from(t.4) / 60.0;
    let sun = solar::solar_geometry(mid_lat, mid_lon, t.1, t.2, utc);
    Geometry {
        range_km: arc.get() * EARTH_RADIUS_M / 1000.0,
        mid_lat,
        mid_lon,
        midpoint_night: !sun.is_day(),
    }
}

/// What a loader read, and what it could not use.
#[derive(Clone, Debug)]
pub struct Loaded<T> {
    pub rows: Vec<T>,
    /// Unreadable rows, with the reason. Reported in full count, first few
    /// verbatim.
    pub problems: Vec<String>,
}

impl<T> Default for Loaded<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            problems: Vec::new(),
        }
    }
}

fn row_id(entry: &Entry, i: usize) -> String {
    format!("{}#{i}", entry.file)
}

/// WSPR positives, through `corpus::read_positives`.
#[must_use]
pub fn load_wspr(entry: &Entry) -> Loaded<Obs> {
    let (spots, problems) = wspr_corpus::read_positives(&entry.text);
    let rows = spots
        .into_iter()
        .enumerate()
        .map(|(i, c)| Obs {
            id: row_id(entry, i),
            source: Source::Wspr,
            tx_call: c.spot.tx_call,
            tx_grid: c.spot.tx_grid,
            rx_call: c.spot.rx_call,
            rx_grid: c.spot.rx_grid,
            tx_lat: c.spot.tx_lat,
            tx_lon: c.spot.tx_lon,
            rx_lat: c.spot.rx_lat,
            rx_lon: c.spot.rx_lon,
            freq_mhz: c.spot.freq_mhz,
            timestamp: c.spot.timestamp,
            tx_dbm: c.spot.tx_dbm,
            ssn: c.ssn,
            measured_db: Some(c.spot.snr_db),
            decoded: true,
        })
        .collect();
    Loaded { rows, problems }
}

fn negative_obs(id: String, source: Source, n: &Negative) -> Obs {
    Obs {
        id,
        source,
        tx_call: n.tx_call.clone(),
        tx_grid: n.tx_grid.clone(),
        rx_call: n.rx_call.clone(),
        rx_grid: n.rx_grid.clone(),
        tx_lat: n.tx_lat,
        tx_lon: n.tx_lon,
        rx_lat: n.rx_lat,
        rx_lon: n.rx_lon,
        freq_mhz: n.freq_mhz,
        timestamp: n.timestamp,
        tx_dbm: n.tx_dbm,
        ssn: n.ssn,
        measured_db: None,
        decoded: false,
    }
}

/// WSPR negatives, through `corpus::read_negatives`.
#[must_use]
pub fn load_wspr_negatives(entry: &Entry) -> Loaded<Obs> {
    let (negs, problems) = wspr_corpus::read_negatives(&entry.text);
    let rows = negs
        .iter()
        .enumerate()
        .map(|(i, n)| negative_obs(row_id(entry, i), Source::Wspr, n))
        .collect();
    Loaded { rows, problems }
}

/// Data lines: every line that is neither blank nor a `#` comment.
fn data_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
}

/// Daily sunspot numbers: year, month, day, SSN, whitespace-separated. The
/// layout `iono_check` reads from `corpus/ssn_daily.tsv`.
#[must_use]
pub fn load_ssn_daily(entry: &Entry) -> Loaded<((i32, u32, u32), f64)> {
    let mut out = Loaded::default();
    for (i, line) in data_lines(&entry.text) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let parsed = (|| {
            Some((
                (
                    f.first()?.parse().ok()?,
                    f.get(1)?.parse().ok()?,
                    f.get(2)?.parse().ok()?,
                ),
                f.get(3)?.parse().ok()?,
            ))
        })();
        match parsed {
            Some(r) => out.rows.push(r),
            None => out.problems.push(format!(
                "{} line {}: not year month day SSN",
                entry.file,
                i + 1
            )),
        }
    }
    out
}

/// Monthly R12: year, month, R12.
#[must_use]
pub fn load_r12(entry: &Entry) -> Loaded<((i32, u32), f64)> {
    let mut out = Loaded::default();
    for (i, line) in data_lines(&entry.text) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let parsed = (|| {
            Some((
                (f.first()?.parse().ok()?, f.get(1)?.parse().ok()?),
                f.get(2)?.parse().ok()?,
            ))
        })();
        match parsed {
            Some(r) => out.rows.push(r),
            None => out
                .problems
                .push(format!("{} line {}: not year month R12", entry.file, i + 1)),
        }
    }
    out
}

/// A station position, from a [`super::manifest::Kind::Stations`] file.
#[derive(Clone, Debug)]
pub struct Station {
    pub grid: String,
    pub lat: f64,
    pub lon: f64,
}

/// Callsign and Maidenhead locator, tab- or space-separated. RBN rows carry
/// no positions, so the corpus has to bring them (the RBN skimmer list gives
/// skimmer locators; the transmitting stations need a lookup of their own).
#[must_use]
pub fn load_stations(entry: &Entry) -> Loaded<(String, Station)> {
    let mut out = Loaded::default();
    for (i, line) in data_lines(&entry.text) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(call), Some(g)) = (f.first(), f.get(1)) else {
            out.problems.push(format!(
                "{} line {}: not callsign locator",
                entry.file,
                i + 1
            ));
            continue;
        };
        match grid::decode(g) {
            Some((lat, lon)) => out.rows.push((
                normalise_call(call),
                Station {
                    grid: (*g).to_string(),
                    lat,
                    lon,
                },
            )),
            None => out
                .problems
                .push(format!("{} line {}: bad locator {g:?}", entry.file, i + 1)),
        }
    }
    out
}

/// Upper-case, and without the `-#` RBN appends to skimmer calls. Whether
/// the raw files carry that suffix has to be confirmed on a real file.
#[must_use]
pub fn normalise_call(call: &str) -> String {
    let c = call.trim().to_ascii_uppercase();
    match c.strip_suffix("-#") {
        Some(s) => s.to_string(),
        None => c,
    }
}

/// Parse `YYYY-MM-DD HH:MM[:SS]` (or with `/` or `T` separators) into a
/// stamp plus seconds.
fn parse_time(s: &str) -> Option<(Stamp, u32)> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once([' ', 'T', '_'])?;
    let d: Vec<&str> = date.split(['-', '/']).collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() < 2 {
        return None;
    }
    let sec = t
        .get(2)
        .map_or(Some(0), |v| v.split('.').next()?.parse().ok())?;
    Some((
        (
            d[0].parse().ok()?,
            d[1].parse().ok()?,
            d[2].parse().ok()?,
            t[0].parse().ok()?,
            t[1].parse().ok()?,
        ),
        sec,
    ))
}

/// What the RBN loader produced.
#[derive(Clone, Debug, Default)]
pub struct RbnLoad {
    pub positives: Vec<Obs>,
    pub negatives: Vec<Obs>,
    pub problems: Vec<String>,
    /// Spots dropped because a station had no position.
    pub no_position: usize,
    /// Spots dropped because their date has no SSN.
    pub no_ssn: usize,
    /// Spots dropped as a repeat of the same pair, band and window.
    pub repeats: usize,
    /// Rows that were not CW.
    pub not_cw: usize,
}

/// Reverse Beacon Network CSV, header row first.
///
/// Both layouts in docs/notes/accuracy-roadmap.md §2.2.2 are accepted, since
/// columns are found by name. `freq` is in kHz in the RBN files. Rows whose
/// `tx_mode` is present and not CW are skipped.
///
/// Negatives are built with `corpus::negatives_from_cycle`, the WSPR
/// construction, with a window of `window_min` minutes standing in for the
/// two-minute cycle. The roadmap asks for the window to be at least the RBN
/// de-duplication interval, measured from the data; it is a manifest setting
/// so a corpus pins it.
#[must_use]
pub fn load_rbn(
    entry: &Entry,
    stations: &BTreeMap<String, Station>,
    ssn: &BTreeMap<(i32, u32, u32), f64>,
    window_min: u32,
) -> RbnLoad {
    let mut out = RbnLoad::default();
    let mut lines = data_lines(&entry.text);
    let Some((_, header)) = lines.next() else {
        out.problems.push(format!("{}: empty", entry.file));
        return out;
    };
    let cols: Vec<String> = header
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .collect();
    let col = |name: &str| cols.iter().position(|c| c == name);
    let (Some(c_rx), Some(c_dx), Some(c_freq), Some(c_db), Some(c_date)) = (
        col("callsign"),
        col("dx"),
        col("freq"),
        col("db"),
        col("date"),
    ) else {
        out.problems.push(format!(
            "{}: header lacks one of callsign, dx, freq, db, date: {header:?}",
            entry.file
        ));
        return out;
    };
    let c_txmode = col("tx_mode");
    let window_min = window_min.max(1);

    let mut seen: BTreeSet<(String, String, String, Stamp)> = BTreeSet::new();
    // Census per (window, band): every positive, as the spot type the WSPR
    // negative construction reads.
    let mut census: BTreeMap<(Stamp, String), Vec<WsprSpot>> = BTreeMap::new();
    for (i, line) in lines {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        let get = |c: usize| f.get(c).copied().unwrap_or("");
        if let Some(c) = c_txmode
            && !get(c).is_empty()
            && !get(c).eq_ignore_ascii_case("CW")
        {
            out.not_cw += 1;
            continue;
        }
        let (Ok(khz), Ok(db), Some((t, _))) = (
            get(c_freq).parse::<f64>(),
            get(c_db).parse::<f64>(),
            parse_time(get(c_date)),
        ) else {
            out.problems.push(format!(
                "{} line {}: unreadable freq, db or date",
                entry.file,
                i + 1
            ));
            continue;
        };
        let rx_call = normalise_call(get(c_rx));
        let tx_call = normalise_call(get(c_dx));
        let (Some(tx), Some(rx)) = (stations.get(&tx_call), stations.get(&rx_call)) else {
            out.no_position += 1;
            continue;
        };
        let Some(&day_ssn) = ssn.get(&(t.0, t.1, t.2)) else {
            out.no_ssn += 1;
            continue;
        };
        let freq_mhz = khz / 1000.0;
        let band = crate::wspr_report::band_label(freq_mhz);
        let window = (t.0, t.1, t.2, t.3, t.4 - t.4 % window_min.min(60));
        if !seen.insert((tx_call.clone(), rx_call.clone(), band.clone(), window)) {
            out.repeats += 1;
            continue;
        }
        let obs = Obs {
            id: row_id(entry, i),
            source: Source::Rbn,
            tx_call,
            tx_grid: tx.grid.clone(),
            rx_call,
            rx_grid: rx.grid.clone(),
            tx_lat: tx.lat,
            tx_lon: tx.lon,
            rx_lat: rx.lat,
            rx_lon: rx.lon,
            freq_mhz,
            timestamp: t,
            tx_dbm: ASSUMED_RBN_DBM,
            ssn: day_ssn,
            measured_db: Some(db),
            decoded: true,
        };
        census.entry((window, band)).or_default().push(WsprSpot {
            timestamp: window,
            ..obs.as_spot()
        });
        out.positives.push(obs);
    }

    // Keep negatives inside the range the positives cover: a pair shorter or
    // longer than anything heard is not a path this corpus says much about.
    let ranges: Vec<f64> = out
        .positives
        .iter()
        .map(|o| great_circle_km(o.tx_lat, o.tx_lon, o.rx_lat, o.rx_lon))
        .collect();
    let min_km = ranges.iter().copied().fold(f64::INFINITY, f64::min);
    let max_km = ranges.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for ((window, band), spots) in &census {
        let Some(&day_ssn) = ssn.get(&(window.0, window.1, window.2)) else {
            continue;
        };
        let negs = wspr_corpus::negatives_from_cycle(spots, min_km, max_km, day_ssn, |s| {
            great_circle_km(s.tx_lat, s.tx_lon, s.rx_lat, s.rx_lon)
        });
        for n in &negs {
            let id = format!(
                "{}#neg:{:04}{:02}{:02}{:02}{:02}:{band}:{}>{}",
                entry.file, window.0, window.1, window.2, window.3, window.4, n.tx_call, n.rx_call
            );
            out.negatives.push(negative_obs(id, Source::Rbn, n));
        }
    }
    out
}

/// NCDXF/IARU beacon schedule, from the published schedule page. Check it
/// against <https://www.ncdxf.org/beacon/beaconschedule.html> before a real
/// run: the order is what makes a missing reception a negative.
pub mod ncdxf {
    /// The eighteen beacons in transmission order.
    pub const BEACONS: [&str; 18] = [
        "4U1UN", "VE8AT", "W6WX", "KH6RS", "ZL6B", "VK6RBP", "JA2IGY", "RR9O", "VR2B", "4S7B",
        "ZS6DN", "5Z4B", "4X6TU", "OH2B", "CS3B", "LU4AA", "OA4B", "YV5B",
    ];
    /// The five frequencies, MHz, in the order a beacon steps through them.
    pub const BANDS_MHZ: [f64; 5] = [14.100, 18.110, 21.150, 24.930, 28.200];
    /// Seconds per slot; eighteen slots make the three-minute cycle.
    pub const SLOT_S: u32 = 10;

    /// Which beacon is on `band` (an index into [`BANDS_MHZ`]) at `second`
    /// seconds past a UTC hour. Beacon `i` starts the cycle on 14.100 MHz and
    /// moves up one band each slot, so slot `s` on band `b` is beacon
    /// `s - b` (mod 18).
    #[must_use]
    pub fn beacon_at(band: usize, second_of_hour: u32) -> &'static str {
        let slot = (second_of_hour % 180) / SLOT_S;
        BEACONS[(slot as usize + 18 - band % 5) % 18]
    }

    /// Index into [`BANDS_MHZ`] for a frequency within 5 kHz of one.
    #[must_use]
    pub fn band_index(mhz: f64) -> Option<usize> {
        BANDS_MHZ.iter().position(|b| (b - mhz).abs() < 0.005)
    }
}

/// NCDXF beacon records, tab-separated:
///
/// ```text
/// timestamp (YYYY-MM-DD HH:MM:SS)  beacon  MHz  rx_call  decoded (1/0)  SNR dB (or -)
/// ```
///
/// A record whose time does not fall in that beacon's slot on that band is
/// refused, and beacons listed as off for the period are dropped.
/// Positions come from the stations file.
#[must_use]
pub fn load_beacons(
    entry: &Entry,
    stations: &BTreeMap<String, Station>,
    ssn: &BTreeMap<(i32, u32, u32), f64>,
    off: &BTreeSet<String>,
) -> Loaded<Obs> {
    let mut out = Loaded::default();
    for (i, line) in data_lines(&entry.text) {
        let f: Vec<&str> = line.split('\t').map(str::trim).collect();
        let bad = |why: &str| format!("{} line {}: {why}", entry.file, i + 1);
        if f.len() < 5 {
            out.problems.push(bad("expected at least 5 fields"));
            continue;
        }
        let Some((t, sec)) = parse_time(f[0]) else {
            out.problems.push(bad("unreadable timestamp"));
            continue;
        };
        let beacon = normalise_call(f[1]);
        if off.contains(&beacon) {
            continue;
        }
        let Ok(mhz) = f[2].parse::<f64>() else {
            out.problems.push(bad("unreadable frequency"));
            continue;
        };
        let Some(band) = ncdxf::band_index(mhz) else {
            out.problems.push(bad("not an NCDXF frequency"));
            continue;
        };
        if ncdxf::beacon_at(band, t.4 * 60 + sec) != beacon {
            out.problems
                .push(bad("time is not this beacon's slot on this band"));
            continue;
        }
        let decoded = match f[4] {
            "1" => true,
            "0" => false,
            _ => {
                out.problems.push(bad("decoded must be 1 or 0"));
                continue;
            }
        };
        let rx_call = normalise_call(f[3]);
        let (Some(tx), Some(rx)) = (stations.get(&beacon), stations.get(&rx_call)) else {
            out.problems.push(bad("station has no position"));
            continue;
        };
        let Some(&day_ssn) = ssn.get(&(t.0, t.1, t.2)) else {
            out.problems.push(bad("no SSN for this date"));
            continue;
        };
        let measured_db = if decoded {
            f.get(5).and_then(|v| v.parse().ok())
        } else {
            None
        };
        out.rows.push(Obs {
            id: row_id(entry, i),
            source: Source::Beacon,
            tx_call: beacon,
            tx_grid: tx.grid.clone(),
            rx_call,
            rx_grid: rx.grid.clone(),
            tx_lat: tx.lat,
            tx_lon: tx.lon,
            rx_lat: rx.lat,
            rx_lon: rx.lon,
            freq_mhz: mhz,
            timestamp: t,
            tx_dbm: NCDXF_CALLSIGN_DBM,
            ssn: day_ssn,
            measured_db,
            decoded,
        });
    }
    out
}

/// One accepted ionosonde sounding.
#[derive(Clone, Debug)]
pub struct Sounding {
    pub id: String,
    pub station: String,
    pub lat: f64,
    pub lon: f64,
    pub timestamp: Stamp,
    pub fof2: f64,
    pub m3000f2: Option<f64>,
    pub hmf2_km: Option<f64>,
    pub ssn: f64,
}

impl Sounding {
    /// Measured MUF(3000)F2 = foF2 x M(3000)F2, where both were scaled.
    #[must_use]
    pub fn muf3000(&self) -> Option<f64> {
        self.m3000f2.map(|m| self.fof2 * m)
    }
}

/// Same confidence floor as `iono_check`: GIRO scores 0-100, 999 for a hand
/// scaled trace.
const MIN_CONFIDENCE: f64 = 75.0;

/// GIRO characteristics, tab-separated. The first seven columns are the
/// `iono_check` layout; M(3000)F2 and hmF2 follow, `-` or empty when not
/// scaled:
///
/// ```text
/// station  lat  lon  time (ISO 8601)  confidence  foF2  foE  M3000F2  hmF2
/// ```
#[must_use]
pub fn load_ionosonde(entry: &Entry, ssn: &BTreeMap<(i32, u32, u32), f64>) -> Loaded<Sounding> {
    let mut out = Loaded::default();
    let opt = |s: Option<&&str>| s.and_then(|v| v.parse::<f64>().ok());
    for (i, line) in data_lines(&entry.text) {
        let f: Vec<&str> = line.split('\t').map(str::trim).collect();
        let bad = |why: &str| format!("{} line {}: {why}", entry.file, i + 1);
        if f.len() < 6 {
            out.problems.push(bad("expected at least 6 fields"));
            continue;
        }
        let (Ok(lat), Ok(lon), Some((t, _)), Ok(cs), Ok(fof2)) = (
            f[1].parse::<f64>(),
            f[2].parse::<f64>(),
            parse_time(f[3]),
            f[4].parse::<f64>(),
            f[5].parse::<f64>(),
        ) else {
            out.problems
                .push(bad("unreadable position, time, score or foF2"));
            continue;
        };
        if cs < MIN_CONFIDENCE && (cs - 999.0).abs() > 0.5 {
            continue;
        }
        let Some(&day_ssn) = ssn.get(&(t.0, t.1, t.2)) else {
            out.problems.push(bad("no SSN for this date"));
            continue;
        };
        out.rows.push(Sounding {
            id: row_id(entry, i),
            station: f[0].to_string(),
            lat,
            lon,
            timestamp: t,
            fof2,
            m3000f2: opt(f.get(7)),
            hmf2_km: opt(f.get(8)),
            ssn: day_ssn,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ncdxf_schedule_steps_one_band_per_slot() {
        assert_eq!(ncdxf::beacon_at(0, 0), "4U1UN");
        assert_eq!(ncdxf::beacon_at(1, 10), "4U1UN");
        assert_eq!(ncdxf::beacon_at(4, 40), "4U1UN");
        assert_eq!(ncdxf::beacon_at(0, 10), "VE8AT");
        assert_eq!(ncdxf::beacon_at(1, 0), "YV5B");
        // The cycle repeats every three minutes.
        assert_eq!(ncdxf::beacon_at(2, 25), ncdxf::beacon_at(2, 205));
    }

    #[test]
    fn times_parse_in_the_forms_the_sources_use() {
        assert_eq!(
            parse_time("2026-10-02 00:03:16"),
            Some(((2026, 10, 2, 0, 3), 16))
        );
        assert_eq!(
            parse_time("2026-07-02T00:03:16.000Z"),
            Some(((2026, 7, 2, 0, 3), 16))
        );
        assert_eq!(
            parse_time("2026/10/02 23:59"),
            Some(((2026, 10, 2, 23, 59), 0))
        );
        assert_eq!(parse_time("garbage"), None);
    }

    #[test]
    fn skimmer_suffix_is_stripped() {
        assert_eq!(normalise_call("zz9aa-#"), "ZZ9AA");
        assert_eq!(normalise_call("ZZ9AA"), "ZZ9AA");
    }
}
