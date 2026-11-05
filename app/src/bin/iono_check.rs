//! Validate the ionosphere model against measured ionosonde characteristics.
//!
//! The WSPR calibrator sits on top of the ionosphere model, so the model's
//! foF2 and foE should be checked against measurement first. The bundled
//! `crates/skipzone-iono/src/assets/fof2_grid.tsv` comes from [`fof2::climatology_fof2`], an
//! order-of-magnitude climatology calibrated only to [`fof2::fof2_from_ssn`],
//! not CCIR, URSI or IRI data.
//!
//! Ionosondes measure foF2 and foE directly, with none of WSPR's
//! identification problems (unknown antennas and noise floors, fading, station
//! effects, decode threshold), so this is a direct measurement of model error
//! in MHz.
//!
//! It fits nothing and only reports error; corrections belong in a separate
//! change with its own test.
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

use skipzone_iono::ccir::CcirMonth;
use skipzone_iono::chapman::chapman_grazing;
use skipzone_iono::fof2::{self, DiurnalShape, Fof2Grid};
use skipzone_iono::solar::{self, Season};
use skipzone_predict::calib::Anchors;

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
    /// Centred twelve-month mean of the daily series, i.e. R12, the index the
    /// CCIR maps are tabulated against, where the series covers
    /// enough of the window to form one. See [`smoothed_ssn`].
    r12: f64,
    /// Days of daily SSN that went into `r12`, out of the 365 the window asks
    /// for. Reported rather than hidden, because a truncated window on a rising
    /// cycle biases the mean.
    r12_days: usize,
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

/// Every foF2 backend's answer for one observation, computed once.
///
/// Computed once because the cuts below re-bin the same numbers several ways,
/// and rebuilding a `Fof2Plane` per observation per cut is slow.
struct Models {
    /// The bundled grid: what a real solve samples.
    grid: f64,
    /// The climatology the grid was generated from.
    clim: f64,
    /// CCIR, driven by the same daily SSN the other two get. This is the fair
    /// head-to-head: same driver, same observation, different map.
    ccir: f64,
    /// CCIR, driven by R12, the index it is tabulated against.
    ccir_r12: f64,
    /// CCIR driven by R12 converted back to the sunspot scale the maps were
    /// fitted on. See [`SSN_V2_OVER_V1`]: a unit conversion, not a calibration.
    ccir_r12_v1: f64,
    /// CCIR on the day's own sunspot number, converted to the maps' scale.
    /// This is what the app can run: `spaceweather` gives it an observed daily
    /// number, not a twelve-month mean.
    ccir_daily_v1: f64,
}

impl Models {
    /// Every backend's value, in [`BACKENDS`] order.
    fn all(&self) -> [f64; 6] {
        [
            self.grid,
            self.clim,
            self.ccir,
            self.ccir_r12,
            self.ccir_r12_v1,
            self.ccir_daily_v1,
        ]
    }
}

/// Running error accumulator: everything is reported in MHz, signed as
/// `model - measured`, so positive means the model reads high.
#[derive(Default)]
struct Err2 {
    n: usize,
    sum: f64,
    sum2: f64,
    abs: Vec<f64>,
    meas: f64,
    /// Sums for the dispersion and correlation of the model against the
    /// measurement. A model can be unbiased and still useless, by being flat;
    /// bias and RMS alone do not say which.
    mm: f64,
    yy: f64,
    my: f64,
    mdl: f64,
}

impl Err2 {
    fn push(&mut self, model: f64, measured: f64) {
        let e = model - measured;
        self.n += 1;
        self.sum += e;
        self.sum2 += e * e;
        self.abs.push(e);
        self.meas += measured;
        self.mdl += model;
        self.mm += model * model;
        self.yy += measured * measured;
        self.my += model * measured;
    }

    /// Standard deviation of the model's own values.
    #[allow(clippy::cast_precision_loss)]
    fn sd_model(&self) -> f64 {
        let n = self.n as f64;
        (self.mm / n - (self.mdl / n).powi(2)).max(0.0).sqrt()
    }

    /// Standard deviation of the measurements, i.e. how much there was to
    /// explain.
    #[allow(clippy::cast_precision_loss)]
    fn sd_measured(&self) -> f64 {
        let n = self.n as f64;
        (self.yy / n - (self.meas / n).powi(2)).max(0.0).sqrt()
    }

