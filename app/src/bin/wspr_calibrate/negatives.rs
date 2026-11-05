//! Negatives: paths that were listening and heard nothing. Solving them, and
//! the decode-probability and false-positive reporting built on them.

use crate::args::*;
use crate::driver::*;
use crate::report::*;
use crate::solving::*;

use std::collections::BTreeMap;

use skipzone_app::corpus::Negative;
use skipzone_app::fit::{self, Cached, CachedParams, NegativeScore, Spread};
use skipzone_app::wspr::WSPR_DECODE_THRESHOLD_DB;
use skipzone_predict::calib::AtmosphericAnchors;
use skipzone_predict::compute::ComputePool;
use skipzone_predict::noise::{self};
use skipzone_predict::scenario::{self, Inputs};
use skipzone_predict::solve;
/// Solve the negatives and map them onto the fit set's station index space.
///
/// # Why the index space is shared
///
/// A negative is a statement about an absolute SNR, which depends on the
/// station's offset, so a negative from a known transmitter is judged against
/// that transmitter's estimated effect.
///
/// Stations absent from the fit corpus keep index `usize::MAX`, which every
/// effect lookup misses and treats as 0. The effects are centred on zero, so
/// that is the population estimate for an unknown station.
///
/// The negatives do not feed back into the effects: a silence carries only a
/// bound, no level to average.
pub(crate) fn solve_negatives(
    negatives: &[Negative],
    base: &Inputs,
    pool: &ComputePool,
    fit_set: &Solved,
) -> Vec<Cached> {
    let index = |names: &[String]| -> BTreeMap<String, usize> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect()
    };
    let tx_index = index(&fit_set.tx_names);
    let rx_index = index(&fit_set.rx_names);

    let (results, timing) = pool.map(negatives, |n| {
        let spot = n.as_spot();
        let inputs = Inputs {
            ssn: n.ssn,
            ..spot.inputs_for(base)
        };
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).ok()?;
        let out = solve::solve(&inputs, &a, &models);
        let best = solve::best_with_es_fallback(&out)?;
        let loss = best.total_system_loss_db - best.total_absorption_db - best.total_gain_db;
        Some(Cached {
            tx: usize::MAX,
            rx: usize::MAX,
            measured_db: f64::NAN,
            tx_power_dbm: skipzone_predict::noise::dbm_from_watts(inputs.tx_power_w),
            loss_without_absorption_db: loss,
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
            date: (n.timestamp.0, n.timestamp.1, n.timestamp.2),
            midpoint_zenith_deg: a.solar.zenith_angle_deg,
            // Not carried for negatives: nothing scores an alternative here,
            // because a negative has no measured SNR to score one against.
            alternative: None,
        })
    });
    eprintln!(
        "[negatives] solved {} in {:.1} s ({:.0} ms each on {} threads)",
        negatives.len(),
        timing.total.as_secs_f64(),
        timing.total.as_secs_f64() * 1e3 / negatives.len().max(1) as f64,
        pool.threads()
    );

    let out: Vec<Cached> = negatives
        .iter()
        .zip(results)
        .filter_map(|(n, solved)| {
            let mut c = solved?;
            // The caller has already filtered to negatives both of whose stations
            // the fit knows, so these lookups must hit. An `unwrap_or` here would
            // silently reintroduce the population mismatch that filter exists to
            // remove, so a miss is a panic rather than a default.
            c.tx = *tx_index
                .get(&n.tx_call)
                .expect("negative reached the solver with a transmitter the fit does not know");
            c.rx = *rx_index
                .get(&n.rx_call)
                .expect("negative reached the solver with a receiver the fit does not know");
            Some(c)
        })
        .collect();
    if !out.is_empty() {
        println!(
            "\nNEGATIVES {} of {} solved non-decode(s) closed a path, every one priced against",
            out.len(),
            negatives.len()
        );
        println!("         its own two stations' estimated effects rather than a population mean.");
    }
    out
}

/// The unattributable global offset each scoring of the negatives is matched
/// against - the one from the baseline station-effect solve, and the one from the
/// fitted solve. See [`report_negatives`] for why both are needed.
pub(crate) struct Levels {
    pub(crate) before: f64,
    pub(crate) after: f64,
}

