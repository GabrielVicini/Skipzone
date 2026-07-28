//! A saved, reproducible WSPR corpus for calibration: positives, negatives, and
//! the per-day sunspot number each was observed under.
//!
//! # Why this is a file and not a query
//!
//! [`crate::wsprlive::Query`] samples with `ORDER BY rand()`, so a fit run
//! against it is optimising against a moving target - a parameter change and a
//! resample look the same in the result. A calibration therefore fetches ONCE,
//! writes here, and fits against the file. The file carries its own provenance
//! header so that a number quoted from a fit can be traced back to the exact
//! selection that produced it.
//!
//! # What a negative is, and what it is not
//!
//! The archive publishes only successful decodes, so a "hit rate" computed from
//! it is not a skill score: a model that predicted every path would score 100 %.
//! The missing half can be reconstructed, carefully. In a given two-minute cycle
//! on a given band:
//!
//! * transmitter A was on the air - somebody spotted it;
//! * receiver W was awake and healthy on that band - it spotted several other
//!   stations in that same cycle;
//! * W did not spot A.
//!
//! That is a path that was attempted and failed, which is exactly the
//! observation the archive lacks. [`Negative`] records one.
//!
//! It is not a clean experiment, and the caveats are structural rather than
//! incidental:
//!
//! * **W may have been listening on another band.** Guarded against by requiring
//!   W to have spotted [`MIN_RX_SPOTS_FOR_HEALTHY`] stations on A's OWN band in
//!   that cycle, not merely to have been active somewhere.
//! * **A's transmission may have collided** with another signal in the same
//!   200 Hz window at W. Unknowable from the archive; it inflates the negative
//!   set with paths that were open but unlucky, which biases the measured
//!   false-positive rate UPWARD, i.e. against the model.
//! * **W's antenna may be directional and pointed elsewhere.** Also unknowable,
//!   and also a bias against the model.
//! * **A's transmission is not in every cycle.** WSPR stations transmit in a
//!   fraction of cycles. Requiring that A was spotted by somebody in the cycle
//!   establishes that it did transmit then.
//!
//! Both unknowables push the same way, so a false-positive rate measured here is
//! an UPPER BOUND on the model's true false-positive rate. That is the honest
//! direction for it to err in, and it is stated wherever the number is reported.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::grid;
use crate::wspr::{WsprSpot, parse_spots};

/// How many DIFFERENT transmitters a receiver must have decoded, in the same
/// cycle and on the same band, before its silence about one more is evidence.
///
/// One spot is not enough: a receiver that heard exactly one station may have
/// been mid-restart, mid-band-change, or running an antenna that only works in
/// one direction. Three is enough to establish that the receiver was awake, tuned
/// to that band, and hearing in more than one direction.
pub const MIN_RX_SPOTS_FOR_HEALTHY: usize = 3;

/// How far the archive's own `distance` may differ from the great-circle range
/// the solver computes before the row is dropped, as a fraction.
///
/// The two are computed from the same grid squares, so they should agree to the
/// grid resolution. A disagreement beyond this means the archive's distance was
/// computed from a DIFFERENT position than the grid states - a station whose
/// reported locator does not match where it actually is - and the path being
/// fitted is then not the path that was measured.
pub const MAX_DISTANCE_MISMATCH: f64 = 0.05;

/// Minimum spots a station must appear in before its fixed effect is estimated.
///
/// A station seen twice contributes two residuals and one unknown, so its effect
/// absorbs its own residual almost exactly and tells the fit nothing about the
/// physics. Ten is the point at which the effect is pinned well enough that the
/// remaining variation is the physics rather than the station.
pub const MIN_SPOTS_PER_STATION: usize = 10;

/// One corpus row: a measured spot plus the solar activity observed on its own
/// day.
///
/// The SSN is per-DAY and not per-corpus: a corpus spanning weeks spans real
/// changes in solar activity, and scoring all of it against one number would
/// charge the model for an input error.
#[derive(Clone, Debug)]
pub struct CorpusSpot {
    pub spot: WsprSpot,
    /// Observed sunspot number for this spot's UTC date.
    pub ssn: f64,
    /// Where that number came from, for the provenance header.
    pub ssn_source: String,
}