    #[allow(clippy::cast_precision_loss)]
    fn correlation(&self) -> f64 {
        let n = self.n as f64;
        let cov = self.my / n - (self.mdl / n) * (self.meas / n);
        let d = self.sd_model() * self.sd_measured();
        if d > 0.0 { cov / d } else { f64::NAN }
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
    /// Median of the absolute error, which differs from `|median error|`; the
    /// latter would make a heavy-tailed error look like a near-perfect fit.
    fn median_abs(&mut self) -> f64 {
        let mut v: Vec<f64> = self.abs.iter().map(|e| e.abs()).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }
}

/// Cells thinner than this print no error. Same discipline as the WSPR
/// calibrator: a median from a handful of points is that handful's own noise.
const MIN_QUOTABLE: usize = 30;

/// Default locations of the three inputs, all overridable so the tool does not
/// silently depend on being run from the repository root.
const DEFAULT_OBS: &str = "corpus/ionosonde.tsv";
const DEFAULT_SSN: &str = "corpus/ssn_daily.tsv";
const DEFAULT_FIT: &str = "corpus/fit.tsv";

/// Value of `--name=VALUE`, if given.
fn flag(name: &str) -> Option<String> {
    let prefix = format!("--{name}=");
    std::env::args().find_map(|a| a.strip_prefix(&prefix).map(str::to_string))
}

fn main() -> ExitCode {
    let path = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| DEFAULT_OBS.to_string());
    let ssn_path = flag("ssn").unwrap_or_else(|| DEFAULT_SSN.to_string());
    let fit_path = flag("fit").unwrap_or_else(|| DEFAULT_FIT.to_string());
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
            r12: 0.0,
            r12_days: 0,
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
    let ssn_table = ssn_daily(&ssn_path);
    let fallback = corpus_ssn(&fit_path);
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

    // R12, for the CCIR maps, which are tabulated against it and not against a
    // daily number. Where the daily series does not cover the whole centred
    // year the mean is formed from what there is and the shortfall is reported.
    let by_day: BTreeMap<i64, f64> = ssn_table
        .iter()
        .map(|(&(y, m, d), &s)| (day_number(y, m, d), s))
        .collect();
    let mut worst_cover = usize::MAX;
    for o in &mut obs {
        let (r12, days) = smoothed_ssn(&by_day, day_number(o.date.0, o.date.1, o.date.2));
        o.r12 = if days == 0 { o.ssn } else { r12 };
        o.r12_days = days;
        worst_cover = worst_cover.min(days);
    }
    let mut r12s: Vec<f64> = obs.iter().map(|o| o.r12).collect();
    r12s.sort_by(f64::total_cmp);
    println!(
        "R12       {:.0} to {:.0}, centred 12-month mean of that series. Thinnest window has\n          \
         {worst_cover} of {SMOOTH_WINDOW_DAYS} days, so a window near the end of the series is a\n          \
         truncated mean and is biased by the cycle's slope, not just noisy.",
        r12s[0],
        r12s[r12s.len() - 1]
    );

    // The observation windows, from the data rather than from a stale comment.
    let mut months: BTreeMap<(i32, u32), usize> = BTreeMap::new();
    for o in &obs {
        *months.entry((o.date.0, o.date.1)).or_default() += 1;
    }
    let window_list: Vec<String> = months
        .iter()
        .map(|((y, m), n)| format!("{y}-{m:02} ({n})"))
        .collect();
    println!("WINDOWS   {}", window_list.join(", "));

