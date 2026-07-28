//! WSPR spot ingest and model-versus-measurement scoring.
//!
//! A WSPR spot is an unusually good validation datum for this model: a measured
//! SNR, on a known frequency, over a known path, at a known instant, from a
//! known transmitter power. There is no operator judgment in it.
//!
//! # What the comparison can and cannot mean
//!
//! WSPR reports SNR in a **2500 Hz reference bandwidth** even though the signal
//! occupies about 6 Hz, which is why reported values cluster well below 0 dB and
//! why the decode threshold is around -29 dB. Comparing anything else against
//! those numbers is meaningless, so [`WsprSpot::inputs_for`] pins the receiver
//! bandwidth to 2500 Hz and the threshold to -29 dB regardless of what the
//! interactive session is set to.
//!
//! Two known one-sided biases, stated rather than corrected for:
//!
//! * **Antennas are assumed, not known.** A spot carries no antenna
//!   information. The harness uses whatever [`Inputs`] carries, which for both
//!   ends is a default. Real WSPR stations are frequently worse than that, so
//!   the model will tend to over-predict.
//! * **A spot is a SUCCESS, and only successes are published.** The database has
//!   no record of the times the same path did not decode. So the hit rate below
//!   is "of the openings that were observed, how many does the model find" - it
//!   says nothing about false positives, and it cannot. A model that predicted
//!   every path would score a perfect hit rate here.
//!
//! Both of those mean the harness measures BIAS AND SPREAD, which is what it is
//! for, and not skill.

use crate::grid;
use crate::noise::OperatingMode;
use crate::scenario::Inputs;

/// Reference bandwidth WSPR SNRs are quoted in, Hz.
pub const WSPR_REFERENCE_BANDWIDTH_HZ: f64 = 2500.0;
/// Nominal WSPR decode threshold in that bandwidth, dB.
pub const WSPR_DECODE_THRESHOLD_DB: f64 = -29.0;

/// One row of a WSPR spot export.
#[derive(Clone, Debug, PartialEq)]
pub struct WsprSpot {
    /// UTC timestamp, as `(year, month, day, hour, minute)`.
    pub timestamp: (i32, u32, u32, u32, u32),
    pub tx_call: String,
    pub freq_mhz: f64,
    /// Reported SNR in the 2500 Hz reference bandwidth, dB.
    pub snr_db: f64,
    pub tx_grid: String,
    /// Transmitter power as reported, dBm.
    pub tx_dbm: f64,
    pub rx_call: String,
    pub rx_grid: String,
    /// Distance as reported by the spot source, km. Kept for cross-checking
    /// against the great-circle distance the solver computes itself.
    pub reported_km: f64,
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
}

impl WsprSpot {
    /// UTC hour as a fraction, for the solar geometry.
    #[must_use]
    pub fn utc_hours(&self) -> f64 {
        f64::from(self.timestamp.3) + f64::from(self.timestamp.4) / 60.0
    }

    /// Transmitter power in watts, from the reported dBm.
    #[must_use]
    pub fn tx_power_w(&self) -> f64 {
        10.0_f64.powf((self.tx_dbm - 30.0) / 10.0)
    }

    /// Scenario inputs for this spot: everything the spot states is taken from
    /// it, everything it does not state is left to `base`.
    ///
    /// The bandwidth and threshold are overridden unconditionally, because a
    /// WSPR SNR only means anything in 2500 Hz. Those two are properties of the
    /// measurement and are not negotiable.
    ///
    /// The receiver's NOISE ENVIRONMENT is not in that category and is left to
    /// `base`. It used to be pinned to [`NoiseEnvironment::Rural`] here, next to
    /// the bandwidth, which put an assumption about every receiver's local noise
    /// in the same place as a fact about the measurement and made it unreachable
    /// from any harness. It is worth roughly 10 dB between city and quiet rural,
    /// it is chosen rather than known, and a calibration must be able to say so
    /// out loud rather than fitting around it. Note that it is also very nearly
    /// UNIDENTIFIABLE from WSPR: the environment is a constant per receiver, so a
    /// two-way fixed-effects fit absorbs it into that receiver's effect almost
    /// exactly (see [`crate::fit`]).
    #[must_use]
    pub fn inputs_for(&self, base: &Inputs) -> Inputs {
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
            tx_power_w: self.tx_power_w(),
            bandwidth_hz: WSPR_REFERENCE_BANDWIDTH_HZ,
            snr_threshold_db: WSPR_DECODE_THRESHOLD_DB,
            op_mode: OperatingMode::Ft8,
            ..base.clone()
        }
    }
}

