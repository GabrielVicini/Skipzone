//! Solving the corpus: turning saved spots into cached model predictions,
//! and reading the corpus files off disk.

use std::collections::BTreeMap;

use skipzone_lab::corpus::{self, CorpusSpot};
use skipzone_lab::fit::{self, Cached};
use skipzone_lab::wspr::WsprSpot;
use skipzone_predict::compute::ComputePool;
use skipzone_predict::scenario::{self, Inputs};
use skipzone_predict::solve;
/// A corpus, solved once, with its station index space.
pub(crate) struct Solved {
    pub(crate) spots: Vec<Cached>,
    /// Every spot in the corpus, including the ones no path was found for. The
    /// difference between this and `spots.len()` is the hit rate, and it has to be
    /// carried separately: a spot with no modelled SNR cannot enter the fit, but
    /// dropping it silently would turn a miss into an absence.
    pub(crate) total: usize,
    /// Callsign for each station index, for the effect distribution report.
    pub(crate) tx_names: Vec<String>,
    pub(crate) rx_names: Vec<String>,
    /// Spots dropped by `--exclude-es`, counted so the hit rate does not treat
    /// them as misses.
    pub(crate) es_excluded: usize,
}

/// Drop the spots the model could only reach through the sporadic-E fallback.
///
/// # Why this is a diagnostic and not a mode
///
/// [`solve::best_with_es_fallback`] uses Es only when nothing deterministic
/// closed. So an Es spot means the deterministic tracer failed to close a path
/// that existed (the spot is a real decode), and the Es sheet answered with a
/// near-lossless mirror at 100 km. Fitting those spots fits the fallback rather
/// than the ionosphere.
///
/// Dropping them shows where the residual lives but fixes nothing; a run that
/// drops a large fraction of its corpus is not a calibration to quote. What is
/// dropped is always reported.
pub(crate) fn drop_es(set: Solved) -> Solved {
    let kept: Vec<Cached> = set
        .spots
        .iter()
        .filter(|c| c.layer != "Es")
        .cloned()
        .collect();
    let es_excluded = set.spots.len() - kept.len();
    Solved {
        spots: kept,
        es_excluded,
        ..set
    }
}

pub(crate) fn main_solve(
    label: &str,
    corpus_spots: &[CorpusSpot],
    base: &Inputs,
    pool: &ComputePool,
) -> Solved {
    // One shared index space per corpus. Station effects are per-corpus because a
    // hold-out set contains partly different stations, and an effect is a property
    // of a station rather than of the physics being tested.
    let mut tx_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut rx_index: BTreeMap<String, usize> = BTreeMap::new();
    for c in corpus_spots {
        let n = tx_index.len();
        tx_index.entry(c.spot.tx_call.clone()).or_insert(n);
        let n = rx_index.len();
        rx_index.entry(c.spot.rx_call.clone()).or_insert(n);
    }

    let (results, timing) = pool.map(corpus_spots, |c| solve_one(c, base));
    eprintln!(
        "[{label}] solved {} spot(s) in {:.1} s ({:.0} ms each on {} threads)",
        corpus_spots.len(),
        timing.total.as_secs_f64(),
        timing.total.as_secs_f64() * 1e3 / corpus_spots.len().max(1) as f64,
        pool.threads()
    );

    let mut spots = Vec::new();
    for (c, solved) in corpus_spots.iter().zip(results) {
        if let Some(mut cached) = solved {
            cached.tx = tx_index[&c.spot.tx_call];
            cached.rx = rx_index[&c.spot.rx_call];
            spots.push(cached);
        }
    }
    let mut tx_names = vec![String::new(); tx_index.len()];
    for (name, i) in tx_index {
        tx_names[i] = name;
    }
    let mut rx_names = vec![String::new(); rx_index.len()];
    for (name, i) in rx_index {
        rx_names[i] = name;
    }
    Solved {
        spots,
        total: corpus_spots.len(),
        tx_names,
        rx_names,
        es_excluded: 0,
    }
}