impl CorpusSpot {
    /// `(year, month, day)` - the key the SSN is looked up by.
    #[must_use]
    pub fn date(&self) -> (i32, u32, u32) {
        let t = self.spot.timestamp;
        (t.0, t.1, t.2)
    }

    /// The two-minute cycle this spot belongs to, as the archive spells it.
    #[must_use]
    pub fn cycle(&self) -> String {
        let t = self.spot.timestamp;
        format!("{:04}-{:02}-{:02} {:02}:{:02}", t.0, t.1, t.2, t.3, t.4)
    }
}

/// A path that was attempted and did not decode. See the module docs for what
/// this can and cannot mean.
#[derive(Clone, Debug)]
pub struct Negative {
    /// The transmitting station, which demonstrably transmitted in this cycle.
    pub tx_call: String,
    pub tx_grid: String,
    /// Claimed power, taken from the spots this transmitter DID produce in the
    /// same cycle - it is a property of the transmission, not of the receiver.
    pub tx_dbm: f64,
    /// The receiving station, which demonstrably decoded
    /// [`MIN_RX_SPOTS_FOR_HEALTHY`] other stations on this band in this cycle.
    pub rx_call: String,
    pub rx_grid: String,
    pub freq_mhz: f64,
    pub timestamp: (i32, u32, u32, u32, u32),
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
    /// How many stations the receiver DID hear on this band in this cycle. The
    /// evidence that its silence means something.
    pub rx_heard: usize,
    pub ssn: f64,
}

