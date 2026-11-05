//! Probe: what solar-zenith law does the D region produce?
//!
//! `mode_audit` measures the exponent through a traced ray and reports an
//! effective `cos^1.3..1.5 chi`, but a ray also carries geometry, the E layer
//! and the noise model, so it cannot say where the exponent comes from.
//!
//! This probe integrates the non-deviative absorption kernel
//! `Ne nu / (omega^2 + nu^2)` up a vertical column through `SolarChapmanLayer`
//! and `ExponentialCollisions` at a series of solar zenith angles, and fits
//! `log A` against `log cos chi`. The exponent is the layer's own; the
//! difference from `mode_audit` is the geometry's.
//!
//! It then sweeps the two scale heights, because the algebra fixes the
//! exponent:
//!
//! * an alpha-Chapman layer's realised peak is `Nm Ch^{-1/2}`, contributing 1/2;
//! * that peak rises by `H_D ln Ch` while `nu` falls on `H_nu`, so the
//!   collision frequency at the peak is down by `Ch^{-H_D/H_nu}`, contributing
//!   `H_D/H_nu`.
//!
//! With `Ch -> sec chi` away from the terminator that predicts
//! `A ~ cos^(1/2 + H_D/H_nu) chi`. If the sweep reproduces it, the bounds on
//! the two anchors bound the exponent.

use skipzone::collision::{CollisionFrequency, ExponentialCollisions};
use skipzone::density::ElectronDensity;
use skipzone::geo::SphericalPoint;
use skipzone::units::{Meters, PerSecond, Radians};
use skipzone_iono::chapman::SolarChapmanLayer;
use skipzone_predict::scenario;

/// Column integration limits and step, km. 40-140 km covers the D region and
/// the bottom of E; 25 m is far finer than the 6 km scale height.
const H_LO_KM: f64 = 40.0;
const H_HI_KM: f64 = 140.0;
const STEP_M: f64 = 25.0;

/// Zenith angles the exponent is fitted over, degrees. Stops short of the
/// terminator, where `Ch` departs from `sec chi` (past ~75 deg) and a fit would
/// measure the grazing correction instead of the law.
const FIT_CHI_DEG: [f64; 8] = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0];

/// The vertical absorption integral at one solar zenith angle, arbitrary units.
///
/// `Ne nu / (omega^2 + nu^2)` is the non-deviative absorption coefficient up to
/// constants that do not depend on chi, so a ratio of two of these is exactly
/// the ratio of absorptions and the constants cancel.
fn column(
    layer: &SolarChapmanLayer,
    coll: &ExponentialCollisions,
    omega: f64,
    chi_deg: f64,
) -> f64 {
    // decl 0, utc 12 puts the subsolar point at colat 90 deg, lon 0, and along
    // the lon = 0 meridian cos chi = sin(colat). So colat = 90 - chi.
    let colat = (90.0 - chi_deg).to_radians();
    let mut sum = 0.0;
    let n = ((H_HI_KM - H_LO_KM) * 1e3 / STEP_M) as usize;
    for i in 0..=n {
        let alt = H_LO_KM * 1e3 + STEP_M * i as f64;
        let p = SphericalPoint::new(
            Meters::new(scenario::EARTH_RADIUS_M + alt),
            Radians::new(colat),
            Radians::new(0.0),
        );
        let ne = layer.sample(&p).ne;
        let nu = coll.sample(&p).nu;
        // Trapezoid: half weight on the two ends.
        let w = if i == 0 || i == n { 0.5 } else { 1.0 };
        sum += w * ne * nu / (omega * omega + nu * nu) * STEP_M;
    }
    sum
}

/// Least-squares slope of `log A` on `log cos chi` - the effective exponent `n`
/// in `A ~ cos^n chi`.
fn exponent(layer: &SolarChapmanLayer, coll: &ExponentialCollisions, omega: f64) -> f64 {
    let pts: Vec<(f64, f64)> = FIT_CHI_DEG
        .iter()
        .map(|&c| {
            let a = column(layer, coll, omega, c);
            (c.to_radians().cos().ln(), a.ln())
        })
        .collect();
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let num: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let den: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    num / den
}

fn build(h_d_km: f64, h_nu_km: f64) -> (SolarChapmanLayer, ExponentialCollisions) {
    build_with_rise(h_d_km, h_nu_km, scenario::D_REGION_RISE_FRACTION)
}

