//! The calibration driver. Fits the anchors, runs the scans, and calls into
//! [`super::report`] for everything it prints.

use crate::args::*;
use crate::jackknife::*;
use crate::negatives::*;
use crate::report::*;
use crate::solving::*;

use skipzone_app::antenna::AntennaConfig;
use skipzone_app::calib::{Anchors, AtmosphericAnchors};
use skipzone_app::compute::{ComputePool, PoolConfig};
use skipzone_app::corpus::{self, CorpusSpot, MIN_SPOTS_PER_STATION, Negative};
use skipzone_app::fit::{self, Cached, CachedParams, Fit, StationEffects};
use skipzone_app::scenario::Inputs;
use skipzone_app::wspr::WSPR_DECODE_THRESHOLD_DB;
use skipzone_app::wspr_report::band_label;
#[allow(clippy::too_many_lines)] // one report, printed top to bottom
pub(crate) fn run(args: &Args) -> Result<(), String> {
    println!("=== CALIBRATING SKIPZONE AGAINST MEASURED WSPR SPOTS ===\n");
    let antenna = AntennaConfig {
        kind: args.antenna,
        height_m: args.antenna_height_m,
        ..AntennaConfig::default()
    };
    let base = Inputs {
        noise_env: args.noise_env,
        tx_antenna: antenna,
        rx_antenna: antenna,
        // Explicitly ZERO, against the shipped default. This binary measures the
        // UNCORRECTED model: the global offset it reports IS the bias, so leaving
        // the shipped correction in would fit an offset on top of an offset. It
        // would also break `fit::Cached`'s exact reconstruction, which rebuilds the
        // SNR arithmetically and carries no bias term - the CACHE CHECK section
        // would start reporting a constant disagreement.
        model_bias_db: 0.0,
        ..Inputs::default()
    };
    println!(
        "ASSUMED  receiver noise environment: {} (an ASSUMPTION; see the identification",
        args.noise_env.label()
    );
    println!("         table below for why WSPR cannot check it)");
    print!("ASSUMED  antenna at BOTH ends: {}", args.antenna.label());
    if args.antenna.uses_height() {
        println!(" at {:.0} m", args.antenna_height_m);
        // Height in wavelengths at the two ends of the WSPR band set, so the
        // span is stated rather than asserted. lambda = c / f.
        let waves = |f_mhz: f64| args.antenna_height_m * f_mhz * 1e6 / 299_792_458.0;
        println!("         A fixed height in METRES is a fixed height in WAVELENGTHS only on one");
        println!(
            "         band. This one spans {:.2} wavelengths on 160 m to {:.2} on 10 m, and its",
            waves(1.8366),
            waves(28.1246)
        );
        println!("         gain at a 5 deg launch angle therefore climbs by over 10 dB per end");
        println!("         across that span - a band-shaped term the station effects CANNOT");
        println!("         absorb, because it is not constant for a station. Every physics");
        println!("         parameter below is then partly fitting this assumption.");
    } else {
        println!();
        println!("         Flat by choice. Absolute gain is constant per station and is absorbed");
        println!("         exactly into that station's effect, so a flat reference discards");
        println!("         nothing this corpus could identify - while a fixed-height wire would");
        println!("         impose a band tilt of its own that the station effects cannot absorb.");
        println!("         `--antenna dipole` restores the GUI's assumption.");
    }

    let fit_corpus = load_capped(&args.fit, args.max_spots)?;
    let pool = ComputePool::new(PoolConfig::default()).map_err(|e| e.to_string())?;
    let mut fit_set = main_solve("fit", &fit_corpus, &base, &pool);
    describe_span("FIT     ", &args.fit, &fit_corpus);
    if !args.include_es {
        fit_set = drop_es(fit_set);
        println!(
            "\nEXCLUDED {} of {} solved fit spot(s) that only the sporadic-E fallback reached.",
            fit_set.es_excluded,
            fit_set.es_excluded + fit_set.spots.len()
        );
        println!("         A DIAGNOSTIC: Es answers only where nothing deterministic closed, so");
        println!("         those spots measure the fallback rather than the ionosphere. This run");
        println!("         localises the residual and is NOT a calibration to quote.");
    }

    let holdout_set = match &args.holdout {
        Some(path) => {
            let c = load_capped(path, args.max_spots)?;
            describe_span("HOLDOUT ", path, &c);
            let h = main_solve("holdout", &c, &base, &pool);
            Some(if args.include_es { h } else { drop_es(h) })
        }
        None => {
            println!("\nHOLDOUT  none given. Every number below is an IN-SAMPLE number and");
            println!("         must not be quoted as model performance.");
            None
        }
    };

    if fit_set.spots.len() < 50 {
        return Err(format!(
            "only {} spot(s) in the fit corpus closed a path; that is too few to fit \
             anything. Build a bigger corpus with wspr_corpus.",
            fit_set.spots.len()
        ));
    }

    // -------------------------------------------------- reconstruction check
    // The cached re-derivation must reproduce the solver bit for bit at the prior,
    // or every fitted number is measured against a slightly different model than
    // the one that will be shipped.
    let prior = CachedParams::prior();
    println!("\n--- CACHE CHECK --------------------------------------------");
    println!("  The fit re-derives each spot's SNR arithmetically instead of re-tracing.");
    println!("  At the prior that must reproduce the solver exactly:");
    let worst = fit_set
        .spots
        .iter()
        .map(|c| {
            (c.modelled_db(1.0, AtmosphericAnchors::default()) - c.modelled_db(1.0, prior.atm))
                .abs()
        })
        .fold(0.0_f64, f64::max);
    println!(
        "    largest disagreement over {} spots: {:.2e} dB",
        fit_set.spots.len(),
        worst
    );

    // ------------------------------------------------- negatives, solved EARLY
    // They are solved before the fit rather than after it because they are part
    // of the objective now, not a score applied to its answer.
    let negatives_raw = match &args.negatives {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let (negatives, problems) = corpus::read_negatives(&text);
            if !problems.is_empty() {
                println!("\n{} unreadable negative row(s):", problems.len());
                for p in problems.iter().take(5) {
                    println!("   {p}");
                }
            }
            // COMMENSURABILITY FILTER, applied before the thinning.
            //
            // A negative enters the objective as a claim about an absolute SNR,
            // and that SNR depends on the two stations' own effects. A negative
            // from a station the fit has never seen has to be judged at the
            // population mean of zero - and that is not a neutral choice here,
            // because the two populations are not the same population. The fit
            // corpus is deliberately the well-equipped core of the network,
            // restricted to its most active few dozen stations; a cycle census
            // sweeps in every station that transmitted, most of them weaker.
            //
            // Measured on this corpus: 53 754 negatives cover 260 transmitters
            // and 518 receivers against the fit's 79 and 30, and only 2.5 % have
            // BOTH stations in the fit's table. Constraining the physics with the
            // other 97.5 % judged at the core's mean does not measure the model's
            // bias, it measures the gap between the core and the whole network -
            // and the fit then removes that gap from the physics, which is the
            // exact failure this module's design exists to prevent.
            //
            // So the constraint is restricted to negatives the fit can price.
            // That narrows what the false-positive rate is ABOUT - core stations
            // rather than everyone - which is a smaller claim and a sound one.
            let known_tx: std::collections::BTreeSet<&str> =
                fit_set.tx_names.iter().map(String::as_str).collect();
            let known_rx: std::collections::BTreeSet<&str> =
                fit_set.rx_names.iter().map(String::as_str).collect();
            let commensurable: Vec<Negative> = negatives
                .iter()
                .filter(|n| {
                    known_tx.contains(n.tx_call.as_str()) && known_rx.contains(n.rx_call.as_str())
                })
                .cloned()
                .collect();
            println!(
                "\nNEGATIVES {} of {} have BOTH stations in the fit's table and are usable as",
                commensurable.len(),
                negatives.len()
            );
            println!("         constraints; the rest would be judged at a mean belonging to a");
            println!("         different station population. Only the usable ones are solved.");
            // Each costs a full solve, so thin by a fixed stride rather than
            // taking a prefix: the file is ordered by cycle, and a prefix would
            // be one cycle on one band.
            let stride = (commensurable.len() / args.max_negatives.max(1)).max(1);
            let sample: Vec<Negative> = commensurable.iter().step_by(stride).cloned().collect();
            Some((sample, commensurable.len(), stride))
        }
        None => None,
    };
    let negative_spots: Vec<Cached> = match &negatives_raw {
        Some((sample, _, _)) => solve_negatives(sample, &base, &pool, &fit_set),
        None => Vec::new(),
    };

    // -------------------------------------------------------------- baseline
    let baseline = score_set(
        &fit_set,
        1.0,
        AtmosphericAnchors::default(),
        args.trim_tails,
    );
    println!("\n=== BASELINE (the model as it stands) ======================");
    report_fit(&baseline.fit, &fit_set);
    report_layers(
        &fit_set,
        1.0,
        AtmosphericAnchors::default(),
        &baseline.effects,
    );

    // ------------------------------------------------------------------- fit
    println!("\n=== FITTING ================================================");
    println!("  Alternating: station effects in closed form, then the physics by");
    println!("  coordinate descent with a pattern move, the absorption scale and the");
    println!("  unattributable global offset solved jointly at every trial point.");
    let constraint = fit::Negatives::balanced(
        &negative_spots,
        WSPR_DECODE_THRESHOLD_DB,
        fit_set.spots.len(),
    );
    if negative_spots.is_empty() {
        println!("\n  NO ONE-SIDED CONSTRAINT: without --negatives the objective is invariant to");
        println!("  the LEVEL of the prediction - the global offset absorbs any constant shift");
        println!("  at no cost - so the fit may close a residual by sliding the model optimistic");
        println!("  and pay nothing for it. Pass --negatives to remove that freedom.");
    } else {
        println!(
            "\n  ONE-SIDED CONSTRAINT: {} solved non-decode(s) enter the objective as",
            negative_spots.len()
        );
        println!(
            "  hinges about the WSPR decode threshold, weighted {:.3} each so their total",
            constraint.weight
        );
        println!("  weight equals the positives'. A non-decode is the only thing in this corpus");
        println!("  that constrains an ABSOLUTE level, because no constant shift can satisfy it.");
    }
    let (fitted, after_fit_effects, notes) = fit::fit_cached(
        &fit_set.spots,
        fit_set.tx_names.len(),
        fit_set.rx_names.len(),
        args.rounds,
        constraint,
    );
    for n in &notes {
        println!("  ! {n}");
    }

    println!("\n--- PARAMETERS ---------------------------------------------");
    println!(
        "  {:<28} {:>12} {:>12} {:>12} {:>7}",
        "parameter", "prior", "fitted", "range", "bound?"
    );
    let scale_prior = prior.absorption_scale;
    let nu_prior = Anchors::default().ionosphere.nu_ref_per_s;
    println!(
        "  {:<28} {:>12.4} {:>12.4} {:>5.2}..{:<5.2} {:>7}",
        "absorption scale",
        scale_prior.value,
        fitted.absorption_scale.value,
        fitted.absorption_scale.min,
        fitted.absorption_scale.max,
        if fitted.absorption_scale.at_bound() {
            "YES"
        } else {
            "no"
        }
    );
    println!(
        "  {:<28} {:>12.2e} {:>12.2e} {:>5.0e}..{:<5.0e} {:>7}",
        "  = nu at 70 km [1/s]",
        nu_prior.value,
        nu_prior.value * fitted.absorption_scale.value,
        nu_prior.min,
        nu_prior.max,
        ""
    );
    for (name, get, _) in CachedParams::fields() {
        let now = get(&fitted);
        let was = get(&prior);
        println!(
            "  {:<28} {:>12.2} {:>12.2} {:>5.0}..{:<5.0} {:>7}",
            name,
            was.value,
            now.value,
            now.min,
            now.max,
            if now.at_bound() { "YES" } else { "no" }
        );
    }
    report_noise_leverage(&fit_set, prior.atm, fitted.atm);
    report_bound_profiles(&fit_set.spots, &after_fit_effects, &fitted, constraint);
    report_local_minimum_check(
        &fit_set.spots,
        fit_set.tx_names.len(),
        fit_set.rx_names.len(),
        &fitted,
        constraint,
    );

    println!("\n  Confidence intervals, by leave-one-day-out refits of the same corpus:");
    let ci = day_jackknife(&fit_set, args.rounds, &negative_spots);
    println!(
        "    absorption scale   {:.3} to {:.3}  (spread over {} day-deleted refits)",
        ci.min, ci.max, ci.n
    );
    println!("    That spread is the parameter's sensitivity to WHICH DAYS are in the corpus,");
    println!("    which is the uncertainty that matters here - far larger than the");
    println!("    within-day sampling error a textbook standard error would report.");

    // ------------------------------------------------------------- fit vs holdout
    let after_fit = score_set(
        &fit_set,
        fitted.absorption_scale.value,
        fitted.atm,
        args.trim_tails,
    );
    println!("\n=== RESULT: FIT SET vs HOLD-OUT ============================");
    println!(
        "  {:<34} {:>12} {:>12} {:>12}",
        "", "fit before", "fit after", "hold-out after"
    );
    let holdout_after = holdout_set.as_ref().map(|h| {
        score_set(
            h,
            fitted.absorption_scale.value,
            fitted.atm,
            args.trim_tails,
        )
    });
    let hv = |f: fn(&Fit) -> f64| -> String {
        holdout_after
            .as_ref()
            .map_or_else(|| "     -".to_string(), |h| format!("{:12.2}", f(&h.fit)))
    };
    let row = |name: &str, f: fn(&Fit) -> f64| {
        println!(
            "  {:<34} {:>12.2} {:>12.2} {}",
            name,
            f(&baseline.fit),
            f(&after_fit.fit),
            hv(f)
        );
    };
    println!("  -- THE HEADLINE: does the model track reality? --");
    row("slope vs measured (raw)", |f| f.slope_raw);
    row("R2 vs measured (raw)", |f| f.r2_raw);
    row("slope, station effects removed", |f| f.slope_adjusted);
    row("R2, station effects removed", |f| f.r2_adjusted);
    println!("  -- bias and spread --");
    row("median error [dB]", |f| f.residual.median);
    row("IQR of error [dB]", |f| f.residual.iqr());
    row("median, effects removed [dB]", |f| {
        f.adjusted_residual.median
    });
    row("IQR, effects removed [dB]", |f| f.adjusted_residual.iqr());
    row("RMS, effects removed [dB]", |f| f.rms_db);

    if let (Some(h), Some(hs)) = (&holdout_after, &holdout_set) {
        let gap = h.fit.rms_db - after_fit.fit.rms_db;
        println!("\n  Fit-to-hold-out gap in RMS: {gap:+.2} dB.");
        if gap.abs() > 2.0 {
            println!("  That is a LARGE gap: the fitted values do not transfer, so the honest");
            println!("  number to quote is the hold-out column, not the fit column.");
        } else {
            println!("  Small, so the fitted values transfer to days the fit never saw.");
        }
        println!(
            "  Hold-out set: {} spots, {} stations at TX, {} at RX; effects re-estimated",
            hs.spots.len(),
            hs.tx_names.len(),
            hs.rx_names.len()
        );
        println!("  on the hold-out itself, since a station effect is a property of a station");
        println!("  and not one of the physics being tested.");
    }

    // ------------------------------------------------------ per-band, per-cut
    println!("\n--- PER BAND (after the fit) -------------------------------");
    print_cuts(&fit_set, &after_fit, &baseline, |c| {
        band_label(c.freq_mhz).to_string()
    });
    println!("\n--- PER PATH LENGTH (after the fit) ------------------------");
    print_cuts(&fit_set, &after_fit, &baseline, |c| {
        match c.range_km {
            r if r < 1000.0 => "a) < 1000 km",
            r if r < 2500.0 => "b) 1000-2500 km",
            r if r < 5000.0 => "c) 2500-5000 km",
            r if r < 10000.0 => "d) 5000-10000 km",
            _ => "e) > 10000 km",
        }
        .to_string()
    });
    println!("\n--- PER LAYER (after the fit) ------------------------------");
    print_cuts(&fit_set, &after_fit, &baseline, |c| c.layer.to_string());
    println!("\n--- PER BAND x LAYER (after the fit) -----------------------");
    println!("  Which layer carries each band, and what it costs there. A band whose bias");
    println!("  belongs to ONE layer is not a frequency-dependent error in the physics, it is");
    println!("  that layer showing up wherever it is geometrically available.");
    print_cuts_min(&fit_set, &after_fit, &baseline, 8, |c| {
        format!("{:<6} {}", band_label(c.freq_mhz), c.layer)
    });
    println!("\n--- PER LAYER x MIDPOINT DAY/NIGHT (after the fit) ---------");
    println!("  Split at the MIDPOINT terminator, not the receiver's: the reflection happens");
    println!("  at the midpoint. A layer that misbehaves on only one side of this is a layer");
    println!("  whose ionisation is wrong, not one whose loss terms are.");
    print_cuts_min(&fit_set, &after_fit, &baseline, 8, |c| {
        format!(
            "{:<3} {}",
            c.layer,
            if c.midpoint_is_night() {
                "night"
            } else {
                "day"
            }
        )
    });
    report_layer_races(&fit_set);
    report_hop_geometry(&fit_set);
    // A PROPAGATION term lives at the midpoint, where the reflection happens. A
    // NOISE term lives at the receiver, which is where the floor is heard. On a
    // long path those two disagree about whether it is daytime, and the cells
    // where they disagree are the only place the two hypotheses make different
    // predictions. P.372 Table 1 has no diurnal term at all, so the model's
    // man-made noise is identical at local noon and local midnight - if the
    // residual tracks the RECEIVER's terminator rather than the midpoint's, that
    // missing swing is the daytime bias and no loss term is involved.
    println!("\n--- RECEIVER TERMINATOR vs MIDPOINT TERMINATOR -------------");
    println!("  A propagation loss belongs to the MIDPOINT; a noise floor belongs to the");
    println!("  RECEIVER. The two disagree on long paths, and those cells discriminate: if the");
    println!("  residual follows the receiver's own day/night, the missing term is noise.");
    print_cuts_min(&fit_set, &after_fit, &baseline, 8, |c| {
        format!(
            "mid {:<5} rx {}",
            if c.midpoint_is_night() {
                "night"
            } else {
                "day"
            },
            if c.rx_is_day { "day" } else { "night" }
        )
    });
    report_terminator_step(
        &fit_set,
        &baseline.effects,
        AtmosphericAnchors::default(),
        Some((&after_fit.effects, after_fit.atm, after_fit.scale)),
    );
    report_confound_census(&fit_set, &baseline.effects);
    report_absorption_range(&fit_set);

    // ------------------------------------------------ station-effect distribution
    println!("\n=== THE STATION-EFFECT DISTRIBUTION ========================");
    println!("  A measurement of the WSPR station population in its own right: how much");
    println!("  better or worse each station is than the model's assumed antenna and");
    println!(
        "  noise floor. Only stations with at least {MIN_SPOTS_PER_STATION} spots are shown -"
    );
    println!("  fewer than that and the 'effect' is just that station's own residual.\n");
    if after_fit.trimmed > 0 {
        println!(
            "  TRIMMED: {} spot(s) belonging to the extreme 1 % of station effects at each",
            after_fit.trimmed
        );
        println!("  end were dropped before this fit. Those are real stations - a contest");
        println!("  station on a mountain, a broken install - but they are not representative,");
        println!("  and a handful of them moves a median. --keep-tails keeps them.\n");
    }
    report_effects(&after_fit.effects, &fit_set, MIN_SPOTS_PER_STATION);
    println!(
        "\n  The GLOBAL offset the fit could not attribute: {:+.2} dB.",
        after_fit.effects.global_db
    );
    println!("  This is the part of the bias WSPR genuinely cannot assign. It could be");
    println!("  the model being optimistic, or the station population being worse than");
    println!("  the assumed antennas, and no amount of this data separates the two.");
    println!(
        "\n  UNATTRIBUTABLE IS NOT UNUSABLE: set Inputs::model_bias_db = {:.2} to ship it.",
        after_fit.effects.global_db
    );
    println!("  Which of the two causes it is does not change the correction. The per-station");
    println!("  effects are centred on zero, so for a station the app knows nothing about -");
    println!("  which is every station a user asks about - this offset IS the best estimate of");
    println!("  the model's own bias. Predicting without it means holding a measured error and");
    println!("  declining to apply it, which is what made the shipped false-positive rate so");
    println!("  much worse than the level-matched one below.");

    // ---------------------------------------------------------------- negatives
    if let Some((sample, available, stride)) = &negatives_raw {
        if sample.is_empty() {
            println!("\nNEGATIVES  file held none; the false-positive rate cannot be measured.");
        } else {
            report_negatives(
                sample,
                *available,
                *stride,
                &negative_spots,
                args,
                Levels {
                    before: baseline.effects.global_db,
                    after: after_fit.effects.global_db,
                },
                &fitted,
            );
            report_decode_probability(&fit_set, &after_fit, &negative_spots, &fitted);
        }
    } else {
        println!("\n=== FALSE POSITIVES ========================================");
        println!("  NOT MEASURED: no --negatives file was given. Without one the hit rate");
        println!("  below is one-sided and is NOT a skill score - a model that predicted");
        println!("  every path would score 100 %.");
    }

    if !negative_spots.is_empty() {
        report_skill(
            &fit_set.spots,
            &negative_spots,
            fitted.absorption_scale.value,
            fitted.atm,
            &after_fit.effects,
        );
    }

    // Add the excluded spots back: the Es exclusion drops spots the model DID find
    // a path for, so counting them as misses would misreport the hit rate.
    let fit_found = fit_set.spots.len() + fit_set.es_excluded;
    println!(
        "\n  Model found a path for {} of {} fit spots ({:.0} %), {} of {} hold-out spots.",
        fit_found,
        fit_set.total,
        100.0 * fit_found as f64 / fit_set.total.max(1) as f64,
        holdout_set
            .as_ref()
            .map_or(0, |h| h.spots.len() + h.es_excluded),
        holdout_set.as_ref().map_or(0, |h| h.total),
    );

    if args.scan > 0 {
        scan_resolve_anchors(&fit_corpus, &base, &pool, args);
    } else {
        println!("\n=== ANCHORS THAT NEED A RE-SOLVE ===========================");
        println!("  NOT SCANNED: pass --scan N to vary the D-region geometry, the collision");
        println!("  profile, foE, the E-layer geometry and the Es anchors, each of which");
        println!("  changes which paths exist and so cannot be re-derived from the cache.");
    }

    print_identification_table();
    Ok(())
}

