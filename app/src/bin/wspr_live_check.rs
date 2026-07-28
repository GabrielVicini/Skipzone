//! Fetch real WSPR spots and the observed sunspot number, score the model
//! against them, and report where it falls behind.
//!
//! ```text
//! cargo run --release -p skipzone-app --bin wspr_live_check
//! cargo run --release -p skipzone-app --bin wspr_live_check -- --minutes 20 --limit 300
//! cargo run --release -p skipzone-app --bin wspr_live_check -- --band 14 --at "2026-07-24 03:22"
//! cargo run --release -p skipzone-app --bin wspr_live_check -- --file spots.tsv --ssn 119
//! ```
//!
//! Every spot is solved through exactly the chain the GUI's RUN TRACE uses, at
//! the spot's own time, frequency, power and endpoints. Nothing about the
//! comparison is tuned: read the module docs of `skipzone_app::wspr` before
//! drawing a conclusion, in particular that only successful decodes are ever
//! published, so the hit rate cannot see false positives.
//!
//! Exit status is 0 whenever the run completed, 1 if it could not run at all.
//! This reports a measurement; it is not a pass/fail gate.

use std::process::ExitCode;

use skipzone_app::compute::{ComputePool, PoolConfig};
use skipzone_app::noise::NoiseEnvironment;
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::spaceweather::{self, Ssn, SsnSource};
use skipzone_app::wspr::{SpotResult, Summary, WsprSpot, parse_spots};
use skipzone_app::wspr_report::{self, Breakdown};
use skipzone_app::wsprlive::{Query, Window};
use skipzone_app::{grid, solve};

struct Args {
    file: Option<String>,
    ssn: Option<f64>,
    minutes: u32,
    limit: u32,
    band_mhz: Option<f64>,
    at: Option<String>,
    min_km: u32,
    max_km: u32,
    max_mhz: f64,
    show: usize,
}

impl Default for Args {
    fn default() -> Self {
        let d = Query::default();
        Self {
            file: None,
            ssn: None,
            minutes: 10,
            limit: d.limit,
            band_mhz: None,
            at: None,
            min_km: d.min_km,
            max_km: d.max_km,
            max_mhz: d.max_mhz,
            show: 8,
        }
    }
}

const USAGE: &str = "\
usage: wspr_live_check [options]

  --minutes N     width of the live sampling window (default 10)
  --at 'Y-M-D H:M'  score a fixed past UTC window instead of the live one
  --limit N       maximum spots to score (default 200); each costs one solve
  --band MHZ      restrict to one band, e.g. --band 14
  --min-km N      shortest path to include (default 300)
  --max-km N      longest path to include (default 20000)
  --max-mhz N     highest frequency to score (default 30, the top of HF; above
                  that this model has no mechanism, so 6 m is out of scope)
  --file PATH     score a saved TSV instead of fetching
  --ssn N         use this sunspot number instead of fetching the observed one
  --show N        how many worst misses / errors to list (default 8)
