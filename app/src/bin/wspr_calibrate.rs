//! Calibrate the model's unverified anchors against a saved WSPR corpus, with
//! transmitter and receiver effects treated as nuisance parameters.
//!
//! ```text
//! cargo run --release -p skipzone-app --bin wspr_calibrate -- \
//!     --fit corpus/fit.tsv --holdout corpus/holdout.tsv --negatives corpus/fit_neg.tsv
//! ```
//!
//! Read `skipzone_app::fit`'s module documentation before believing any number
//! this prints. In particular: absolute levels - absolute antenna gain, absolute
//! noise floor - are UNIDENTIFIABLE from WSPR by construction, because they are
//! constant per station and are absorbed into that station's effect. What is
//! identifiable is how the signal VARIES with frequency, path length, zenith
//! angle, hop count and layer, and that is what is calibrated here.
//!
//! The hold-out is separated by DAY and, separately, by REGION. A random row
//! split would not be a hold-out at all: adjacent spots share an ionosphere, so
//! a model fitted on half of a cycle predicts the other half of the same cycle
//! for reasons that have nothing to do with generalisation.

use std::collections::BTreeMap;
use std::process::ExitCode;

use skipzone_app::calib::{Anchors, AtmosphericAnchors};
use skipzone_app::compute::{ComputePool, PoolConfig};
use skipzone_app::corpus::{self, CorpusSpot, MIN_SPOTS_PER_STATION, Negative};
use skipzone_app::fit::{
    self, Cached, CachedParams, EffectDistribution, Fit, NegativeScore, Spread, StationEffects,
};
use skipzone_app::noise::NoiseEnvironment;
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve;
use skipzone_app::wspr::{WSPR_DECODE_THRESHOLD_DB, WsprSpot};
use skipzone_app::wspr_report::band_label;

struct Args {
    fit: String,
    holdout: Option<String>,
    negatives: Option<String>,
    noise_env: NoiseEnvironment,
    rounds: usize,
    trim_tails: bool,
    /// How many spots to subsample for the re-solve scan; 0 skips it.
    scan: usize,
    /// Cap on how many negatives to solve. A cycle census produces tens of
    /// thousands - 76 252 from fourteen cycles - and each costs a full solve, so
    /// scoring all of them would take hours. Thinned deterministically by a fixed
    /// stride, which preserves the mix of cycles, bands and ranges.
    max_negatives: usize,
    /// Cap on spots per corpus. A solve costs a few hundred milliseconds, so a
    /// 9000-spot corpus is nearly an hour per set. Thinned by a fixed stride from
    /// the whole file, which preserves the band, hour and day balance the corpus
    /// was built to have - a prefix would be one day on one band.
    max_spots: usize,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            fit: "corpus/fit.tsv".to_string(),
            holdout: None,
            negatives: None,
            noise_env: NoiseEnvironment::Rural,
            rounds: 8,
            trim_tails: true,
            scan: 0,
            max_negatives: 2000,
            max_spots: 3000,
        }
    }
}

const USAGE: &str = "\
usage: wspr_calibrate [options]

  --fit PATH        corpus to fit on (default corpus/fit.tsv)
  --holdout PATH    corpus to test on: a DIFFERENT week, ideally a different month
  --negatives PATH  negatives file, for the false-positive rate and a skill score
  --noise-env NAME  receiver noise environment: city, residential, rural,
                    quiet-rural (default rural). An ASSUMPTION, not a measurement,
                    and very nearly unidentifiable here - see the report.
  --rounds N        alternating fit rounds (default 8)
  --keep-tails      do NOT drop the extreme 1 % of station effects
  --scan N          also scan the anchors that need a re-solve (D-region and
                    collision geometry, foE, E-layer geometry, Es), using an N-spot
                    subsample. Costs a full re-solve per value, so start small.
  --max-spots N     how many spots per corpus to solve (default 3000). Thinned by
                    a fixed stride, so the band, hour and day balance the corpus
                    was built to have is preserved.
  --max-negatives N how many negatives to solve (default 2000). Thinned the same
                    way, from the whole file rather than its first N rows.
