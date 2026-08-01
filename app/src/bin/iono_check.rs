//! Validate the ionosphere model against measured ionosonde characteristics.
//!
//! # Why this exists
//!
//! Everything the WSPR calibrator does sits on top of the ionosphere model, and
//! the largest block of that model has never been checked against a measurement.
//! `app/src/assets/fof2_grid.tsv` says so in its own header: its LAYOUT is the
//! operational one, but its VALUES come from [`fof2::climatology_fof2`], an
//! order-of-magnitude climatology calibrated only to [`fof2::fof2_from_ssn`]. It
//! is not CCIR, not URSI, not IRI. Calibrating loss terms against WSPR while the
//! layer underneath them is a guess is optimising the wrong thing.
//!
//! Ionosondes measure foF2 and foE directly. They have none of WSPR's
//! identification problems - no unknown antennas, no unknown noise floor, no
//! fading, no station effects, no decode threshold - so the comparison here is a
//! straight measurement of model error in MHz, with nothing absorbed and nothing
//! fitted.
//!
//! # What it does NOT do
//!
//! It does not fit anything. It reports error. Anything that looks like a
//! correction belongs in a separate, deliberate change with its own test.
//!
//! # Data
//!
//! `corpus/ionosonde.tsv`, fetched from the Lowell GIRO Data Center's DIDBase
//! (FastChar GetBest). GIRO releases under CC-BY-NC-SA 4.0 and asks that each
//! contributing station's operator be acknowledged; the file header carries the
//! licence and the rules-of-the-road link, and any published use of these
//! numbers must carry them too.
//!
//! Run:
//! ```text
//! cargo run --release -p skipzone-app --bin iono_check
//! ```

use std::collections::BTreeMap;
use std::process::ExitCode;

use skipzone_app::calib::Anchors;
use skipzone_app::chapman::chapman_grazing;
use skipzone_app::fof2::{self, DiurnalShape, Fof2Grid};
use skipzone_app::solar::{self, Season};

/// Reject autoscaled points below this confidence score. GIRO uses 0-100, with
/// 999 for a hand-scaled trace and -1 for unknown. Autoscaling failures produce
/// wild values, and a model comparison that swallows them is measuring the
/// scaler rather than the model.
const MIN_CONFIDENCE: f64 = 75.0;

/// One accepted ionosonde observation.
struct Obs {
    station: String,
    lat: f64,
    lon: f64,
    month: u32,
    utc_h: f64,
    date: (i32, u32, u32),
    fof2: f64,
    foe: Option<f64>,
    /// Daily sunspot number for this observation's own date.
    ssn: f64,
}

impl Obs {
    /// Local solar time, hours. The grid and the climatology are both indexed by
    /// this, not by UTC.
    fn lst_h(&self) -> f64 {
        (self.utc_h + self.lon / 15.0).rem_euclid(24.0)
    }
    fn season(&self) -> Season {
        solar::season_at(self.month, self.lat)
    }
}

/// Running error accumulator: everything is reported in MHz, signed as
/// `model - measured`, so POSITIVE means the model reads HIGH.
#[derive(Default)]
struct Err2 {
    n: usize,
    sum: f64,
    sum2: f64,
    abs: Vec<f64>,
    meas: f64,
}

impl Err2 {
    fn push(&mut self, model: f64, measured: f64) {
        let e = model - measured;
        self.n += 1;
        self.sum += e;
        self.sum2 += e * e;
        self.abs.push(e);
        self.meas += measured;
    }
    #[allow(clippy::cast_precision_loss)]
    fn bias(&self) -> f64 {
        self.sum / self.n as f64
    }
    #[allow(clippy::cast_precision_loss)]
    fn rms(&self) -> f64 {
        (self.sum2 / self.n as f64).sqrt()
    }
    #[allow(clippy::cast_precision_loss)]
    fn mean_measured(&self) -> f64 {
        self.meas / self.n as f64
    }
    /// Median of the ABSOLUTE error. Not `|median error|`: those differ by the
    /// whole point of the statistic, and an earlier version of this printed the
    /// second under the first's name, which made a heavy-tailed error look like a
    /// near-perfect fit.
    fn median_abs(&mut self) -> f64 {
        let mut v: Vec<f64> = self.abs.iter().map(|e| e.abs()).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }
}