/// Scan the anchors that a cached fit cannot reach, one at a time.
///
/// # Why these are scanned rather than optimised
///
/// The cached fit works because absorption is linear in the collision frequency
/// and because the D region does not bend the ray. Neither holds for the anchors
/// here. Moving the E-layer geometry, foE or foEs changes WHICH GEOMETRIES
/// CLOSE, meaning a path appears or disappears, and that is a step change in
/// the objective, not something a least-squares gradient can follow. Moving
/// the D-region or
/// collision profile does not change the ray, but it changes the SHAPE of
/// absorption against frequency and zenith angle, and the shape is exactly what
/// the absorption scale cannot absorb.
///
/// So each is set to a few values across its plausible range, the corpus is
/// re-solved, and the absorption scale is refitted underneath each one. What
/// improves is then the shape rather than the level, which is the only part a
/// scan of this kind can honestly claim.
///
/// A subsample is used because a re-solve costs a few hundred milliseconds per
/// spot: the point is to see which direction the data prefers, not to resolve the
/// fourth decimal of a parameter the corpus can barely see.
#[allow(clippy::too_many_lines)]
pub(crate) fn scan_resolve_anchors(
    corpus_spots: &[CorpusSpot],
    base: &Inputs,
    pool: &ComputePool,
    args: &Args,
) {
    println!("\n=== ANCHORS THAT NEED A RE-SOLVE ===========================");
    // A deterministic subsample: every Nth spot in corpus order, which is already
    // a hash-ordered sample, so this is a uniform thinning rather than a slice of
    // one day.
    let stride = (corpus_spots.len() / args.scan.max(1)).max(1);
    let sample: Vec<CorpusSpot> = corpus_spots.iter().step_by(stride).cloned().collect();
    println!(
        "  {} spot(s) subsampled from {} (every {}th), re-solved at each value.",
        sample.len(),
        corpus_spots.len(),
        stride
    );
    println!("  The absorption scale is REFITTED under every setting, so what these");
    println!("  columns compare is the SHAPE of the model, not its level.\n");

    let prior = Anchors::default().ionosphere;
    // (name, values to try, how to apply)
    #[allow(clippy::type_complexity)]
    let sweeps: Vec<(&str, Vec<f64>, fn(&mut Anchors, f64))> = vec![
        (
            "D peak altitude [km]",
            vec![80.0, prior.d_peak_alt_km.value, 90.0],
            |a, v| a.ionosphere.d_peak_alt_km.value = v,
        ),
        (
            "D scale height [km]",
            vec![4.0, prior.d_scale_height_km.value, 10.0],
            |a, v| a.ionosphere.d_scale_height_km.value = v,
        ),
        (
            "nu reference alt [km]",
            vec![65.0, prior.nu_ref_alt_km.value, 80.0],
            |a, v| a.ionosphere.nu_ref_alt_km.value = v,
        ),
        (
            "nu scale height [km]",
            vec![5.0, prior.nu_scale_height_km.value, 9.0],
            |a, v| a.ionosphere.nu_scale_height_km.value = v,
        ),
        (
            "E peak altitude [km]",
            vec![100.0, prior.e_peak_alt_km.value, 115.0],
            |a, v| a.ionosphere.e_peak_alt_km.value = v,
        ),
        (
            "E scale height [km]",
            vec![5.0, prior.e_scale_height_km.value, 15.0],
            |a, v| a.ionosphere.e_scale_height_km.value = v,
        ),
        (
            "foE overhead quiet [MHz]",
            vec![3.0, prior.foe_overhead_quiet_mhz.value, 3.8],
            |a, v| a.ionosphere.foe_overhead_quiet_mhz.value = v,
        ),
        (
            "foEs at occurrence max [MHz]",
            vec![5.0, 7.0, prior.es_foes_max_mhz.value, 12.0],
            |a, v| a.ionosphere.es_foes_max_mhz.value = v,
        ),
        (
            "Es peak probability",
            vec![0.15, 0.30, prior.es_peak_probability.value],
            |a, v| a.ionosphere.es_peak_probability.value = v,
        ),
    ];

    println!(
        "  {:<30} {:>8} {:>7} {:>9} {:>8} {:>8} {:>7}",
        "anchor = value", "hits", "hit %", "RMS dB*", "median", "IQR", "slope*"
    );
    for (name, values, apply) in sweeps {
        for v in values {
            let mut anchors = Anchors::default();
            apply(&mut anchors, v);
            let trial_base = Inputs {
                anchors,
                ..base.clone()
            };
            let solved = main_solve("scan", &sample, &trial_base, pool);
            if solved.spots.len() < 30 {
                println!(
                    "  {:<30} too few paths closed to score",
                    format!("{name} = {v}")
                );
                continue;
            }
            let (p, _e, _n) = fit::fit_cached(
                &solved.spots,
                solved.tx_names.len(),
                solved.rx_names.len(),
                args.rounds,
                // The scan re-solves the corpus at each anchor value; the
                // negatives are not re-solved with it, so constraining these
                // fits with negatives from a DIFFERENT ionosphere would be
                // comparing each row against the wrong evidence.
                fit::Negatives::none(),
            );
            let scored = score_set(&solved, p.absorption_scale.value, p.atm, args.trim_tails);
            let marker = if (v - prior_value_for(name, &prior)).abs() < 1e-9 {
                " <- prior"
            } else {
                ""
            };
            println!(
                "  {:<30} {:>8} {:>6.0}% {:>9.2} {:>+8.1} {:>8.1} {:>7.2}{marker}",
                format!("{name} = {v}"),
                solved.spots.len(),
                100.0 * solved.spots.len() as f64 / solved.total.max(1) as f64,
                scored.fit.rms_db,
                scored.fit.residual.median,
                scored.fit.residual.iqr(),
                scored.fit.slope_adjusted,
            );
        }
        println!();
    }
    println!("  * RMS and slope are with station effects removed and the absorption scale");
    println!("    refitted, so a row that only shifts the level cannot look like an");
    println!("    improvement. A value that does not beat the prior on RMS is not evidence");
    println!("    for changing the anchor, whatever it does to the hit rate.");
}

