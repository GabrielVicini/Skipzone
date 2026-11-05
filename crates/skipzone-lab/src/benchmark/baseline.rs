//! The ITU-R P.533 baseline, through ITURHFProp
//! (docs/notes/accuracy-roadmap.md §2.5).
//!
//! The benchmark neither builds nor runs ITURHFProp. It writes one input
//! file per (path, month, hour, power, mode) group for the owner to run
//! elsewhere, then reads the output files back and scores them on the same
//! rows as Skipzone.
//!
//! Both file formats follow the keys the roadmap lists. They have not been
//! checked against a real ITURHFProp run: the input keys, the hour
//! convention and the output column names must be confirmed on the first
//! real run, and the output parser's test fixture is synthetic.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::corpus::{Obs, Source};
use crate::wspr::{WSPR_DECODE_THRESHOLD_DB, WSPR_REFERENCE_BANDWIDTH_HZ};
use skipzone_predict::noise::{self, OperatingMode};

/// Normal-distribution decile point: P.533's decile deviations sit at
/// +/-1.2816 sigma.
pub const DECILE_Z: f64 = 1.281_551_565_545;

/// One ITURHFProp input file's worth of rows.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: String,
    pub source: Source,
    pub tx: (f64, f64),
    pub rx: (f64, f64),
    pub year: i32,
    pub month: u32,
    /// P.533 hour, 1 to 24. UTC hour `H` maps to `H`, and 00 UTC to 24.
    pub hour: u32,
    /// The R12 given to P.533, as the integer it takes (1 to 311).
    pub ssn: u32,
    /// Where `ssn` came from.
    pub ssn_source: &'static str,
    /// Frequencies, kHz, distinct.
    pub freqs_khz: Vec<i64>,
    pub txpower_dbkw: f64,
    pub bw_hz: f64,
    pub snr_required_db: f64,
}

/// Frequency key, kHz, so a row and a P.533 output line meet on equal keys.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn khz(mhz: f64) -> i64 {
    (mhz * 1000.0).round() as i64
}

/// P.533's hour for a UTC hour 0 to 23.
#[must_use]
pub fn p533_hour(utc_hour: u32) -> u32 {
    if utc_hour == 0 { 24 } else { utc_hour }
}

/// Bandwidth and required SNR for a row's source, the values Skipzone
/// predicts it with (`WsprSpot::inputs_for` for WSPR, the CW preset
/// otherwise).
#[must_use]
pub fn receiver_for(source: Source) -> (f64, f64) {
    match source {
        Source::Wspr => (WSPR_REFERENCE_BANDWIDTH_HZ, WSPR_DECODE_THRESHOLD_DB),
        Source::Rbn | Source::Beacon => OperatingMode::Cw.defaults(),
    }
}

/// Every row, grouped. Returns the groups and, per row id, its group id and
/// frequency key.
///
/// P.533 wants R12. Where the corpus has an R12 file it is used; otherwise
/// the mean daily SSN of the group's rows stands in, and the group says so.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn groups(
    rows: &[Obs],
    r12: &BTreeMap<(i32, u32), f64>,
) -> (Vec<Group>, BTreeMap<String, (String, i64)>) {
    type Key = (Source, [i64; 4], i32, u32, u32, i64);
    let mut members: BTreeMap<Key, Vec<&Obs>> = BTreeMap::new();
    let q = |v: f64| (v * 1e4).round() as i64;
    for o in rows {
        let key = (
            o.source,
            [q(o.tx_lat), q(o.tx_lon), q(o.rx_lat), q(o.rx_lon)],
            o.timestamp.0,
            o.timestamp.1,
            p533_hour(o.timestamp.3),
            (o.tx_dbm * 10.0).round() as i64,
        );
        members.entry(key).or_default().push(o);
    }
    let mut out = Vec::new();
    let mut index = BTreeMap::new();
    for (n, (key, obs)) in members.iter().enumerate() {
        let id = format!("g{n:06}");
        let o = obs[0];
        let mut freqs: Vec<i64> = obs.iter().map(|r| khz(r.freq_mhz)).collect();
        freqs.sort_unstable();
        freqs.dedup();
        let (ssn, ssn_source) = match r12.get(&(key.2, key.3)) {
            Some(v) => (*v, "r12 file"),
            None => {
                #[allow(clippy::cast_precision_loss)]
                let m = obs.iter().map(|r| r.ssn).sum::<f64>() / obs.len() as f64;
                (m, "mean daily SSN (no R12 in corpus)")
            }
        };
        let (bw_hz, snr_required_db) = receiver_for(key.0);
        for r in obs {
            index.insert(r.id.clone(), (id.clone(), khz(r.freq_mhz)));
        }
        out.push(Group {
            id,
            source: key.0,
            tx: (o.tx_lat, o.tx_lon),
            rx: (o.rx_lat, o.rx_lon),
            year: key.2,
            month: key.3,
            hour: key.4,
            ssn: ssn.round().clamp(1.0, 311.0) as u32,
            ssn_source,
            freqs_khz: freqs,
            // dB(kW) from dBm: -30 dB for W, -30 more for kW.
            txpower_dbkw: o.tx_dbm - 60.0,
            bw_hz,
            snr_required_db,
        });
    }
    (out, index)
}