/// Parse a WSPR spot list.
///
/// Tab-separated, one spot per line, `#` comments and blank lines ignored:
///
/// ```text
/// timestamp  tx_call  MHz  SNR  tx_grid  dBm  rx_call  rx_grid  km
/// ```
///
/// The timestamp accepts the two forms the common exports use,
/// `YYYY-MM-DD HH:MM` and `YYYY-MM-DD_HH:MM`; since the field separator is a
/// tab, a space inside the timestamp is unambiguous.
///
/// A malformed row is REPORTED, not skipped. A validation harness that silently
/// drops rows it cannot read reports a bias measured over an unknown subset of
/// the data, which is worse than reporting no bias at all.
#[must_use]
pub fn parse_spots(text: &str) -> (Vec<WsprSpot>, Vec<String>) {
    let mut spots = Vec::new();
    let mut problems = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        match parse_row(line) {
            Ok(s) => spots.push(s),
            Err(e) => problems.push(format!("line {}: {e}", i + 1)),
        }
    }
    (spots, problems)
}

fn parse_row(line: &str) -> Result<WsprSpot, String> {
    let f: Vec<&str> = line.split('\t').map(str::trim).collect();
    if f.len() < 9 {
        return Err(format!("expected 9 tab-separated fields, got {}", f.len()));
    }
    let timestamp = parse_timestamp(f[0])?;
    let num = |s: &str, what: &str| -> Result<f64, String> {
        s.parse::<f64>()
            .map_err(|e| format!("bad {what} {s:?}: {e}"))
    };
    let tx_grid = f[4].to_string();
    let rx_grid = f[7].to_string();
    let (tx_lat, tx_lon) =
        grid::decode(&tx_grid).ok_or_else(|| format!("unparseable TX grid {tx_grid:?}"))?;
    let (rx_lat, rx_lon) =
        grid::decode(&rx_grid).ok_or_else(|| format!("unparseable RX grid {rx_grid:?}"))?;

    Ok(WsprSpot {
        timestamp,
        tx_call: f[1].to_string(),
        freq_mhz: num(f[2], "frequency")?,
        snr_db: num(f[3], "SNR")?,
        tx_grid,
        tx_dbm: num(f[5], "power")?,
        rx_call: f[6].to_string(),
        rx_grid,
        reported_km: num(f[8], "distance")?,
        tx_lat,
        tx_lon,
        rx_lat,
        rx_lon,
    })
}

fn parse_timestamp(s: &str) -> Result<(i32, u32, u32, u32, u32), String> {
    let (date, time) = s
        .split_once([' ', '_'])
        .ok_or_else(|| format!("timestamp {s:?} has no date/time separator"))?;
    let d: Vec<&str> = date.split('-').collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() < 2 {
        return Err(format!("timestamp {s:?} is not YYYY-MM-DD HH:MM"));
    }
    let p = |v: &str, what: &str| -> Result<u32, String> {
        v.parse::<u32>()
            .map_err(|e| format!("bad {what} in timestamp {s:?}: {e}"))
    };
    let year: i32 = d[0]
        .parse()
        .map_err(|e| format!("bad year in timestamp {s:?}: {e}"))?;
    let (month, day) = (p(d[1], "month")?, p(d[2], "day")?);
    let (hour, minute) = (p(t[0], "hour")?, p(t[1], "minute")?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return Err(format!("timestamp {s:?} is out of range"));
    }
    Ok((year, month, day, hour, minute))
}

/// How one spot scored.
#[derive(Clone)]
pub struct SpotResult {
    pub spot: WsprSpot,
    /// Great-circle range the solver computed, km. Compared against the spot's
    /// own reported distance as a cross-check on the grid decode.
    pub solved_km: f64,
    /// Modelled SNR of the best DETERMINISTIC path, dB; `None` if none closed.
    pub deterministic_snr_db: Option<f64>,
    /// Best sporadic-E path as `(SNR dB, occurrence probability)`; `None` if
    /// none closed.
    pub es: Option<(f64, f64)>,
    /// The layer behind `modelled_snr_db`.
    pub layer: Option<&'static str>,
    /// Combined antenna gain the model credited this path with, dB (both ends
    /// summed, read at the launch and arrival elevations the ray actually
    /// used). Carried because it is an ASSUMPTION, not a measurement - the spot
    /// says nothing about either station's antenna - and a report that quotes
    /// a bias without also quoting the gain it handed itself is not showing
    /// where that bias came from.
    pub assumed_gain_db: Option<f64>,
    /// Noise floor the model scored against, dBm. Also an assumption: the
    /// receiver's noise environment is chosen, not known.
    pub noise_dbm: Option<f64>,
    /// Best modelled SNR of any kind, dB; `None` if the path did not close.
    pub modelled_snr_db: Option<f64>,
    /// `modelled - measured` [dB]; `None` if the path did not close. Positive
    /// means the model is optimistic.
    pub error_db: Option<f64>,
    pub hops: u32,
}

