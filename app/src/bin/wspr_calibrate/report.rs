//! Everything the run prints. No fitting happens here: each function takes a
//! solved set and reports what is in it.
//!
//! These are separate functions rather than one report because each answers a
//! different question, and a run that only wants one of them should not have to
//! compute the rest.

use crate::driver::*;
use crate::solving::*;

use std::collections::BTreeMap;

use skipzone_app::calib::{Anchors, AtmosphericAnchors};
use skipzone_app::fit::{
    self, Cached, CachedParams, EffectDistribution, Fit, Spread, StationEffects,
};
use skipzone_app::noise::NoiseFloor;
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::wspr_report::band_label;
/// How far the fitted atmospheric anchors actually moved the noise floor, band by
/// band, over this corpus's own day/night and latitude mix.
///
/// # Why a bound hit on these anchors needs this table beside it
///
/// The atmospheric term is one of THREE figures summed in power - the other two
/// are the man-made and galactic curves of P.372-9 Table 1, which are not
/// calibration targets and do not move. Above roughly 10 MHz those two floor the
/// total and the atmospheric term sits 10 dB or more underneath, so the anchors
/// can be driven anywhere at all without changing the composed floor by more than
/// a fraction of a dB.
///
/// That makes a bound hit ambiguous unless this is printed next to it. An anchor
/// on its bound may be chasing a residual it can reach, or it may be chasing one
/// it cannot reach at any value - and only the second is evidence that the error
/// is elsewhere in the model. The column below distinguishes them: a band where
/// the floor barely moved is a band the noise model could not have fixed however
/// the fit set these numbers.
pub(crate) fn report_noise_leverage(
    set: &Solved,
    prior: AtmosphericAnchors,
    fitted: AtmosphericAnchors,
) {
    println!("\n--- WHERE THE NOISE MODEL HAS LEVERAGE ---------------------");
    println!("  Median shift of the COMPOSED noise floor between the prior and the fitted");
    println!("  atmospheric anchors, over each band's own spots. Negative means quieter, i.e.");
    println!("  the model reads more optimistic there. The atmospheric term is power-summed");
    println!("  with the man-made and galactic curves of P.372-9 Table 1, which do not move,");
    println!("  so a band where those dominate cannot be shifted by these anchors at all.\n");
    let mut by_band: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for c in &set.spots {
        by_band
            .entry(band_label(c.freq_mhz).to_string())
            .or_default()
            .push(c.noise_dbm(fitted) - c.noise_dbm(prior));
    }
    println!("  {:<10} {:>6} {:>20}", "band", "spots", "floor shift [dB]");
    for (band, mut shifts) in by_band {
        shifts.sort_by(f64::total_cmp);
        println!(
            "  {:<10} {:>6} {:>20.1}",
            band,
            shifts.len(),
            fit::percentile(&shifts, 0.5)
        );
    }
}

pub(crate) fn report_fit(f: &Fit, set: &Solved) {
    println!(
        "  {} spots closed of {} ({:.0} % hit rate, ONE-SIDED - see the negatives section)",
        f.n,
        set.total,
        100.0 * f.n as f64 / set.total.max(1) as f64
    );
    println!("  median error        {:+7.1} dB", f.residual.median);
    println!("  IQR                 {:7.1} dB", f.residual.iqr());
    println!(
        "  10th / 90th pct     {:+7.1} / {:+.1} dB",
        f.residual.p10, f.residual.p90
    );
    println!(
        "  slope vs measured   {:7.2}  (raw)   {:7.2}  (station effects removed)",
        f.slope_raw, f.slope_adjusted
    );
    println!(
        "  R2                  {:7.2}  (raw)   {:7.2}  (station effects removed)",
        f.r2_raw, f.r2_adjusted
    );
}

pub(crate) fn report_layers(
    set: &Solved,
    scale: f64,
    atm: AtmosphericAnchors,
    effects: &StationEffects,
) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &set.spots {
        *counts.entry(c.layer).or_default() += 1;
    }
    println!("  layer chosen:");
    for (layer, n) in counts {
        let group: Vec<Cached> = set
            .spots
            .iter()
            .filter(|c| c.layer == layer)
            .cloned()
            .collect();
        let f = Fit::of(&group, scale, atm, effects);
        println!(
            "    {layer:<4} {n:>5} spots, median {:+6.1} dB, IQR {:5.1} dB",
            f.residual.median,
            f.residual.iqr()
        );
    }
}

pub(crate) fn print_cuts(
    set: &Solved,
    after: &Scored,
    before: &Scored,
    key: impl Fn(&Cached) -> String + Copy,
) {
    print_cuts_min(set, after, before, 0, key);
}

/// As [`print_cuts`], but suppressing cells thinner than `min_spots` and saying
/// how much was suppressed.
///
/// A two-way cut multiplies out into many cells, most of them holding a handful
/// of spots whose median is that handful's own noise. Printing them all would
/// bury the cells that carry the corpus; dropping them silently would hide how
/// much of it went. So the thin cells are pooled into one reported total.
pub(crate) fn print_cuts_min(
    set: &Solved,
    after: &Scored,
    before: &Scored,
    min_spots: usize,
    key: impl Fn(&Cached) -> String + Copy,
) {
    println!(
        "  {:<20} {:>6} {:>10} {:>9} {:>10} {:>9} {:>8}",
        "cut", "spots", "med before", "med after", "IQR before", "IQR after", "slope*"
    );
    let a = fit::cuts_by(
        &set.spots,
        after.fit.n as f64 * 0.0 + 1.0,
        AtmosphericAnchors::default(),
        &after.effects,
        key,
    );
    // Recompute properly with the fitted parameters rather than the placeholder
    // above, which exists only to keep the two cut lists in the same order.
    let _ = a;
    let before_cuts = fit::cuts_by(
        &set.spots,
        1.0,
        AtmosphericAnchors::default(),
        &before.effects,
        key,
    );
    let after_cuts = fit::cuts_by(&set.spots, after.scale, after.atm, &after.effects, key);
    let lookup: BTreeMap<&str, &fit::Cut> =
        before_cuts.iter().map(|c| (c.label.as_str(), c)).collect();
    let mut suppressed_cells = 0usize;
    let mut suppressed_spots = 0usize;
    for c in &after_cuts {
        if c.fit.n < min_spots {
            suppressed_cells += 1;
            suppressed_spots += c.fit.n;
            continue;
        }
        let b = lookup.get(c.label.as_str());
        println!(
            "  {:<20} {:>6} {:>10} {:>9} {:>10} {:>9} {:>8}",
            c.label,
            c.fit.n,
            b.map_or("-".to_string(), |b| format!(
                "{:+.1}",
                b.fit.residual.median
            )),
            format!("{:+.1}", c.fit.residual.median),
            b.map_or("-".to_string(), |b| format!("{:.1}", b.fit.residual.iqr())),
            format!("{:.1}", c.fit.residual.iqr()),
            format!("{:.2}", c.fit.slope_adjusted),
        );
    }
    if suppressed_cells > 0 {
        println!(
            "  ({suppressed_cells} cell(s) holding {suppressed_spots} spot(s) below the \
             {min_spots}-spot floor are not shown)"
        );
    }
    println!("  * slope with station effects removed; below ~8 spots a cut is not a trend");
}

