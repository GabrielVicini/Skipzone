//! Breaking a validation run down into the places the model is weakest.
//!
//! [`crate::wspr::Summary`] answers "how far off is the model overall". That is
//! the wrong grain to improve anything with: a median error of +6 dB could be a
//! uniform +6 dB everywhere, or +1 dB on most paths and +25 dB on one band. This
//! module cuts the same results along the axes the physics would fail along, so
//! the report can point at a specific band, distance or illumination rather than
//! at the model in general.
//!
//! Every cut reports its own sample size, and a cut with too few spots to mean
//! anything is shown with its count rather than being hidden or padded. The
//! caller decides what to trust; the harness does not decide for it.
//!
//! The one-sided biases in [`crate::wspr`] apply to every number here: only
//! successful decodes are published, so a "miss" is a genuine model failure but
//! a "hit" is not evidence against false positives.

use crate::wspr::SpotResult;

/// Fewer spots than this and a cut's statistics are noise; it is still printed,
/// with its count, but flagged so nobody reads a trend into three spots.
pub const MIN_MEANINGFUL: usize = 8;

/// One slice of the results.
pub struct Cut {
    pub label: String,
    pub spots: usize,
    /// Spots in this cut for which the model found any path.
    pub closed: usize,
    /// Median of `modelled - measured`, dB. NaN when nothing closed.
    pub median_error_db: f64,
    /// Interquartile range of the error, dB. NaN when too few closed.
    pub iqr_db: f64,
    /// Fraction of this cut's spots the model found a path for, 0..1.
    pub hit_rate: f64,
}

impl Cut {
    fn of(label: String, results: &[&SpotResult]) -> Self {
        let mut errors: Vec<f64> = results.iter().filter_map(|r| r.error_db).collect();
        errors.sort_by(f64::total_cmp);
        #[allow(clippy::cast_precision_loss)]
        let (n, total) = (errors.len(), results.len() as f64);
        #[allow(clippy::cast_precision_loss)]
        let hit_rate = if results.is_empty() {
            f64::NAN
        } else {
            n as f64 / total
        };
        Self {
            label,
            spots: results.len(),
            closed: n,
            median_error_db: pct(&errors, 0.5),
            iqr_db: if n < 4 {
                f64::NAN
            } else {
                pct(&errors, 0.75) - pct(&errors, 0.25)
            },
            hit_rate,
        }
    }

    /// Is this cut big enough to read a trend from?
    #[must_use]
    pub fn meaningful(&self) -> bool {
        self.spots >= MIN_MEANINGFUL
    }
}

/// A named family of cuts, e.g. "by band".
pub struct Breakdown {
    pub axis: &'static str,
    pub cuts: Vec<Cut>,
}

/// Cut the results every way the report shows.
#[must_use]
pub fn breakdowns(results: &[SpotResult]) -> Vec<Breakdown> {
    vec![
        Breakdown {
            axis: "band",
            cuts: group(results, |r| band_label(r.spot.freq_mhz)),
        },
        Breakdown {
            axis: "path length",
            cuts: group(results, |r| distance_bucket(r.solved_km).to_string()),
        },
        Breakdown {
            axis: "hops the model chose",
            cuts: group(results, |r| {
                if r.closed() {
                    format!("{} hop(s)", r.hops)
                } else {
                    "no path found".to_string()
                }
            }),
        },
        Breakdown {
            axis: "layer the model chose",
            cuts: group(results, |r| {
                r.layer.unwrap_or("no path found").to_string()
            }),
        },
        Breakdown {
            axis: "measured signal strength",
            cuts: group(results, |r| snr_bucket(r.spot.snr_db).to_string()),
        },
    ]
}