/// Cells thinner than this print no error. Same discipline as the WSPR
/// calibrator: a median from a handful of points is that handful's own noise.
const MIN_QUOTABLE: usize = 30;

fn main() -> ExitCode {
    let path = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| "corpus/ionosonde.tsv".to_string());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut obs: Vec<Obs> = Vec::new();
    let mut rejected = 0usize;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let p: Vec<&str> = line.split('\t').collect();
        if p.len() < 6 {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (p[1].parse::<f64>(), p[2].parse::<f64>()) else {
            continue;
        };
        // 2026-07-02T00:03:16.000Z
        let t = p[3];
        let (Ok(year), Ok(month), Ok(day)) = (
            t[0..4].parse::<i32>(),
            t[5..7].parse::<u32>(),
            t[8..10].parse::<u32>(),
        ) else {
            continue;
        };
        let (Ok(hh), Ok(mm)) = (t[11..13].parse::<f64>(), t[14..16].parse::<f64>()) else {
            continue;
        };
        let Ok(cs) = p[4].parse::<f64>() else {
            continue;
        };
        let Ok(fof2) = p[5].parse::<f64>() else {
            continue;
        };
        // 999 is a manually scaled trace, which is better than any autoscale.
        if cs < MIN_CONFIDENCE && (cs - 999.0).abs() > 0.5 {
            rejected += 1;
            continue;
        }
        obs.push(Obs {
            station: p[0].to_string(),
            lat,
            lon,
            month,
            utc_h: hh + mm / 60.0,
            date: (year, month, day),
            fof2,
            foe: p.get(6).and_then(|s| s.parse::<f64>().ok()),
            ssn: 0.0,
        });
    }

    if obs.is_empty() {
        eprintln!("no usable rows in {path}");
        return ExitCode::FAILURE;
    }

    println!("=== IONOSPHERE MODEL vs MEASURED IONOSONDE CHARACTERISTICS ===\n");
    println!("Source   {path}");
    println!("         Lowell GIRO Data Center DIDBase, CC-BY-NC-SA 4.0. Each contributing");
    println!("         station's operator must be acknowledged in any published use.");
    println!(
        "ACCEPTED {} observation(s); {rejected} rejected below confidence {MIN_CONFIDENCE:.0}",
        obs.len()
    );
    let stations: BTreeMap<&str, ()> = obs.iter().map(|o| (o.station.as_str(), ())).collect();
    println!("         {} station(s)", stations.len());
    println!();
    println!("Sign convention: MODEL - MEASURED, in MHz. POSITIVE means the model reads HIGH.");
    println!("Nothing here is fitted and nothing is absorbed: unlike the WSPR corpus, an");
    println!("ionosonde has no antenna, no noise floor and no station effect to hide error in.");

    // The corpus's own SSN, so the comparison uses the number the model would
    // have been driven with on those days rather than a guess.
    let ssn_table = ssn_daily();
    let fallback = corpus_ssn();
    for o in &mut obs {
        o.ssn = ssn_table.get(&o.date).copied().unwrap_or(fallback);
    }
    let mut used: Vec<f64> = obs.iter().map(|o| o.ssn).collect();
    used.sort_by(f64::total_cmp);
    let known = ssn_table.len();
    println!(
        "\nSSN       {:.0} to {:.0} across the windows, daily from SILSO ({known} dates known;\n          \
         any not yet published fall back to the WSPR corpus median {fallback:.0})",
        used[0],
        used[used.len() - 1]
    );

    let grid = match Fof2Grid::bundled() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("bundled foF2 grid unavailable: {e}");
            return ExitCode::FAILURE;
        }
    };

    // ---------------------------------------------------------------- foF2
    println!("\n--- foF2: THE HEADLINE ------------------------------------");
    println!("  Two backends, because the app has two and they are supposed to agree: the");
    println!("  bundled GRID (what a real run samples) and the CLIMATOLOGY that generated it.");

    let mut g_all = Err2::default();
    let mut c_all = Err2::default();
    for o in &obs {
        let season = o.season();
        let plane = grid.plane(season, o.ssn);
        g_all.push(plane.sample(o.lat, o.lst_h()).0, o.fof2);
        c_all.push(
            fof2::climatology_fof2(o.lat, o.lst_h(), season, o.ssn),
            o.fof2,
        );
    }
    println!(
        "\n  {:<14} {:>7} {:>10} {:>9} {:>9} {:>12}",
        "backend", "n", "measured", "bias", "RMS", "median |err|"
    );
    for (name, e) in [("grid", &mut g_all), ("climatology", &mut c_all)] {
        let (n, meas, bias, rms, med) = (e.n, e.mean_measured(), e.bias(), e.rms(), e.median_abs());
        println!("  {name:<14} {n:>7} {meas:>10.2} {bias:>+9.2} {rms:>9.2} {med:>12.2}");
    }

    cut(
        "foF2 by LOCAL SOLAR TIME",
        "  The diurnal shape is one cosine harmonic by construction. This is where that\n  \
         costs, and it is the cheapest thing in the model to improve.",
        &obs,
        grid,
        |o| {
            let l = o.lst_h();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let b = (l / 3.0).floor() as usize * 3;
            format!("{b:02}-{:02} LST", b + 3)
        },
    );

    cut(
        "foF2 by LATITUDE",
        "  The equatorial-anomaly and polar-trough terms use GEOGRAPHIC latitude as a stand-in\n  \
         for magnetic dip latitude, which the module docs flag as a real limitation.",
        &obs,
        grid,
        |o| {
            let a = o.lat.abs();
            let b = if a < 20.0 {
                "a) |lat| < 20"
            } else if a < 40.0 {
                "b) 20-40"
            } else if a < 55.0 {
                "c) 40-55"
            } else if a < 65.0 {
                "d) 55-65"
            } else {
                "e) > 65"
            };
            b.to_string()
        },
    );

    cut(
        "foF2 by STATION",
        "  A station is a fixed latitude, longitude and instrument. A bias that belongs to one\n  \
         station is not a model error in the same sense as one shared across many.",
        &obs,
        grid,
        |o| o.station.clone(),
    );

    // ----------------------------------------------------------------- foE
    //
    // foE is reported only when the E layer is detectable, which is essentially
    // daytime, so this cut is day-only whether or not it says so.
    println!("\n--- foE ----------------------------------------------------");
    println!("  The E layer sets which low bands are screened off F2 by day, so its error");
    println!("  propagates into WHICH PATHS EXIST, not just into their loss. Modelled as");
    println!("  foe_overhead(SSN) / Ch(X, chi)^(1/4) - the same generalised Chapman layer the");
    println!("  engine builds, evaluated at each station's own solar zenith angle.");

    let anchors = Anchors::default();
    let quiet = anchors.ionosphere.foe_overhead_quiet_mhz.value;
    let r_peak = 6_371_000.0 + anchors.ionosphere.e_peak_alt_km.value * 1e3;
    let big_x = r_peak / (anchors.ionosphere.e_scale_height_km.value * 1e3);

    let mut e_all = Err2::default();
    let mut e_by_lst: BTreeMap<String, Err2> = BTreeMap::new();
    for o in &obs {
        let Some(measured) = o.foe else { continue };
        if measured <= 0.0 {
            continue;
        }
        let g = solar::solar_geometry(o.lat, o.lon, o.month, o.date.2, o.utc_h);
        let chi = g.zenith_angle_deg.to_radians();
        let (ch, _) = chapman_grazing(big_x, chi);
        if !ch.is_finite() || ch <= 0.0 {
            continue;
        }
        let model = fof2::foe_overhead(o.ssn, quiet) / ch.powf(0.25);
        e_all.push(model, measured);
        let l = o.lst_h();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let b = (l / 3.0).floor() as usize * 3;
        e_by_lst
            .entry(format!("{b:02}-{:02} LST", b + 3))
            .or_default()
            .push(model, measured);
    }
    if e_all.n < MIN_QUOTABLE {
        println!(
            "\n  only {} foE observation(s); under the {MIN_QUOTABLE} floor",
            e_all.n
        );
    } else {
        println!(
            "\n  {:<14} {:>7} {:>10} {:>9} {:>9} {:>12}",
            "cut", "n", "measured", "bias", "RMS", "median |err|"
        );
        let (n, meas, bias, rms, med) = (
            e_all.n,
            e_all.mean_measured(),
            e_all.bias(),
            e_all.rms(),
            e_all.median_abs(),
        );
        println!(
            "  {:<14} {n:>7} {meas:>10.2} {bias:>+9.2} {rms:>9.2} {med:>12.2}",
            "all"
        );
        for (k, e) in &mut e_by_lst {
            if e.n < MIN_QUOTABLE {
                continue;
            }
            let (n, meas, bias, rms, med) =
                (e.n, e.mean_measured(), e.bias(), e.rms(), e.median_abs());
            println!("  {k:<14} {n:>7} {meas:>10.2} {bias:>+9.2} {rms:>9.2} {med:>12.2}");
        }
    }

    if std::env::args().any(|a| a == "--propose") {
        propose(&obs, quiet, big_x);
    } else {
        println!("\n  (pass --propose to search the diurnal shape and the foE anchor against");
        println!("  these observations and print the values that minimise the error)");
    }

    println!("\n=== WHAT THIS DOES AND DOES NOT SETTLE =====================");
    println!("  SETTLES: the model's foF2 and foE error in MHz, directly, with nothing fitted");
    println!("  and nothing absorbed into a nuisance parameter.");
    println!("  DOES NOT SETTLE: anything about the LINK BUDGET. An ionosonde is a vertical");
    println!("  sounding at one point; it says what the layer is, not what a 2000 km oblique");
    println!("  path through it loses. Absorption, ground reflections and the noise floor are");
    println!("  untouched by this file.");
    println!("  SPAN: one week in July, so the seasonal terms are not tested at all.");

    ExitCode::SUCCESS
}