impl SpotResult {
    /// Did the model find any path at all, for a spot that demonstrably
    /// happened?
    #[must_use]
    pub fn closed(&self) -> bool {
        self.modelled_snr_db.is_some()
    }
}

/// Aggregate statistics over a run.
pub struct Summary {
    pub spots: usize,
    /// Spots for which the model found a path of any kind.
    pub closed: usize,
    /// Of those, how many needed sporadic E.
    pub es_only: usize,
    /// Median of `modelled - measured` over the spots that closed, dB.
    pub median_error_db: f64,
    pub mean_error_db: f64,
    pub stdev_db: f64,
    /// Interquartile range of the error, dB - a spread one wild outlier cannot
    /// dominate, unlike the standard deviation.
    pub iqr_db: f64,
    pub p10_db: f64,
    pub p90_db: f64,
    /// Fraction of spots for which a path was found at all, 0..1.
    pub hit_rate: f64,
}

impl Summary {
    /// Score a set of results.
    ///
    /// Only spots whose path closed contribute to the error statistics; the
    /// ones that did not are counted in `hit_rate`, which is the honest place
    /// for them. Averaging in a "miss" as though it were an error of some
    /// particular size would invent a number.
    #[must_use]
    pub fn of(results: &[SpotResult]) -> Self {
        let mut errors: Vec<f64> = results.iter().filter_map(|r| r.error_db).collect();
        errors.sort_by(f64::total_cmp);
        let n = errors.len();
        #[allow(clippy::cast_precision_loss)]
        let count = n as f64;
        let mean = if n == 0 {
            f64::NAN
        } else {
            errors.iter().sum::<f64>() / count
        };
        let stdev = if n < 2 {
            f64::NAN
        } else {
            (errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / (count - 1.0)).sqrt()
        };
        #[allow(clippy::cast_precision_loss)]
        let total = results.len() as f64;
        Self {
            spots: results.len(),
            closed: n,
            es_only: results.iter().filter(|r| r.layer == Some("Es")).count(),
            median_error_db: percentile(&errors, 0.5),
            mean_error_db: mean,
            stdev_db: stdev,
            iqr_db: percentile(&errors, 0.75) - percentile(&errors, 0.25),
            p10_db: percentile(&errors, 0.1),
            p90_db: percentile(&errors, 0.9),
            hit_rate: if results.is_empty() {
                f64::NAN
            } else {
                count / total
            },
        }
    }
}