    let grid = match Fof2Grid::bundled() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("bundled foF2 grid unavailable: {e}");
            return ExitCode::FAILURE;
        }
    };

    // FoF2
    println!("\n--- foF2: THE HEADLINE ------------------------------------");
    println!("  The app's two backends - the bundled GRID (what a real run samples) and the");
    println!("  CLIMATOLOGY that generated it, which are supposed to agree - against the CCIR");
    println!("  numerical maps, which are real measured coefficients and sat unread in");
    println!("  data/nequick/ until app/src/ccir.rs was written.");
    println!();
    println!("  CCIR appears four times because it takes a solar driver and there are two");
    println!("  choices in each of two independent directions, neither of them free:");
    println!("    DAILY vs R12   - the maps are tabulated against R12, a twelve-month mean,");
    println!("                     but the app only ever holds an observed daily number;");
    println!(
        "    v2 vs v1       - SILSO rescaled the sunspot number by {SSN_V2_OVER_V1:.2} in 2015 and the"
    );
    println!("                     maps were fitted in 1966, so feeding v2 straight in is a");
    println!("                     unit error. Dividing is a conversion, not a fit.");

    let models = evaluate(&obs, grid);

    let mut all: [Err2; 6] = Default::default();
    for (o, m) in obs.iter().zip(&models) {
        for (acc, v) in all.iter_mut().zip(m.all()) {
            acc.push(v, o.fof2);
        }
    }
    println!(
        "\n  {:<16} {:>7} {:>10} {:>9} {:>9} {:>12} {:>8} {:>7}",
        "backend", "n", "measured", "bias", "RMS", "median |err|", "SD mdl", "corr"
    );
    for (name, e) in BACKENDS.iter().zip(all.iter_mut()) {
        let (n, meas, bias, rms, med) = (e.n, e.mean_measured(), e.bias(), e.rms(), e.median_abs());
        let (sd, corr) = (e.sd_model(), e.correlation());
        println!(
            "  {name:<16} {n:>7} {meas:>10.2} {bias:>+9.2} {rms:>9.2} {med:>12.2} {sd:>8.2} {corr:>7.3}"
        );
    }
    println!(
        "\n  SD of the measured foF2 is {:.2} MHz. A backend whose SD is well below that is",
        all[0].sd_measured()
    );
    println!("  COMPRESSING - it will read low on the high days and high on the low ones, and");
    println!("  a level scale cannot fix that. Correlation is the part a level scale keeps.");

    cut(
        "foF2 by LOCAL SOLAR TIME",
        "  The app's diurnal shape is one cosine harmonic by construction. CCIR carries six\n  \
         harmonics in universal time, which is the single biggest structural difference.",
        &obs,
        &models,
        |o| {
            let l = o.lst_h();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let b = (l / 3.0).floor() as usize * 3;
            format!("{b:02}-{:02} LST", b + 3)
        },
    );

    cut(
        "foF2 by LATITUDE",
        "  The app's equatorial-anomaly and polar-trough terms use GEOGRAPHIC latitude as a\n  \
         stand-in for magnetic dip latitude, which the module docs flag as a real limitation.\n  \
         CCIR is expanded in MODIFIED DIP LATITUDE, so this cut is where that should show.",
        &obs,
        &models,
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
        "foF2 by SEASON (local)",
        "  All four windows, split by the local season each observation fell in. Every\n  \
         seasonal term in the app is untested by a single-season run; this is the cut that\n  \
         tests them.",
        &obs,
        &models,
        |o| format!("{}-{:02}  {}", o.date.0, o.month, o.season().label()),
    );

    cut(
        "foF2 by STATION",
        "  A station is a fixed latitude, longitude and instrument. A bias that belongs to one\n  \
         station is not a model error in the same sense as one shared across many.",
        &obs,
        &models,
        |o| o.station.clone(),
    );

    // FoE
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
    println!("  SPAN: the windows listed at the top, and nothing outside them. In particular");
    println!("  one position on the solar cycle, so nothing here tests the solar-activity law.");

    ExitCode::SUCCESS
}

/// Search the diurnal shape and the foE anchor for the values that minimise RMS
/// against these soundings, and print them.
///
/// Nothing is written back: changing a constant in `fof2.rs` is a manual edit
/// with a test and a grid regeneration.
///
/// The search is coordinate descent with a shrinking step, restarted once from
/// the best point. Five parameters against ten thousand observations is an easy
/// problem, and the profile is printed so a flat direction is visible, as the
/// WSPR calibrator does for its bound hits.
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

    // Is the model over-dispersed? Shrinking an over-dispersed predictor towards
    // its mean always lowers RMS, but for a propagation model it lowers every
    // MUF and closes open paths, so a level scale below 1 may be a trap. The
    // spread is reported before any scale is proposed.
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

/// Daily sunspot number by date, from the SILSO daily series (`--ssn=PATH`).
///
/// The seasonal windows sit months apart on the solar cycle, and one SSN for
/// all of them would count the solar cycle as model error.
///
/// A missing file falls back to the corpus median for every observation, which
/// changes what the run measures, so it is reported.
fn ssn_daily(path: &str) -> BTreeMap<(i32, u32, u32), f64> {
    let mut out = BTreeMap::new();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "warning: cannot read daily SSN from {path}: {e}
                 every observation will use the corpus median instead;                  pass --ssn=PATH to point at the SILSO series."
            );
            return out;
        }
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
///
/// The 90.0 fallback is a mid-cycle guess; the run says when it is used.
const SSN_FALLBACK: f64 = 90.0;

fn corpus_ssn(path: &str) -> f64 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "warning: cannot read the WSPR corpus from {path}: {e}
                 falling back to SSN {SSN_FALLBACK} for undated observations;                  pass --fit=PATH to point at the corpus."
            );
            return SSN_FALLBACK;
        }
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
        eprintln!("warning: {path} carries no SSN column; falling back to {SSN_FALLBACK}.");
        return SSN_FALLBACK;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The backends, in the order [`evaluate`] fills them and every table prints
/// them. One list, so a column heading cannot drift from the number under it.
const BACKENDS: [&str; 6] = [
    "grid",
    "climatology",
    "ccir (daily v2)",
    "ccir (R12 v2)",
    "ccir (R12 v1)",
    "ccir (daily v1)",
];