/// For every spot a layer BELOW F2 was reported on: was an F2 path also
/// available, by how much did the lower layer beat it, and what would the
/// residual have been had F2 been reported instead?
///
/// # The question this settles
///
/// A lower layer reading optimistic has two quite different causes, and they
/// need opposite fixes.
///
/// If an F2 path was available and lost, the layer was decided by a COMPARISON.
/// A reflection at 105 km has a shorter ray than one at 300 km, so on raw SNR the
/// lower layer can win by construction wherever it closes at all - not because
/// the ionosphere favoured it. That is a selection rule to change, and the last
/// column says so outright: if the F2 alternative's residual is near zero while
/// the winner's is large, the physics was right and only the choice was wrong.
///
/// If no F2 path was available, the lower layer was the only answer there was,
/// and the question is instead why it closed - the layer's ionisation, not the
/// ranking. Those spots are counted separately for exactly that reason.
///
/// Scored at the BASELINE parameters throughout. The point is what the model does
/// before anything is fitted to it; a fitted scale would let the level absorb part
/// of the very gap being measured.
pub(crate) fn report_layer_races(set: &Solved) {
    println!("\n--- WHEN A LAYER BELOW F2 WAS REPORTED ---------------------");
    println!("  For each such spot: did an F2 path also close, by how much was it out-scored,");
    println!("  and what would the residual have been had F2 been reported instead? All at the");
    println!("  BASELINE parameters, so no fitted level can absorb the gap being measured.\n");
    let atm = AtmosphericAnchors::default();

    let mut by_layer: BTreeMap<&str, Vec<&Cached>> = BTreeMap::new();
    for c in &set.spots {
        if c.layer != "F2" {
            by_layer.entry(c.layer).or_default().push(c);
        }
    }
    if by_layer.is_empty() {
        println!("  No spot was reported on a layer below F2.");
        return;
    }
    println!(
        "  {:<6} {:>6} {:>10} {:>12} {:>14} {:>14}",
        "layer", "spots", "F2 also", "median win", "median resid", "resid if F2"
    );
    for (layer, group) in by_layer {
        let raced: Vec<&&Cached> = group.iter().filter(|c| c.layer_was_a_race()).collect();
        let median_of = |mut v: Vec<f64>| -> String {
            if v.is_empty() {
                return "-".to_string();
            }
            v.sort_by(f64::total_cmp);
            format!("{:+.1}", fit::percentile(&v, 0.5))
        };
        // How far the reported path out-scored the F2 one it beat.
        let win = median_of(
            raced
                .iter()
                .filter_map(|c| {
                    Some(c.modelled_db(1.0, atm) - c.alternative_modelled_db(1.0, atm)?)
                })
                .collect(),
        );
        // The residual as reported, over the SAME raced spots, so the two
        // residual columns are computed on one population and are comparable.
        let resid = median_of(
            raced
                .iter()
                .map(|c| c.modelled_db(1.0, atm) - c.measured_db)
                .collect(),
        );
        let resid_alt = median_of(
            raced
                .iter()
                .filter_map(|c| Some(c.alternative_modelled_db(1.0, atm)? - c.measured_db))
                .collect(),
        );
        println!(
            "  {:<6} {:>6} {:>10} {:>12} {:>14} {:>14}",
            layer,
            group.len(),
            raced.len(),
            win,
            resid,
            resid_alt
        );
    }
    println!("\n  'F2 also' counts the spots where an F2 path closed TOO and was out-scored.");
    println!("  Those were decided by a comparison. The rest had no F2 to compare against, so");
    println!("  for them the question is why the lower layer closed at all, not how it ranked.");
    println!("  If 'resid if F2' sits near zero while 'median resid' does not, the physics was");
    println!("  right and the SELECTION was wrong - the two need entirely different fixes.");
}