/// Linearly interpolated percentile of a SORTED slice; NaN when empty.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    #[allow(clippy::cast_precision_loss)]
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = lo as usize;
    let frac = pos - lo;
    if i + 1 >= sorted.len() {
        sorted[i]
    } else {
        sorted[i] + (sorted[i + 1] - sorted[i]) * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::NoiseEnvironment;

    const SAMPLE: &str = "\
# a comment
2026-07-24 03:22\tK1ABC\t18.106\t-11\tDN70\t23\tW9XYZ\tDN80\t406

2026-07-24_03:22\tK1ABC\t14.097\t-24\tDN70\t23\tG0ZZZ\tIO91\t7500
";

    #[test]
    fn parses_both_timestamp_forms_and_decodes_grids() {
        let (spots, problems) = parse_spots(SAMPLE);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(spots.len(), 2);

        let s = &spots[0];
        assert_eq!(s.timestamp, (2026, 7, 24, 3, 22));
        assert_eq!(s.tx_call, "K1ABC");
        assert!((s.freq_mhz - 18.106).abs() < 1e-9);
        assert!((s.snr_db - (-11.0)).abs() < 1e-9);
        assert!((s.utc_hours() - (3.0 + 22.0 / 60.0)).abs() < 1e-9);
        // 23 dBm is 200 mW, the power in the reported validation case.
        assert!(
            (s.tx_power_w() - 0.1995).abs() < 1e-3,
            "23 dBm = {} W",
            s.tx_power_w()
        );
        // DN70 is in Colorado; the decode must put it there.
        assert!((39.0..41.0).contains(&s.tx_lat), "tx lat {}", s.tx_lat);
        assert!((-106.0..-104.0).contains(&s.tx_lon), "tx lon {}", s.tx_lon);
        // The underscore form parses to the same instant.
        assert_eq!(spots[1].timestamp, s.timestamp);
    }

    /// A WSPR SNR is only meaningful in 2500 Hz, so the harness must pin the
    /// bandwidth and threshold whatever the session was set to.
    #[test]
    fn inputs_pin_the_wspr_reference_bandwidth() {
        let (spots, _) = parse_spots(SAMPLE);
        let base = Inputs {
            bandwidth_hz: 300.0,
            snr_threshold_db: 10.0,
            ssn: 42.0,
            ..Inputs::default()
        };
        let got = spots[0].inputs_for(&base);
        assert!((got.bandwidth_hz - WSPR_REFERENCE_BANDWIDTH_HZ).abs() < 1e-9);
        assert!((got.snr_threshold_db - WSPR_DECODE_THRESHOLD_DB).abs() < 1e-9);
        // The noise environment is an ASSUMPTION about the receiver, not a
        // property of the measurement, so it must come from the caller and be
        // changeable. Pinning it here would hide a ~10 dB choice.
        let city = Inputs {
            noise_env: NoiseEnvironment::City,
            ..base.clone()
        };
        assert_eq!(spots[0].inputs_for(&city).noise_env, NoiseEnvironment::City);
        assert_eq!(
            spots[0].inputs_for(&base).noise_env,
            base.noise_env,
            "the environment must be inherited, not overridden"
        );
        // Everything the spot does not state is inherited untouched.
        assert!((got.ssn - 42.0).abs() < 1e-9);
        // Everything it does state is taken from it.
        assert!((got.freq_mhz - 18.106).abs() < 1e-9);
        assert_eq!(got.month, 7);
        assert_eq!(got.day_of_month, 24);
    }

    /// A malformed row must be reported, never silently dropped.
    #[test]
    fn malformed_rows_are_reported_not_skipped() {
        let text = "2026-07-24 03:22\tK1ABC\t18.106\t-11\tDN70\t23\tW9XYZ\tDN80\t406\n\
                    too\tfew\tfields\n\
                    2026-07-24 03:22\tK1ABC\tnope\t-11\tDN70\t23\tW9XYZ\tDN80\t406\n\
                    2026-13-99 03:22\tK1ABC\t18.1\t-11\tDN70\t23\tW9XYZ\tDN80\t406\n\
                    2026-07-24 03:22\tK1ABC\t18.1\t-11\tZZ99\t23\tW9XYZ\tDN80\t406\n";
        let (spots, problems) = parse_spots(text);
        assert_eq!(spots.len(), 1);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems[0].contains("line 2") && problems[0].contains("9 tab"));
        assert!(problems[1].contains("frequency"));
        assert!(problems[2].contains("out of range"));
        assert!(problems[3].contains("grid"));
    }

    /// The statistics are computed only over spots that closed, and the hit
    /// rate is where the ones that did not are counted.
    #[test]
    fn summary_separates_error_statistics_from_the_hit_rate() {
        let (spots, _) = parse_spots(SAMPLE);
        let make = |err: Option<f64>, layer: Option<&'static str>| SpotResult {
            spot: spots[0].clone(),
            solved_km: 406.0,
            deterministic_snr_db: None,
            es: None,
            layer,
            modelled_snr_db: err.map(|e| e - 11.0),
            error_db: err,
            hops: 1,
            assumed_gain_db: err.map(|_| 10.4),
            noise_dbm: err.map(|_| -97.8),
        };
        let results = vec![
            make(Some(2.0), Some("F2")),
            make(Some(6.0), Some("Es")),
            make(Some(-4.0), Some("F2")),
            make(Some(10.0), Some("Es")),
            make(None, None),
        ];
        let s = Summary::of(&results);
        assert_eq!(s.spots, 5);
        assert_eq!(s.closed, 4);
        assert_eq!(s.es_only, 2);
        assert!((s.hit_rate - 0.8).abs() < 1e-12);
        // Sorted errors are [-4, 2, 6, 10]: median 4, mean 3.5.
        assert!(
            (s.median_error_db - 4.0).abs() < 1e-12,
            "{}",
            s.median_error_db
        );
        assert!((s.mean_error_db - 3.5).abs() < 1e-12);
        // Quartiles by linear interpolation on [-4, 2, 6, 10]: Q1 = 0.5,
        // Q3 = 7.0, so IQR = 6.5.
        assert!((s.iqr_db - 6.5).abs() < 1e-12, "{}", s.iqr_db);
        assert!(s.stdev_db > 0.0);

        // No spots at all must produce NaNs rather than a confident zero.
        let empty = Summary::of(&[]);
        assert!(empty.median_error_db.is_nan());
        assert!(empty.hit_rate.is_nan());
    }

    #[test]
    fn percentiles_interpolate_and_handle_edges() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert!((percentile(&v, 0.0) - 1.0).abs() < 1e-12);
        assert!((percentile(&v, 1.0) - 4.0).abs() < 1e-12);
        assert!((percentile(&v, 0.5) - 2.5).abs() < 1e-12);
        assert!(percentile(&[], 0.5).is_nan());
        assert!((percentile(&[7.0], 0.9) - 7.0).abs() < 1e-12);
    }
}