fn group(results: &[SpotResult], key: impl Fn(&SpotResult) -> String) -> Vec<Cut> {
    let mut keys: Vec<String> = Vec::new();
    for r in results {
        let k = key(r);
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
    keys.sort();
    keys.into_iter()
        .map(|k| {
            let members: Vec<&SpotResult> = results.iter().filter(|r| key(r) == k).collect();
            Cut::of(k, &members)
        })
        .collect()
}

/// The amateur band a frequency sits in, by name. Anything outside the
/// allocations is labelled by its megahertz rather than forced into a band.
///
/// The VHF bands are here even though the model has no mechanism above HF (see
/// `wsprlive::HF_TOP_MHZ`), because a run that deliberately asks for them still
/// has to group them: WSPR uses two dial frequencies on 6 m, and without a band
/// name they split into a "50.294 MHz" row and a "50.295 MHz" row that are the
/// same band.
#[must_use]
pub fn band_label(mhz: f64) -> String {
    const BANDS: [(f64, f64, &str); 15] = [
        (0.135, 0.139, "2200 m"),
        (0.472, 0.479, "630 m"),
        (1.8, 2.0, "160 m"),
        (3.5, 4.0, "80 m"),
        (5.2, 5.5, "60 m"),
        (7.0, 7.3, "40 m"),
        (10.1, 10.15, "30 m"),
        (14.0, 14.35, "20 m"),
        (18.06, 18.17, "17 m"),
        (21.0, 21.45, "15 m"),
        (24.89, 25.0, "12 m"),
        (28.0, 29.7, "10 m"),
        (50.0, 54.0, "6 m"),
        (70.0, 70.5, "4 m"),
        (144.0, 148.0, "2 m"),
    ];
    for (lo, hi, name) in BANDS {
        if (lo..=hi).contains(&mhz) {
            return name.to_string();
        }
    }
    format!("{mhz:.3} MHz")
}

/// Path-length buckets chosen at the geometry boundaries that matter: one hop
/// of E, one hop of F2, then multi-hop, then the very long paths where the
/// equal-hop geometry and the great-circle assumption are under most strain.
fn distance_bucket(km: f64) -> &'static str {
    match km {
        k if k < 1000.0 => "a) < 1000 km",
        k if k < 2500.0 => "b) 1000-2500 km",
        k if k < 5000.0 => "c) 2500-5000 km",
        k if k < 10000.0 => "d) 5000-10000 km",
        _ => "e) > 10000 km",
    }
}

/// Measured-SNR buckets. The weakest decodes sit near the -29 dB WSPR
/// threshold, where a model that is optimistic by a few dB still "finds" the
/// path; the strong ones are where an over-prediction shows up plainly.
fn snr_bucket(db: f64) -> &'static str {
    match db {
        d if d < -25.0 => "a) -29..-25 dB (barely decoded)",
        d if d < -15.0 => "b) -25..-15 dB",
        d if d < -5.0 => "c) -15..-5 dB",
        _ => "d) > -5 dB (strong)",
    }
}

