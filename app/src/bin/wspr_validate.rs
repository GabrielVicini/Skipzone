//! Score the model against a list of WSPR spots.
//!
//! ```text
//! cargo run --release -p skipzone-app --bin wspr_validate -- spots.tsv [--ssn 70] [--quiet]
//! ```
//!
//! Reads a tab-separated spot list (timestamp, TX call, MHz, SNR, TX grid, dBm,
//! RX call, RX grid, km), solves each spot through exactly the same chain the
//! GUI's RUN TRACE uses, and reports modelled minus measured SNR per spot plus
//! the median error, the spread and the hit rate over the paths that closed.
//!
//! Read the module documentation of `skipzone_app::wspr` before drawing a
//! conclusion from the numbers: WSPR SNRs are in a 2500 Hz reference bandwidth,
//! the antennas at both ends are assumed rather than known, and the database
//! records only successes - so the hit rate cannot see false positives.
//!
//! Exit status is 0 whenever the run completed. This reports a measurement; it
//! is not a pass/fail gate, and inventing a threshold for it here would just be
//! a number nobody chose.

use std::process::ExitCode;

use skipzone_app::compute::{ComputePool, PoolConfig};
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve;
use skipzone_app::wspr::{SpotResult, Summary, WsprSpot, parse_spots};

struct Args {
    path: String,
    ssn: Option<f64>,
    quiet: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut path = None;
    let mut ssn = None;
    let mut quiet = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ssn" => {
                let v = it.next().ok_or("--ssn needs a value")?;
                ssn = Some(v.parse::<f64>().map_err(|e| format!("bad --ssn: {e}"))?);
            }
            "--quiet" => quiet = true,
            "-h" | "--help" => return Err("usage: wspr_validate <spots.tsv> [--ssn N] [--quiet]".into()),
            other if other.starts_with('-') => return Err(format!("unknown flag {other}")),
            other => path = Some(other.to_string()),
        }
    }
    Ok(Args {
        path: path.ok_or("no spot file given; usage: wspr_validate <spots.tsv> [--ssn N]")?,
        ssn,
        quiet,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let text = match std::fs::read_to_string(&args.path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", args.path);
            return ExitCode::FAILURE;
        }
    };

    let (spots, problems) = parse_spots(&text);
    if !problems.is_empty() {
        // Loud, and before the results: a bias measured over a silently
        // truncated dataset is worse than no bias at all.
        eprintln!("{} unreadable row(s) - NOT included in any statistic below:", problems.len());
        for p in &problems {
            eprintln!("  {p}");
        }
    }
    if spots.is_empty() {
        eprintln!("no usable spots in {}", args.path);
        return ExitCode::FAILURE;
    }

    let base = Inputs {
        ssn: args.ssn.unwrap_or(Inputs::default().ssn),
        ..Inputs::default()
    };
    println!(
        "# {} spot(s) from {}, SSN {:.0}{}",
        spots.len(),
        args.path,
        base.ssn,
        if args.ssn.is_some() {
            " (from --ssn)"
        } else {
            " (default; pass --ssn for the real value on the day)"
        }
    );

    // Each spot is an independent solve, so this is exactly the compute pool's
    // shape. If the pool cannot be built the run still happens, sequentially:
    // a validation harness that refuses to run is worse than a slow one.
    let started = std::time::Instant::now();
    let results: Vec<SpotResult> = match ComputePool::new(PoolConfig::default()) {
        Ok(p) => p.map(&spots, |s| score(s, &base)).0,
        Err(e) => {
            eprintln!("compute pool unavailable ({e}); running sequentially");
            spots.iter().map(|s| score(s, &base)).collect()
        }
    };

    if !args.quiet {
        print_rows(&results);
    }
    print_summary(&Summary::of(&results), &results);
    println!("# wall time {:.1} s", started.elapsed().as_secs_f64());
    ExitCode::SUCCESS
}

/// Solve one spot and score it. Uses the same `resolve` / `build_models` /
/// `solve` chain as the GUI, with no shortcut anywhere.
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
        };
    };
    let out = solve::solve(&inputs, &a, &models);

    let deterministic_snr_db = solve::best_by_snr(&out).map(|s| s.link.snr_db);
    let es = solve::best_es(&out).map(|s| (s.link.snr_db, s.probability));
    let best = solve::best_including_es(&out);

    SpotResult {
        spot: spot.clone(),
        solved_km: out.great_circle_km,
        deterministic_snr_db,
        es,
        layer: best.map(|s| s.layer.label()),
        modelled_snr_db: best.map(|s| s.link.snr_db),
        error_db: best.map(|s| s.link.snr_db - spot.snr_db),
        hops: best.map_or(0, |s| s.hops),
    }
}

fn print_rows(results: &[SpotResult]) {
    println!(
        "{:<16} {:>7} {:>6} {:>6} {:>7} {:>7} {:>7} {:>4} {:>4}  path",
        "timestamp", "MHz", "km", "meas", "model", "error", "det", "hop", "lyr"
    );
    for r in results {
        let (y, mo, d, h, mi) = r.spot.timestamp;
        let opt = |v: Option<f64>| v.map_or_else(|| "    -".to_string(), |x| format!("{x:>7.1}"));
        println!(
            "{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} {:>7.3} {:>6.0} {:>6.0} {} {} {} {:>4} {:>4}  {} -> {}",
            r.spot.freq_mhz,
            r.solved_km,
            r.spot.snr_db,
            opt(r.modelled_snr_db),
            opt(r.error_db),
            opt(r.deterministic_snr_db),
            r.hops,
            r.layer.unwrap_or("-"),
            r.spot.tx_grid,
            r.spot.rx_grid,
        );
    }
    println!();
}

fn print_summary(s: &Summary, results: &[SpotResult]) {
    println!("== summary ==");
    println!("spots                  {}", s.spots);
    println!(
        "paths found            {} ({:.0} % hit rate)",
        s.closed,
        100.0 * s.hit_rate
    );
    println!("  needing sporadic E   {}", s.es_only);
    println!("median error           {:+.1} dB", s.median_error_db);
    println!("mean error             {:+.1} dB", s.mean_error_db);
    println!("stdev                  {:.1} dB", s.stdev_db);
    println!("IQR                    {:.1} dB", s.iqr_db);
    println!(
        "10th / 90th percentile {:+.1} / {:+.1} dB",
        s.p10_db, s.p90_db
    );

    // A grid-decode cross-check: if the solver's own great-circle range
    // disagrees with the spot's reported distance, the coordinates are wrong
    // and every SNR below is being computed for the wrong path.
    let worst = results
        .iter()
        .filter(|r| r.solved_km.is_finite() && r.spot.reported_km > 0.0)
        .map(|r| (r.solved_km - r.spot.reported_km).abs())
        .fold(0.0_f64, f64::max);
    println!("worst range disagreement vs the spot's own km column: {worst:.0} km");

    println!();
    println!(
        "A positive median means the model is OPTIMISTIC. Two known one-sided biases push it \
         that way and are not corrected for: the antennas at both ends are assumed (defaults, \
         usually better than a real WSPR station), and the spot database records only \
         successes - so the hit rate above cannot see false positives, and a model that \
         predicted every path would score 100 %."
    );
}