/// Search the diurnal shape and the foE anchor for the values that minimise RMS
/// against these soundings, and PRINT them.
///
/// Deliberately does not write anything back. Moving a constant in `fof2.rs` is a
/// deliberate edit with a test attached and a grid regeneration behind it; a tool
/// that silently retunes the model is how a model stops being inspectable.
///
/// The search is coordinate descent with a shrinking step, restarted once from
/// the best point. Five parameters against ten thousand observations is not a
/// hard optimisation, and the profile is printed so a flat direction is visible
/// rather than hidden - the same discipline the WSPR calibrator uses for its
/// bound hits.
fn propose(obs: &[Obs], quiet: f64, big_x: f64) {
    println!("\n--- PROPOSED CONSTANTS -------------------------------------");
    println!("  Values that minimise RMS against these soundings. NOT applied: paste them into");
    println!("  fof2.rs deliberately, regenerate the grid, and re-run this to confirm.");

    let rms = |s: DiurnalShape| -> f64 {
        let mut acc = 0.0;
        for o in obs {
            let e = fof2::climatology_fof2_with(o.lat, o.lst_h(), o.season(), o.ssn, s) - o.fof2;
            acc += e * e;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            (acc / obs.len() as f64).sqrt()
        }
    };

    // (name, getter, setter, lower, upper, initial step)
    type Field = (
        &'static str,
        fn(&DiurnalShape) -> f64,
        fn(&mut DiurnalShape, f64),
        f64,
        f64,
        f64,
    );
    let fields: [Field; 5] = [
        (
            "DIURNAL_PEAK_LST_H",
            |s| s.peak_lst_h,
            |s, v| s.peak_lst_h = v,
            10.0,
            18.0,
            1.0,
        ),
        (
            "DIURNAL_MIN_FRACTION",
            |s| s.min_fraction,
            |s, v| s.min_fraction = v,
            0.20,
            0.95,
            0.05,
        ),
        (
            "DIURNAL_SECOND_AMP",
            |s| s.second_amp,
            |s, v| s.second_amp = v,
            -0.30,
            0.30,
            0.05,
        ),
        (
            "DIURNAL_SECOND_PHASE_H",
            |s| s.second_phase_h,
            |s, v| s.second_phase_h = v,
            -6.0,
            6.0,
            1.0,
        ),
        (
            "level scale on fof2_from_ssn",
            |s| s.level_scale,
            |s, v| s.level_scale = v,
            0.70,
            1.30,
            0.05,
        ),
    ];

    // Is the model OVER-DISPERSED? This decides whether a level scale below 1 is
    // a finding or a trap. Shrinking any over-dispersed predictor towards its own
    // mean always lowers RMS, and for a propagation model that is a bad trade: it
    // buys the metric by lowering every MUF, closing paths that are in fact open.
    // So the spread is reported before any scale is proposed.
    #[allow(clippy::cast_precision_loss)]
    let n = obs.len() as f64;
    let d = DiurnalShape::default();
    let (mut sm, mut sy, mut smm, mut syy, mut smy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for o in obs {
        let m = fof2::climatology_fof2_with(o.lat, o.lst_h(), o.season(), o.ssn, d);
        sm += m;
        sy += o.fof2;
        smm += m * m;
        syy += o.fof2 * o.fof2;
        smy += m * o.fof2;
    }
    let (mbar, ybar) = (sm / n, sy / n);
    let sd_m = (smm / n - mbar * mbar).sqrt();
    let sd_y = (syy / n - ybar * ybar).sqrt();
    let cov = smy / n - mbar * ybar;
    println!("\n  DISPERSION at the current constants:");
    println!(
        "    SD of model {sd_m:.2} MHz vs SD of measured {sd_y:.2} MHz   (ratio {:.2})",
        sd_m / sd_y
    );
    println!("    correlation {:.3}", cov / (sd_m * sd_y));
    println!("    A ratio above 1 means the model swings harder than the ionosphere does, and");
    println!("    a level scale below 1 will then cut RMS whether or not the level is wrong.");

    let mut best = DiurnalShape::default();
    let start = rms(best);
    let mut best_rms = start;
    for _restart in 0..2 {
        let mut step: Vec<f64> = fields.iter().map(|f| f.5).collect();
        for _sweep in 0..60 {
            let mut moved = false;
            for (i, f) in fields.iter().enumerate() {
                for dir in [1.0, -1.0] {
                    let mut trial = best;
                    let v = (f.1(&best) + dir * step[i]).clamp(f.3, f.4);
                    if (v - f.1(&best)).abs() < 1e-12 {
                        continue;
                    }
                    f.2(&mut trial, v);
                    let r = rms(trial);
                    if r < best_rms - 1e-9 {
                        best = trial;
                        best_rms = r;
                        moved = true;
                    }
                }
            }
            if !moved {
                for s in &mut step {
                    *s *= 0.5;
                }
                if step.iter().all(|s| *s < 1e-4) {
                    break;
                }
            }
        }
    }

    println!("\n  foF2 diurnal shape           current      proposed");
    for f in &fields {
        let mut d = DiurnalShape::default();
        let cur = f.1(&d);
        let prop = f.1(&best);
        // How flat is the objective here? A parameter the data does not
        // constrain must not be presented as if it had been measured.
        f.2(&mut d, prop);
        let mut flat = "";
        let probe = (f.4 - f.3) * 0.05;
        let mut a = best;
        f.2(&mut a, (prop + probe).min(f.4));
        let mut b = best;
        f.2(&mut b, (prop - probe).max(f.3));
        if (rms(a) - best_rms).abs() < 0.01 && (rms(b) - best_rms).abs() < 0.01 {
            flat = "   FLAT - not identified by this corpus";
        }
        println!("  {:<28} {cur:>7.3} {prop:>13.3}{flat}", f.0);
    }
    println!(
        "\n  foF2 RMS   {start:.3} -> {best_rms:.3} MHz   ({:+.1} %)",
        100.0 * (best_rms - start) / start
    );

    // foE is linear in the anchor, so its optimum is closed-form: the scale that
    // minimises sum (k*m - y)^2 is sum(m*y)/sum(m*m). No search, no local minimum.
    let (mut num, mut den) = (0.0, 0.0);
    for o in obs {
        let Some(measured) = o.foe else { continue };
        if measured <= 0.0 {
            continue;
        }
        let g = solar::solar_geometry(o.lat, o.lon, o.month, o.date.2, o.utc_h);
        let (ch, _) = chapman_grazing(big_x, g.zenith_angle_deg.to_radians());
        if !ch.is_finite() || ch <= 0.0 {
            continue;
        }
        let unit = fof2::foe_overhead(o.ssn, 1.0) / ch.powf(0.25);
        num += unit * measured;
        den += unit * unit;
    }
    if den > 0.0 {
        println!(
            "\n  FOE_OVERHEAD_QUIET_MHZ       {quiet:>7.3} {:>13.3}   (closed form, not searched)",
            num / den
        );
    }
}

/// Daily sunspot number by date, from `corpus/ssn_daily.tsv` (SILSO).
///
/// The seasonal windows sit months apart on the solar cycle. Driving them all at
/// one SSN would charge the model for that cycle and call the difference model
/// error, which is the same confound as fitting a diurnal shape in one season.
fn ssn_daily() -> BTreeMap<(i32, u32, u32), f64> {
    let mut out = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string("corpus/ssn_daily.tsv") else {
        return out;
    };
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let p: Vec<&str> = line.split('\t').collect();
        if p.len() < 4 {
            continue;
        }
        if let (Ok(y), Ok(m), Ok(d), Ok(s)) = (
            p[0].parse::<i32>(),
            p[1].parse::<u32>(),
            p[2].parse::<u32>(),
            p[3].parse::<f64>(),
        ) {
            out.insert((y, m, d), s);
        }
    }
    out
}