/// How much of the headline bias is the harness's own assumptions.
///
/// The overall median error is not, on its own, a statement about the physics.
/// Two inputs to every scored spot are chosen rather than measured, and both are
/// chosen optimistically:
///
/// * the ANTENNA at each end, which the spot does not carry, and
/// * the receiver's NOISE ENVIRONMENT, which the spot does not carry either.
///
/// Both enter the modelled SNR as a straight dB offset - they change the link
/// budget, not the ray path - so their contribution can be quantified exactly
/// rather than argued about. That is what this does: it reports the gain the
/// model handed itself, and what the bias would have been under each of the
/// other noise environments. Whatever is left after those is the physics.
pub struct BiasBudget {
    /// Median combined antenna gain credited, dB over both ends.
    pub median_assumed_gain_db: f64,
    /// Median noise floor scored against, dBm.
    pub median_noise_dbm: f64,
    /// `(environment label, median error if that had been assumed)`.
    pub under_noise_env: Vec<(&'static str, f64)>,
    /// The measured median error, for reference.
    pub median_error_db: f64,
}

/// Structural misses: paths too long for the configured hop limit to reach at
/// all, whatever the ionosphere is doing.
///
/// A single F2 hop tops out near 4000 km, so a 17 000 km path needs at least
/// five of them. Scored at `max_hops = 4` such a spot CANNOT close, and
/// counting it as a physics failure alongside a 700 km path the model genuinely
/// could not explain buries the one real signal in the other. This separates
/// them.
#[must_use]
pub fn needs_more_hops_than(km: f64, max_hops: u32) -> bool {
    const MAX_F2_HOP_KM: f64 = 4000.0;
    #[allow(clippy::cast_precision_loss)]
    let reach = MAX_F2_HOP_KM * f64::from(max_hops);
    km > reach
}

/// Spots the model found nothing at all for, worst first by measured SNR.
///
/// These are the sharpest failures in the run and the most useful thing in the
/// report: a spot is proof that the path was open, so "no path found" is
/// unambiguously the model missing something. A strong spot the model cannot
/// explain is a bigger problem than a marginal one.
#[must_use]
pub fn worst_misses(results: &[SpotResult], n: usize) -> Vec<&SpotResult> {
    let mut misses: Vec<&SpotResult> = results.iter().filter(|r| !r.closed()).collect();
    misses.sort_by(|a, b| b.spot.snr_db.total_cmp(&a.spot.snr_db));
    misses.truncate(n);
    misses
}

/// Spots the model got most wrong in dB, worst first.
#[must_use]
pub fn worst_errors(results: &[SpotResult], n: usize) -> Vec<&SpotResult> {
    let mut scored: Vec<&SpotResult> = results.iter().filter(|r| r.error_db.is_some()).collect();
    scored.sort_by(|a, b| {
        b.error_db
            .unwrap_or(0.0)
            .abs()
            .total_cmp(&a.error_db.unwrap_or(0.0).abs())
    });
    scored.truncate(n);
    scored
}

/// Quantify how much of the bias the assumptions account for.
///
/// `noise_shift` supplies, per spot, the noise floor that WOULD have applied
/// under each alternative environment. The SNR moves by exactly the negative of
/// the floor's change, because nothing else in the link budget depends on it.
#[must_use]
pub fn bias_budget(
    results: &[SpotResult],
    noise_shift: &[(&'static str, Vec<f64>)],
) -> BiasBudget {
    let med = |mut v: Vec<f64>| -> f64 {
        v.sort_by(f64::total_cmp);
        pct(&v, 0.5)
    };
    let errors: Vec<f64> = results.iter().filter_map(|r| r.error_db).collect();
    let under_noise_env = noise_shift
        .iter()
        .map(|(label, floors)| {
            let shifted: Vec<f64> = results
                .iter()
                .zip(floors)
                .filter_map(|(r, alt)| {
                    // SNR = Prx - noise, so raising the floor lowers the SNR by
                    // exactly the same number of dB.
                    Some(r.error_db? - (alt - r.noise_dbm?))
                })
                .collect();
            (*label, med(shifted))
        })
        .collect();
    BiasBudget {
        median_assumed_gain_db: med(results.iter().filter_map(|r| r.assumed_gain_db).collect()),
        median_noise_dbm: med(results.iter().filter_map(|r| r.noise_dbm).collect()),
        under_noise_env,
        median_error_db: med(errors),
    }
}

fn pct(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    #[allow(clippy::cast_precision_loss)]
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = pos.floor() as usize;
    let frac = pos - pos.floor();
    if i + 1 >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[i] * (1.0 - frac) + sorted[i + 1] * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wspr::WsprSpot;

    fn spot(freq_mhz: f64, snr_db: f64) -> WsprSpot {
        WsprSpot {
            timestamp: (2026, 7, 27, 3, 22),
            tx_call: "K1ABC".into(),
            freq_mhz,
            snr_db,
            tx_grid: "FN42".into(),
            tx_dbm: 37.0,
            rx_call: "W9XYZ".into(),
            rx_grid: "EM48".into(),
            reported_km: 1420.0,
            tx_lat: 42.0,
            tx_lon: -71.0,
            rx_lat: 38.0,
            rx_lon: -90.0,
        }
    }

    fn result(freq_mhz: f64, snr_db: f64, modelled: Option<f64>, km: f64) -> SpotResult {
        SpotResult {
            spot: spot(freq_mhz, snr_db),
            solved_km: km,
            deterministic_snr_db: modelled,
            es: None,
            layer: modelled.map(|_| "F2"),
            modelled_snr_db: modelled,
            error_db: modelled.map(|m| m - snr_db),
            hops: 1,
            assumed_gain_db: modelled.map(|_| 11.0),
            noise_dbm: modelled.map(|_| -110.0),
        }
    }

    /// Raising the assumed noise floor must lower the modelled SNR by exactly
    /// the same number of dB, so the attributed bias moves one-for-one. This is
    /// the arithmetic the whole attribution rests on.
    #[test]
    fn noise_attribution_moves_the_bias_one_for_one() {
        let results = vec![
            result(14.097, -20.0, Some(0.0), 1400.0),
            result(14.097, -20.0, Some(4.0), 1400.0),
        ];
        // Both spots scored against -110 dBm; ask what a 12 dB noisier site does.
        let budget = bias_budget(&results, &[("City", vec![-98.0, -98.0])]);
        assert!((budget.median_error_db - 22.0).abs() < 1e-9, "{:?}", budget.median_error_db);
        assert!(
            (budget.under_noise_env[0].1 - 10.0).abs() < 1e-9,
            "12 dB more noise must remove 12 dB of optimism, got {}",
            budget.under_noise_env[0].1
        );
        assert!((budget.median_assumed_gain_db - 11.0).abs() < 1e-9);
    }

    /// A path longer than the hop limit can physically reach is a
    /// configuration limit, not the physics failing, and must be separable.
    #[test]
    fn structural_misses_are_distinguished_from_physics_misses() {
        // 4 hops of F2 reach about 16 000 km.
        assert!(needs_more_hops_than(17_185.0, 4));
        assert!(needs_more_hops_than(18_176.0, 4));
        assert!(!needs_more_hops_than(6_107.0, 4), "6100 km is well within 4 hops");
        assert!(!needs_more_hops_than(730.0, 1));
        // Raising the limit brings the long path back inside reach.
        assert!(!needs_more_hops_than(17_185.0, 5));
    }

    #[test]
    fn bands_are_named_and_unknown_frequencies_are_not_forced_into_one() {
        assert_eq!(band_label(14.0971), "20 m");
        assert_eq!(band_label(7.0401), "40 m");
        assert_eq!(band_label(10.1402), "30 m");
        assert_eq!(band_label(0.4742), "630 m");
        // Both WSPR dial frequencies on 6 m must land in one band, not two.
        assert_eq!(band_label(50.294), "6 m");
        assert_eq!(band_label(50.295), "6 m");
        assert_eq!(band_label(35.0), "35.000 MHz");
    }

    /// A cut has to carry its own sample size and flag itself when it is too
    /// small to read anything into - the report must not present three spots
    /// and eighty spots as equally solid.
    #[test]
    fn cuts_carry_their_sample_size_and_flag_thin_ones() {
        let mut results: Vec<SpotResult> = (0..12)
            .map(|i| result(14.097, -20.0, Some(-20.0 + f64::from(i) - 6.0), 1400.0))
            .collect();
        results.push(result(7.04, -20.0, Some(-14.0), 1400.0));

        let by_band = &breakdowns(&results)[0];
        assert_eq!(by_band.axis, "band");
        let twenty = by_band.cuts.iter().find(|c| c.label == "20 m").unwrap();
        let forty = by_band.cuts.iter().find(|c| c.label == "40 m").unwrap();
        assert_eq!(twenty.spots, 12);
        assert!(twenty.meaningful());
        assert_eq!(forty.spots, 1);
        assert!(!forty.meaningful(), "one spot is not a trend");
    }

    /// A cut where nothing closed reports a hit rate of zero and a NaN error,
    /// rather than an error of zero. Those mean opposite things.
    #[test]
    fn a_cut_that_found_nothing_reports_no_error_not_a_zero_error() {
        let results = [result(21.0, -10.0, None, 9000.0), result(21.0, -12.0, None, 9000.0)];
        let cut = Cut::of("15 m".into(), &results.iter().collect::<Vec<_>>());
        assert_eq!(cut.closed, 0);
        assert!((cut.hit_rate - 0.0).abs() < 1e-12);
        assert!(cut.median_error_db.is_nan(), "no error is not zero error");
    }

    /// The misses list is ordered by how strong the unexplained spot was: a
    /// loud signal the model cannot account for is the more serious failure.
    #[test]
    fn worst_misses_lead_with_the_strongest_unexplained_spot() {
        let results = vec![
            result(14.097, -27.0, None, 3000.0),
            result(14.097, -8.0, None, 3000.0),
            result(14.097, -19.0, None, 3000.0),
            result(14.097, -12.0, Some(-11.0), 3000.0),
        ];
        let misses = worst_misses(&results, 10);
        assert_eq!(misses.len(), 3, "the one that closed is not a miss");
        assert!((misses[0].spot.snr_db + 8.0).abs() < 1e-9);
        assert!((misses[2].spot.snr_db + 27.0).abs() < 1e-9);
    }

    /// Worst errors are ranked by magnitude, so a large under-prediction is as
    /// visible as a large over-prediction.
    #[test]
    fn worst_errors_rank_by_magnitude_in_both_directions() {
        let results = vec![
            result(14.097, -20.0, Some(-18.0), 3000.0),  // +2
            result(14.097, -20.0, Some(-45.0), 3000.0),  // -25
            result(14.097, -20.0, Some(-8.0), 3000.0),   // +12
        ];
        let worst = worst_errors(&results, 2);
        assert!((worst[0].error_db.unwrap() + 25.0).abs() < 1e-9);
        assert!((worst[1].error_db.unwrap() - 12.0).abs() < 1e-9);
    }
}