/// Output columns the benchmark asks for.
pub const RPT_FORMAT: &str = "RPT_SNR | RPT_SNRD | RPT_BMUF | RPT_BCR | RPT_DOMMODE";

/// The text of one ITURHFProp input file.
#[must_use]
pub fn input_file(g: &Group, data_path: &str) -> String {
    let mut s = String::new();
    let freqs: Vec<String> = g
        .freqs_khz
        .iter()
        .map(|k| format!("{}.{:03}", k / 1000, k % 1000))
        .collect();
    let _ = writeln!(s, "PathName \"{}\"", g.id);
    let _ = writeln!(s, "Path.L_tx.lat {:.4}", g.tx.0);
    let _ = writeln!(s, "Path.L_tx.lng {:.4}", g.tx.1);
    let _ = writeln!(s, "Path.L_rx.lat {:.4}", g.rx.0);
    let _ = writeln!(s, "Path.L_rx.lng {:.4}", g.rx.1);
    let _ = writeln!(s, "Path.year {}", g.year);
    let _ = writeln!(s, "Path.month {}", g.month);
    let _ = writeln!(s, "Path.hour {}", g.hour);
    let _ = writeln!(s, "Path.SSN {}", g.ssn);
    let _ = writeln!(s, "Path.frequency {}", freqs.join(", "));
    let _ = writeln!(s, "Path.txpower {:.2}", g.txpower_dbkw);
    let _ = writeln!(s, "Path.BW {:.0}", g.bw_hz);
    let _ = writeln!(s, "Path.SNRr {:.1}", g.snr_required_db);
    let _ = writeln!(s, "Path.SNRXXp 90");
    let _ = writeln!(s, "Path.ManMadeNoise \"RURAL\"");
    let _ = writeln!(s, "TXAntFilePath \"ISOTROPIC\"");
    let _ = writeln!(s, "RXAntFilePath \"ISOTROPIC\"");
    let _ = writeln!(s, "RptFileFormat \"{RPT_FORMAT}\"");
    let _ = writeln!(s, "DataFilePath \"{data_path}\"");
    s
}

/// The group table written beside the input files. The run reads it back
/// to join outputs to rows.
#[must_use]
pub fn groups_tsv(
    groups: &[Group],
    index: &BTreeMap<String, (String, i64)>,
    manifests: &[(String, String)],
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# ITURHFProp groups for the benchmark.");
    for (name, sha) in manifests {
        let _ = writeln!(s, "# manifest: {name} {sha}");
    }
    let _ = writeln!(
        s,
        "# Run each in/<id>.in with ITURHFProp, writing out/<id>.out, then pass this"
    );
    let _ = writeln!(s, "# directory to `benchmark run --p533`.");
    let _ = writeln!(
        s,
        "# group\tsource\ttx_lat\ttx_lon\trx_lat\trx_lon\tyear\tmonth\thour\tssn\tssn_source\tfreqs_khz"
    );
    for g in groups {
        let f: Vec<String> = g.freqs_khz.iter().map(i64::to_string).collect();
        let _ = writeln!(
            s,
            "G\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}\t{}\t{}\t{}",
            g.id,
            g.source.label(),
            g.tx.0,
            g.tx.1,
            g.rx.0,
            g.rx.1,
            g.year,
            g.month,
            g.hour,
            g.ssn,
            g.ssn_source,
            f.join(",")
        );
    }
    let _ = writeln!(s, "# row\tgroup\tfreq_khz");
    for (row, (gid, k)) in index {
        let _ = writeln!(s, "R\t{row}\t{gid}\t{k}");
    }
    s
}