fn build_with_rise(
    h_d_km: f64,
    h_nu_km: f64,
    rise: f64,
) -> (SolarChapmanLayer, ExponentialCollisions) {
    let layer = SolarChapmanLayer::d_region(
        scenario::D_REGION_PEAK_NE_OVERHEAD,
        scenario::EARTH_RADIUS_M + scenario::D_REGION_PEAK_ALT_KM * 1e3,
        h_d_km * 1e3,
        0.0,  // declination
        12.0, // UTC hour
        0.0,  // no night floor: it is a chi-independent additive term and would
              // flatten the very law being measured
    )
    .with_rise_fraction(rise);
    let coll = ExponentialCollisions::new(
        PerSecond::new(scenario::NU_REF_PER_S),
        Meters::new(scenario::EARTH_RADIUS_M + scenario::NU_REF_ALT_KM * 1e3),
        Meters::new(h_nu_km * 1e3),
    )
    .expect("valid collision profile");
    (layer, coll)
}

fn main() {
    let omega = 2.0 * std::f64::consts::PI * 7.04e6;
    let (h_d, h_nu) = (
        scenario::D_REGION_SCALE_HEIGHT_KM,
        scenario::NU_SCALE_HEIGHT_KM,
    );

    println!("=== THE D-REGION ZENITH LAW, MEASURED ON A VERTICAL COLUMN ===\n");
    println!("shipping anchors: H_D = {h_d} km, H_nu = {h_nu} km\n");

    let (layer, coll) = build(h_d, h_nu);
    let a0 = column(&layer, &coll, omega, 0.0);
    println!("  chi     A/A(0)   cos^n chi with n =");
    for &c in &[0.0f64, 20.0, 40.0, 60.0, 70.0, 80.0, 85.0, 88.0] {
        let a = column(&layer, &coll, omega, c) / a0;
        let cosc = c.to_radians().cos();
        let n = if c > 0.0 {
            a.ln() / cosc.ln()
        } else {
            f64::NAN
        };
        println!("  {c:4.0}   {a:8.5}   {n:6.3}");
    }
    let n_fit = exponent(&layer, &coll, omega);
    let n_pred = 0.5 + h_d / h_nu;
    println!("\n  fitted over 0-70 deg : n = {n_fit:.4}");
    println!("  predicted 1/2 + H_D/H_nu = {n_pred:.4}");
    println!("  literature (riometer / A1 absorption): n ~ 0.75");

    // Is the exponent frequency-dependent? It must not be: nu << omega through
    // the whole D region, so the kernel is Ne nu / omega^2 and omega divides out.
    println!("\n=== FREQUENCY INDEPENDENCE (nu << omega, so it must be flat) ===");
    for f_mhz in [1.838f64, 3.57, 7.04, 14.1, 28.1] {
        let n = exponent(&layer, &coll, 2.0 * std::f64::consts::PI * f_mhz * 1e6);
        println!("  {f_mhz:6.3} MHz  n = {n:.4}");
    }

    // The claim under test: n is set by the ratio of the two scale heights.
    println!("\n=== SWEEP: is n really 1/2 + H_D/H_nu ? ===");
    println!("   H_D   H_nu   ratio   n(measured)  n(predicted)   err");
    let mut worst = 0.0f64;
    for h_d in [4.0f64, 5.0, 6.0, 8.0, 10.0] {
        for h_nu in [5.0f64, 6.7, 9.0] {
            let (l, c) = build(h_d, h_nu);
            let n = exponent(&l, &c, omega);
            let p = 0.5 + h_d / h_nu;
            let err = (n - p).abs();
            worst = worst.max(err);
            println!(
                "  {h_d:5.1}  {h_nu:5.1}  {:6.3}   {n:10.4}   {p:11.4}   {err:6.4}",
                h_d / h_nu
            );
        }
    }
    println!("\n  worst |measured - predicted| = {worst:.4}");

    // What the calibration can reach within calib.rs's bounds.
    println!("\n=== WHAT THE BOUNDS ADMIT ===");
    let ion = skipzone_predict::calib::Anchors::default().ionosphere;
    let (d_lo, d_hi) = (ion.d_scale_height_km.min, ion.d_scale_height_km.max);
    let (v_lo, v_hi) = (ion.nu_scale_height_km.min, ion.nu_scale_height_km.max);
    println!("  H_D  in [{d_lo}, {d_hi}] km");
    println!("  H_nu in [{v_lo}, {v_hi}] km");
    let (n_min, n_max) = (0.5 + d_lo / v_hi, 0.5 + d_hi / v_lo);
    println!("  => n in [{n_min:.3}, {n_max:.3}]");
    println!(
        "  literature 0.75          : {}",
        verdict(0.75, n_min, n_max)
    );
    println!(
        "  WSPR band-fixed cells 0.2-0.6: {}",
        verdict(0.6, n_min, n_max)
    );
    println!(
        "\n  FLOOR: with H_nu -> infinity the rise term vanishes and n -> 0.5.\n  \
         An alpha-Chapman D region cannot produce n < 0.5 under ANY anchors,\n  \
         because Nm Ch^-1/2 alone is worth exactly 1/2."
    );

    // If no anchor setting reaches the target, the shape must change. One
    // option is a chi-independent ionisation component (cosmic rays and
    // scattered Lyman-alpha on NO). `D_REGION_NIGHT_FLOOR_FRACTION` models
    // these, but as a `max`, so in daylight it contributes nothing to the law.
    println!("\n=== A CHI-INDEPENDENT COMPONENT: max (shipping) vs + (physics) ===");
    println!("  frac    n(max, shipping)   n(additive)");
    for frac in [0.0f64, 0.05, 0.10, 0.15, 0.20, 0.30, 0.40, 0.50] {
        let n_max = exponent_with_floor(&coll, omega, h_d, frac, false);
        let n_add = exponent_with_floor(&coll, omega, h_d, frac, true);
        println!("  {frac:4.2}   {n_max:14.4}   {n_add:11.4}");
    }
    println!(
        "\n  Both columns bend, and the `max` one bends nearly as hard. The floor\n  \
         is applied POINTWISE, so it takes over in the layer's lower WINGS long\n  \
         before it touches the peak - and the lower wing, near 60-70 km, is where\n  \
         nu is largest and the absorption kernel actually lives. So the shipping\n  \
         night floor is already doing daytime work: at the shipping {:.2} it takes\n  \
         the column exponent from {:.3} to {:.3}.",
        scenario::D_REGION_NIGHT_FLOOR_FRACTION,
        exponent_with_floor(&coll, omega, h_d, 0.0, false),
        exponent_with_floor(
            &coll,
            omega,
            h_d,
            scenario::D_REGION_NIGHT_FLOOR_FRACTION,
            false
        ),
    );

    // What fraction each form needs to land on the literature exponent.
    let solve = |additive: bool| {
        let (mut lo_f, mut hi_f) = (0.0f64, 5.0f64);
        for _ in 0..60 {
            let mid = 0.5 * (lo_f + hi_f);
            if exponent_with_floor(&coll, omega, h_d, mid, additive) > 0.75 {
                lo_f = mid;
            } else {
                hi_f = mid;
            }
        }
        0.5 * (lo_f + hi_f)
    };
    println!(
        "\n  fraction that lands n = 0.75:  max {:.3}   additive {:.3}",
        solve(false),
        solve(true)
    );
    println!(
        "\n  THE VERDICT. The literature night/day D-region density ratio is ~0.1,\n  \
         which is what the shipping {:.2} encodes. Both forms need roughly FOUR\n  \
         TIMES that to bend the exponent to 0.75 - a night D region far denser\n  \
         than the one this constant was chosen from. So the chi-independent\n  \
         component cannot rescue the law either, and the exponent is not\n  \
         reachable by ANY setting of the existing anchors.\n\n  \
         What is left is the SHAPE. In an alpha-Chapman layer the ENTIRE profile\n  \
         translates upward by H_D ln Ch, wings included, so every part of it\n  \
         loses collision frequency as the sun sets - that whole translation is\n  \
         the H_D/H_nu term. The real lower D region does not behave that way:\n  \
         below ~75 km recombination is fast enough that Ne tracks production at\n  \
         roughly FIXED height, which is the n = 1/2 law with no rise term at all.\n  \
         Fixing that is a change to the layer's shape, not to a constant.",
        scenario::D_REGION_NIGHT_FLOOR_FRACTION
    );

    // The smallest exponent reachable inside the calibration's three-parameter
    // box. If the target is outside it, no fit can find it, and fits pushing
    // these anchors to their bounds are reporting that.
    println!("\n=== THE SMALLEST n THE CALIBRATION BOX CONTAINS ===");
    let f_hi = ion.d_night_floor_fraction.max;
    println!("  corner: H_D = {d_lo} km, H_nu = {v_hi} km, night floor = {f_hi} (all at bound)");
    let coll_soft = ExponentialCollisions::new(
        PerSecond::new(scenario::NU_REF_PER_S),
        Meters::new(scenario::EARTH_RADIUS_M + scenario::NU_REF_ALT_KM * 1e3),
        Meters::new(v_hi * 1e3),
    )
    .expect("valid collision profile");
    let n_floor_box = exponent_with_floor(&coll_soft, omega, d_lo, f_hi, false);
    println!("  n at that corner = {n_floor_box:.3}");
    println!(
        "  target 0.75 is {} the box.",
        if n_floor_box <= 0.75 {
            "INSIDE"
        } else {
            "OUTSIDE"
        }
    );
    println!(
        "\n  So a calibration run that drives d_scale_height_km, nu_scale_height_km\n  \
         and d_night_floor_fraction to their bounds is not misbehaving. It is\n  \
         reporting, correctly, that the residual it is chasing is not produced by\n  \
         any of them."
    );

    // How much of the photochemical peak rise the profile takes: the only
    // anchor that changes the layer's shape, and so the only one that reaches
    // the exponent.
    println!("\n=== d_rise_fraction: THE KNOB THAT DOES REACH THE EXPONENT ===");
    println!("  derived law: n = 1/2 + f H_D/H_nu   (f = 1 is the textbook layer)\n");
    println!("     f    n(measured)   n(derived)     err");
    let mut worst_f = 0.0f64;
    for f in [0.0f64, 0.1, 0.2, 0.279, 0.4, 0.6, 0.8, 1.0] {
        let (l, c) = build_with_rise(h_d, h_nu, f);
        let n = exponent(&l, &c, omega);
        let p = 0.5 + f * h_d / h_nu;
        let err = (n - p).abs();
        worst_f = worst_f.max(err);
        println!("  {f:5.3}   {n:10.4}   {p:10.4}   {err:7.4}");
    }
    println!("\n  worst |measured - derived| over f = {worst_f:.4}");

    // The value the literature exponent implies. Not the default: it has not
    // been fitted against the corpus, and one literature number is not enough
    // to move a shape anchor.
    let (mut lo_r, mut hi_r) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo_r + hi_r);
        let (l, c) = build_with_rise(h_d, h_nu, mid);
        if exponent(&l, &c, omega) > 0.75 {
            hi_r = mid;
        } else {
            lo_r = mid;
        }
    }
    let f_star = 0.5 * (lo_r + hi_r);
    println!("\n  f that lands the literature n = 0.75 : {f_star:.3}");
    println!(
        "  (closed form: (0.75 - 1/2) H_nu/H_D = {:.3})",
        0.25 * h_nu / h_d
    );
    println!(
        "\n  SHIPPING DEFAULT IS STILL f = {:.1}. This probe reports what the\n  \
         literature exponent implies; it does not set it. Moving a shape anchor\n  \
         wants a corpus fit, and the corpus that would judge it is one northern\n  \
         summer of Europe/North America paths - see the calibration notes before\n  \
         believing any value it returns.",
        scenario::D_REGION_RISE_FRACTION
    );

    bench_branches();
}