/// How much absorption the model actually produces, against the two shapes it is
/// asked to serve at once: solar zenith angle and frequency.
///
/// # What this exists to decide
///
/// Absorption is the ONLY diurnal term in the link budget, and it is also the
/// dominant frequency-shaped one. It is moved by a single multiplicative scale.
/// So when the residual carries a diurnal structure and a frequency structure
/// that want the scale moved in opposite directions, the scale can satisfy
/// neither, and no amount of fitting it will help.
///
/// Before adding any parameter to relieve that, the model's existing dynamic
/// range has to be measured, because there are two quite different failures and
/// the columns below tell them apart:
///
/// * absorption barely varies across zenith angle - then the diurnal signal is
///   missing from the model rather than mis-scaled, and the fix is in the
///   absorption PROFILE. The D region is the layer that appears and disappears
///   with the sun, so a D region carrying little of the integral produces exactly
///   this;
/// * absorption varies, but by far less than the residual does - then the shape
///   is present and too weak, which is the classic alpha-Chapman `cos^0.5 chi`
///   against the observed `cos^0.75 chi`, and the fix is in that law.
///
/// Restricted to F2, deliberately. It is the layer whose residual is otherwise
/// unbiased, so its diurnal structure is not contaminated by a layer-selection
/// question. Scored at the BASELINE scale of 1.0: the point is what the model
/// produces before anything is fitted to it.
/// Does the model's day/night step in the NOISE FLOOR match the step the
/// residual actually takes across the receiver's terminator?
///
/// # What this measures
///
/// The noise floor is the only term in the link budget that switches
/// DISCONTINUOUSLY at the terminator: [`crate::noise::atmospheric_noise_figure_db`]
/// selects one of two `(Fa, slope)` pairs on a boolean. Everything else - the D
/// region above all - varies smoothly through it, because `Ch(X, chi)` is smooth.
///
/// So if the residual also steps discontinuously there, the noise model is
/// implicated. But the SIZE of that jump is not the noise model's error on its
/// own, and an earlier version of this function said it was. The identity is:
///
/// ```text
/// resid = modelled - measured - station effects
/// modelled = P_tx - loss - absorption - noise
///
/// observed = resid(day) - resid(night)
///          = (noise_night - noise_day) - (abs_day - abs_night) - d_measured
///          =  model step          -  d_absorb              -  d_measured
/// ```
///
/// so `model step - observed` is `d_absorb + d_measured`, NOT reality's noise
/// step. Absorption is also a day/night term - that is the whole point of it -
/// and it does not drop out. All four quantities are printed instead, so the
/// decomposition is visible and the identity can be checked by eye.
///
/// `d_measured` is the only column with no model in it at all: it is what the
/// receivers actually reported, day minus night, corrected for which stations
/// were in each cell. The model gets the day/night structure of a band right
/// when `model step - d_absorb` equals `d_measured`.
///
/// Split at the RECEIVER's terminator, not the midpoint's, because that is what
/// the noise floor is keyed on and where it is heard.
pub(crate) fn report_terminator_step(
    set: &Solved,
    effects: &StationEffects,
    atm: AtmosphericAnchors,
    fitted: Option<(&StationEffects, AtmosphericAnchors, f64)>,
) {
    println!("\n--- THE STEP AT THE TERMINATOR -----------------------------");
    println!("  The noise floor is the ONLY term that switches discontinuously at the");
    println!("  terminator; absorption varies smoothly through it because Ch(X, chi) does.");
    println!("  So a discontinuity in the residual there is the noise model's, and its size");
    println!("  is (model's step - reality's step). 'true' near 0 means the model's whole");
    println!("  day/night step is spurious. Split at the RECEIVER's terminator, which is what");
    println!("  the noise floor is keyed on. Cells under {MIN_QUOTABLE} spots are refused.");
    println!();
    println!("  READ THE 'F2 day' COLUMN FIRST. It is the fraction of the DAY cell that is an");
    println!("  F2 path. Where it is near zero the day side is made of daytime E-layer paths,");
    println!("  which carry the largest residuals in the corpus and a defect of their own, so");
    println!("  that band's 'true' step is measuring the E-layer defect and not the noise");
    println!("  floor. Only rows with a substantial F2 day fraction constrain the surrogate.");
    println!(
        "\n  {:<8} {:>7} {:>7} {:>7} {:>11} {:>10} {:>11} {:>10}",
        "band", "n day", "n night", "F2 day", "d_measured", "d_absorb", "model step", "observed"
    );

    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        fit::percentile(&v, 0.5)
    };
    let mut by_band: BTreeMap<String, Vec<&Cached>> = BTreeMap::new();
    for c in &set.spots {
        by_band
            .entry(band_label(c.freq_mhz).to_string())
            .or_default()
            .push(c);
    }

    for (band, group) in &by_band {
        let resid = |g: &[&&Cached]| -> Vec<f64> {
            g.iter()
                .map(|c| c.modelled_db(1.0, atm) - c.measured_db - effects.offset_for(c))
                .collect()
        };
        let day: Vec<&&Cached> = group.iter().filter(|c| c.rx_is_day).collect();
        let night: Vec<&&Cached> = group.iter().filter(|c| !c.rx_is_day).collect();
        if day.len() < MIN_QUOTABLE || night.len() < MIN_QUOTABLE {
            continue;
        }
        // The model's own step, evaluated per spot at ITS frequency, latitude and
        // season and then taken as a median, so the comparison uses the same
        // population as the residual either side of it.
        let model_step = median(
            group
                .iter()
                .map(|c| {
                    let night_floor = NoiseFloor::compute(
                        c.freq_mhz,
                        c.bandwidth_hz,
                        c.noise_env,
                        false,
                        c.rx_season,
                        c.rx_lat,
                        atm,
                    );
                    let day_floor = NoiseFloor::compute(
                        c.freq_mhz,
                        c.bandwidth_hz,
                        c.noise_env,
                        true,
                        c.rx_season,
                        c.rx_lat,
                        atm,
                    );
                    night_floor.power_dbm - day_floor.power_dbm
                })
                .collect(),
        );
        let r_day = median(resid(&day));
        let r_night = median(resid(&night));
        // The station correction rides with the measurement, so that d_measured
        // compares like with like when the two cells hold different stations.
        let meas = |g: &[&&Cached]| -> Vec<f64> {
            g.iter()
                .map(|c| c.measured_db + effects.offset_for(c))
                .collect()
        };
        let d_measured = median(meas(&day)) - median(meas(&night));
        let absorb = |g: &[&&Cached]| -> Vec<f64> { g.iter().map(|c| c.absorption_db).collect() };
        let d_absorb = median(absorb(&day)) - median(absorb(&night));
        #[allow(clippy::cast_precision_loss)]
        let f2_frac = day.iter().filter(|c| c.layer == "F2").count() as f64 / day.len() as f64;
        println!(
            "  {:<8} {:>7} {:>7} {:>6.0}% {:>11.1} {:>10.1} {:>11.1} {:>10.1}",
            band,
            day.len(),
            night.len(),
            f2_frac * 100.0,
            d_measured,
            d_absorb,
            model_step,
            r_day - r_night,
        );
    }
    println!("\n  observed = model step - d_absorb - d_measured, so the row can be checked by");
    println!("  medians: a median does not distribute over a subtraction, and the day and night");
    println!("  cells hold different stations. Four separate measurements, not an identity.");
    println!("  A surrogate that has the day/night structure right prints observed ~ 0.");

    // Same table under the fitted anchors. The point of the day/night step being
    // a fitted parameter rather than the difference of two absolute levels is
    // that the fit can now aim at THIS column directly, so it is worth showing
    // whether it managed to.
    let Some((fit_effects, fit_atm, scale)) = fitted else {
        return;
    };
    println!("\n  The same, under the FITTED anchors. 'observed' is what the fit failed to");
    println!("  remove; if the reparameterised step can carry this at all, it shrinks here.");
    println!(
        "\n  {:<8} {:>7} {:>7} {:>7} {:>11} {:>10} {:>11} {:>10}",
        "band", "n day", "n night", "F2 day", "d_measured", "d_absorb", "model step", "observed"
    );
    for (band, group) in &by_band {
        let resid = |g: &[&&Cached]| -> Vec<f64> {
            g.iter()
                .map(|c| c.modelled_db(scale, fit_atm) - c.measured_db - fit_effects.offset_for(c))
                .collect()
        };
        let day: Vec<&&Cached> = group.iter().filter(|c| c.rx_is_day).collect();
        let night: Vec<&&Cached> = group.iter().filter(|c| !c.rx_is_day).collect();
        if day.len() < MIN_QUOTABLE || night.len() < MIN_QUOTABLE {
            continue;
        }
        let model_step = median(
            group
                .iter()
                .map(|c| {
                    let n = |is_day| {
                        NoiseFloor::compute(
                            c.freq_mhz,
                            c.bandwidth_hz,
                            c.noise_env,
                            is_day,
                            c.rx_season,
                            c.rx_lat,
                            fit_atm,
                        )
                        .power_dbm
                    };
                    n(false) - n(true)
                })
                .collect(),
        );
        let r_day = median(resid(&day));
        let r_night = median(resid(&night));
        let meas = |g: &[&&Cached]| -> Vec<f64> {
            g.iter()
                .map(|c| c.measured_db + fit_effects.offset_for(c))
                .collect()
        };
        let d_measured = median(meas(&day)) - median(meas(&night));
        let absorb =
            |g: &[&&Cached]| -> Vec<f64> { g.iter().map(|c| scale * c.absorption_db).collect() };
        let d_absorb = median(absorb(&day)) - median(absorb(&night));
        #[allow(clippy::cast_precision_loss)]
        let f2_frac = day.iter().filter(|c| c.layer == "F2").count() as f64 / day.len() as f64;
        println!(
            "  {:<8} {:>7} {:>7} {:>6.0}% {:>11.1} {:>10.1} {:>11.1} {:>10.1}",
            band,
            day.len(),
            night.len(),
            f2_frac * 100.0,
            d_measured,
            d_absorb,
            model_step,
            r_day - r_night,
        );
    }
}