";

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = |what: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{what} needs a value"))
        };
        match flag.as_str() {
            "--file" => a.file = Some(val("--file")?),
            "--ssn" => a.ssn = Some(num(&val("--ssn")?, "--ssn")?),
            "--minutes" => a.minutes = num(&val("--minutes")?, "--minutes")? as u32,
            "--limit" => a.limit = num(&val("--limit")?, "--limit")? as u32,
            "--band" => a.band_mhz = Some(num(&val("--band")?, "--band")?),
            "--at" => a.at = Some(val("--at")?),
            "--min-km" => a.min_km = num(&val("--min-km")?, "--min-km")? as u32,
            "--max-km" => a.max_km = num(&val("--max-km")?, "--max-km")? as u32,
            "--max-mhz" => a.max_mhz = num(&val("--max-mhz")?, "--max-mhz")?,
            "--show" => a.show = num(&val("--show")?, "--show")? as usize,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

fn num(s: &str, what: &str) -> Result<f64, String> {
    s.parse::<f64>().map_err(|e| format!("bad {what}: {e}"))
}

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
            eprintln!("\nvalidation run could not start: {e}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_lines)] // one report, printed top to bottom
fn run(args: &Args) -> Result<(), String> {
    println!("=== SKIPZONE MODEL VALIDATION AGAINST MEASURED WSPR SPOTS ===\n");

    // ---------------------------------------------------------------- spots
    let (tsv, provenance) = match &args.file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            (text, format!("saved file {path}"))
        }
        None => {
            let window = match &args.at {
                Some(utc) => Window::At {
                    utc: utc.clone(),
                    minutes: args.minutes,
                },
                None => Window::Recent {
                    minutes: args.minutes,
                },
            };
            let q = Query {
                window: window.clone(),
                min_km: args.min_km,
                max_km: args.max_km,
                band_mhz: args.band_mhz,
                max_mhz: args.max_mhz,
                limit: args.limit,
            };
            println!("SOURCE   wspr.live (the WSPRnet archive)");
            println!("WINDOW   {}", window.describe());
            println!("FILTER   {}", q.describe_filter());
            // Say how much the frequency ceiling declined to score. A scope
            // decision that hides its own size is just a silent filter.
            //
            // Only when the ceiling is what is doing the excluding: with an
            // explicit --band the band itself is the filter, and printing a
            // note about 6 m being out of scope while scoring 6 m on request
            // would contradict itself.
            match if args.band_mhz.is_some() {
                Ok((0, 0))
            } else {
                q.excluded_above_ceiling()
            } {
                Ok((above, below)) if above > 0 => {
                    #[allow(clippy::cast_precision_loss)]
                    let pct = 100.0 * above as f64 / (above + below).max(1) as f64;
                    println!(
                        "SCOPE    {above} spot(s) in this window ({pct:.1} %) are above \
                         {:.0} MHz and were not scored.",
                        q.effective_max_mhz()
                    );
                    println!(
                        "         Those openings are sporadic E, meteor scatter or TEP; the \
                         F2 layer cannot"
                    );
                    println!(
                        "         reach 6 m at any launch angle, so counting them as misses \
                         would blame the"
                    );
                    println!("         model for a mechanism it does not carry. --max-mhz 60 includes them.");
                }
                Ok(_) => {}
                Err(e) => println!("SCOPE    could not count what the ceiling excluded: {e}"),
            }
            // Measure the assumption rather than trusting it: how full was the
            // window that was actually sampled?
            match q.completeness() {
                Ok(c) => match c.fraction() {
                    Some(f) if c.is_settled() => {
                        println!(
                            "SETTLED  window cycles hold {:.0} % of a settled cycle ({} vs {}),",
                            100.0 * f,
                            c.window_median,
                            c.settled_median
                        );
                        println!("         so the sample is not skewed towards fast uploaders");
                    }
                    Some(f) => {
                        println!(
                            "WARNING  window cycles hold only {:.0} % of a settled cycle ({} vs {}).",
                            100.0 * f,
                            c.window_median,
                            c.settled_median
                        );
                        println!("         It was read before the archive filled, so this sample");
                        println!("         favours whichever receivers upload fastest. Widen");
                        println!("         --minutes, or re-run in a few minutes.");
                    }
                    None => println!("SETTLED  not measurable (no settled cycles to compare with)"),
                },
                Err(e) => println!("SETTLED  not measurable: {e}"),
            }
            let text = q.fetch_tsv().map_err(|e| e.to_string())?;
            (text, "wspr.live".to_string())
        }
    };

    let (spots, problems) = parse_spots(&tsv);
    if !problems.is_empty() {
        println!("\n{} unreadable row(s), reported not skipped:", problems.len());
        for p in problems.iter().take(5) {
            println!("   {p}");
        }
    }
    if spots.is_empty() {
        return Err(format!("no spots came back from {provenance}"));
    }

    // What actually came back, as opposed to what was asked for.
    let (first, last) = time_span(&spots);
    println!("\nRETURNED {} spot(s), {first} to {last} UTC", spots.len());
    println!("         {} distinct transmitters, {} distinct receivers",
        distinct(&spots, |s| &s.tx_call), distinct(&spots, |s| &s.rx_call));

    // ---------------------------------------------------------------- SSN
    let ssn = resolve_ssn(args, &spots);
    println!("\nSOLAR    {ssn}");
    if let SsnSource::Operator = ssn.source {
        println!("         (no observed value was fetched, so the run is scoring this number \
                  as much as the model)");
    }

    // ---------------------------------------------------------------- solve
    let base = Inputs {
        ssn: ssn.value,
        ..Inputs::default()
    };
    println!("\nSolving {} spot(s)...", spots.len());
    let pool = ComputePool::new(PoolConfig::default()).map_err(|e| e.to_string())?;
    let (results, timing) = pool.map(&spots, |spot| score(spot, &base));
    println!(
        "         {:.2} s wall, {:.0} ms per spot on {} thread(s)",
        timing.total.as_secs_f64(),
        timing.total.as_secs_f64() * 1e3 / spots.len() as f64,
        pool.threads()
    );

    // ---------------------------------------------------------------- report
    let summary = Summary::of(&results);
    println!("\n--- OVERALL -------------------------------------------------");
    println!("  spots scored                {}", summary.spots);
    println!(
        "  model found a path for      {} ({:.0} %)",
        summary.closed,
        100.0 * summary.hit_rate
    );
    println!(
        "  of those, needed sporadic E {} (probabilistic, not a deterministic opening)",
        summary.es_only
    );
    if summary.closed > 0 {
        println!("\n  modelled minus measured SNR, over the paths that closed:");
        println!("    median   {:+7.1} dB   <- the model's bias", summary.median_error_db);
        println!("    mean     {:+7.1} dB", summary.mean_error_db);
        println!("    IQR       {:6.1} dB   <- the model's spread", summary.iqr_db);
        println!("    10th pct {:+7.1} dB", summary.p10_db);
        println!("    90th pct {:+7.1} dB", summary.p90_db);
        println!("    st.dev    {:6.1} dB", summary.stdev_db);
    }

    for Breakdown { axis, cuts } in wspr_report::breakdowns(&results) {
        println!("\n--- BY {} {}", axis.to_uppercase(), "-".repeat(52usize.saturating_sub(axis.len())));
        println!("  {:<32} {:>5} {:>7} {:>9} {:>7}", " ", "spots", "found", "median dB", "IQR");
        for c in &cuts {
            let flag = if c.meaningful() { " " } else { "*" };
            println!(
                "{flag} {:<32} {:>5} {:>6.0}% {:>9} {:>7}",
                c.label,
                c.spots,
                100.0 * c.hit_rate,
                fmt_db(c.median_error_db),
                fmt_plain(c.iqr_db),
            );
        }
        if cuts.iter().any(|c| !c.meaningful()) {
            println!("  * fewer than {} spots: shown for completeness, not a trend", wspr_report::MIN_MEANINGFUL);
        }
    }

    // ------------------------------------------------------- bias attribution
    if summary.closed > 0 {
        let alternatives: Vec<(&'static str, Vec<f64>)> = [
            ("City", NoiseEnvironment::City),
            ("Residential", NoiseEnvironment::Residential),
            ("Quiet rural", NoiseEnvironment::QuietRural),
        ]
        .into_iter()
        .map(|(label, env)| (label, noise_floors_under(&spots, &base, env)))
        .collect();
        let budget = wspr_report::bias_budget(&results, &alternatives);

        println!("\n--- WHAT THE BIAS IS MADE OF -------------------------------");
        println!(
            "  The {:+.1} dB median is not all physics. Two inputs to every spot are",
            budget.median_error_db
        );
        println!("  CHOSEN, not measured, and both are chosen optimistically:\n");
        println!(
            "  antenna gain the model credited   {:+.1} dB   (both ends, at the angles",
            budget.median_assumed_gain_db
        );
        println!(
            "                                              the ray used - a dipole at {:.0} m",
            base.tx_antenna.height_m
        );
        println!("                                              is assumed at BOTH ends)");
        println!(
            "  noise floor scored against        {:.1} dBm  ({} receiver site assumed)",
            budget.median_noise_dbm,
            base.noise_env.label()
        );
        println!("\n  The same spots, had the receiver's noise environment been:\n");
        for (label, median) in &budget.under_noise_env {
            println!(
                "    {label:<14} median error {:+6.1} dB   ({:+.1} dB of the bias)",
                median,
                median - budget.median_error_db
            );
        }
        println!(
            "\n  So the receiver-environment choice alone spans {:.0} dB of the {:+.1} dB.",
            budget
                .under_noise_env
                .iter()
                .map(|(_, m)| *m)
                .fold(f64::NEG_INFINITY, f64::max)
                - budget
                    .under_noise_env
                    .iter()
                    .map(|(_, m)| *m)
                    .fold(f64::INFINITY, f64::min),
            budget.median_error_db
        );
        println!("  Whatever survives all of that is the model's own optimism.");
    }

    // ---------------------------------------------------------------- misses
    let misses = wspr_report::worst_misses(&results, args.show);
    println!("\n--- WHERE IT FALLS BEHIND ----------------------------------");
    if misses.is_empty() {
        println!("  Every scored spot got a path. Note that this cannot be read as the model\n  \
                  being right: the archive publishes only successful decodes, so nothing here\n  \
                  tests whether the model also predicts paths that do not exist.");
    } else {
        println!(
            "  {} spot(s) the model found NO path for. Each one is a signal that was\n  \
             demonstrably received, so these are unambiguous model failures - strongest first:",
            results.iter().filter(|r| !r.closed()).count()
        );
        println!(
            "\n  {:<10} {:>6} {:>8} {:>7}  {:<22} why",
            "band", "km", "meas dB", "TX dBm", "path"
        );
        let mut structural = 0usize;
        for m in misses {
            // A path longer than max_hops can physically span is a limit of the
            // run's configuration, not of the physics, and saying so keeps the
            // genuine failures visible instead of buried among them.
            let why = if wspr_report::needs_more_hops_than(m.solved_km, base.max_hops) {
                structural += 1;
                format!("needs > {} hops", base.max_hops)
            } else {
                "model found nothing".to_string()
            };
            println!(
                "  {:<10} {:>6.0} {:>8.0} {:>7.0}  {:<22} {}",
                wspr_report::band_label(m.spot.freq_mhz),
                m.solved_km,
                m.spot.snr_db,
                m.spot.tx_dbm,
                format!("{} -> {}", m.spot.tx_grid, m.spot.rx_grid),
                why
            );
        }
        if structural > 0 {
            println!(
                "\n  {structural} of those are beyond reach at --max-hops {}: one F2 hop spans",
                base.max_hops
            );
            println!(
                "  about 4000 km, so no launch angle can cross that distance in {} of them.",
                base.max_hops
            );
            println!("  That is the run's configuration, not the ionospheric model.");
        }
    }

    let worst = wspr_report::worst_errors(&results, args.show);
    if !worst.is_empty() {
        println!("\n  Largest SNR errors among the paths that DID close (positive = optimistic):");
        println!(
            "\n  {:<10} {:>6} {:>8} {:>8} {:>8}  {:<6} hops",
            "band", "km", "meas dB", "model dB", "err dB", "layer"
        );
        for w in worst {
            println!(
                "  {:<10} {:>6.0} {:>8.0} {:>8.1} {:>+8.1}  {:<6} {}",
                wspr_report::band_label(w.spot.freq_mhz),
                w.solved_km,
                w.spot.snr_db,
                w.modelled_snr_db.unwrap_or(f64::NAN),
                w.error_db.unwrap_or(f64::NAN),
                w.layer.unwrap_or("-"),
                w.hops
            );
        }
    }

    println!("\n--- HOW TO READ THIS ---------------------------------------");
    println!(
        "  * Only successful decodes are published, so the hit rate says how many real
    openings the model finds. It says NOTHING about false positives: a model that
    predicted every path would score 100 % here.
  * Antennas at both ends are ASSUMED (the spot does not carry them). Real WSPR
    stations are often worse than the default, so the model will tend to read
    optimistic - a positive median is partly this, not only the physics.
  * WSPR SNR is quoted in a 2500 Hz reference bandwidth; the harness pins the
    receiver bandwidth and the -29 dB threshold to match, whatever the GUI is set to.
  * The bias to act on is the MEDIAN, and the thing to chase is a cut whose median
    is far from the others - that is a specific piece of physics, not general error."
    );
    Ok(())
}