";

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\ncalibration could not run: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = |what: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{what} needs a value"))
        };
        match flag.as_str() {
            "--fit" => a.fit = val("--fit")?,
            "--holdout" => a.holdout = Some(val("--holdout")?),
            "--negatives" => a.negatives = Some(val("--negatives")?),
            "--noise-env" => {
                a.noise_env = match val("--noise-env")?.as_str() {
                    "city" => NoiseEnvironment::City,
                    "residential" => NoiseEnvironment::Residential,
                    "rural" => NoiseEnvironment::Rural,
                    "quiet-rural" => NoiseEnvironment::QuietRural,
                    other => return Err(format!("unknown --noise-env {other}")),
                };
            }
            "--rounds" => {
                a.rounds = val("--rounds")?
                    .parse()
                    .map_err(|e| format!("bad --rounds: {e}"))?;
            }
            "--keep-tails" => a.trim_tails = false,
            "--scan" => {
                a.scan = val("--scan")?
                    .parse()
                    .map_err(|e| format!("bad --scan: {e}"))?;
            }
            "--max-spots" => {
                a.max_spots = val("--max-spots")?
                    .parse()
                    .map_err(|e| format!("bad --max-spots: {e}"))?;
            }
            "--max-negatives" => {
                a.max_negatives = val("--max-negatives")?
                    .parse()
                    .map_err(|e| format!("bad --max-negatives: {e}"))?;
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

/// A corpus, solved once, with its station index space.
struct Solved {
    spots: Vec<Cached>,
    /// Every spot in the corpus, including the ones no path was found for. The
    /// difference between this and `spots.len()` is the hit rate, and it has to be
    /// carried separately: a spot with no modelled SNR cannot enter the fit, but
    /// dropping it silently would turn a miss into an absence.
    total: usize,
    /// Callsign for each station index, for the effect distribution report.
    tx_names: Vec<String>,
    rx_names: Vec<String>,
}

fn main_solve(
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
    }
}

/// Solve one corpus spot and reduce it to the cached terms the fit needs.
///
/// `None` when the model found no path at all: there is then no modelled SNR to
/// compare against, and inventing one would put a fabricated number into the fit.
/// Those spots are counted separately and reported as a hit rate.
fn solve_one(c: &CorpusSpot, base: &Inputs) -> Option<Cached> {
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
    Some(Cached {
        tx: 0,
        rx: 0,
        measured_db: c.spot.snr_db,
        tx_power_dbm: skipzone_app::noise::dbm_from_watts(inputs.tx_power_w),
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
    })
}

#[allow(clippy::too_many_lines)] // one report, printed top to bottom
fn run(args: &Args) -> Result<(), String> {
    println!("=== CALIBRATING SKIPZONE AGAINST MEASURED WSPR SPOTS ===\n");
    let base = Inputs {
        noise_env: args.noise_env,
        ..Inputs::default()
    };
    println!(
        "ASSUMED  receiver noise environment: {} (an ASSUMPTION; see the identification",
        args.noise_env.label()
    );
    println!("         table below for why WSPR cannot check it)");

    let fit_corpus = load_capped(&args.fit, args.max_spots)?;
    let pool = ComputePool::new(PoolConfig::default()).map_err(|e| e.to_string())?;
    let fit_set = main_solve("fit", &fit_corpus, &base, &pool);
    describe_span("FIT     ", &args.fit, &fit_corpus);

    let holdout_set = match &args.holdout {
        Some(path) => {
            let c = load_capped(path, args.max_spots)?;
            describe_span("HOLDOUT ", path, &c);
            Some(main_solve("holdout", &c, &base, &pool))
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
        .map(|c| (c.modelled_db(1.0, AtmosphericAnchors::default()) - c.modelled_db(1.0, prior.atm)).abs())
        .fold(0.0_f64, f64::max);
    println!("    largest disagreement over {} spots: {:.2e} dB", fit_set.spots.len(), worst);

    // -------------------------------------------------------------- baseline
    let baseline = score_set(&fit_set, 1.0, AtmosphericAnchors::default(), args.trim_tails);
    println!("\n=== BASELINE (the model as it stands) ======================");
    report_fit(&baseline.fit, &fit_set);
    report_layers(&fit_set, 1.0, AtmosphericAnchors::default(), &baseline.effects);

    // ------------------------------------------------------------------- fit
    println!("\n=== FITTING ================================================");
    println!("  Alternating: station effects in closed form, then the physics by");
    println!("  coordinate descent with a pattern move, the absorption scale and the");
    println!("  unattributable global offset solved jointly at every trial point.");
    let (fitted, _fit_effects, notes) = fit::fit_cached(
        &fit_set.spots,
        fit_set.tx_names.len(),
        fit_set.rx_names.len(),
        args.rounds,
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
        if fitted.absorption_scale.at_bound() { "YES" } else { "no" }
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
    println!("\n  Confidence intervals, by leave-one-day-out refits of the same corpus:");
    let ci = day_jackknife(&fit_set, args.rounds);
    println!(
        "    absorption scale   {:.3} to {:.3}  (spread over {} day-deleted refits)",
        ci.min, ci.max, ci.n
    );
    println!(
        "    That spread is the parameter's sensitivity to WHICH DAYS are in the corpus,"
    );
    println!("    which is the uncertainty that matters here - far larger than the");
    println!("    within-day sampling error a textbook standard error would report.");

    // ------------------------------------------------------------- fit vs holdout
    let after_fit = score_set(&fit_set, fitted.absorption_scale.value, fitted.atm, args.trim_tails);
    println!("\n=== RESULT: FIT SET vs HOLD-OUT ============================");
    println!(
        "  {:<34} {:>12} {:>12} {:>12}",
        "", "fit before", "fit after", "hold-out after"
    );
    let holdout_after = holdout_set.as_ref().map(|h| {
        score_set(h, fitted.absorption_scale.value, fitted.atm, args.trim_tails)
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
    row("median, effects removed [dB]", |f| f.adjusted_residual.median);
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
    print_cuts(
        &fit_set,
        &after_fit,
        &baseline,
        |c| band_label(c.freq_mhz).to_string(),
    );
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

    // ------------------------------------------------ station-effect distribution
    println!("\n=== THE STATION-EFFECT DISTRIBUTION ========================");
    println!("  A measurement of the WSPR station population in its own right: how much");
    println!("  better or worse each station is than the model's assumed antenna and");
    println!("  noise floor. Only stations with at least {MIN_SPOTS_PER_STATION} spots are shown -");
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

    // ---------------------------------------------------------------- negatives
    if let Some(path) = &args.negatives {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let (negatives, problems) = corpus::read_negatives(&text);
        if !problems.is_empty() {
            println!("\n{} unreadable negative row(s):", problems.len());
            for p in problems.iter().take(5) {
                println!("   {p}");
            }
        }
        if negatives.is_empty() {
            println!("\nNEGATIVES  file held none; the false-positive rate cannot be measured.");
        } else {
            // A cycle census produces tens of thousands of negatives and each
            // costs a full solve, so take a fixed-stride thinning of the whole
            // file rather than its first N rows: the file is ordered by cycle, so
            // a prefix would be one cycle on one band.
            let stride = (negatives.len() / args.max_negatives.max(1)).max(1);
            let sample: Vec<Negative> = negatives.iter().step_by(stride).cloned().collect();
            report_negatives(&sample, negatives.len(), stride, &base, &pool, &fitted, args);
        }
    } else {
        println!("\n=== FALSE POSITIVES ========================================");
        println!("  NOT MEASURED: no --negatives file was given. Without one the hit rate");
        println!("  below is one-sided and is NOT a skill score - a model that predicted");
        println!("  every path would score 100 %.");
    }

    println!(
        "\n  Model found a path for {} of {} fit spots ({:.0} %), {} of {} hold-out spots.",
        fit_set.spots.len(),
        fit_set.total,
        100.0 * fit_set.spots.len() as f64 / fit_set.total.max(1) as f64,
        holdout_set.as_ref().map_or(0, |h| h.spots.len()),
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
/// here. Moving the E-layer geometry, foE or foEs changes WHICH GEOMETRIES CLOSE
/// - a path appears or disappears - and that is a step change in the objective,
/// not something a least-squares gradient can follow. Moving the D-region or
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
fn scan_resolve_anchors(
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
    let sample: Vec<CorpusSpot> = corpus_spots
        .iter()
        .step_by(stride)
        .cloned()
        .collect();
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
                println!("  {:<30} too few paths closed to score", format!("{name} = {v}"));
                continue;
            }
            let (p, _e, _n) = fit::fit_cached(
                &solved.spots,
                solved.tx_names.len(),
                solved.rx_names.len(),
                args.rounds,
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
fn prior_value_for(name: &str, prior: &skipzone_app::calib::IonosphereAnchors) -> f64 {
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
struct Scored {
    effects: StationEffects,
    fit: Fit,
    /// How many spots the tail trim removed.
    trimmed: usize,
    /// The parameters this set was scored under, carried so a later breakdown
    /// cannot accidentally re-score it under different ones.
    scale: f64,
    atm: AtmosphericAnchors,
}

/// Solve the station effects for a set under given physics, then score it.
///
/// `trim_tails` drops the extreme 1 % of station effects at each end and refits.
/// Those are real stations - a contest station on a mountain, a broken install -
/// but they are not representative, and a handful of them can move a median. What
/// was dropped is reported.
fn score_set(set: &Solved, scale: f64, atm: AtmosphericAnchors, trim_tails: bool) -> Scored {
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

fn report_fit(f: &Fit, set: &Solved) {
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

fn report_layers(set: &Solved, scale: f64, atm: AtmosphericAnchors, effects: &StationEffects) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &set.spots {
        *counts.entry(c.layer).or_default() += 1;
    }
    println!("  layer chosen:");
    for (layer, n) in counts {
        let group: Vec<Cached> = set.spots.iter().filter(|c| c.layer == layer).cloned().collect();
        let f = Fit::of(&group, scale, atm, effects);
        println!(
            "    {layer:<4} {n:>5} spots, median {:+6.1} dB, IQR {:5.1} dB",
            f.residual.median,
            f.residual.iqr()
        );
    }
}

fn print_cuts(
    set: &Solved,
    after: &Scored,
    before: &Scored,
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
    let before_cuts = fit::cuts_by(&set.spots, 1.0, AtmosphericAnchors::default(), &before.effects, key);
    let after_cuts = fit::cuts_by(&set.spots, after.scale, after.atm, &after.effects, key);
    let lookup: BTreeMap<&str, &fit::Cut> =
        before_cuts.iter().map(|c| (c.label.as_str(), c)).collect();
    for c in &after_cuts {
        let b = lookup.get(c.label.as_str());
        println!(
            "  {:<20} {:>6} {:>10} {:>9} {:>10} {:>9} {:>8}",
            c.label,
            c.fit.n,
            b.map_or("-".to_string(), |b| format!("{:+.1}", b.fit.residual.median)),
            format!("{:+.1}", c.fit.residual.median),
            b.map_or("-".to_string(), |b| format!("{:.1}", b.fit.residual.iqr())),
            format!("{:.1}", c.fit.residual.iqr()),
            format!("{:.2}", c.fit.slope_adjusted),
        );
    }
    println!("  * slope with station effects removed; below ~8 spots a cut is not a trend");
}

fn report_effects(effects: &StationEffects, set: &Solved, min_spots: usize) {
    let d: EffectDistribution = effects.distribution(min_spots);
    let show = |what: &str, s: &Spread| {
        println!(
            "  {what:<12} n={:<5} median {:+6.1}  IQR {:5.1}  p10 {:+6.1}  p90 {:+6.1}  \
             min {:+6.1}  max {:+6.1}",
            s.n, s.median, s.iqr(), s.p10, s.p90, s.min, s.max
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
    extremes(&set.tx_names, &effects.tx, &effects.tx_counts, "transmitters");
    extremes(&set.rx_names, &effects.rx, &effects.rx_counts, "receivers");
}

/// Score the negatives: paths that were attempted and did not decode.
fn report_negatives(
    negatives: &[Negative],
    available: usize,
    stride: usize,
    base: &Inputs,
    pool: &ComputePool,
    fitted: &CachedParams,
    args: &Args,
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

    let (results, _) = pool.map(negatives, |n| {
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
            tx: 0,
            rx: 0,
            measured_db: f64::NAN,
            tx_power_dbm: skipzone_app::noise::dbm_from_watts(inputs.tx_power_w),
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
        })
    });
    let cached: Vec<Cached> = results.into_iter().flatten().collect();

    let score = |scale: f64, atm: AtmosphericAnchors| -> NegativeScore {
        let mut margins = Vec::new();
        let mut decodable = 0usize;
        let mut via_es = 0usize;
        for c in &cached {
            let snr = c.modelled_db(scale, atm);
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
    let before = score(1.0, AtmosphericAnchors::default());
    let after = score(fitted.absorption_scale.value, fitted.atm);

    println!(
        "  {:<44} {:>12} {:>12}",
        "", "before fit", "after fit"
    );
    println!(
        "  {:<44} {:>12} {:>12}",
        "negatives constructed", before.n, after.n
    );
    println!(
        "  {:<44} {:>12} {:>12}",
        "model found SOME path", before.path_found, after.path_found
    );
    println!(
        "  {:<44} {:>12} {:>12}",
        "model predicted it would DECODE", before.predicted_decodable, after.predicted_decodable
    );
    println!(
        "  {:<44} {:>11.1}% {:>11.1}%",
        "FALSE POSITIVE RATE (upper bound)",
        100.0 * before.false_positive_rate(),
        100.0 * after.false_positive_rate()
    );
    println!(
        "  {:<44} {:>12} {:>12}",
        "  of those, needed sporadic E", before.via_es, after.via_es
    );
    println!(
        "  {:<44} {:>12.1} {:>12.1}",
        "median margin over threshold [dB]", before.margin.median, after.margin.median
    );
    if args.trim_tails {
        println!("\n  Note: negatives are NOT trimmed - a station being unrepresentative does");
        println!("  not make its silence less real.");
    }
}

/// Leave-one-day-out refits, as the interval that actually matters.
struct Jackknife {
    n: usize,
    min: f64,
    max: f64,
}

fn day_jackknife(set: &Solved, rounds: usize) -> Jackknife {
    let days: Vec<(i32, u32, u32)> = {
        let mut d: Vec<_> = set.spots.iter().map(|c| c.date).collect();
        d.sort_unstable();
        d.dedup();
        d
    };
    let mut values = Vec::new();
    for drop in &days {
        let kept: Vec<Cached> = set
            .spots
            .iter()
            .filter(|c| c.date != *drop)
            .cloned()
            .collect();
        if kept.len() < 50 {
            continue;
        }
        let (p, _, _) = fit::fit_cached(&kept, set.tx_names.len(), set.rx_names.len(), rounds);
        values.push(p.absorption_scale.value);
    }
    Jackknife {
        n: values.len(),
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

fn print_identification_table() {
    println!("\n=== WHAT THIS CORPUS CAN AND CANNOT IDENTIFY ===============");
    println!("  Any quantity that is CONSTANT for a given station is absorbed exactly into");
    println!("  that station's fixed effect. That is not a limitation of the method - it is");
    println!("  what WSPR contains.\n");
    println!("  {:<40} identifiable here?", "quantity");
    for (q, a) in [
        ("absorption magnitude", "YES - from its variation, not its mean"),
        ("absorption vs frequency", "YES - cross-band within a station"),
        ("absorption vs zenith angle", "YES - diurnal within a station"),
        ("atmospheric noise day-night difference", "YES - diurnal within a station"),
        ("atmospheric noise frequency slopes", "YES - cross-band within a station"),
        ("atmospheric noise absolute level", "NO - a constant, absorbed"),
        ("receiver noise environment", "NO - constant per receiver"),
        ("noise model latitude terms", "NO - a station's latitude is fixed"),
        ("seasonal swing", "NO - one month of corpus"),
        ("absolute antenna gain, either end", "NO - constant per station"),
        ("claimed transmit power accuracy", "NO - constant per station"),
    ] {
        println!("  {q:<40} {a}");
    }
    println!("\n  So the calibrated quantities are SHAPES, not levels. Anything claiming to");
    println!("  have calibrated an absolute level from WSPR has fitted the station");
    println!("  population and called it physics.");
}

fn describe_span(tag: &str, path: &str, spots: &[CorpusSpot]) {
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
/// order; a stride keeps the balance the corpus was deliberately built to have.
fn load_capped(path: &str, max: usize) -> Result<Vec<CorpusSpot>, String> {
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

fn load(path: &str) -> Result<Vec<CorpusSpot>, String> {
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