/// Which backends the per-cut tables show. `climatology` is dropped from them
/// because it is the function the grid was generated from and the headline
/// shows the two agreeing to a hundredth of a megahertz; carrying it through
/// every cut would be two columns of the same number.
const CUT_BACKENDS: [usize; 3] = [0, 4, 5];

/// Ratio of the SILSO version 2.0 sunspot number to the version 1 series it
/// replaced in July 2015. The recalibration multiplied the whole historical
/// record by about this factor.
///
/// `corpus/ssn_daily.tsv` is `SN_d_tot_V2.0.csv`, i.e. version 2, while the
/// CCIR maps were fitted in 1966 against R12 on the version 1 scale. Feeding a
/// version 2 number straight in is a unit error worth roughly a third of the
/// solar term.
///
/// This is a published conversion, not a fitted coefficient, and it is applied
/// to the driver rather than to the model's output.
const SSN_V2_OVER_V1: f64 = 1.44;

/// Every backend's foF2, for every observation, computed once.
fn evaluate(obs: &[Obs], grid: &Fof2Grid) -> Vec<Models> {
    // The grid collapses to a plane per (season, SSN) pair, and the corpus has
    // far fewer distinct pairs than observations; CCIR collapses to a plane per
    // (month, UT, R12). Neither is cached here - the planes are cheap next to
    // the parse - but the CCIR month is parsed once per process, by OnceLock.
    obs.iter()
        .map(|o| {
            let season = o.season();
            let plane = grid.plane(season, o.ssn);
            let lst = o.lst_h();
            let ccir = |r12: f64| {
                CcirMonth::bundled(o.month)
                    .expect("bundled CCIR coefficients parse")
                    .plane(o.utc_h, r12)
                    .fof2(o.lat, o.lon)
                    .expect("bundled modip grid parses")
            };
            Models {
                grid: plane.sample(o.lat, lst).0,
                clim: fof2::climatology_fof2(o.lat, lst, season, o.ssn),
                ccir: ccir(o.ssn),
                ccir_r12: ccir(o.r12),
                ccir_r12_v1: ccir(o.r12 / SSN_V2_OVER_V1),
                ccir_daily_v1: ccir(o.ssn / SSN_V2_OVER_V1),
            }
        })
        .collect()
}

fn cut(title: &str, note: &str, obs: &[Obs], models: &[Models], key: impl Fn(&Obs) -> String) {
    println!("\n--- {title} ---");
    println!("{note}");
    let mut cells: BTreeMap<String, [Err2; 6]> = BTreeMap::new();
    for (o, m) in obs.iter().zip(models) {
        let e = cells.entry(key(o)).or_default();
        for (acc, v) in e.iter_mut().zip(m.all()) {
            acc.push(v, o.fof2);
        }
    }
    print!("\n  {:<18} {:>7} {:>9}", "cut", "n", "measured");
    for b in CUT_BACKENDS {
        print!(" {:>19}", format!("{} bias/RMS", BACKENDS[b]));
    }
    println!();
    let mut thin = 0usize;
    for (k, e) in &mut cells {
        if e[0].n < MIN_QUOTABLE {
            thin += e[0].n;
            continue;
        }
        print!("  {:<18} {:>7} {:>9.2}", k, e[0].n, e[0].mean_measured());
        for b in CUT_BACKENDS {
            print!(" {:>+11.2} {:>7.2}", e[b].bias(), e[b].rms());
        }
        println!();
    }
    if thin > 0 {
        println!("  ({thin} observation(s) in cells below the {MIN_QUOTABLE} floor are not shown)");
    }
}

/// Width of the smoothing window for R12, in days: a centred twelve months.
const SMOOTH_WINDOW_DAYS: usize = 365;

/// Days since 1970-01-01 for a proleptic Gregorian date. Howard Hinnant's
/// `days_from_civil`, which is exact for every date in range and needs no
/// table; used only to make "within 182 days of" a subtraction.
fn day_number(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Centred mean of the daily sunspot series about a day, and how many days of
/// the window were available.
///
/// This is R12 only where the window is complete. Near the end of the
/// published series it is a truncated mean, biased on a sloping cycle, so the
/// count is returned and reported.
fn smoothed_ssn(by_day: &BTreeMap<i64, f64>, day: i64) -> (f64, usize) {
    #[allow(clippy::cast_possible_wrap)]
    let half = SMOOTH_WINDOW_DAYS as i64 / 2;
    let mut sum = 0.0;
    let mut n = 0usize;
    for (_, v) in by_day.range(day - half..=day + half) {
        sum += v;
        n += 1;
    }
    #[allow(clippy::cast_precision_loss)]
    if n == 0 {
        (0.0, 0)
    } else {
        (sum / n as f64, n)
    }
}