/// Score one spot: solve it exactly as the GUI would, and record what the model
/// said against what was measured.
fn score(spot: &WsprSpot, base: &Inputs) -> SpotResult {
    let inputs = spot.inputs_for(base);
    let a = scenario::resolve(&inputs);
    let Ok(models) = scenario::build_models(&inputs, &a) else {
        return SpotResult {
            spot: spot.clone(),
            solved_km: f64::NAN,
            deterministic_snr_db: None,
            es: None,
            layer: None,
            modelled_snr_db: None,
            error_db: None,
            hops: 0,
            assumed_gain_db: None,
            noise_dbm: None,
        };
    };
    let out = solve::solve(&inputs, &a, &models);
    let det = solve::best_by_snr(&out);
    let es = solve::best_es(&out);
    let best = solve::best_with_es_fallback(&out);
    SpotResult {
        spot: spot.clone(),
        solved_km: out.great_circle_km,
        deterministic_snr_db: det.map(|s| s.link.snr_db),
        es: es.map(|s| (s.link.snr_db, s.probability)),
        layer: best.map(|s| s.layer.label()),
        modelled_snr_db: best.map(|s| s.link.snr_db),
        error_db: best.map(|s| s.link.snr_db - spot.snr_db),
        hops: best.map_or(0, |s| s.hops),
        assumed_gain_db: best.map(|s| s.total_gain_db),
        noise_dbm: Some(out.noise.power_dbm),
    }
}