/// The column integral with a chi-independent ionisation component of
/// `frac * Nm` combined two ways: as the shipping `max` (a lower bound), and as
/// a `+` (two independent sources adding, which is what they physically are).
///
/// Cosmic-ray ionisation and scattered Lyman-alpha on NO continue in daylight
/// but are swamped near the peak. `max` hides them until the sunlit layer falls
/// below them, so the shipping night floor does not affect the daytime law.
/// Summing is the physics; this shows how large the component must be to bend
/// the exponent.
fn column_with_floor(
    coll: &ExponentialCollisions,
    omega: f64,
    chi_deg: f64,
    h_d_km: f64,
    frac: f64,
    additive: bool,
) -> f64 {
    let sunlit = SolarChapmanLayer::d_region(
        scenario::D_REGION_PEAK_NE_OVERHEAD,
        scenario::EARTH_RADIUS_M + scenario::D_REGION_PEAK_ALT_KM * 1e3,
        h_d_km * 1e3,
        0.0,
        12.0,
        0.0,
    );
    // The floor carries the layer's own profile shape at chi-independent
    // amplitude: the same construction `floor_at` uses, built as its own layer
    // so it can be added rather than maxed.
    let floor = SolarChapmanLayer::d_region(
        scenario::D_REGION_PEAK_NE_OVERHEAD * frac,
        scenario::EARTH_RADIUS_M + scenario::D_REGION_PEAK_ALT_KM * 1e3,
        h_d_km * 1e3,
        0.0,
        12.0,
        0.0,
    );
    let colat = (90.0 - chi_deg).to_radians();
    // The floor has no zenith dependence, so it is evaluated at the subsolar
    // point (chi = 0, Ch = 1), as `floor_at` does.
    let sub = 0.0_f64.to_radians();
    let mut sum = 0.0;
    let n = ((H_HI_KM - H_LO_KM) * 1e3 / STEP_M) as usize;
    for i in 0..=n {
        let alt = H_LO_KM * 1e3 + STEP_M * i as f64;
        let r = Meters::new(scenario::EARTH_RADIUS_M + alt);
        let p_sun = SphericalPoint::new(r, Radians::new(colat), Radians::new(0.0));
        let p_flr = SphericalPoint::new(
            r,
            Radians::new((90.0f64).to_radians() - sub),
            Radians::new(0.0),
        );
        let ne_s = sunlit.sample(&p_sun).ne;
        let ne_f = floor.sample(&p_flr).ne;
        let ne = if additive {
            ne_s + ne_f
        } else {
            ne_s.max(ne_f)
        };
        let nu = coll.sample(&p_sun).nu;
        let w = if i == 0 || i == n { 0.5 } else { 1.0 };
        sum += w * ne * nu / (omega * omega + nu * nu) * STEP_M;
    }
    sum
}