/// The prior value of a scanned anchor, by the name the sweep table uses.
pub(crate) fn prior_value_for(name: &str, prior: &skipzone_app::calib::IonosphereAnchors) -> f64 {
    match name {
        "D peak altitude [km]" => prior.d_peak_alt_km.value,
        "D scale height [km]" => prior.d_scale_height_km.value,
        "nu reference alt [km]" => prior.nu_ref_alt_km.value,
        "nu scale height [km]" => prior.nu_scale_height_km.value,
        "E peak altitude [km]" => prior.e_peak_alt_km.value,
        "E scale height [km]" => prior.e_scale_height_km.value,
        "foE overhead quiet [MHz]" => prior.foe_overhead_quiet_mhz.value,
        "foEs at occurrence max [MHz]" => prior.es_foes_max_mhz.value,
        "Es peak probability" => prior.es_peak_probability.value,
        _ => f64::NAN,
    }
}

/// One scored set: the station effects for it and the resulting fit.
pub(crate) struct Scored {
    pub(crate) effects: StationEffects,
    pub(crate) fit: Fit,
    /// How many spots the tail trim removed.
    pub(crate) trimmed: usize,
    /// The parameters this set was scored under, carried so a later breakdown
    /// cannot accidentally re-score it under different ones.
    pub(crate) scale: f64,
    pub(crate) atm: AtmosphericAnchors,
}