/// The census under every cut the daytime residual gets read from, and what
/// each cell is confounded with.
///
/// # Why this exists
///
/// Two separate failures were reached by quoting medians that could not carry
/// them, and both are invisible in a table that prints only the median:
///
///   * a cell too THIN to have a median at all - the zenith block's `< 30 deg`
///     cell holds 17 F2 spots, and the claim that the residual is worst where
///     absorption is weakest rests on it;
///   * a cell big enough but not BALANCED - this corpus works the high bands by
///     day and the low bands at night, so a "day" cell whose median frequency is
///     14 MHz against a "night" cell at 5 MHz is a frequency contrast wearing a
///     diurnal label.
///
/// So this prints the count, the median frequency, the median path length and
/// the median midpoint zenith of every cell, and REFUSES to print a residual for
/// any cell under [`MIN_QUOTABLE`] spots. A refusal is the finding: it says the
/// corpus cannot answer that question, which is worth more than a number that
/// looks like it can.
pub(crate) const MIN_QUOTABLE: usize = 30;

pub(crate) fn report_confound_census(set: &Solved, effects: &StationEffects) {
    println!("\n--- CENSUS FIRST: WHAT IS EACH CELL MADE OF? ---------------");
    println!("  A median is worth quoting only from a cell big enough to have one, and a cell");
    println!("  isolates the variable it is named after only if the OTHER variables are");
    println!("  balanced across it. Both facts, for every cut the daytime residual is read");
    println!(
        "  from. Cells under {MIN_QUOTABLE} spots print NO median: the corpus cannot answer there."
    );
    println!("  'resid' is raw modelled-measured at the BASELINE anchors; 'resid-eff' has the");
    println!("  station effects and the global offset removed, so it is the shape alone.");

    let atm = AtmosphericAnchors::default();
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        fit::percentile(&v, 0.5)
    };

    let table = |title: &str, spots: &[&Cached], key: &dyn Fn(&Cached) -> String| {
        println!(
            "\n  {:<20} {:>6} {:>8} {:>10} {:>8} {:>6} {:>6} {:>8} {:>10}",
            title,
            "spots",
            "f [MHz]",
            "range [km]",
            "zenith",
            "hops",
            "absorb",
            "resid",
            "resid-eff"
        );
        let mut cells: BTreeMap<String, Vec<&Cached>> = BTreeMap::new();
        for c in spots {
            cells.entry(key(c)).or_default().push(c);
        }
        for (label, g) in &cells {
            let quotable = g.len() >= MIN_QUOTABLE;
            let raw = median(
                g.iter()
                    .map(|c| c.modelled_db(1.0, atm) - c.measured_db)
                    .collect(),
            );
            let eff = median(
                g.iter()
                    .map(|c| c.modelled_db(1.0, atm) - c.measured_db - effects.offset_for(c))
                    .collect(),
            );
            println!(
                "  {:<20} {:>6} {:>8.1} {:>10.0} {:>8.0} {:>6.1} {:>6.1} {:>8} {:>10}",
                label,
                g.len(),
                median(g.iter().map(|c| c.freq_mhz).collect()),
                median(g.iter().map(|c| c.range_km).collect()),
                median(g.iter().map(|c| c.midpoint_zenith_deg).collect()),
                median(g.iter().map(|c| f64::from(c.hops)).collect()),
                // Absorption the model APPLIED in this cell, at the baseline
                // scale. A multiplicative fix can only move a cell by
                // (s-1) x this, so it is the number that decides whether one
                // scale can carry the day-night gap on two different bands.
                median(g.iter().map(|c| c.absorption_db).collect()),
                if quotable {
                    format!("{raw:+.1}")
                } else {
                    "under 30".to_string()
                },
                if quotable {
                    format!("{eff:+.1}")
                } else {
                    "under 30".to_string()
                },
            );
        }
    };

    let all: Vec<&Cached> = set.spots.iter().collect();
    let f2: Vec<&Cached> = set.spots.iter().filter(|c| c.layer == "F2").collect();
    let day = |c: &Cached| {
        if c.midpoint_is_night() {
            "night"
        } else {
            "day"
        }
    };

    table("layer x midpoint", &all, &|c| {
        format!("{:<3} {}", c.layer, day(c))
    });

    // A propagation loss belongs to the MIDPOINT; a noise floor belongs to the
    // RECEIVER, which is where it is heard. On a path long enough to straddle the
    // terminator those two disagree, and only there do the two hypotheses predict
    // different things. Under uniform thinning these cells hold 10-15 spots and
    // the question cannot be asked at all.
    println!("\n  Receiver terminator vs midpoint terminator. A loss belongs to the midpoint, a");
    println!("  noise floor to the receiver; only the cells where they DISAGREE tell them apart.");
    table("midpoint / rx", &all, &|c| {
        format!(
            "mid {:<5} rx {}",
            day(c),
            if c.rx_is_day { "day" } else { "night" }
        )
    });

    // THE cut. Every band-by-day cell in the F2-only block is under 30, because
    // the E layer carries most of the daytime corpus and is filtered out of it.
    // Pooling the layers back together is what makes a band legal on both sides
    // of the terminator - and a band that is legal on both sides holds FREQUENCY
    // FIXED, which is the only way this corpus can ask a diurnal question at all.
    println!("\n  Band x midpoint over ALL layers, not F2 alone. The F2-only version of this");
    println!("  table has no legal daytime cell on any band: the E layer carries the daytime");
    println!("  corpus and filtering it out empties the day side. A band legal on BOTH sides");
    println!("  holds frequency fixed, which is the only diurnal question this corpus can ask.");
    table("band x midpoint", &all, &|c| {
        format!("{:<6} {}", band_label(c.freq_mhz), day(c))
    });

    // THE cut this corpus has never been able to pose. Holding the band fixed
    // answers the diurnal question; holding the LAYER fixed too is what separates
    // the two candidate objects inside it:
    //
    //   * if E-day and F2-day on one band are both elevated, the missing loss is
    //     charged to a region both modes cross - the D region - and it is
    //     absorption, whatever the mode selection does;
    //   * if F2-day on that band is unbiased while E-day carries the excess, the
    //     model is REPORTING a mode reality did not use, and no absorption term
    //     will fix it.
    //
    // Those need entirely different repairs and are worth the whole low-band
    // daytime deficit, so nothing else in this report matters as much.
    println!("\n  Band x layer x midpoint. Frequency AND layer held fixed - the only cut that");
    println!("  separates 'the D region is under-charged' from 'the wrong mode was reported'.");
    table("band x layer x mid", &all, &|c| {
        format!("{:<6} {:<3} {}", band_label(c.freq_mhz), c.layer, day(c))
    });

    // The zenith block's two legal bins sit at different median frequencies, so
    // it measures frequency at least as much as zenith. Crossing the two is the
    // only way to tell which, and it needs a corpus dense enough to fill the
    // cells - which uniform thinning is precisely what destroys.
    println!("\n  F2 zenith x band. The zenith bins are not band-balanced, so the zenith block");
    println!("  alone cannot say whether it measures the sun or the frequency.");
    table("F2 zenith x band", &f2, &|c| {
        let z = match c.midpoint_zenith_deg {
            z if z < 30.0 => "a<30",
            z if z < 60.0 => "b30-60",
            z if z < 80.0 => "c60-80",
            z if z < 90.0 => "d80-90",
            _ => "e night",
        };
        format!("{:<7} {}", z, band_label(c.freq_mhz))
    });

    table("F2 zenith bin", &f2, &|c| {
        match c.midpoint_zenith_deg {
            z if z < 30.0 => "a) < 30 deg",
            z if z < 60.0 => "b) 30-60 deg",
            z if z < 80.0 => "c) 60-80 deg",
            z if z < 90.0 => "d) 80-90 deg",
            _ => "e) > 90 (night)",
        }
        .to_string()
    });
}