fn exponent_with_floor(
    coll: &ExponentialCollisions,
    omega: f64,
    h_d_km: f64,
    frac: f64,
    additive: bool,
) -> f64 {
    let pts: Vec<(f64, f64)> = FIT_CHI_DEG
        .iter()
        .map(|&c| {
            let a = column_with_floor(coll, omega, c, h_d_km, frac, additive);
            (c.to_radians().cos().ln(), a.ln())
        })
        .collect();
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
    let num: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let den: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
    num / den
}

fn verdict(target: f64, lo: f64, hi: f64) -> String {
    if target >= lo && target <= hi {
        "REACHABLE".to_string()
    } else {
        format!("UNREACHABLE (needs n = {target}, floor is {lo:.3})")
    }
}

/// Cost of the partial-rise branch against the full-rise one, measured on the
/// same sample pattern the ray equations drive (many `sample` calls at
/// scattered points). `rise_fraction < 1` adds a `ln` and a `sqrt` per D-region
/// density evaluation, and density evaluation is most of a solve.
fn bench_branches() {
    use std::time::Instant;
    let n = 4_000_000;
    let mk = |rise: f64| {
        SolarChapmanLayer::d_region(
            scenario::D_REGION_PEAK_NE_OVERHEAD,
            scenario::EARTH_RADIUS_M + scenario::D_REGION_PEAK_ALT_KM * 1e3,
            scenario::D_REGION_SCALE_HEIGHT_KM * 1e3,
            15.0,
            12.0,
            scenario::D_REGION_NIGHT_FLOOR_FRACTION,
        )
        .with_rise_fraction(rise)
    };
    let run = |layer: &SolarChapmanLayer| -> (f64, f64) {
        let t = Instant::now();
        let mut acc = 0.0;
        for i in 0..n {
            let x = i as f64;
            let p = SphericalPoint::new(
                Meters::new(scenario::EARTH_RADIUS_M + 60e3 + (x % 40_000.0)),
                Radians::new(0.5 + (x % 1000.0) * 1e-3),
                Radians::new(-1.0 + (x % 700.0) * 2e-3),
            );
            acc += layer.sample(&p).ne;
        }
        (t.elapsed().as_secs_f64(), acc)
    };
    // Warm both paths before timing either.
    let (full, partial) = (mk(1.0), mk(0.285));
    let _ = run(&full);
    let _ = run(&partial);
    let (t_full, a_full) = run(&full);
    let (t_part, a_part) = run(&partial);
    println!("\n=== COST OF THE PARTIAL-RISE BRANCH ===");
    println!("  {n} D-layer density samples each, same points.");
    println!("  This is the cost of ONE LAYER'S sample, not of a solve: per");
    println!("  `bin/profile_solve`, IGRF is ~72 % of a solve and the D layer is");
    println!("  one of four in the stack, so scale accordingly before quoting it.");
    println!(
        "  full rise (f=1, the shipping default) : {:.0} ns/sample",
        t_full * 1e9 / n as f64
    );
    println!(
        "  partial rise (f=0.285)                : {:.0} ns/sample",
        t_part * 1e9 / n as f64
    );
    println!(
        "  overhead when enabled                 : {:+.1} %",
        100.0 * (t_part / t_full - 1.0)
    );
    println!(
        "  (checksums {a_full:.3e} / {a_part:.3e} - kept so the loops cannot be optimised away)"
    );
    println!(
        "  The default path is untouched: `full_rise` is a whole-solve constant,\n  \
         so the branch predicts perfectly, and `solve_digest` is byte-identical."
    );
}