impl Negative {
    /// This negative as a [`WsprSpot`], so the solver can be handed it through
    /// exactly the same path a positive takes. The SNR field carries
    /// [`NEGATIVE_SNR_SENTINEL`] and must never be scored as a measurement.
    #[must_use]
    pub fn as_spot(&self) -> WsprSpot {
        WsprSpot {
            timestamp: self.timestamp,
            tx_call: self.tx_call.clone(),
            freq_mhz: self.freq_mhz,
            snr_db: NEGATIVE_SNR_SENTINEL,
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
}

/// The SNR a negative carries. Not a measurement: a negative has no measured
/// SNR, and anything that averages this into an error statistic is wrong.
///
/// NaN rather than a large negative number, deliberately. A sentinel like
/// -999 would flow through an arithmetic mean and produce a plausible-looking
/// wrong answer; NaN makes any such mistake visible immediately.
pub const NEGATIVE_SNR_SENTINEL: f64 = f64::NAN;

impl Negative {
    /// Confirms this negative carries no measured SNR. The guard that makes the
    /// sentinel choice above self-enforcing.
    #[must_use]
    pub fn snr_is_absent(&self) -> bool {
        self.as_spot().snr_db.is_nan()
    }
}

/// What each filter removed while a corpus was assembled. Reported, never
/// silently applied.
#[derive(Clone, Debug, Default)]
pub struct Rejections {
    pub unparseable_rows: usize,
    pub duplicate_spots: usize,
    pub distance_mismatch: usize,
    pub station_too_rare: usize,
    pub extreme_station_effect: usize,
    /// The worst distance disagreements seen, for the report to show rather than
    /// summarise: `(tx_grid, rx_grid, archive_km, solved_km)`.
    pub worst_mismatches: Vec<(String, String, f64, f64)>,
}

impl Rejections {
    fn note_mismatch(&mut self, tx: &str, rx: &str, archive_km: f64, solved_km: f64) {
        self.distance_mismatch += 1;
        if self.worst_mismatches.len() < 8 {
            self.worst_mismatches
                .push((tx.to_string(), rx.to_string(), archive_km, solved_km));
        }
    }
}

/// De-duplicate and reject rows the fit cannot use.
///
/// `great_circle_km` is passed in rather than computed here so that the check is
/// against the SAME range function the solver uses; a cross-check against an
/// independent reimplementation of the great circle would be testing this module
/// instead of the data.
#[must_use]
pub fn clean(
    spots: Vec<CorpusSpot>,
    great_circle_km: impl Fn(&WsprSpot) -> f64,
) -> (Vec<CorpusSpot>, Rejections) {
    let mut rej = Rejections::default();
    let mut seen: BTreeSet<(String, String, String, i64)> = BTreeSet::new();
    let mut kept = Vec::with_capacity(spots.len());

    for s in spots {
        // One transmission heard by one receiver in one cycle on one band is ONE
        // observation however many times the archive carries it. The band is
        // keyed by rounded kHz so two rows a few Hz apart - the same transmission
        // measured with slightly different frequency estimates - collapse.
        #[allow(clippy::cast_possible_truncation)]
        let key = (
            s.cycle(),
            s.spot.tx_call.clone(),
            s.spot.rx_call.clone(),
            (s.spot.freq_mhz * 100.0).round() as i64,
        );
        if !seen.insert(key) {
            rej.duplicate_spots += 1;
            continue;
        }

        let solved = great_circle_km(&s.spot);
        let archive = s.spot.reported_km;
        // Compared against the LONGER of the two so a near-zero archive distance
        // cannot pass by making the denominator small.
        let scale = solved.max(archive).max(1.0);
        if !solved.is_finite() || (solved - archive).abs() / scale > MAX_DISTANCE_MISMATCH {
            rej.note_mismatch(&s.spot.tx_grid, &s.spot.rx_grid, archive, solved);
            continue;
        }
        kept.push(s);
    }
    (kept, rej)
}

/// Drop spots whose TX or RX station appears fewer than
/// [`MIN_SPOTS_PER_STATION`] times, iterating until the corpus is stable.
///
/// One pass is not enough: removing a rare receiver's spots can take a
/// transmitter below the threshold, which can take another receiver below it. The
/// loop runs to a fixed point so the surviving corpus really does satisfy the
/// condition it claims to.
#[must_use]
pub fn require_identifiable_stations(
    mut spots: Vec<CorpusSpot>,
    rej: &mut Rejections,
) -> Vec<CorpusSpot> {
    loop {
        let mut tx_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut rx_counts: BTreeMap<String, usize> = BTreeMap::new();
        for s in &spots {
            *tx_counts.entry(s.spot.tx_call.clone()).or_default() += 1;
            *rx_counts.entry(s.spot.rx_call.clone()).or_default() += 1;
        }
        let before = spots.len();
        spots.retain(|s| {
            tx_counts[&s.spot.tx_call] >= MIN_SPOTS_PER_STATION
                && rx_counts[&s.spot.rx_call] >= MIN_SPOTS_PER_STATION
        });
        let removed = before - spots.len();
        rej.station_too_rare += removed;
        if removed == 0 {
            return spots;
        }
    }
}

/// Build negatives from one cycle-and-band census.
///
/// `census` is every spot the archive holds for that cycle on that band, already
/// hygiene-filtered. `min_km`/`max_km` bound which of the constructed pairs are
/// kept, matching the corpus's own range window - a 40 km pair is not a path this
/// model claims to predict.
#[must_use]
pub fn negatives_from_cycle(
    census: &[WsprSpot],
    min_km: f64,
    max_km: f64,
    ssn: f64,
    great_circle_km: impl Fn(&WsprSpot) -> f64,
) -> Vec<Negative> {
    // Who transmitted, and with what claimed power. A transmitter that appears in
    // this census demonstrably radiated in this cycle.
    let mut transmitters: BTreeMap<&str, &WsprSpot> = BTreeMap::new();
    // Which distinct transmitters each receiver decoded. `BTreeSet` because
    // hearing the same station twice is not two pieces of evidence of health.
    let mut receiver_heard: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    // The pairs that DID work, which are exactly the ones that are not negatives.
    let mut decoded: BTreeSet<(&str, &str)> = BTreeSet::new();

    for s in census {
        transmitters.entry(s.tx_call.as_str()).or_insert(s);
        receiver_heard
            .entry(s.rx_call.as_str())
            .or_default()
            .insert(s.tx_call.as_str());
        decoded.insert((s.tx_call.as_str(), s.rx_call.as_str()));
    }

    let mut out = Vec::new();
    for (rx_call, heard) in &receiver_heard {
        if heard.len() < MIN_RX_SPOTS_FOR_HEALTHY {
            continue;
        }
        // Any spot this receiver produced carries its grid.
        let Some(rx_row) = census.iter().find(|s| s.rx_call == *rx_call) else {
            continue;
        };
        for (tx_call, tx_row) in &transmitters {
            if tx_call == rx_call || decoded.contains(&(*tx_call, *rx_call)) {
                continue;
            }
            let Some((tx_lat, tx_lon)) = grid::decode(&tx_row.tx_grid) else {
                continue;
            };
            let probe = WsprSpot {
                tx_lat,
                tx_lon,
                rx_lat: rx_row.rx_lat,
                rx_lon: rx_row.rx_lon,
                ..(*tx_row).clone()
            };
            let km = great_circle_km(&probe);
            if !km.is_finite() || km < min_km || km > max_km {
                continue;
            }
            out.push(Negative {
                tx_call: (*tx_call).to_string(),
                tx_grid: tx_row.tx_grid.clone(),
                tx_dbm: tx_row.tx_dbm,
                rx_call: (*rx_call).to_string(),
                rx_grid: rx_row.rx_grid.clone(),
                freq_mhz: tx_row.freq_mhz,
                timestamp: tx_row.timestamp,
                tx_lat,
                tx_lon,
                rx_lat: rx_row.rx_lat,
                rx_lon: rx_row.rx_lon,
                rx_heard: heard.len(),
                ssn,
            });
        }
    }
    out
}

// --- On-disk format -------------------------------------------------------

/// Serialise positives as the nine-column TSV [`parse_spots`] reads, with the
/// per-day SSN as a tenth column and a `#` provenance header.
///
/// Kept parser-compatible on purpose: the ten-column rows load through the same
/// [`parse_spots`] the live path uses, which ignores trailing fields, so there is
/// no second spot parser to keep in step with the first.
#[must_use]
pub fn write_positives(spots: &[CorpusSpot], provenance: &str) -> String {
    let mut s = String::new();
    for line in provenance.lines() {
        let _ = writeln!(s, "# {line}");
    }
    let _ = writeln!(
        s,
        "# columns: timestamp  tx_call  MHz  SNR  tx_grid  dBm  rx_call  rx_grid  km  SSN"
    );
    for c in spots {
        let t = c.spot.timestamp;
        let _ = writeln!(
            s,
            "{:04}-{:02}-{:02} {:02}:{:02}\t{}\t{:.6}\t{}\t{}\t{}\t{}\t{}\t{:.0}\t{:.1}",
            t.0,
            t.1,
            t.2,
            t.3,
            t.4,
            c.spot.tx_call,
            c.spot.freq_mhz,
            c.spot.snr_db,
            c.spot.tx_grid,
            c.spot.tx_dbm,
            c.spot.rx_call,
            c.spot.rx_grid,
            c.spot.reported_km,
            c.ssn,
        );
    }
    s
}

/// Read back what [`write_positives`] wrote.
///
/// The SSN column is required: a corpus row whose solar activity is unknown would
/// silently fall back to a default, and the fit would then be scoring that
/// default as much as the model.
#[must_use]
pub fn read_positives(text: &str) -> (Vec<CorpusSpot>, Vec<String>) {
    let (spots, mut problems) = parse_spots(text);
    let ssns: Vec<Option<f64>> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|l| l.split('\t').nth(9).and_then(|v| v.trim().parse().ok()))
        .collect();
    if ssns.len() != spots.len() {
        problems.push(format!(
            "{} readable spot(s) but {} row(s): the SSN column cannot be matched up, \
             so the corpus is not usable",
            spots.len(),
            ssns.len()
        ));
        return (Vec::new(), problems);
    }
    let mut out = Vec::with_capacity(spots.len());
    for (i, (spot, ssn)) in spots.into_iter().zip(ssns).enumerate() {
        match ssn {
            Some(v) => out.push(CorpusSpot {
                spot,
                ssn: v,
                ssn_source: "corpus file".to_string(),
            }),
            None => problems.push(format!("row {}: missing or unreadable SSN column", i + 1)),
        }
    }
    (out, problems)
}

/// Serialise negatives. A separate format from the positives on purpose: a
/// negative has no measured SNR, and a file that gave it one - even a sentinel in
/// an SNR column - invites something downstream to average it in.
#[must_use]
pub fn write_negatives(negatives: &[Negative], provenance: &str) -> String {
    let mut s = String::new();
    for line in provenance.lines() {
        let _ = writeln!(s, "# {line}");
    }
    let _ = writeln!(
        s,
        "# NO MEASURED SNR: each row is a path that was attempted and did not decode."
    );
    let _ = writeln!(
        s,
        "# columns: timestamp  tx_call  MHz  tx_grid  dBm  rx_call  rx_grid  rx_heard  SSN"
    );
    for n in negatives {
        let t = n.timestamp;
        let _ = writeln!(
            s,
            "{:04}-{:02}-{:02} {:02}:{:02}\t{}\t{:.6}\t{}\t{}\t{}\t{}\t{}\t{:.1}",
            t.0,
            t.1,
            t.2,
            t.3,
            t.4,
            n.tx_call,
            n.freq_mhz,
            n.tx_grid,
            n.tx_dbm,
            n.rx_call,
            n.rx_grid,
            n.rx_heard,
            n.ssn,
        );
    }
    s
}

/// Read back what [`write_negatives`] wrote.
#[must_use]
pub fn read_negatives(text: &str) -> (Vec<Negative>, Vec<String>) {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        match parse_negative(line) {
            Ok(n) => out.push(n),
            Err(e) => problems.push(format!("line {}: {e}", i + 1)),
        }
    }
    (out, problems)
}

