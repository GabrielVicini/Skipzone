//! Does `d_rise_fraction` reach a solve?
//!
//! `bin/zenith_probe` shows the layer responds to the anchor, but not that the
//! anchor survives `Inputs -> resolve -> build_models -> solve`; a dropped
//! anchor would also explain a calibration scan reporting no change. This
//! solves one high-sun daytime path at several rise fractions and prints the
//! absorption charged. If it moves, the scan's null result is about the
//! corpus; if not, it is a bug.

use skipzone_app::calib::Anchors;
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve::{self, SolveOutcome};

fn best_absorption(o: &SolveOutcome) -> Option<(f64, f64, f64)> {
    o.solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.partial_cmp(&b.link.snr_db).unwrap())
        .map(|s| (s.total_absorption_db, s.link.snr_db, s.tx_elev_deg))
}

fn main() {
    // A short low-band path at high sun: 1.838 MHz, ~550 km, local noon. This is
    // the geometry `mode_audit` uses, and the one where the zenith law bites
    // hardest - the corpus has almost nothing like it.
    let base = Inputs {
        tx_lat: 40.0,
        tx_lon: -100.0,
        rx_lat: 44.5,
        rx_lon: -97.0,
        freq_mhz: 1.838,
        utc_hours: 18.0, // local noon at -100 deg lon
        ssn: 100.0,
        ..Inputs::default()
    };

    println!("=== DOES d_rise_fraction REACH THE SOLVER? ===\n");
    println!("  1.838 MHz, ~550 km, local noon, SSN 100\n");
    println!("      f    D peak alt   absorption    SNR     launch");
    for f in [0.0_f64, 0.285, 0.6, 1.0] {
        let mut anchors = Anchors::default();
        anchors.ionosphere.d_rise_fraction.value = f;
        let inputs = Inputs {
            anchors,
            ..base.clone()
        };
        let a = scenario::resolve(&inputs);
        let models = match scenario::build_models(&inputs, &a) {
            Ok(m) => m,
            Err(e) => {
                println!("  {f:5.3}   build_models failed: {e}");
                continue;
            }
        };
        let out = solve::solve(&inputs, &a, &models);
        match best_absorption(&out) {
            Some((abs, snr, el)) => println!(
                "  {f:5.3}   {:8.2} km  {abs:9.2} dB  {snr:7.2}  {el:6.2} deg",
                a.d_region_peak_alt_km
            ),
            None => println!(
                "  {f:5.3}   {:8.2} km   no path closed",
                a.d_region_peak_alt_km
            ),
        }
    }
    println!(
        "\n  If the absorption column is FLAT, the anchor is not reaching the\n  \
         solver and the calibration scan's null result means nothing."
    );

    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().map_or("corpus/fit.tsv", String::as_str);
    let max: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(600);
    corpus_leverage(path, max);
}