pub(crate) fn report_absorption_range(set: &Solved) {
    println!("\n--- ABSORPTION'S DYNAMIC RANGE -----------------------------");
    println!("  Absorption is the only diurnal term in the link budget AND the dominant");
    println!("  frequency-shaped one, moved by one scale. These columns say whether it has");
    println!("  the range to carry both. F2 spots only - the layer with no selection question");
    println!("  hanging over it - at the BASELINE scale, before any fitting.\n");
    let atm = AtmosphericAnchors::default();
    let f2: Vec<&Cached> = set.spots.iter().filter(|c| c.layer == "F2").collect();
    if f2.len() < 20 {
        println!(
            "  Only {} F2 spot(s); too few to read a shape from.",
            f2.len()
        );
        return;
    }

    // `absorption_db` is stored at the baseline anchors, which is the quantity
    // wanted here: what the model produces before the fit touches it.
    let summarise = |label: &str, group: &[&Cached]| {
        if group.is_empty() {
            return;
        }
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            fit::percentile(&v, 0.5)
        };
        println!(
            "  {:<18} {:>6} {:>22.1} {:>20.1}",
            label,
            group.len(),
            median(group.iter().map(|c| c.absorption_db).collect()),
            median(
                group
                    .iter()
                    .map(|c| c.modelled_db(1.0, atm) - c.measured_db)
                    .collect()
            ),
        );
    };

    println!(
        "  {:<18} {:>6} {:>22} {:>20}",
        "midpoint zenith", "spots", "median absorption [dB]", "median residual [dB]"
    );
    for (label, lo, hi) in [
        ("a) < 30 deg", 0.0, 30.0),
        ("b) 30-60 deg", 30.0, 60.0),
        ("c) 60-80 deg", 60.0, 80.0),
        ("d) 80-90 deg", 80.0, 90.0),
        ("e) > 90 (night)", 90.0, 181.0),
    ] {
        let group: Vec<&Cached> = f2
            .iter()
            .copied()
            .filter(|c| c.midpoint_zenith_deg >= lo && c.midpoint_zenith_deg < hi)
            .collect();
        summarise(label, &group);
    }

    println!(
        "\n  {:<18} {:>6} {:>22} {:>20}",
        "band", "spots", "median absorption [dB]", "median residual [dB]"
    );
    let mut by_band: BTreeMap<String, Vec<&Cached>> = BTreeMap::new();
    for c in &f2 {
        by_band
            .entry(band_label(c.freq_mhz).to_string())
            .or_default()
            .push(c);
    }
    for (band, group) in &by_band {
        summarise(band, group);
    }

    // The corpus works low bands at night and high bands by day, so neither the
    // zenith block nor the band block alone can say which of the two the residual
    // belongs to. Crossing them is the only cut that separates them: a band
    // present on both sides of the terminator answers it within itself, with
    // frequency held fixed.
    println!(
        "\n  {:<18} {:>6} {:>22} {:>20}",
        "band x midpoint", "spots", "median absorption [dB]", "median residual [dB]"
    );
    let mut crossed: BTreeMap<String, Vec<&Cached>> = BTreeMap::new();
    for c in &f2 {
        crossed
            .entry(format!(
                "{:<6} {}",
                band_label(c.freq_mhz),
                if c.midpoint_is_night() {
                    "night"
                } else {
                    "day"
                }
            ))
            .or_default()
            .push(c);
    }
    let mut thin = 0usize;
    for (label, group) in &crossed {
        if group.len() < 6 {
            thin += group.len();
            continue;
        }
        summarise(label, group);
    }
    if thin > 0 {
        println!("  ({thin} spot(s) in cells below the 6-spot floor are not shown)");
    }
    println!("  A band appearing on BOTH sides holds frequency fixed, so the day-night gap");
    println!("  inside one of those pairs is the diurnal effect with the band effect removed.");

    println!("\n  Read the ZENITH block first. If absorption is nearly flat down that column");
    println!("  while the residual is not, the model has no diurnal signal to scale and the");
    println!("  fault is in the absorption profile - not in the scale, and not in the noise");
    println!("  model that the fit has been pushing to its bounds instead.");
}

