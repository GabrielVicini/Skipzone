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

use skipzone_app::antenna::{AntennaConfig, AntennaKind};
use skipzone_app::calib::{Anchors, AtmosphericAnchors};
use skipzone_app::compute::{ComputePool, PoolConfig};
use skipzone_app::corpus::{self, CorpusSpot, MIN_SPOTS_PER_STATION, Negative};
use skipzone_app::fit::{
    self, Cached, CachedParams, EffectDistribution, Fit, NegativeScore, Spread, StationEffects,
};
use skipzone_app::noise::{NoiseEnvironment, NoiseFloor};
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
    /// The antenna ASSUMED at both ends. See [`Args::default`] for why the
    /// calibration default is not the GUI default.
    antenna: AntennaKind,
    /// Height above ground of that antenna, m. Ignored by the isotropic
    /// reference, which is the calibration default.
    antenna_height_m: f64,
    /// Keep the spots the model could only reach through the sporadic-E fallback
    /// in the fit. Off by default - see [`drop_es`] for why they are not physics.
    include_es: bool,
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
            // ISOTROPIC, deliberately, and NOT the GUI's 10 m dipole.
            //
            // A station's absolute gain is constant for that station and is
            // absorbed exactly into its fixed effect, so a flat reference throws
            // away nothing this corpus could ever identify - the identification
            // table at the foot of the report says so.
            //
            // A dipole at a fixed HEIGHT IN METRES is a different matter. It is
            // 0.06 wavelengths up on 160 m and 0.94 on 10 m, so its gain at the
            // 5 deg launch angle a long path uses climbs 11.6 dB per end across
            // that span - 23 dB across the pair - and at 30 deg the same tilt
            // REVERSES SIGN. That is a band-shaped, elevation-coupled term, so a
            // per-station constant cannot absorb it and it lands in the residual,
            // where the only things able to chase it are the absorption scale and
            // the atmospheric noise slopes. Measured: they all run to their
            // bounds doing so.
            //
            // So the flat reference is not a simplification, it is the removal of
            // an assumption the data cannot see past. Pass `--antenna dipole` to
            // put it back and reproduce the older runs.
            antenna: AntennaKind::Isotropic,
            antenna_height_m: 10.0,
            // Es spots are EXCLUDED from the fit by default. `best_with_es_fallback`
            // consults Es only where nothing deterministic closed, so an Es spot
            // records that the deterministic tracer failed to close a path which
            // demonstrably existed - the spot is a decode that really happened.
            // Fitting physics to the sheet's answer is fitting the fallback.
            // Measured on this corpus: they were 41 % of the solved spots at
            // +21 dB, and their presence inverted the fitted slope (0.69 -> 0.59)
            // while the same fit without them left it alone (0.73 -> 0.74).
            // They are still solved, still reported, and still scored.
            include_es: false,
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
  --antenna NAME    antenna ASSUMED at both ends: isotropic, dipole, vertical,
                    efhw (default isotropic). The default is deliberately NOT the
                    GUI's 10 m dipole: absolute gain is absorbed into the station
                    effect anyway, whereas a fixed height in METRES imposes a band
                    tilt of its own that the station effects cannot absorb. Pass
                    `dipole` to reproduce runs made before this flag existed.
  --antenna-height M height above ground of that antenna, m (default 10). Ignored
                    by the isotropic reference.
  --include-es      keep the spots only the sporadic-E fallback reached IN the fit.
                    They are excluded by default: Es answers only where nothing
                    deterministic closed, so such a spot measures the fallback and
                    not the ionosphere. They are reported either way.
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
            "--antenna" => {
                a.antenna = match val("--antenna")?.as_str() {
                    "isotropic" => AntennaKind::Isotropic,
                    "dipole" => AntennaKind::HorizontalDipole,
                    "vertical" => AntennaKind::VerticalMonopole,
                    "efhw" => AntennaKind::Efhw,
                    other => return Err(format!("unknown --antenna {other}")),
                };
            }
            "--antenna-height" => {
                a.antenna_height_m = val("--antenna-height")?
                    .parse()
                    .map_err(|e| format!("bad --antenna-height: {e}"))?;
            }
            "--include-es" => a.include_es = true,
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
    /// Spots dropped by `--exclude-es`, counted so the hit rate stays honest:
    /// an excluded spot is not a spot the model missed.
    es_excluded: usize,
}

/// Drop the spots the model could only reach through the sporadic-E fallback.
///
/// # Why this is a diagnostic and not a mode
///
/// [`solve::best_with_es_fallback`] consults Es only when NOTHING deterministic
/// closed. So "the layer is Es" does not mean the ionosphere used Es - it means
/// the deterministic tracer failed to close a path that demonstrably existed,
/// because the spot is a decode that really happened. The Es sheet then answers
/// with a near-lossless mirror at 100 km, and that number enters the fit as
/// though it were a prediction of the received signal.
///
/// Fitting the physics on those spots is therefore fitting the fallback rather
/// than the ionosphere. Dropping them says where the residual lives; it does not
/// fix anything, and a run that drops a large fraction of its corpus is not a
/// calibration to quote. What is dropped is always reported.
fn drop_es(set: Solved) -> Solved {
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
        es_excluded: 0,
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

    // The F2 path this one beat, if the reported layer was a lower one and an F2
    // path also closed. Taken from the DETERMINISTIC list, which is where F2
    // lives; when the winner came from the Es fallback this asks whether the
    // deterministic stack really had nothing, or only nothing that outscored it.
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
        midpoint_zenith_deg: a.solar.zenith_angle_deg,
        alternative,
    })
}