/// Solve the station effects for a set under given physics, then score it.
///
/// `trim_tails` drops the extreme 1 % of station effects at each end and refits.
/// Those are real stations - a contest station on a mountain, a broken install -
/// but they are not representative, and a handful of them can move a median. What
/// was dropped is reported.
pub(crate) fn score_set(
    set: &Solved,
    scale: f64,
    atm: AtmosphericAnchors,
    trim_tails: bool,
) -> Scored {
    let solve_effects = |spots: &[Cached]| -> StationEffects {
        let residuals: Vec<f64> = spots
            .iter()
            .map(|c| c.modelled_db(scale, atm) - c.measured_db)
            .collect();
        StationEffects::solve(&residuals, spots, set.tx_names.len(), set.rx_names.len())
    };
    let first = solve_effects(&set.spots);
    if !trim_tails {
        return Scored {
            fit: Fit::of(&set.spots, scale, atm, &first),
            effects: first,
            trimmed: 0,
            scale,
            atm,
        };
    }
    // Which stations sit in the extreme 1 % at either end?
    let extremes = |effects: &[f64], counts: &[usize]| -> Vec<usize> {
        let mut seen: Vec<(usize, f64)> = effects
            .iter()
            .enumerate()
            .filter(|(i, _)| counts[*i] >= MIN_SPOTS_PER_STATION)
            .map(|(i, e)| (i, *e))
            .collect();
        seen.sort_by(|a, b| a.1.total_cmp(&b.1));
        let cut = (seen.len() / 100).max(if seen.len() >= 50 { 1 } else { 0 });
        let mut out: Vec<usize> = seen.iter().take(cut).map(|(i, _)| *i).collect();
        out.extend(seen.iter().rev().take(cut).map(|(i, _)| *i));
        out
    };
    let bad_tx = extremes(&first.tx, &first.tx_counts);
    let bad_rx = extremes(&first.rx, &first.rx_counts);
    let kept: Vec<Cached> = set
        .spots
        .iter()
        .filter(|c| !bad_tx.contains(&c.tx) && !bad_rx.contains(&c.rx))
        .cloned()
        .collect();
    let trimmed = set.spots.len() - kept.len();
    let effects = {
        let residuals: Vec<f64> = kept
            .iter()
            .map(|c| c.modelled_db(scale, atm) - c.measured_db)
            .collect();
        StationEffects::solve(&residuals, &kept, set.tx_names.len(), set.rx_names.len())
    };
    Scored {
        fit: Fit::of(&kept, scale, atm, &effects),
        effects,
        trimmed,
        scale,
        atm,
    }
}