/// Is every reported path geometrically possible at the height its layer sits at?
///
/// # The geometry
///
/// A ray leaving at zero elevation and turning at height `h` lands at a ground
/// range of `2 R acos(R / (R + h))` - the horizon limit for that reflection
/// height. Every real path launches above zero and therefore falls SHORT of it,
/// so the figure is a hard ceiling per hop and not a target.
///
/// At the E layer's 105 km that ceiling is about 2300 km; at an F2 height of
/// 300 km it is about 3800 km. So a single-hop E path over 2500 km is not a
/// marginal case, it is impossible, and a solver reporting one has bridged the
/// Earth's curvature.
///
/// # Why it would produce exactly the residual being chased
///
/// The daytime residual is a near-constant 10-18 dB that is NOT proportional to
/// the absorption applied - the correction needed is largest where the model
/// absorbs least, which no scale and no `cos^n chi` law can produce. A missing
/// HOP does produce it: each one omitted skips a ground reflection (3-6 dB over
/// land) and two further transits of the D region, and it skips them as a lump
/// rather than in proportion to anything.
///
/// And it is diurnal by construction, which is the part that matters here.
/// Daytime paths reflect from E at ~105 km and night paths from F2 at ~300 km,
/// so for the same ground range the day path needs roughly three times the hops.
/// A solver that under-counts hops therefore under-charges daytime paths
/// specifically, while leaving the night alone.
pub(crate) fn report_hop_geometry(set: &Solved) {
    println!("\n--- IS EVERY HOP GEOMETRICALLY POSSIBLE? -------------------");
    println!("  A ray turning at height h cannot exceed 2 R acos(R/(R+h)) of ground range in");
    println!("  ONE hop, and every real launch angle falls short of that. Below: the range each");
    println!("  reported path covers PER HOP, against that ceiling for its own layer.\n");
    let r = scenario::EARTH_RADIUS_M / 1000.0;
    // Nominal reflection heights. The apex is not carried on the cache, so these
    // are the layers' own anchors - which is conservative for this test, because
    // a real apex sits at or below the peak and the true ceiling is therefore
    // LOWER than the one used here.
    let height_km = |layer: &str| match layer {
        "E" => Anchors::default().ionosphere.e_peak_alt_km.value,
        "Es" => skipzone_app::sporadic_e::ES_HEIGHT_KM,
        _ => Inputs::default().hmf2_km,
    };
    let ceiling = |h: f64| 2.0 * r * (r / (r + h)).acos();

    let mut by_layer: BTreeMap<&str, Vec<&Cached>> = BTreeMap::new();
    for c in &set.spots {
        by_layer.entry(c.layer).or_default().push(c);
    }
    println!(
        "  {:<5} {:>9} {:>10} {:>7} {:>12} {:>14} {:>16}",
        "layer", "height", "max hop", "spots", "median hops", "median km/hop", "OVER THE LIMIT"
    );
    let mut total_over = 0usize;
    for (layer, group) in &by_layer {
        let h = height_km(layer);
        let limit = ceiling(h);
        let mut per_hop: Vec<f64> = Vec::new();
        let mut hops: Vec<f64> = Vec::new();
        let mut over = 0usize;
        for c in group {
            let n = f64::from(c.hops.max(1));
            let each = c.range_km / n;
            per_hop.push(each);
            hops.push(n);
            if each > limit {
                over += 1;
            }
        }
        total_over += over;
        per_hop.sort_by(f64::total_cmp);
        hops.sort_by(f64::total_cmp);
        #[allow(clippy::cast_precision_loss)]
        let pct = 100.0 * over as f64 / group.len().max(1) as f64;
        println!(
            "  {layer:<5} {:>6.0} km {:>7.0} km {:>7} {:>12.1} {:>14.0} {:>11} ({pct:.0}%)",
            h,
            limit,
            group.len(),
            fit::percentile(&hops, 0.5),
            fit::percentile(&per_hop, 0.5),
            over
        );
    }

    // The diurnal half of the claim: day and night should differ in hop count for
    // the same ground range, because they reflect from different heights.
    println!(
        "\n  {:<18} {:>7} {:>12} {:>14} {:>16}",
        "range x midpoint", "spots", "median hops", "median km/hop", "median layer h"
    );
    let mut crossed: BTreeMap<String, Vec<&Cached>> = BTreeMap::new();
    for c in &set.spots {
        let bucket = match c.range_km {
            x if x < 1000.0 => "a) <1000 km",
            x if x < 2000.0 => "b) 1000-2000",
            x if x < 3000.0 => "c) 2000-3000",
            _ => "d) >3000 km",
        };
        crossed
            .entry(format!(
                "{bucket:<13}{}",
                if c.midpoint_is_night() {
                    "night"
                } else {
                    "day"
                }
            ))
            .or_default()
            .push(c);
    }
    for (label, group) in &crossed {
        if group.len() < 6 {
            continue;
        }
        let mut hops: Vec<f64> = group.iter().map(|c| f64::from(c.hops.max(1))).collect();
        let mut each: Vec<f64> = group
            .iter()
            .map(|c| c.range_km / f64::from(c.hops.max(1)))
            .collect();
        let mut hs: Vec<f64> = group.iter().map(|c| height_km(c.layer)).collect();
        hops.sort_by(f64::total_cmp);
        each.sort_by(f64::total_cmp);
        hs.sort_by(f64::total_cmp);
        println!(
            "  {label:<18} {:>7} {:>12.1} {:>14.0} {:>13.0} km",
            group.len(),
            fit::percentile(&hops, 0.5),
            fit::percentile(&each, 0.5),
            fit::percentile(&hs, 0.5)
        );
    }
    println!();
    if total_over > 0 {
        println!("  {total_over} path(s) cover more ground per hop than their reflection height");
        println!("  allows. Those are not marginal geometries, they are impossible ones, and each");
        println!("  omitted hop skips a ground reflection AND two more D-region transits - which");
        println!("  is a lump penalty, exactly the shape of the daytime residual.");
    } else {
        println!("  Every reported path is within its layer's single-hop horizon, so the daytime");
        println!("  deficit is NOT missing hops. That kills the hypothesis rather than supporting");
        println!("  it, and the near-constant daytime term is something else.");
    }
}

/// Settle the station effects, the absorption scale and the global offset at ONE
/// fixed set of noise anchors, exactly as the fit's alternation does, and return
/// the objective there.
///
/// No coordinate descent: the noise anchors are held where the caller put them.
/// That is the point - it lets two candidate noise settings be compared on equal
/// terms, each with its own best absorption scale and its own best effects, which
/// is the only fair way to ask which one the data actually prefers.
pub(crate) fn settle_at(
    spots: &[Cached],
    atm: AtmosphericAnchors,
    n_tx: usize,
    n_rx: usize,
    negatives: fit::Negatives<'_>,
) -> (f64, f64, StationEffects) {
    let prior_scale = CachedParams::prior().absorption_scale;
    let mut scale = 1.0;
    let mut effects = StationEffects::default();
    let mut objective = f64::INFINITY;
    for _ in 0..12 {
        let residuals: Vec<f64> = spots
            .iter()
            .map(|c| c.modelled_db(scale, atm) - c.measured_db)
            .collect();
        effects = StationEffects::solve(&residuals, spots, n_tx, n_rx);
        let (obj, s, g) = fit::profiled_objective(spots, atm, &effects, prior_scale, negatives);
        scale = s.value;
        effects.global_db = g;
        if (objective - obj).abs() < 1e-9 {
            objective = obj;
            break;
        }
        objective = obj;
    }
    (scale, objective, effects)
}

/// Is the fit's answer actually better than leaving the noise model alone?
///
/// # Why this has to be asked separately
///
/// Every bound-hit warning, and the profile table beside it, describes the point
/// the search STOPPED at. A coordinate profile varies one parameter with the
/// others held at that point, so it can only see the local shape: it establishes
/// that the search sits in a genuine minimum, and says nothing whatever about
/// whether a better minimum exists in a diagonal direction. Coordinate descent
/// with a pattern move is good at valleys aligned near the axes and can still be
/// trapped by one that is not.
///
/// That distinction decides what the bound hits mean. If the prior noise anchors,
/// each given their own best absorption scale and their own best station effects,
/// score BETTER than the fitted ones, then the fit walked away from a good answer
/// and the rails are a property of the search rather than of the physics. If they
/// score worse, the pressure is real and the missing term is in the model.
///
/// The comparison is also run without the one-sided term, because the negatives
/// pull hardest on paths carrying almost no absorption and can therefore move the
/// scale for reasons that have nothing to do with the daytime residual.
pub(crate) fn report_local_minimum_check(
    spots: &[Cached],
    n_tx: usize,
    n_rx: usize,
    fitted: &CachedParams,
    negatives: fit::Negatives<'_>,
) {
    println!("\n--- DID THE SEARCH FIND THE BEST ANSWER, OR JUST AN ANSWER? ");
    println!("  Each row settles the absorption scale, the global offset and the station");
    println!("  effects at ONE fixed set of noise anchors. The profile table above can only");
    println!("  see the shape around the point the search stopped at; this asks whether a");
    println!("  DIFFERENT point scores better, which a coordinate profile cannot answer.\n");
    #[allow(clippy::cast_precision_loss)]
    let n = spots.len().max(1) as f64;
    let prior_atm = AtmosphericAnchors::default();

    println!(
        "  {:<34} {:>11} {:>11} {:>11}",
        "noise anchors", "absorption", "RMS [dB]", "offset [dB]"
    );
    let row = |label: &str, atm: AtmosphericAnchors, negs: fit::Negatives<'_>| -> f64 {
        let (scale, obj, effects) = settle_at(spots, atm, n_tx, n_rx, negs);
        let rms = (obj / n).sqrt();
        println!(
            "  {label:<34} {scale:>11.4} {rms:>11.3} {:>11.2}",
            effects.global_db
        );
        rms
    };
    println!("  -- with the one-sided constraint --");
    let prior_rms = row("at the PRIOR anchors", prior_atm, negatives);
    let fitted_rms = row("at the FITTED anchors", fitted.atm, negatives);
    println!("  -- positives only, no constraint --");
    row("at the PRIOR anchors", prior_atm, fit::Negatives::none());
    row("at the FITTED anchors", fitted.atm, fit::Negatives::none());

    println!();
    if prior_rms < fitted_rms {
        println!("  THE PRIOR ANCHORS SCORE BETTER. The coordinate descent walked away from a");
        println!("  better answer, so the bound hits above are a property of the SEARCH and not");
        println!("  of the physics, and none of them is evidence about a missing term. Fix the");
        println!("  optimiser before reading anything else in this report as a finding.");
    } else {
        println!("  The fitted anchors score better, so the search did not simply walk away from");
        println!("  the prior. That does not prove no better minimum exists elsewhere - only a");
        println!("  global search could - but the pressure driving these parameters to their");
        println!("  bounds is not an artefact of leaving a good answer behind.");
    }
}