/// Solve one corpus spot and reduce it to the cached terms the fit needs.
///
/// `None` when the model found no path, so there is no modelled SNR to compare.
/// Those spots are counted separately and reported as a hit rate.
pub(crate) fn solve_one(c: &CorpusSpot, base: &Inputs) -> Option<Cached> {
    let inputs = Inputs {
        ssn: c.ssn,
        ..c.spot.inputs_for(base)
    };
    let a = scenario::resolve(&inputs);
    let models = scenario::build_models(&inputs, &a).ok()?;
    let out = solve::solve(&inputs, &a, &models);
    let best = solve::best_with_es_fallback(&out)?;

    // Split the loss so absorption can be rescaled without re-tracing. This has
    // to reconstruct the SNR exactly; `reconstruction_matches_the_solver` below
    // checks that on every spot rather than trusting the algebra.
    let loss_without_absorption_db =
        best.total_system_loss_db - best.total_absorption_db - best.total_gain_db;

    // The F2 path this one beat, if the reported layer was a lower one and an F2
    // path also closed. Taken from the deterministic list, where F2 lives; when
    // the winner came from the Es fallback this shows whether the deterministic
    // stack had nothing or only nothing that outscored it.
    let alternative = (best.layer != solve::LayerMode::F2)
        .then(|| {
            out.solutions
                .iter()
                .filter(|s| s.layer == solve::LayerMode::F2)
                .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db))
        })
        .flatten()
        .map(|s| fit::Alternative {
            loss_without_absorption_db: s.total_system_loss_db
                - s.total_absorption_db
                - s.total_gain_db,
            absorption_db: s.total_absorption_db,
        });

    Some(Cached {
        tx: 0,
        rx: 0,
        measured_db: c.spot.snr_db,
        tx_power_dbm: skipzone_predict::noise::dbm_from_watts(inputs.tx_power_w),
        loss_without_absorption_db,
        absorption_db: best.total_absorption_db,
        freq_mhz: inputs.freq_mhz,
        bandwidth_hz: inputs.bandwidth_hz,
        rx_is_day: a.rx_is_day,
        rx_season: a.rx_season,
        rx_lat: inputs.rx_lat,
        noise_env: inputs.noise_env,
        layer: best.layer.label(),
        hops: best.hops,
        range_km: out.great_circle_km,
        probability: best.probability,
        date: c.date(),
        midpoint_zenith_deg: a.solar.zenith_angle_deg,
        alternative,
    })
}

pub(crate) fn describe_span(tag: &str, path: &str, spots: &[CorpusSpot]) {
    let mut days: Vec<(i32, u32, u32)> = spots.iter().map(CorpusSpot::date).collect();
    days.sort_unstable();
    days.dedup();
    let mut ssns: Vec<f64> = spots.iter().map(|s| s.ssn).collect();
    ssns.sort_by(f64::total_cmp);
    println!(
        "\n{tag} {path}: {} spots over {} day(s), SSN {:.0} to {:.0}",
        spots.len(),
        days.len(),
        ssns.first().copied().unwrap_or(f64::NAN),
        ssns.last().copied().unwrap_or(f64::NAN),
    );
}

/// Load a corpus, thinned to at most `max` spots by a fixed stride.
///
/// A prefix would be one day on one band, because the file is written in fetch
/// order; a stride keeps the corpus balance.
pub(crate) fn load_capped(path: &str, max: usize) -> Result<Vec<CorpusSpot>, String> {
    let all = load(path)?;
    if max == 0 || all.len() <= max {
        return Ok(all);
    }
    let stride = (all.len() / max).max(1);
    let thinned: Vec<CorpusSpot> = all.iter().step_by(stride).cloned().collect();
    println!(
        "         thinned to {} of {} spots (every {stride}th, preserving band, hour and",
        thinned.len(),
        all.len()
    );
    println!("         day balance)");
    Ok(thinned)
}

pub(crate) fn load(path: &str) -> Result<Vec<CorpusSpot>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let (spots, problems) = corpus::read_positives(&text);
    if !problems.is_empty() {
        eprintln!("  ! {} unreadable row(s) in {path}:", problems.len());
        for p in problems.iter().take(5) {
            eprintln!("    {p}");
        }
    }
    if spots.is_empty() {
        return Err(format!("{path} held no usable spots"));
    }
    Ok(spots)
}

// Referenced so the dependency on the spot type used throughout is explicit.
#[allow(unused_imports)]
use WsprSpot as _SpotUsedThroughout;