fn parse_negative(line: &str) -> Result<Negative, String> {
    let f: Vec<&str> = line.split('\t').map(str::trim).collect();
    if f.len() < 9 {
        return Err(format!("expected 9 fields, got {}", f.len()));
    }
    let num = |s: &str, what: &str| -> Result<f64, String> {
        s.parse::<f64>().map_err(|e| format!("bad {what}: {e}"))
    };
    let (date, time) = f[0]
        .split_once([' ', '_'])
        .ok_or_else(|| format!("timestamp {:?} has no date/time separator", f[0]))?;
    let d: Vec<&str> = date.split('-').collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() < 2 {
        return Err(format!("timestamp {:?} is not YYYY-MM-DD HH:MM", f[0]));
    }
    let p = |v: &str| -> Result<u32, String> { v.parse().map_err(|e| format!("{e}")) };
    let timestamp = (
        d[0].parse::<i32>().map_err(|e| format!("bad year: {e}"))?,
        p(d[1])?,
        p(d[2])?,
        p(t[0])?,
        p(t[1])?,
    );
    let tx_grid = f[3].to_string();
    let rx_grid = f[6].to_string();
    let (tx_lat, tx_lon) =
        grid::decode(&tx_grid).ok_or_else(|| format!("unparseable TX grid {tx_grid:?}"))?;
    let (rx_lat, rx_lon) =
        grid::decode(&rx_grid).ok_or_else(|| format!("unparseable RX grid {rx_grid:?}"))?;
    Ok(Negative {
        tx_call: f[1].to_string(),
        tx_grid,
        tx_dbm: num(f[4], "power")?,
        rx_call: f[5].to_string(),
        rx_grid,
        freq_mhz: num(f[2], "frequency")?,
        timestamp,
        tx_lat,
        tx_lon,
        rx_lat,
        rx_lon,
        rx_heard: f[7]
            .parse()
            .map_err(|e| format!("bad rx_heard: {e}"))?,
        ssn: num(f[8], "SSN")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(cycle_min: u32, tx: &str, rx: &str, mhz: f64, km: f64) -> WsprSpot {
        // DN70 is Colorado, DN80 is ~400 km east of it; EM48 is Missouri.
        let (tx_grid, rx_grid) = ("DN70aa", "DN80aa");
        let (tx_lat, tx_lon) = grid::decode(tx_grid).unwrap();
        let (rx_lat, rx_lon) = grid::decode(rx_grid).unwrap();
        WsprSpot {
            timestamp: (2026, 7, 27, 4, cycle_min),
            tx_call: tx.to_string(),
            freq_mhz: mhz,
            snr_db: -15.0,
            tx_grid: tx_grid.to_string(),
            tx_dbm: 23.0,
            rx_call: rx.to_string(),
            rx_grid: rx_grid.to_string(),
            reported_km: km,
            tx_lat,
            tx_lon,
            rx_lat,
            rx_lon,
        }
    }

    fn corpus(spot: WsprSpot) -> CorpusSpot {
        CorpusSpot {
            spot,
            ssn: 119.0,
            ssn_source: "test".into(),
        }
    }

    /// The same transmission heard by the same receiver in the same cycle is one
    /// observation, however many rows the archive carries for it - including rows
    /// whose frequency estimate differs by a few hertz.
    #[test]
    fn duplicates_collapse_to_one_observation() {
        let rows = vec![
            corpus(spot(2, "K1ABC", "W9XYZ", 14.097_100, 400.0)),
            corpus(spot(2, "K1ABC", "W9XYZ", 14.097_104, 400.0)),
            corpus(spot(4, "K1ABC", "W9XYZ", 14.097_100, 400.0)),
            corpus(spot(2, "K1ABC", "N0DEF", 14.097_100, 400.0)),
        ];
        let (kept, rej) = clean(rows, |_| 400.0);
        assert_eq!(kept.len(), 3, "one duplicate should have gone");
        assert_eq!(rej.duplicate_spots, 1);
    }

    /// A row whose archive distance disagrees with the solved great circle is a
    /// row whose stated location is wrong, and it must be dropped and COUNTED.
    #[test]
    fn distance_mismatch_is_dropped_and_reported() {
        let rows = vec![
            corpus(spot(2, "A", "B", 14.097, 400.0)),
            corpus(spot(4, "C", "D", 14.097, 4000.0)),
        ];
        let (kept, rej) = clean(rows, |_| 400.0);
        assert_eq!(kept.len(), 1);
        assert_eq!(rej.distance_mismatch, 1);
        assert_eq!(rej.worst_mismatches.len(), 1);
        // A few percent must still pass: the grids really are +/- a few km.
        let ok = vec![corpus(spot(2, "A", "B", 14.097, 408.0))];
        let (kept, rej) = clean(ok, |_| 400.0);
        assert_eq!(kept.len(), 1, "2 % is inside the grid resolution");
        assert_eq!(rej.distance_mismatch, 0);
    }

    /// The station-count filter has to run to a fixed point: dropping a rare
    /// receiver can push a transmitter below the threshold in turn.
    #[test]
    fn rare_stations_are_removed_to_a_fixed_point() {
        let mut rows = Vec::new();
        // A busy pair, comfortably over the threshold.
        for i in 0..MIN_SPOTS_PER_STATION {
            #[allow(clippy::cast_possible_truncation)]
            rows.push(corpus(spot(2 * i as u32, "BUSYTX", "BUSYRX", 14.097, 400.0)));
        }
        // A rare receiver that heard the busy transmitter a handful of times.
        // Removing those rows must not take BUSYTX below the threshold, because
        // it still has its own ten.
        for i in 0..3 {
            rows.push(corpus(spot(40 + i, "BUSYTX", "RARERX", 14.097, 400.0)));
        }
        // A pair that only ever heard each other twice: both must go.
        rows.push(corpus(spot(50, "RARETX", "RARERX2", 14.097, 400.0)));
        rows.push(corpus(spot(52, "RARETX", "RARERX2", 14.097, 400.0)));

        let mut rej = Rejections::default();
        let kept = require_identifiable_stations(rows, &mut rej);
        assert!(kept.iter().all(|s| s.spot.rx_call == "BUSYRX"));
        assert_eq!(kept.len(), MIN_SPOTS_PER_STATION);
        assert_eq!(rej.station_too_rare, 5);

        // And the invariant the function claims really holds afterwards.
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &kept {
            *counts.entry(s.spot.rx_call.as_str()).or_default() += 1;
        }
        assert!(counts.values().all(|&n| n >= MIN_SPOTS_PER_STATION));
    }

    /// The negatives construction, on a census small enough to check by hand.
    ///
    /// Four transmitters were on the air. RX_GOOD heard all four, so it is
    /// demonstrably healthy but has no silence left to explain. RX_OK heard
    /// three, enough to be healthy, and did not hear TX4, which is exactly one
    /// negative. RX_DEAF heard two, which is below
    /// [`MIN_RX_SPOTS_FOR_HEALTHY`] and so is not evidence of anything, however
    /// many stations it missed.
    #[test]
    fn negatives_need_a_demonstrably_healthy_receiver() {
        let census = vec![
            spot(2, "TX1", "RX_GOOD", 14.097, 400.0),
            spot(2, "TX2", "RX_GOOD", 14.097, 400.0),
            spot(2, "TX3", "RX_GOOD", 14.097, 400.0),
            spot(2, "TX4", "RX_GOOD", 14.097, 400.0),
            spot(2, "TX1", "RX_OK", 14.097, 400.0),
            spot(2, "TX2", "RX_OK", 14.097, 400.0),
            spot(2, "TX3", "RX_OK", 14.097, 400.0),
            spot(2, "TX1", "RX_DEAF", 14.097, 400.0),
            spot(2, "TX2", "RX_DEAF", 14.097, 400.0),
        ];
        let negs = negatives_from_cycle(&census, 300.0, 20_000.0, 119.0, |_| 400.0);
        assert_eq!(negs.len(), 1, "{negs:#?}");
        let n = &negs[0];
        assert_eq!(n.tx_call, "TX4");
        assert_eq!(n.rx_call, "RX_OK");
        assert_eq!(n.rx_heard, MIN_RX_SPOTS_FOR_HEALTHY);
        assert!(n.snr_is_absent());
        // The power comes from the transmission, not from the silent receiver.
        assert!((n.tx_dbm - 23.0).abs() < 1e-9);

        // One more station heard by RX_DEAF tips it over the threshold, and then
        // its two misses DO become negatives. The threshold is the only thing
        // standing between "quiet receiver" and "evidence".
        let mut more = census.clone();
        more.push(spot(2, "TX3", "RX_DEAF", 14.097, 400.0));
        let negs = negatives_from_cycle(&more, 300.0, 20_000.0, 119.0, |_| 400.0);
        assert_eq!(negs.len(), 2, "{negs:#?}");
        assert!(negs.iter().any(|n| n.rx_call == "RX_DEAF"));
    }

    /// A pair outside the corpus's range window is not a path the model claims,
    /// so it is not a fair negative either.
    #[test]
    fn negatives_respect_the_range_window() {
        let census = vec![
            spot(2, "TX1", "RX", 14.097, 400.0),
            spot(2, "TX2", "RX", 14.097, 400.0),
            spot(2, "TX3", "RX", 14.097, 400.0),
            spot(2, "TX4", "RX2", 14.097, 400.0),
        ];
        // TX4 is unheard by RX, but at 50 km it is below the window.
        let near = negatives_from_cycle(&census, 300.0, 20_000.0, 119.0, |_| 50.0);
        assert!(near.is_empty(), "{near:#?}");
        let inside = negatives_from_cycle(&census, 300.0, 20_000.0, 119.0, |_| 900.0);
        assert_eq!(inside.len(), 1);
    }

    /// Positives and negatives must survive a round trip through their files,
    /// SSN included. A corpus that loses its solar activity on save would have
    /// the fit silently scoring a default.
    #[test]
    fn both_files_round_trip() {
        let spots = vec![
            corpus(spot(2, "K1ABC", "W9XYZ", 14.097_1, 400.0)),
            corpus(spot(4, "K1ABC", "N0DEF", 7.040_1, 1200.0)),
        ];
        let text = write_positives(&spots, "test corpus\nsecond provenance line");
        assert!(text.starts_with("# test corpus"));
        let (back, problems) = read_positives(&text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].spot.tx_call, "K1ABC");
        assert!((back[0].ssn - 119.0).abs() < 1e-9);
        assert!((back[1].spot.freq_mhz - 7.040_1).abs() < 1e-6);
        assert_eq!(back[1].spot.timestamp, (2026, 7, 27, 4, 4));

        // A corpus row with no SSN column must be REPORTED, not defaulted.
        let (bad, problems) = read_positives(
            "2026-07-27 04:02\tK1ABC\t14.0971\t-15\tDN70aa\t23\tW9XYZ\tDN80aa\t400\n",
        );
        assert!(bad.is_empty());
        assert!(problems.iter().any(|p| p.contains("SSN")), "{problems:?}");

        let negs = negatives_from_cycle(
            &[
                spot(2, "TX1", "RX", 14.097, 400.0),
                spot(2, "TX2", "RX", 14.097, 400.0),
                spot(2, "TX3", "RX", 14.097, 400.0),
                spot(2, "TX4", "RX2", 14.097, 400.0),
            ],
            300.0,
            20_000.0,
            119.0,
            |_| 900.0,
        );
        let text = write_negatives(&negs, "test negatives");
        assert!(text.contains("NO MEASURED SNR"));
        let (back, problems) = read_negatives(&text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.len(), negs.len());
        assert_eq!(back[0].tx_call, negs[0].tx_call);
        assert_eq!(back[0].rx_heard, negs[0].rx_heard);
        assert!((back[0].ssn - 119.0).abs() < 1e-9);
    }
}