/// Whether each bound hit is a FINDING or an artefact of a flat objective.
///
/// # Why a bound hit cannot be read from the fitted value alone
///
/// The report warns, on every bound hit, that the data wanting to leave a
/// physical range means the residual comes from somewhere else in the model.
/// That is true of ONE of the two ways a parameter reaches a bound, and
/// [`Bounded::at_bound`] cannot tell them apart, because it sees only where the
/// value landed:
///
/// * the objective genuinely falls all the way to the edge, so the minimum is
///   outside the range. That is the finding the warning describes;
/// * the objective goes FLAT partway across the range, and a coordinate descent
///   keeps taking the last improving step until it runs out of room. The value
///   on the bound then carries no information at all, and the warning is
///   describing evidence that does not exist.
///
/// The second is not hypothetical here. Both night anchors act on the same
/// quantity - how far the atmospheric term sits under the P.372 man-made floor -
/// and once it is buried, moving them further changes the composed noise by
/// nothing. Past that point the objective cannot distinguish any two values.
///
/// So the curve is what gets printed, not the endpoint. `flat %` is the fraction
/// of the parameter's own range over which the objective stays within
/// [`FLAT_TOLERANCE_DB`] of its best: a parameter flat over most of its range is
/// unidentified there, and its landing on an edge is arithmetic, not evidence.
pub(crate) const FLAT_TOLERANCE_DB: f64 = 0.05;

/// How many points each parameter is profiled at across its range.
pub(crate) const PROFILE_STEPS: usize = 41;

pub(crate) fn report_bound_profiles(
    spots: &[Cached],
    effects: &StationEffects,
    fitted: &CachedParams,
    negatives: fit::Negatives<'_>,
) {
    println!("\n--- IS EACH BOUND HIT A FINDING? ---------------------------");
    println!("  The objective along each parameter's OWN range, everything else held at the");
    println!("  fit's answer, as an RMS-equivalent so it reads in dB. A parameter whose");
    println!("  objective falls all the way to an edge has a minimum outside its physical");
    println!("  range - that is the finding the warnings above describe. A parameter whose");
    println!("  objective is FLAT has no minimum to find, and lands on an edge only because a");
    println!("  descent runs out of room: the warning then describes evidence that is not");
    println!(
        "  there. 'flat %' is how much of the range sits within {FLAT_TOLERANCE_DB} dB of the best.\n"
    );
    #[allow(clippy::cast_precision_loss)]
    let n = spots.len().max(1) as f64;
    let rms = |sum_sq: f64| (sum_sq / n).sqrt();
    let prior_scale = CachedParams::prior().absorption_scale;

    println!(
        "  {:<26} {:>9} {:>9} {:>9} {:>9} {:>7}  verdict",
        "parameter", "at min", "at fitted", "at max", "best at", "flat %"
    );

    // Each row: the objective sampled across the range, as RMS dB.
    let row = |name: &str, b: skipzone_app::calib::Bounded, curve: &[(f64, f64)]| {
        let (best_at, best) =
            curve
                .iter()
                .copied()
                .fold((f64::NAN, f64::INFINITY), |acc, (v, o)| {
                    if o < acc.1 { (v, o) } else { acc }
                });
        let flat = curve
            .iter()
            .filter(|(_, o)| *o - best <= FLAT_TOLERANCE_DB)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let flat_pct = 100.0 * flat as f64 / curve.len() as f64;
        let at = |target: f64| {
            curve
                .iter()
                .min_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()))
                .map_or(f64::NAN, |(_, o)| *o)
        };
        // A parameter is only reported as a finding if it BOTH ends on a bound
        // and has a real gradient there to have been pushed by.
        let verdict = if !b.at_bound() {
            "interior optimum"
        } else if flat_pct > 50.0 {
            "FLAT - the bound is arbitrary, not evidence"
        } else if flat_pct > 20.0 {
            "weak - identified over part of the range only"
        } else {
            "REAL - the objective falls to the edge"
        };
        println!(
            "  {:<26} {:>9.3} {:>9.3} {:>9.3} {:>9.4} {:>6.0}%  {verdict}",
            name,
            at(b.min),
            at(b.value),
            at(b.max),
            best_at,
            flat_pct
        );
    };

    for (name, get, set) in CachedParams::fields() {
        let b = get(fitted);
        let curve: Vec<(f64, f64)> = (0..PROFILE_STEPS)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let u = i as f64 / (PROFILE_STEPS - 1) as f64;
                let mut trial = *fitted;
                set(&mut trial, b.at_unit_position(u));
                let (obj, _, _) =
                    fit::profiled_objective(spots, trial.atm, effects, prior_scale, negatives);
                (get(&trial).value, rms(obj))
            })
            .collect();
        row(name, b, &curve);
    }

    // The absorption scale is solved in closed form rather than stepped, so its
    // curve has to be drawn by holding it fixed and re-solving only the offset.
    let b = fitted.absorption_scale;
    let curve: Vec<(f64, f64)> = (0..PROFILE_STEPS)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let u = i as f64 / (PROFILE_STEPS - 1) as f64;
            let v = b.at_unit_position(u).value;
            (
                v,
                rms(fit::objective_at_scale(
                    spots, v, fitted.atm, effects, negatives,
                )),
            )
        })
        .collect();
    row("absorption scale", b, &curve);

    println!("\n  A row reading FLAT is not a smaller finding than a row reading REAL. It is");
    println!("  the ABSENCE of one, and the bound-hit warning printed for it above should be");
    println!("  disregarded: nothing in this corpus distinguishes that parameter's value over");
    println!("  the flat span, so where it stopped is an accident of the search.");
}