/// How much leverage does the corpus have on the zenith law?
///
/// The calibration scan reported no change across `f`. Either the corpus
/// cannot see the law, or the 305-spot scan subsample contains almost none of
/// the daytime low-band spots where it matters.
///
/// The mean shift in absorption does not decide this, since a fit refits the
/// absorption scale and an offset. The spread of the per-spot shift does: if
/// every spot moves by nearly the same dB, no fit can identify `f`; if the
/// shift varies by several dB, the law is identifiable and the null result
/// comes from looking at too few spots.
fn corpus_leverage(path: &str, max_spots: usize) {
    use skipzone_app::compute::{ComputePool, PoolConfig};
    use skipzone_app::corpus;

    let Ok(text) = std::fs::read_to_string(path) else {
        println!("\n(no corpus at {path}; skipping the leverage measurement)");
        return;
    };
    let (all, _problems) = corpus::read_positives(&text);
    let stride = (all.len() / max_spots.max(1)).max(1);
    let spots: Vec<_> = all.iter().step_by(stride).cloned().collect();

    println!("\n=== HOW MUCH LEVERAGE DOES THE CORPUS HAVE ON f? ===");
    println!(
        "  {} spot(s) of {} (every {}th), each solved at f = 1 and f = 0.",
        spots.len(),
        all.len(),
        stride
    );

    let pool = ComputePool::new(PoolConfig::default()).expect("pool");
    let solve_at = |f: f64| -> Vec<Option<(f64, f64)>> {
        let (out, _t) = pool.map(&spots, |cs| {
            let mut anchors = Anchors::default();
            anchors.ionosphere.d_rise_fraction.value = f;
            let base = Inputs {
                anchors,
                ssn: cs.ssn,
                ..Inputs::default()
            };
            let inputs = cs.spot.inputs_for(&base);
            let a = scenario::resolve(&inputs);
            let Ok(models) = scenario::build_models(&inputs, &a) else {
                return None;
            };
            let out = solve::solve(&inputs, &a, &models);
            best_absorption(&out).map(|(abs, snr, _)| (abs, snr))
        });
        out
    };

    let full = solve_at(1.0);
    let none = solve_at(0.0);

    let mut deltas = Vec::new();
    // The same delta split by band, showing what a corpus would need to contain
    // for the law to be identifiable.
    let mut by_band: std::collections::BTreeMap<&'static str, Vec<f64>> = Default::default();
    for ((a, b), cs) in full.iter().zip(none.iter()).zip(spots.iter()) {
        if let (Some((abs1, _)), Some((abs0, _))) = (a, b) {
            let d = abs0 - abs1; // f=0 charges MORE at high chi
            deltas.push(d);
            let m = cs.spot.freq_mhz;
            let band = if m < 2.5 {
                "160m"
            } else if m < 4.5 {
                "80m"
            } else if m < 6.5 {
                "60m"
            } else if m < 8.5 {
                "40m"
            } else if m < 12.0 {
                "30m"
            } else if m < 16.0 {
                "20m"
            } else if m < 19.5 {
                "17m"
            } else if m < 22.5 {
                "15m"
            } else if m < 26.5 {
                "12m"
            } else {
                "10m"
            };
            by_band.entry(band).or_default().push(d);
        }
    }
    if deltas.len() < 10 {
        println!("  too few spots closed at both settings ({})", deltas.len());
        return;
    }
    deltas.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let n = deltas.len();
    let mean = deltas.iter().sum::<f64>() / n as f64;
    let sd = (deltas.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
    let q = |p: f64| deltas[((p * (n - 1) as f64) as usize).min(n - 1)];

    println!("\n  per-spot change in charged absorption, f=0 minus f=1, over {n} spots:");
    println!("    mean  {mean:+.2} dB   <- a fit ABSORBS this exactly (scale + offset)");
    println!("    sd    {sd:.2} dB      <- this is the only identifiable part");
    println!(
        "    p10 {:+.2}   p50 {:+.2}   p90 {:+.2}   max {:+.2}",
        q(0.10),
        q(0.50),
        q(0.90),
        deltas[n - 1]
    );
    let moved = deltas.iter().filter(|d| d.abs() > 1.0).count();
    println!(
        "    {moved} of {n} spots ({:.0} %) move by more than 1 dB",
        100.0 * moved as f64 / n as f64
    );
    println!("\n  by band - where the lever exists at all:");
    println!("    band      n    mean d    max |d|   n(>1 dB)");
    for b in [
        "160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m",
    ] {
        let Some(v) = by_band.get(b) else { continue };
        if v.is_empty() {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let mx = v.iter().fold(0.0_f64, |a, &x| a.max(x.abs()));
        let big = v.iter().filter(|x| x.abs() > 1.0).count();
        println!("    {b:6} {:5}   {m:+7.2}   {mx:7.2}    {big:5}", v.len());
    }
    println!(
        "\n  Compare against the fit's own residual IQR of ~13.6 dB. An sd well\n  \
         under that is a law this corpus cannot identify no matter how many\n  \
         spots are thrown at it - and the by-band table says WHICH spots a\n  \
         corpus would need instead."
    );
}