#[allow(clippy::too_many_lines)] // one report, printed top to bottom
fn run(args: &Args) -> Result<(), String> {
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
fn report_noise_leverage(set: &Solved, prior: AtmosphericAnchors, fitted: AtmosphericAnchors) {
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

fn print_cuts(
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
fn print_cuts_min(
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
fn report_layer_races(set: &Solved) {
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
fn report_terminator_step(
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
        "\n  {:<8} {:>7} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9} {:>7}",
        "band", "n day", "n night", "F2 day", "resid day", "resid ngt", "observed", "model", "true"
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
    println!("  eye. The model has a band's day/night structure right when 'model step' minus");
    println!("  'd_absorb' equals 'd_measured', which is the only column with no model in it.");
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
        "\n  {:<8} {:>7} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9} {:>7}",
        "band", "n day", "n night", "F2 day", "resid day", "resid ngt", "observed", "model", "true"
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
const MIN_QUOTABLE: usize = 30;

fn report_confound_census(set: &Solved, effects: &StationEffects) {
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

fn report_absorption_range(set: &Solved) {
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
fn report_hop_geometry(set: &Solved) {
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
fn settle_at(
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
fn report_local_minimum_check(
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
const FLAT_TOLERANCE_DB: f64 = 0.05;

/// How many points each parameter is profiled at across its range.
const PROFILE_STEPS: usize = 41;

fn report_bound_profiles(
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
fn report_skill(
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

fn report_effects(effects: &StationEffects, set: &Solved, min_spots: usize) {
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

/// Solve the negatives and map them onto the FIT SET's station index space.
///
/// # Why the index space has to be shared
///
/// A negative enters the objective as a statement about an absolute SNR, and the
/// SNR a station would have produced depends on that station's own offset. So a
/// negative from a transmitter the fit knows about must be judged against that
/// transmitter's estimated effect, or the constraint is applied to a station the
/// fit does not believe in.
///
/// Stations absent from the fit corpus keep index `usize::MAX`, which every
/// effect lookup misses and therefore treats as 0. That is not a fallback, it is
/// the correct estimate: the effects are gauge-centred to mean zero, so zero IS
/// the population value for a station nothing is known about.
///
/// The negatives do NOT feed back into estimating the effects themselves. A
/// station's offset is estimated from what it was heard at; a silence carries no
/// level to average in, only a bound.
fn solve_negatives(
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
struct Levels {
    before: f64,
    after: f64,
}

/// Score the negatives: paths that were attempted and did not decode.
///
/// # Why this is reported at two levels
///
/// The fit's objective subtracts the unattributable global offset from every
/// residual, so it is EXACTLY INVARIANT to the level of the prediction: a
/// parameter move that shifts every spot by a constant costs the objective
/// nothing, because the offset re-solves to swallow it. The decode threshold is
/// not invariant to that at all - it is an absolute comparison.
///
/// Score the negatives on the raw modelled SNR and those two facts collide. A fit
/// that slides the whole model optimistic pays nothing in RMS and is charged the
/// entire slide in false positives, so the false-positive column moves for a
/// reason that has nothing to do with the model's SHAPE getting worse. Measured:
/// between the baseline and the fit the offset grew by about 8 dB and the median
/// margin over threshold grew by 12, most of it that slide.
///
/// So both are printed. AS SHIPPED is what a user of the app would actually see,
/// and is the number that matters for the product. LEVEL-MATCHED subtracts each
/// scoring's own offset, which is the number that says whether the fit made the
/// model genuinely more permissive about which paths open, and it is the only one
/// of the two that is comparable before against after.
#[allow(clippy::too_many_arguments)]
fn report_negatives(
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

    // The decoded corpus is CONDITIONED ON DECODING, so any bias measured on it
    // can belong to the selection rather than to the model - and fading
    // survivorship biases the surviving MEASURED values upward, which shows up as
    // a NEGATIVE residual, not a positive one. The negatives carry no such
    // conditioning: they are the paths that did not decode. A model that really
    // over-predicts by day must produce its false positives by day. If instead
    // the daytime and night-time false-positive rates are the same, the daytime
    // bias measured on the decodes is not an absolute over-prediction at all.
    // Level-matched, so no constant offset can move either side.
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

/// Leave-one-day-out refits, as the interval that actually matters.
struct Jackknife {
    n: usize,
    min: f64,
    max: f64,
}

fn day_jackknife(set: &Solved, rounds: usize, negatives: &[Cached]) -> Jackknife {
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
        // The negatives are dropped for the same day, so each refit sees one
        // consistent ionosphere's worth of evidence on both sides.
        let kept_negatives: Vec<Cached> = negatives
            .iter()
            .filter(|c| c.date != *drop)
            .cloned()
            .collect();
        let (p, _, _) = fit::fit_cached(
            &kept,
            set.tx_names.len(),
            set.rx_names.len(),
            rounds,
            fit::Negatives::balanced(&kept_negatives, WSPR_DECODE_THRESHOLD_DB, kept.len()),
        );
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