/// Score the negatives: paths that were attempted and did not decode.
///
/// # Why this is reported at two levels
///
/// The fit's objective subtracts the global offset from every residual, so it
/// is invariant to the prediction's level, while the decode threshold is an
/// absolute comparison. A fit that slides the whole model optimistic pays
/// nothing in RMS but the whole slide in false positives: between the baseline
/// and the fit the offset grew by about 8 dB and the median margin over
/// threshold by 12.
///
/// So both are printed. AS SHIPPED is what a user of the app would see.
/// LEVEL-MATCHED subtracts each scoring's own offset, showing whether the fit
/// made the model more permissive about which paths open; only it is
/// comparable before against after.
#[allow(clippy::too_many_arguments)]
/// Fit the predictive spread and report whether the resulting probabilities
/// are calibrated, i.e. whether "70 %" means seventy per cent.
///
/// # Why a probability
///
/// [`skipzone_predict::noise::PathState`] compares one SNR with a threshold, but
/// the same path measured twice inside ten minutes differs with an SD of
/// 3.5 dB (129 pairs from the corpus), and the model's out-of-sample error is
/// 7.3 dB. So the modelled SNR is treated as the median of a fading
/// distribution: `P(decode) = Phi((margin + b) / sigma)`.
///
/// # What is identifiable
///
/// `sigma`, the slope of the S-curve, is fixed by how the decode rate changes
/// with margin.
///
/// The intercept `b` does not transfer: the corpus is decodes plus a small
/// constructed negative set, so its class balance is an artefact of sampling
/// and `b` absorbs it. The negatives are also an upper bound on false positives
/// (a collision or off-beam arrival counts as a non-decode), so the fitted
/// `sigma` is if anything too wide.
///
/// `Phi` is monotone in the margin, so the AUC is unchanged: this improves
/// calibration, not discrimination.
pub(crate) fn report_decode_probability(
    set: &Solved,
    after: &Scored,
    negatives: &[Cached],
    fitted: &CachedParams,
) {
    println!("\n--- PREDICTED DECODE PROBABILITY ---------------------------");
    println!("  The model emits one SNR; reality fades around it. Treating that SNR as the");
    println!("  MEDIAN of a log-normal and reporting P(decode) = Phi(margin / sigma) turns a");
    println!("  brittle yes/no into a calibrated probability. Phi is monotone, so this cannot");
    println!("  change the AUC by a single bit - it buys CALIBRATION, not discrimination.");

    let scale = fitted.absorption_scale.value;
    let atm = fitted.atm;
    // margin = modelled - station effects - threshold, for decodes (y = 1) and
    // for the constructed non-decodes (y = 0).
    let mut pts: Vec<(f64, bool)> = set
        .spots
        .iter()
        .map(|c| {
            (
                c.modelled_db(scale, atm) - after.effects.offset_for(c) - WSPR_DECODE_THRESHOLD_DB,
                true,
            )
        })
        .collect();
    for c in negatives {
        pts.push((
            c.modelled_db(scale, atm) - after.effects.offset_for(c) - WSPR_DECODE_THRESHOLD_DB,
            false,
        ));
    }
    let n_pos = pts.iter().filter(|p| p.1).count();
    let n_neg = pts.len() - n_pos;
    if n_neg < 30 {
        println!("\n  only {n_neg} non-decode(s) scored; under the 30 floor, sigma is not fitted.");
        return;
    }

    // Mean negative log-likelihood. Clamped away from 0 and 1 so one confident
    // mistake cannot make the objective infinite.
    let nll = |sigma: f64, b: f64| -> f64 {
        let mut acc = 0.0;
        for (m, y) in &pts {
            let p = noise::decode_probability(m + b, sigma).clamp(1e-9, 1.0 - 1e-9);
            acc -= if *y { p.ln() } else { (1.0_f64 - p).ln() };
        }
        #[allow(clippy::cast_precision_loss)]
        {
            acc / pts.len() as f64
        }
    };

    let (mut sigma, mut b) = (8.0, 0.0);
    let mut best = nll(sigma, b);
    let (mut ds, mut db) = (4.0, 8.0);
    for _ in 0..80 {
        let mut moved = false;
        for (d, is_sigma) in [(ds, true), (db, false)] {
            for dir in [1.0, -1.0] {
                let (s2, b2) = if is_sigma {
                    ((sigma + dir * d).clamp(0.5, 40.0), b)
                } else {
                    (sigma, b + dir * d)
                };
                let v = nll(s2, b2);
                if v < best - 1e-12 {
                    best = v;
                    sigma = s2;
                    b = b2;
                    moved = true;
                }
            }
        }
        if !moved {
            ds *= 0.5;
            db *= 0.5;
            if ds < 1e-3 && db < 1e-3 {
                break;
            }
        }
    }

    println!("\n  fitted predictive spread sigma   {sigma:.2} dB");
    println!("  fitted intercept b               {b:+.2} dB   (absorbs this corpus's class");
    println!("                                   balance - NOT shippable, see the doc comment)");
    println!(
        "  mean negative log-likelihood     {best:.4}   (a coin flip on every spot is 0.6931)"
    );
    println!("  scored on {n_pos} decode(s) and {n_neg} non-decode(s)");
    println!(
        "\n  For reference: repeat measurements of one path give 3.5 dB, and the model's own\n  \
         out-of-sample error is 7.3 dB. A fitted sigma near the latter says the spread the\n  \
         operator faces is dominated by MODEL error, not by fading."
    );

    // Reliability: does a predicted 70 % decode 70 % of the time? This is the
    // only question a probability has to answer, and it is not the AUC.
    println!("\n  RELIABILITY - predicted probability against observed decode rate:");
    println!(
        "  {:<16} {:>8} {:>12} {:>12} {:>10}",
        "predicted P", "n", "mean pred", "observed", "gap"
    );
    let mut brier = 0.0;
    for (lo, hi) in [
        (0.0, 0.1),
        (0.1, 0.3),
        (0.3, 0.5),
        (0.5, 0.7),
        (0.7, 0.9),
        (0.9, 1.001),
    ] {
        let cell: Vec<(f64, bool)> = pts
            .iter()
            .map(|(m, y)| (noise::decode_probability(m + b, sigma), *y))
            .filter(|(p, _)| *p >= lo && *p < hi)
            .collect();
        #[allow(clippy::cast_precision_loss)]
        let n = cell.len() as f64;
        if cell.len() < 30 {
            println!(
                "  {:<16} {:>8} {:>12} {:>12} {:>10}",
                format!("{lo:.1} - {hi:.1}"),
                cell.len(),
                "under 30",
                "under 30",
                "-"
            );
            continue;
        }
        let mean_p = cell.iter().map(|(p, _)| p).sum::<f64>() / n;
        #[allow(clippy::cast_precision_loss)]
        let obs = cell.iter().filter(|(_, y)| *y).count() as f64 / n;
        println!(
            "  {:<16} {:>8} {:>12.3} {:>12.3} {:>+10.3}",
            format!("{lo:.1} - {hi:.1}"),
            cell.len(),
            mean_p,
            obs,
            mean_p - obs
        );
    }
    for (m, y) in &pts {
        let p = noise::decode_probability(m + b, sigma);
        let t = if *y { 1.0 } else { 0.0 };
        brier += (p - t) * (p - t);
    }
    #[allow(clippy::cast_precision_loss)]
    let brier = brier / pts.len() as f64;
    println!("\n  Brier score {brier:.4}   (lower is better; 0.25 is a constant 50 % guess)");
    println!("  A 'gap' column near zero means the probabilities are honest. Large gaps mean");
    println!("  the S-curve has the wrong slope, which is a statement about sigma and nothing");
    println!("  else - the ranking is untouched by any of this.");

    // The shipped sigma, scored the same way. The app uses
    // `noise::PREDICTIVE_SPREAD_DB`, the measured out-of-sample error, rather
    // than the fitted value, which the negatives inflate. Only the intercept is
    // re-optimised, by a plain scan, since it absorbs this corpus's class
    // balance.
    let shipped = noise::PREDICTIVE_SPREAD_DB;
    let mut b_ship = 0.0;
    let mut best_ship = f64::INFINITY;
    for cand in 0..=160 {
        let trial = -20.0 + f64::from(cand) * 0.25;
        let v = nll(shipped, trial);
        if v < best_ship {
            best_ship = v;
            b_ship = trial;
        }
    }
    println!("\n  THE SHIPPED SIGMA, scored the same way: {shipped:.1} dB");
    println!("  (the app uses this, not the fitted value above - see PREDICTIVE_SPREAD_DB)");
    println!(
        "  {:<16} {:>8} {:>12} {:>12} {:>10}",
        "predicted P", "n", "mean pred", "observed", "gap"
    );
    let mut brier_ship = 0.0;
    for (lo, hi) in [
        (0.0, 0.1),
        (0.1, 0.3),
        (0.3, 0.5),
        (0.5, 0.7),
        (0.7, 0.9),
        (0.9, 1.001),
    ] {
        let cell: Vec<(f64, bool)> = pts
            .iter()
            .map(|(m, y)| (noise::decode_probability(m + b_ship, shipped), *y))
            .filter(|(p, _)| *p >= lo && *p < hi)
            .collect();
        if cell.len() < 30 {
            println!(
                "  {:<16} {:>8} {:>12} {:>12} {:>10}",
                format!("{lo:.1} - {hi:.1}"),
                cell.len(),
                "under 30",
                "under 30",
                "-"
            );
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let n = cell.len() as f64;
        let mean_p = cell.iter().map(|(p, _)| p).sum::<f64>() / n;
        #[allow(clippy::cast_precision_loss)]
        let obs = cell.iter().filter(|(_, y)| *y).count() as f64 / n;
        println!(
            "  {:<16} {:>8} {:>12.3} {:>12.3} {:>+10.3}",
            format!("{lo:.1} - {hi:.1}"),
            cell.len(),
            mean_p,
            obs,
            mean_p - obs
        );
    }
    for (m, y) in &pts {
        let p = noise::decode_probability(m + b_ship, shipped);
        let t = if *y { 1.0 } else { 0.0 };
        brier_ship += (p - t) * (p - t);
    }
    #[allow(clippy::cast_precision_loss)]
    let brier_ship = brier_ship / pts.len() as f64;
    println!("  mean NLL {best_ship:.4}, Brier {brier_ship:.4}, intercept {b_ship:+.2} dB");
    println!("  If the shipped sigma's gaps below a half are SMALLER than the fitted one's,");
    println!("  the narrower spread is the better answer and the fit was chasing the");
    println!("  negatives' contamination rather than the model's real spread.");
}

pub(crate) fn report_negatives(
    negatives: &[Negative],
    available: usize,
    stride: usize,
    cached: &[Cached],
    args: &Args,
    levels: Levels,
    fitted: &CachedParams,
) {
    println!("\n=== FALSE POSITIVES, AGAINST CONSTRUCTED NEGATIVES =========");
    println!("  Each negative is a path where the transmitter demonstrably transmitted,");
    println!("  the receiver demonstrably decoded several OTHER stations on the same band");
    println!("  in the same cycle, and the receiver did not decode this transmitter.");
    println!("  It may still have collided, or arrived off the back of a beam, so the");
    println!("  rate below is an UPPER BOUND on the model's true false-positive rate.");
    if stride > 1 {
        println!(
            "  Scored on {} of {available} available negatives (every {stride}th), because",
            negatives.len()
        );
        println!("  each one costs a full solve.\n");
    } else {
        println!();
    }

    // `offset_db` is subtracted from the modelled SNR before the threshold
    // comparison: 0 scores the model exactly as the app would ship it, and the
    // scoring's own global offset scores it with the bias the fit measured but
    // could not attribute already taken off.
    let score = |scale: f64, atm: AtmosphericAnchors, offset_db: f64| -> NegativeScore {
        let mut margins = Vec::new();
        let mut decodable = 0usize;
        let mut via_es = 0usize;
        for c in cached {
            let snr = c.modelled_db(scale, atm) - offset_db;
            margins.push(snr - WSPR_DECODE_THRESHOLD_DB);
            if snr >= WSPR_DECODE_THRESHOLD_DB {
                decodable += 1;
                if c.layer == "Es" {
                    via_es += 1;
                }
            }
        }
        NegativeScore {
            n: negatives.len(),
            path_found: cached.len(),
            predicted_decodable: decodable,
            via_es,
            margin: Spread::of(&margins),
        }
    };
    let prior_atm = AtmosphericAnchors::default();
    let before = score(1.0, prior_atm, 0.0);
    let after = score(fitted.absorption_scale.value, fitted.atm, 0.0);
    let before_matched = score(1.0, prior_atm, levels.before);
    let after_matched = score(fitted.absorption_scale.value, fitted.atm, levels.after);

    println!("  {:<44} {:>12} {:>12}", "", "before fit", "after fit");
    println!(
        "  {:<44} {:>12} {:>12}",
        "negatives constructed", before.n, after.n
    );
    println!(
        "  {:<44} {:>12} {:>12}",
        "model found SOME path", before.path_found, after.path_found
    );

    let block = |b: &NegativeScore, a: &NegativeScore| {
        println!(
            "  {:<44} {:>12} {:>12}",
            "model predicted it would DECODE", b.predicted_decodable, a.predicted_decodable
        );
        println!(
            "  {:<44} {:>11.1}% {:>11.1}%",
            "FALSE POSITIVE RATE (upper bound)",
            100.0 * b.false_positive_rate(),
            100.0 * a.false_positive_rate()
        );
        println!(
            "  {:<44} {:>12} {:>12}",
            "  of those, needed sporadic E", b.via_es, a.via_es
        );
        println!(
            "  {:<44} {:>12.1} {:>12.1}",
            "median margin over threshold [dB]", b.margin.median, a.margin.median
        );
    };

    println!("\n  -- AS SHIPPED: the raw modelled SNR against the threshold --");
    block(&before, &after);

    println!(
        "\n  -- LEVEL-MATCHED: each column's own global offset ({:+.2} / {:+.2} dB) removed --",
        levels.before, levels.after
    );
    block(&before_matched, &after_matched);
    println!("  The fit's objective is invariant to the level of the prediction - the global");
    println!("  offset absorbs any constant shift at no cost - but the decode threshold is an");
    println!("  ABSOLUTE comparison, so the AS SHIPPED columns charge the fit for a slide that");
    println!("  cost it nothing. Only the LEVEL-MATCHED pair compares like with like, and so");
    println!("  only it says whether the fit made the model's SHAPE more permissive.");
    println!("  Neither pair is decoration: AS SHIPPED is what a user of the app sees, and if");
    println!("  it is far worse than LEVEL-MATCHED then the app is shipping a known bias it has");
    println!("  measured and declined to apply.");

    // The decoded corpus is conditioned on decoding, so a bias measured on it
    // may come from selection; fading survivorship biases the measured values
    // upward, giving a negative residual. The negatives have no such
    // conditioning. A model that over-predicts by day must produce its false
    // positives by day; equal day and night rates would mean the daytime bias
    // on the decodes is not an absolute over-prediction. Level-matched, so no
    // constant offset moves either side.
    println!("\n  -- LEVEL-MATCHED FALSE POSITIVES, SPLIT AT THE MIDPOINT TERMINATOR --");
    println!("  The decodes are conditioned on decoding; these are not. A daytime bias that is");
    println!("  a real absolute over-prediction has to spend itself here, by day.");
    println!(
        "  {:<20} {:>8} {:>8} {:>14} {:>14} {:>16}",
        "cut", "spots", "f [MHz]", "FP before", "FP after", "med margin after"
    );
    for (label, want_night) in [("midpoint day", false), ("midpoint night", true)] {
        let group: Vec<Cached> = cached
            .iter()
            .filter(|c| c.midpoint_is_night() == want_night)
            .cloned()
            .collect();
        // The two cells have entirely different band populations, so the median
        // frequency is printed beside the rate: a day/night comparison between a
        // 7 MHz cell and a 3.5 MHz cell is the same confound as everywhere else.
        let mut freqs: Vec<f64> = group.iter().map(|c| c.freq_mhz).collect();
        freqs.sort_by(f64::total_cmp);
        let med_f = fit::percentile(&freqs, 0.5);
        if group.len() < MIN_QUOTABLE {
            println!(
                "  {:<20} {:>8} {:>8.1} {:>14} {:>14} {:>16}",
                label,
                group.len(),
                med_f,
                "under 30",
                "under 30",
                "under 30"
            );
            continue;
        }
        let rate = |scale: f64, atm: AtmosphericAnchors, offset: f64| -> (f64, f64) {
            let mut margins: Vec<f64> = group
                .iter()
                .map(|c| c.modelled_db(scale, atm) - offset - WSPR_DECODE_THRESHOLD_DB)
                .collect();
            #[allow(clippy::cast_precision_loss)]
            let fp =
                margins.iter().filter(|m| **m >= 0.0).count() as f64 * 100.0 / group.len() as f64;
            margins.sort_by(f64::total_cmp);
            (fp, fit::percentile(&margins, 0.5))
        };
        let (fp_b, _) = rate(1.0, prior_atm, levels.before);
        let (fp_a, margin_a) = rate(fitted.absorption_scale.value, fitted.atm, levels.after);
        println!(
            "  {:<20} {:>8} {:>8.1} {:>13.1}% {:>13.1}% {:>15.1}",
            label,
            group.len(),
            med_f,
            fp_b,
            fp_a,
            margin_a
        );
    }

    if args.trim_tails {
        println!("\n  Note: negatives are NOT trimmed - a station being unrepresentative does");
        println!("  not make its silence less real.");
    }
}