/// The noise floor each spot WOULD have been scored against under a different
/// receiver environment. Nothing is re-solved: the environment moves the floor
/// and nothing else, so the shift can be computed directly.
fn noise_floors_under(
    spots: &[WsprSpot],
    base: &Inputs,
    env: NoiseEnvironment,
) -> Vec<f64> {
    spots
        .iter()
        .map(|spot| {
            let inputs = Inputs {
                noise_env: env,
                ..spot.inputs_for(base)
            };
            let a = scenario::resolve(&inputs);
            scenario::noise_floor_at(&inputs, &a, inputs.freq_mhz).power_dbm
        })
        .collect()
}

/// The sunspot number to run with: the operator's if given, otherwise the
/// observed value for the date the spots actually came from - not for today,
/// which is a different day once `--at` is used.
fn resolve_ssn(args: &Args, spots: &[WsprSpot]) -> Ssn {
    if let Some(v) = args.ssn {
        return Ssn {
            value: v,
            source: SsnSource::Operator,
            as_of: (0, 0, 0),
            stdev: None,
        };
    }
    let (y, m, d) = {
        let t = spots[0].timestamp;
        (t.0, t.1, t.2)
    };
    match spaceweather::ssn_for(y, m, d) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("  ! could not fetch an observed sunspot number: {e}");
            eprintln!("  ! falling back to the app default; the run scores that assumption too");
            Ssn {
                value: Inputs::default().ssn,
                source: SsnSource::Operator,
                as_of: (y, m, d),
                stdev: None,
            }
        }
    }
}

fn time_span(spots: &[WsprSpot]) -> (String, String) {
    let fmt = |t: (i32, u32, u32, u32, u32)| {
        format!("{:04}-{:02}-{:02} {:02}:{:02}", t.0, t.1, t.2, t.3, t.4)
    };
    let mut times: Vec<_> = spots.iter().map(|s| s.timestamp).collect();
    times.sort_unstable();
    (fmt(times[0]), fmt(times[times.len() - 1]))
}

fn distinct(spots: &[WsprSpot], key: impl Fn(&WsprSpot) -> &String) -> usize {
    let mut seen: Vec<&String> = Vec::new();
    for s in spots {
        let k = key(s);
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    seen.len()
}

fn fmt_db(v: f64) -> String {
    if v.is_nan() {
        "-".to_string()
    } else {
        format!("{v:+.1}")
    }
}

fn fmt_plain(v: f64) -> String {
    if v.is_nan() {
        "-".to_string()
    } else {
        format!("{v:.1}")
    }
}

// `grid` is re-exported through the crate root; referenced so the dependency on
// grid decoding (which `parse_spots` performs) is visible at this call site.
#[allow(unused_imports)]
use grid as _grid_used_by_parse_spots;