/// What `groups_tsv` wrote, read back: manifest hashes, each group's
/// `(source, hour)`, and each row's `(group, freq_khz)`.
#[derive(Clone, Debug, Default)]
pub struct GroupTable {
    pub manifests: Vec<(String, String)>,
    pub groups: BTreeMap<String, (String, u32)>,
    pub rows: BTreeMap<String, (String, i64)>,
}

/// Parse a `groups.tsv`.
///
/// # Errors
/// A malformed group or row line.
pub fn parse_groups_tsv(text: &str) -> Result<GroupTable, String> {
    let mut t = GroupTable::default();
    for (i, line) in text.lines().enumerate() {
        if let Some(rest) = line.strip_prefix("# manifest: ") {
            if let Some((n, sha)) = rest.trim().rsplit_once(' ') {
                t.manifests.push((n.to_string(), sha.to_string()));
            }
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("groups.tsv line {}: malformed", i + 1);
        match f.first().copied() {
            Some("G") => {
                if f.len() < 13 {
                    return Err(bad());
                }
                t.groups.insert(
                    f[1].to_string(),
                    (f[2].to_string(), f[9].parse().map_err(|_| bad())?),
                );
            }
            Some("R") => {
                if f.len() < 4 {
                    return Err(bad());
                }
                t.rows.insert(
                    f[1].to_string(),
                    (f[2].to_string(), f[3].parse().map_err(|_| bad())?),
                );
            }
            _ => {}
        }
    }
    Ok(t)
}

/// One line of ITURHFProp output.
#[derive(Clone, Debug, PartialEq)]
pub struct P533Row {
    pub month: u32,
    pub hour: u32,
    pub freq_mhz: f64,
    /// Median SNR in `Path.BW`, dB.
    pub snr_db: f64,
    /// Lower and upper decile deviations of the SNR, dB, both positive.
    pub snr_dl: f64,
    pub snr_du: f64,
    pub bmuf_mhz: f64,
    /// Basic circuit reliability, %.
    pub bcr_pct: f64,
    pub dom_mode: String,
}

impl P533Row {
    /// P(SNR > required) from a split normal with the deciles at
    /// +/-[`DECILE_Z`] sigma. Above the median the lower half's spread
    /// decides, below it the upper half's (docs/notes/accuracy-roadmap.md §2.5).
    #[must_use]
    pub fn p_decode(&self, snr_required_db: f64) -> f64 {
        let margin = self.snr_db - snr_required_db;
        let dev = if margin >= 0.0 {
            self.snr_dl
        } else {
            self.snr_du
        };
        noise::decode_probability(margin, dev.abs() / DECILE_Z)
    }
}

/// Parse an ITURHFProp report.
///
/// Lines starting with `*`, `#` or `//`, and blank lines, are skipped. The
/// first remaining line names the comma-separated columns; the rest are
/// values. Names are matched without case or spaces, with the aliases below,
/// so the parser can be pointed at the real header once one is seen.
///
/// # Errors
/// A missing required column or an unreadable value.
pub fn parse_output(text: &str) -> Result<Vec<P533Row>, String> {
    const ALIASES: [(&str, &[&str]); 9] = [
        ("month", &["month", "mo"]),
        ("hour", &["hour", "hr"]),
        ("freq", &["freq", "frequency", "f(mhz)", "freq(mhz)"]),
        ("snr", &["snr", "snr(db)"]),
        ("snrdl", &["snrld", "snrdl", "snr_dl", "dl"]),
        ("snrdu", &["snrud", "snrdu", "snr_du", "du"]),
        ("bmuf", &["bmuf", "bmuf(mhz)"]),
        ("bcr", &["bcr", "bcr(%)"]),
        ("dommode", &["dommode", "dom_mode", "dominantmode"]),
    ];
    let mut lines = text.lines().enumerate().filter(|(_, l)| {
        let t = l.trim();
        !(t.is_empty() || t.starts_with('*') || t.starts_with('#') || t.starts_with("//"))
    });
    let Some((_, header)) = lines.next() else {
        return Ok(Vec::new());
    };
    let names: Vec<String> = header
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase().replace(' ', ""))
        .collect();
    let mut col = BTreeMap::new();
    for (key, aliases) in ALIASES {
        let Some(i) = names.iter().position(|n| aliases.contains(&n.as_str())) else {
            return Err(format!("ITURHFProp output has no {key} column: {header:?}"));
        };
        col.insert(key, i);
    }
    let mut out = Vec::new();
    for (i, line) in lines {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        let bad = |what: &str| format!("ITURHFProp output line {}: bad {what}", i + 1);
        let num = |k: &str| -> Result<f64, String> {
            f.get(col[k])
                .and_then(|v| v.parse::<f64>().ok())
                .ok_or_else(|| bad(k))
        };
        out.push(P533Row {
            month: f
                .get(col["month"])
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| bad("month"))?,
            hour: f
                .get(col["hour"])
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| bad("hour"))?,
            freq_mhz: num("freq")?,
            snr_db: num("snr")?,
            snr_dl: num("snrdl")?,
            snr_du: num("snrdu")?,
            bmuf_mhz: num("bmuf")?,
            bcr_pct: num("bcr")?,
            dom_mode: f
                .get(col["dommode"])
                .map_or_else(String::new, |v| (*v).to_string()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // SYNTHETIC: written to the column list in the roadmap. The header must
    // be confirmed against a real ITURHFProp output file.
    const FIXTURE: &str = include_str!("../../tests/fixtures/benchmark/p533/out/g000000.out");

    #[test]
    fn the_synthetic_output_fixture_parses() {
        let rows = parse_output(FIXTURE).unwrap();
        assert!(!rows.is_empty());
        let r = &rows[0];
        assert_eq!(r.month, 1);
        assert!(r.snr_dl > 0.0 && r.snr_du > 0.0);
    }

    #[test]
    fn a_missing_column_is_an_error() {
        let err = parse_output("Month, Hour, Freq, SNR\n7, 12, 14.097, -10\n").unwrap_err();
        assert!(err.contains("snrdl"), "{err}");
    }

    #[test]
    fn split_normal_probability_by_hand() {
        let r = P533Row {
            month: 1,
            hour: 1,
            freq_mhz: 14.0,
            snr_db: -20.0,
            snr_dl: 2.0 * DECILE_Z,
            snr_du: 4.0 * DECILE_Z,
            bmuf_mhz: 20.0,
            bcr_pct: 0.0,
            dom_mode: String::new(),
        };
        // At the median: one half.
        assert!((r.p_decode(-20.0) - 0.5).abs() < 1e-12);
        // Threshold one lower sigma (2 dB) below the median: Phi(1).
        let phi1 = noise::normal_cdf(1.0);
        assert!((r.p_decode(-22.0) - phi1).abs() < 1e-12);
        // Threshold one upper sigma (4 dB) above: Phi(-1).
        assert!((r.p_decode(-16.0) - (1.0 - phi1)).abs() < 1e-12);
        // The lower decile point itself: 90 %.
        assert!((r.p_decode(-20.0 - 2.0 * DECILE_Z) - 0.9).abs() < 1e-9);
    }

    #[test]
    fn hour_convention_maps_midnight_to_24() {
        assert_eq!(p533_hour(0), 24);
        assert_eq!(p533_hour(13), 13);
    }
}