/// Median SSN over the WSPR corpus, used only for dates SILSO has not published
/// a daily value for yet - its series lags real time by about a month.
fn corpus_ssn() -> f64 {
    let Ok(text) = std::fs::read_to_string("corpus/fit.tsv") else {
        return 90.0;
    };
    let mut v: Vec<f64> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            l.split('\t')
                .nth(9)
                .and_then(|s| s.trim().parse::<f64>().ok())
        })
        .collect();
    if v.is_empty() {
        return 90.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn cut(title: &str, note: &str, obs: &[Obs], grid: &Fof2Grid, key: impl Fn(&Obs) -> String) {
    println!("\n--- {title} ---");
    println!("{note}");
    let mut cells: BTreeMap<String, (Err2, Err2)> = BTreeMap::new();
    for o in obs {
        let season = o.season();
        let plane = grid.plane(season, o.ssn);
        let e = cells.entry(key(o)).or_default();
        e.0.push(plane.sample(o.lat, o.lst_h()).0, o.fof2);
        e.1.push(
            fof2::climatology_fof2(o.lat, o.lst_h(), season, o.ssn),
            o.fof2,
        );
    }
    println!(
        "\n  {:<16} {:>7} {:>10} {:>11} {:>10} {:>12} {:>11}",
        "cut", "n", "measured", "grid bias", "grid RMS", "clim bias", "clim RMS"
    );
    let mut thin = 0usize;
    for (k, (g, c)) in &mut cells {
        if g.n < MIN_QUOTABLE {
            thin += g.n;
            continue;
        }
        println!(
            "  {:<16} {:>7} {:>10.2} {:>+11.2} {:>10.2} {:>+12.2} {:>11.2}",
            k,
            g.n,
            g.mean_measured(),
            g.bias(),
            g.rms(),
            c.bias(),
            c.rms()
        );
    }
    if thin > 0 {
        println!("  ({thin} observation(s) in cells below the {MIN_QUOTABLE} floor are not shown)");
    }
}