/// Can the model tell a path that decoded from one that did not?
///
/// # Why this is the score the rest of the report cannot give
///
/// Every headline here - RMS, slope, R2 - is measured over DECODES only, and a
/// hit rate over decodes is one-sided by construction: a model that predicted
/// every path would score perfectly. The false-positive rate fixes half of that
/// but is read at one threshold, so it confounds the model's ability to SEPARATE
/// the two populations with where the threshold happens to sit inside them.
///
/// The area under the ROC curve separates those. It is the probability that a
/// randomly chosen decode is predicted stronger than a randomly chosen
/// non-decode, taken over every threshold at once, so it is invariant to any
/// monotone shift of the predictions - including the whole unattributable global
/// offset. 0.5 is a coin toss; 1.0 is perfect ordering.
///
/// That invariance is the point. If the separation is poor, then no setting of a
/// level-like parameter can produce a good false-positive rate, and the residual
/// spread rather than the residual bias is what limits the model.
///
/// Both populations are restricted to the same station table, and the station
/// effects are removed from both, so the comparison is of PHYSICS and not of
/// which stations happened to appear on which side.
pub(crate) fn report_skill(
    positives: &[Cached],
    negatives: &[Cached],
    scale: f64,
    atm: AtmosphericAnchors,
    effects: &StationEffects,
) {
    println!("\n--- SKILL: CAN IT SEPARATE DECODES FROM NON-DECODES? -------");
    if positives.len() < 20 || negatives.len() < 20 {
        println!(
            "  Too few on one side to measure: {} decode(s), {} non-decode(s).",
            positives.len(),
            negatives.len()
        );
        return;
    }
    let predicted = |c: &Cached| {
        let station = effects.tx.get(c.tx).copied().unwrap_or(0.0)
            + effects.rx.get(c.rx).copied().unwrap_or(0.0);
        c.modelled_db(scale, atm) - station
    };
    let pos: Vec<f64> = positives.iter().map(predicted).collect();
    let neg: Vec<f64> = negatives.iter().map(predicted).collect();

    // Mann-Whitney U over the pooled ranks, which is the AUC exactly. Ties count
    // a half, as they must: a tie is no evidence either way.
    let mut wins = 0.0;
    for p in &pos {
        for q in &neg {
            if p > q {
                wins += 1.0;
            } else if (p - q).abs() < 1e-12 {
                wins += 0.5;
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let auc = wins / (pos.len() as f64 * neg.len() as f64);

    let median = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        fit::percentile(&s, 0.5)
    };
    println!(
        "  {} decode(s) vs {} non-decode(s), station effects removed from both.",
        pos.len(),
        neg.len()
    );
    println!(
        "  median predicted SNR: {:+.1} dB decoded, {:+.1} dB not decoded, gap {:.1} dB",
        median(&pos),
        median(&neg),
        median(&pos) - median(&neg)
    );
    println!("\n  AREA UNDER THE ROC CURVE: {auc:.3}");
    let reading = if auc < 0.6 {
        "barely better than a coin toss - the model does not order these paths"
    } else if auc < 0.75 {
        "weak - the populations overlap heavily"
    } else if auc < 0.9 {
        "useful separation"
    } else {
        "strong separation"
    };
    println!("  {reading}.");
    println!("\n  This number is invariant to any constant shift of the predictions, so unlike");
    println!("  the false-positive rate it cannot be improved by moving the level, and unlike");
    println!("  the hit rate it cannot be improved by predicting more paths. If it is low, the");
    println!("  limit on this model is the SPREAD of its residual and not the bias, and no");
    println!("  amount of fitting a smooth systematic parameter will move it.");
}

pub(crate) fn report_effects(effects: &StationEffects, set: &Solved, min_spots: usize) {
    let d: EffectDistribution = effects.distribution(min_spots);
    let show = |what: &str, s: &Spread| {
        println!(
            "  {what:<12} n={:<5} median {:+6.1}  IQR {:5.1}  p10 {:+6.1}  p90 {:+6.1}  \
             min {:+6.1}  max {:+6.1}",
            s.n,
            s.median,
            s.iqr(),
            s.p10,
            s.p90,
            s.min,
            s.max
        );
    };
    show("transmitters", &d.tx);
    show("receivers", &d.rx);
    println!("\n  (Sign convention: a POSITIVE effect means the model over-predicts that");
    println!("  station, i.e. the real station is worse than the model assumed.)");

    // Name the tails, since "a spread of 30 dB" is far less useful than knowing
    // which stations are at the ends of it.
    let extremes = |names: &[String], eff: &[f64], counts: &[usize], label: &str| {
        let mut v: Vec<(&str, f64)> = names
            .iter()
            .enumerate()
            .filter(|(i, _)| counts[*i] >= min_spots)
            .map(|(i, n)| (n.as_str(), eff[i]))
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        let fmt = |v: &[(&str, f64)]| {
            v.iter()
                .map(|(n, e)| format!("{n} {e:+.0}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if v.len() >= 6 {
            println!("  best {label:<13} {}", fmt(&v[..3]));
            println!("  worst {label:<12} {}", fmt(&v[v.len() - 3..]));
        }
    };
    extremes(
        &set.tx_names,
        &effects.tx,
        &effects.tx_counts,
        "transmitters",
    );
    extremes(&set.rx_names, &effects.rx, &effects.rx_counts, "receivers");
}

pub(crate) fn print_identification_table() {
    println!("\n=== WHAT THIS CORPUS CAN AND CANNOT IDENTIFY ===============");
    println!("  Any quantity that is CONSTANT for a given station is absorbed exactly into");
    println!("  that station's fixed effect. That is not a limitation of the method - it is");
    println!("  what WSPR contains.\n");
    println!("  {:<40} identifiable here?", "quantity");
    for (q, a) in [
        (
            "absorption magnitude",
            "YES - from its variation, not its mean",
        ),
        (
            "absorption vs frequency",
            "YES - cross-band within a station",
        ),
        (
            "absorption vs zenith angle",
            "YES - diurnal within a station",
        ),
        (
            "atmospheric noise day-night difference",
            "YES - diurnal within a station",
        ),
        (
            "atmospheric noise frequency slopes",
            "YES - cross-band within a station",
        ),
        (
            "atmospheric noise absolute level",
            "NO - a constant, absorbed",
        ),
        ("receiver noise environment", "NO - constant per receiver"),
        (
            "noise model latitude terms",
            "NO - a station's latitude is fixed",
        ),
        ("seasonal swing", "NO - one month of corpus"),
        (
            "absolute antenna gain, either end",
            "NO - constant per station",
        ),
        (
            "claimed transmit power accuracy",
            "NO - constant per station",
        ),
    ] {
        println!("  {q:<40} {a}");
    }
    println!("\n  So the calibrated quantities are SHAPES, not levels. Anything claiming to");
    println!("  have calibrated an absolute level from WSPR has fitted the station");
    println!("  population and called it physics.");
}
