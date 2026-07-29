//! Ray-tracing helpers that drive the engine's tracer and homing to build
//! full multi-hop solutions and, when nothing homes, a near-miss report.
//! Calls the engine's public API only; no physics is implemented here.

use rayon::prelude::*;

use skipzone::collision::CollisionFrequency;
use skipzone::density::ElectronDensity;
use skipzone::error::TraceError;
use skipzone::geo::{SphericalPoint, bearing, central_angle, track_errors};
use skipzone::homing::HomingConfig;
use skipzone::mag::MagneticField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};

use crate::noise::{LinkBudget, LinkSettings};
use crate::scenario::{Assumptions, EARTH_RADIUS_M, GroundType, to_lat_lon};

use super::link_budget::{NEPERS_TO_DB, free_space_loss_db, ground_reflection_loss_db};
use super::types::{HopDetail, LayerMode, NearMiss, Solution, mode_label};

/// Cap on drawn points per hop; the ray polyline is decimated to this.
const MAX_POLY_POINTS: usize = 400;

/// Practical homing miss tolerance for interactive HF prediction, m. The
/// engine default (30 m) is set for its own validation and, near a skip-zone
/// edge / caustic, the bisection legitimately stalls at a few hundred metres
/// after the iteration budget - a miss that is already a match for any HF use
/// (< ~0.1 % of path length). Accepting it here turns those "practically a
/// match" cases into the connections they are, instead of reporting no path.
const PRACTICAL_MISS_TOLERANCE_M: f64 = 2000.0;

/// How the surface at a ground reflection is decided for this solve.
///
/// `Fixed` is the historical behaviour and stays bit-for-bit what it was: one
/// operator-chosen surface applied to every bounce. `Auto` defers the choice to
/// the reflection point's own position, hop by hop.
#[derive(Clone, Copy)]
pub(super) enum GroundModel {
    Fixed(GroundType),
    Auto { land_fallback: GroundType },
}

impl GroundModel {
    /// The surface at one reflection point, and (for auto-detection only) the
    /// reason it was picked, so the per-hop readout can justify itself.
    fn at(self, lat: f64, lon: f64) -> (GroundType, Option<String>) {
        match self {
            Self::Fixed(g) => (g, None),
            Self::Auto { land_fallback } => match crate::coastline::get() {
                Ok(c) => {
                    let pick = c.classify(lat, lon, land_fallback);
                    (pick.ground, Some(pick.reason))
                }
                // A missing or unreadable dataset must not sink the solve: fall
                // back to the operator's land choice and say so, loudly, on the
                // hop itself.
                Err(e) => (
                    land_fallback,
                    Some(format!(
                        "coastline data unavailable ({e}); used the land fallback"
                    )),
                ),
            },
        }
    }
}

struct Captured {
    result: skipzone::trace::TraceResult,
    polyline: Vec<(f64, f64)>,
    apex_lat_lon: (f64, f64),
    apex_alt_km: f64,
}

fn decimate(points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if points.len() <= MAX_POLY_POINTS {
        return points;
    }
    let stride = points.len().div_ceil(MAX_POLY_POINTS);
    let mut out: Vec<(f64, f64)> = points.iter().step_by(stride).copied().collect();
    if let Some(last) = points.last()
        && out.last() != Some(last)
    {
        out.push(*last);
    }
    out
}

/// Trace one hop, capturing the ground-track polyline and apex position from
/// the engine's own accepted steps via `trace_with_observer`.
fn trace_capture<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    start: &SphericalPoint,
    elev: Radians,
    az: Radians,
) -> Result<Captured, TraceError>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let mut polyline: Vec<(f64, f64)> = Vec::new();
    let start_ll = to_lat_lon(start);
    polyline.push(start_ll);
    let mut apex_alt = f64::NEG_INFINITY;
    let mut apex_ll = start_ll;

    let result = {
        let mut observer = |_sigma: f64, y: &[f64; 10]| {
            let lat = 90.0 - y[1].to_degrees();
            let lon = ((y[2].to_degrees() + 180.0).rem_euclid(360.0)) - 180.0;
            polyline.push((lat, lon));
            let alt = y[0] - EARTH_RADIUS_M;
            if alt > apex_alt {
                apex_alt = alt;
                apex_ll = (lat, lon);
            }
        };
        tracer.trace_with_observer(start, elev, az, &mut observer)?
    };
    polyline.push(to_lat_lon(&result.end));

    Ok(Captured {
        result,
        polyline: decimate(polyline),
        apex_lat_lon: apex_ll,
        apex_alt_km: if apex_alt.is_finite() {
            apex_alt / 1e3
        } else {
            f64::NAN
        },
    })
}

/// Launch angles for the next hop after a specular ground reflection: the
/// horizontal momentum is unchanged and the radial component flips sign.
fn reflect(end_m: [f64; 3]) -> (Radians, Radians) {
    let n = (end_m[0] * end_m[0] + end_m[1] * end_m[1] + end_m[2] * end_m[2]).sqrt();
    let up = -end_m[0];
    let elev = (up / n).clamp(-1.0, 1.0).asin();
    let az = end_m[2].atan2(-end_m[1]);
    (Radians::new(elev), Radians::new(az))
}

/// Arrival angles at a landing point, from the downgoing momentum.
fn arrival_angles(end_m: [f64; 3]) -> (f64, f64) {
    let n = (end_m[0] * end_m[0] + end_m[1] * end_m[1] + end_m[2] * end_m[2]).sqrt();
    // Elevation below horizontal, reported positive.
    let elev = (-end_m[0] / n).clamp(-1.0, 1.0).asin().to_degrees();
    let az = end_m[2].atan2(-end_m[1]).to_degrees().rem_euclid(360.0);
    (elev, az)
}

fn outcome_label(o: Outcome) -> &'static str {
    match o {
        Outcome::Landed => "Landed",
        Outcome::Escaped => "Escaped (penetrated)",
    }
}

/// Step-control settings for one density stack.
///
/// The deterministic stack uses the engine's defaults and is unchanged. The
/// sporadic-E stack cannot: the engine's `QuasiParabolicLayer` is C0 with a
/// documented GRADIENT KINK at each of its zeros, and an Es sheet is only about
/// 3 km thick, so at the default `rtol` of 1e-10 the step controller refines
/// into that kink until it hits `min_step` and the trace fails outright with
/// "step collapsed". Measured over an elevation sweep on a real Es geometry
/// (18.1 MHz, foEs 8.7 MHz, 400 km):
///
/// | rtol  | step cap | landings | trace failures | max drift |
/// |-------|----------|----------|----------------|-----------|
/// | 1e-10 | 25 km    | 0        | 28             | -         |
/// | 1e-10 | 1.5 km   | 0        | 73             | -         |
/// | 1e-9  | 1.5 km   | 20       | 3              | 4.4e-7    |
/// | 3e-9  | 1.5 km   | 23       | 0              | 1.1e-6    |
/// | 1e-8  | 1.5 km   | 23       | 0              | 3.5e-6    |
///
/// So the Es stack takes `rtol = 3e-9` (the loosest-but-one that clears every
/// failure) and a step cap below the sheet thickness, which stops a single step
/// straddling the whole layer unseen. Both are settings on the engine's own
/// public `TraceConfig`; no physics is changed, and the resulting Hamiltonian
/// drift is still reported per solution so the cost is visible rather than
/// assumed away.
#[derive(Clone, Copy)]
pub(super) struct StepTuning {
    pub rtol: Option<f64>,
    pub max_step_m: Option<f64>,
    /// Measure the on-shell drift diagnostic. Only the traces whose numbers are
    /// reported need it; see `TraceConfig::measure_drift`.
    pub measure_drift: bool,
}

impl StepTuning {
    /// The engine's defaults, untouched.
    pub const DEFAULT: Self = Self {
        rtol: None,
        max_step_m: None,
        measure_drift: true,
    };

    /// Relative tolerance the sporadic-E stack needs; see the type docs.
    pub const ES_RTOL: f64 = 3e-9;

    /// Relative tolerance for the BRACKETING scan only.
    ///
    /// The scan does not produce any reported number. Its entire output is a
    /// set of one-degree elevation intervals over which the along-track error
    /// changes sign; everything the operator sees comes from the engine's
    /// refinement and from `propagate`, both of which stay at the engine
    /// default. So the scan can run as loosely as it can while still choosing
    /// the same intervals, and 1e-8 is measured (three geometries, every hop
    /// count, `check_scan_rtol`) to choose exactly the intervals the 1e-10 scan
    /// does, with landing ranges shifted by at most 0.5 m over paths of
    /// 2500-7500 km. For the sign of `range - target` to flip at that margin the
    /// target would have to fall within half a metre of a scanned ray's landing
    /// point; the range-vs-elevation slope there is >100 km per degree.
    pub const SCAN_RTOL: f64 = 1e-8;

    /// Settings for a stack containing a sheet of the given semi-thickness.
    pub fn for_thin_sheet(semi_thickness_m: f64) -> Self {
        Self {
            rtol: Some(Self::ES_RTOL),
            max_step_m: Some(semi_thickness_m),
            measure_drift: true,
        }
    }

    /// The same settings, relaxed for the bracketing scan. A stack that already
    /// names its own tolerance keeps it: the sporadic-E stack's 3e-9 is chosen
    /// against a documented failure mode of that stack, not for accuracy, and
    /// is looser than `SCAN_RTOL` anyway. The step cap always survives - it is
    /// there so no single step straddles a thin sheet unseen, which the scan
    /// needs just as much as the refinement does.
    pub fn for_scan(self) -> Self {
        Self {
            rtol: Some(
                self.rtol
                    .map_or(Self::SCAN_RTOL, |r| r.max(Self::SCAN_RTOL)),
            ),
            max_step_m: self.max_step_m,
            // A scanned ray is thrown away as soon as its landing range has
            // been read off it, so its integrator-quality diagnostic has no
            // reader.
            measure_drift: false,
        }
    }

    /// Relative tolerance for the terminal homing SEARCH.
    ///
    /// The engine's 1e-10 default resolves a landing point to about 10 cm over
    /// a 1000 km path. The search does not need that: it is looking for a
    /// launch elevation, and it is followed by a polish pass on the reporting
    /// tracer at full tolerance, so its only job is to land in the polish
    /// pass's basin of convergence. 1e-8 still resolves the landing to metres -
    /// three orders inside the kilometre-scale acceptance bound - while the
    /// step count falls as roughly `rtol^(-1/5)`.
    pub const SEARCH_RTOL: f64 = 1e-8;

    /// Settings for the traces the terminal homing search throws away: looser
    /// tolerance, no drift diagnostic. A stack that names its own tolerance
    /// keeps it, for the reasons in [`Self::for_scan`].
    pub fn for_search(self) -> Self {
        Self {
            rtol: Some(
                self.rtol
                    .map_or(Self::SEARCH_RTOL, |r| r.max(Self::SEARCH_RTOL)),
            ),
            max_step_m: self.max_step_m,
            measure_drift: false,
        }
    }
}

/// A tracer over one density stack. The density is passed in rather than read
/// off `Models` because the solver now runs the same homing against two stacks
/// (with and without a sporadic-E sheet) while the field and collision models
/// are shared between them.
pub(super) type DynTracer<'a> = Tracer<
    'a,
    dyn ElectronDensity + Sync + 'a,
    dyn MagneticField + Sync + 'a,
    dyn CollisionFrequency + Sync + 'a,
>;

pub(super) fn make_tracer<'a>(
    density: &'a (dyn ElectronDensity + Sync + 'a),
    field: &'a (dyn MagneticField + Sync + 'a),
    collisions: &'a (dyn CollisionFrequency + Sync + 'a),
    freq_mhz: f64,
    mode: Mode,
    a: &Assumptions,
    tuning: StepTuning,
) -> DynTracer<'a> {
    let mut config = TraceConfig::new(Meters::new(a.r_ground_m), Meters::new(a.r_top_m));
    if let Some(rtol) = tuning.rtol {
        config.rtol = rtol;
    }
    if let Some(cap) = tuning.max_step_m {
        config.max_step = config.max_step.min(cap);
    }
    config.measure_drift = tuning.measure_drift;
    Tracer::new(
        density,
        field,
        collisions,
        Hertz::new(freq_mhz * 1e6),
        mode,
        config,
    )
}

/// How close the END of the whole path must come to the receiver to count as a
/// connection, m.
///
/// Scaled with path length rather than fixed, because the quantity being homed
/// is the accumulated result of N hops: the same launch precision that puts a
/// 400 km path within metres puts a 10 000 km one within a few km. The floor is
/// the single-hop [`PRACTICAL_MISS_TOLERANCE_M`], so a multi-hop path is never
/// held to a tighter standard than the one-hop homing that seeds it, and the
/// 0.05 % term keeps a long path's allowance proportionate (3.8 km at 7500 km).
/// Either way this is orders of magnitude below what used to get through: paths
/// missing by 800-1500 km were being reported as connections.
pub(super) fn terminal_tolerance_m(great_circle_km: f64) -> f64 {
    PRACTICAL_MISS_TOLERANCE_M.max(0.000_5 * great_circle_km * 1e3)
}

/// Iteration budget for terminal homing. The terminal landing point is a
/// smooth, near-linear function of launch elevation away from caustics, so a
/// secant converges in a handful of steps or not at all.
const TERMINAL_MAX_ITERS: usize = 12;
/// Finite-difference step for the terminal slope, rad. Same scale as the
/// engine's own Jacobian step: large enough to sit well above landing-position
/// noise, small enough that the map is linear across it.
const TERMINAL_FD_STEP: f64 = 1e-4;
/// Largest elevation correction accepted in one secant step, rad (~1 deg). Near
/// the maximum range of a layer the terminal map steepens sharply and an
/// undamped secant will throw the launch outside the physical range entirely.
const TERMINAL_MAX_CORRECTION: f64 = 0.017_453_292_519_943_295;

/// A launch that puts the END of the whole multi-hop path on the receiver.
pub(super) struct TerminalHomed {
    pub elevation: Radians,
    pub azimuth: Radians,
}

/// Where an N-hop path launched at `(elev, az)` actually ends, or `None` if it
/// never got there - a hop escaped through the top of the domain, or a trace
/// failed. Deliberately lighter than [`propagate`]: no polyline, no per-hop
/// record, no ground lookups, because this runs inside a search loop.
fn propagate_terminal<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    elev: Radians,
    az: Radians,
    hops: u32,
) -> Option<SphericalPoint>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let mut point = *tx;
    let (mut e, mut a) = (elev, az);
    for _ in 0..hops {
        let res = tracer.trace(&point, e, a).ok()?;
        if res.outcome != Outcome::Landed {
            return None;
        }
        let (ne, na) = reflect(res.end_m);
        e = ne;
        a = na;
        point = res.end;
    }
    Some(point)
}

/// Along-track and cross-track error of the terminal point, in radians.
fn terminal_errors<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    rx: &SphericalPoint,
    hops: u32,
    elev: f64,
    az: f64,
) -> Option<(f64, f64)>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let end = propagate_terminal(tracer, tx, Radians::new(elev), Radians::new(az), hops)?;
    let track = bearing(tx, rx);
    let target = central_angle(tx, rx).get();
    let (along, cross) = track_errors(tx, track, &end);
    Some((along.get() - target, cross.get()))
}

/// Drive the END of the whole multi-hop path onto the receiver.
///
/// The engine's homing solves ONE hop. For an N-hop path the app was homing a
/// single hop to 1/N of the great-circle arc and then propagating N hops by
/// specular reflection, assuming the remaining N-1 hops would repeat the first.
/// In a horizontally varying ionosphere they do not. Each ground reflection
/// returns the ray at a slightly different elevation than it left, the error
/// compounds, and the measured consequences were severe: paths landing 800 to
/// 1500 km from the receiver and still being reported as connections, and - near
/// a layer's maximum range, where d(range)/d(elevation) is enormous - paths
/// whose third hop flattened out until it penetrated the layer and left the
/// planet. The equal-hop miss was recorded as `terminal_miss_km` and shown, but
/// nothing acted on it.
///
/// So the app homes what it actually cares about: the terminal point. The
/// single-hop solution seeds it, and a damped secant on launch elevation
/// (against the along-track error of the FULL path) plus the engine's own
/// cross-track azimuth correction closes it. Each evaluation is N traces with
/// no allocation; the polyline-building [`propagate`] runs once, at the end, on
/// the launch this converged to.
pub(super) fn home_terminal<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    rx: &SphericalPoint,
    hops: u32,
    seed_elev: f64,
    limits: (f64, f64),
    tolerance_m: f64,
) -> Option<TerminalHomed>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let r0 = tx.r.get();
    let (mut e, mut az) = (seed_elev, bearing(tx, rx).get());
    // The search is bounded by the SCANNED elevation range, not by the bracket.
    // The bracket comes from the equal-hop assumption, and the whole point of
    // homing the terminal point is that that assumption is wrong: on a
    // Denver-Buenos Aires path the true hops run 2533/2392/2357/2266 km, and
    // the launch elevation that lands the last one on the receiver sits outside
    // the interval the equal-hop scan bracketed. Clamping to the bracket threw
    // exactly those solutions away. The scan limits are a real bound - outside
    // them the app does not model the geometry at all - so they are the ones to
    // hold the secant to.
    let (lo, hi) = limits;
    // Converge comfortably inside the acceptance bound rather than stopping the
    // moment it is met - a path that merely scrapes in is not the same as one
    // that lands on the receiver - but no further. Chasing the last few metres
    // is what this loop was doing before, and it cost the full iteration budget
    // on paths that were already inside a hundred metres of a two-kilometre
    // bound. Each of those iterations is a full N-hop propagation on the
    // critical path of the whole solve.
    let tight = tolerance_m * 0.05;
    let mut best: Option<(f64, f64, f64)> = None;
    // The previous iterate, which is the second point of the secant. Only the
    // FIRST step has to spend a finite-difference probe to get a slope; after
    // that the point the last step already paid for supplies it, halving the
    // full N-hop propagations this loop performs.
    let mut prev: Option<(f64, f64)> = None;
    for _ in 0..TERMINAL_MAX_ITERS {
        let Some((along, cross)) = terminal_errors(tracer, tx, rx, hops, e, az) else {
            // The trial launch no longer completes the path. Retreat halfway to
            // the best launch found so far; with nothing to retreat to, this
            // ray simply has no complete path near it.
            let (be, ba, _) = best?;
            e = 0.5 * (e + be);
            az = 0.5 * (az + ba);
            prev = None;
            continue;
        };
        let miss = along.hypot(cross) * r0;
        let improved = best.is_none_or(|(_, _, m)| miss < 0.7 * m);
        if best.is_none_or(|(_, _, m)| miss < m) {
            best = Some((e, az, miss));
        }
        // Stop when it is as close as it is going to get, or when a further
        // step is no longer buying anything.
        if miss < tight || (!improved && best.is_some_and(|(_, _, m)| m < tolerance_m)) {
            break;
        }
        // Slope of the terminal along-track error against launch elevation:
        // from the previous iterate when there is one and it is far enough away
        // to divide by, otherwise from a probe on whichever side still
        // completes the path.
        let slope = match prev {
            Some((pe, pa)) if (e - pe).abs() > TERMINAL_FD_STEP => (along - pa) / (e - pe),
            _ => {
                let h = TERMINAL_FD_STEP;
                match terminal_errors(tracer, tx, rx, hops, e + h, az) {
                    Some((up, _)) => (up - along) / h,
                    None => {
                        let (down, _) = terminal_errors(tracer, tx, rx, hops, e - h, az)?;
                        (along - down) / h
                    }
                }
            }
        };
        if !slope.is_finite() || slope == 0.0 {
            break;
        }
        prev = Some((e, along));
        e = (e + (-along / slope).clamp(-TERMINAL_MAX_CORRECTION, TERMINAL_MAX_CORRECTION))
            .clamp(lo, hi);
        // Exact for a spherically symmetric medium, first order otherwise -
        // the same correction the engine's own refinement applies.
        az -= cross;
    }

    let (e, az, miss_m) = best?;
    (miss_m < tolerance_m).then_some(TerminalHomed {
        elevation: Radians::new(e),
        azimuth: Radians::new(az),
    })
}

/// One elevation scan, traced once and reused for every hop count.
///
/// The engine's `Homing::home_scan` opens with an elevation scan at the
/// great-circle bearing to find the intervals where the along-track error
/// changes sign. That scan does not depend on the hop count: the launch point,
/// the launch bearing, the tracer and the scanned elevations are the same for a
/// 1-hop target as for a 4-hop one, and only the target arc that the landing
/// ranges are COMPARED against differs. Calling `home_scan` once per hop count
/// therefore re-traced the identical fan of rays `max_hops` times - measured at
/// 77 traces each, 74 % of all the tracing a solve did.
///
/// This type traces the fan once and then answers "which elevation intervals
/// bracket THIS target arc" arithmetically. Each interval it reports is one
/// candidate ray, handed to [`home_terminal`] to be refined against the end of
/// the whole multi-hop path.
pub(super) struct ElevationScan {
    /// `(elevation, along-track angle)` per scanned elevation, in scan order.
    /// The angle is `None` where the ray escaped or the trace failed - which is
    /// how the engine treats those too: a boundary, not an error.
    points: Vec<(f64, Option<f64>)>,
}

impl ElevationScan {
    /// Elevation intervals over which the along-track error changes sign for a
    /// target at `target_arc` radians. Mirrors the engine's `scan_brackets`
    /// bracketing rule exactly, on rays that were already traced.
    pub fn brackets(&self, target_arc: f64) -> Vec<(f64, f64)> {
        let mut brackets = Vec::new();
        let mut prev: Option<(f64, f64)> = None;
        for &(e, along) in &self.points {
            let here = along.map(|a| (e, a - target_arc));
            if let (Some((pe, pv)), Some((ce, cv))) = (prev, here)
                && pv.signum() != cv.signum()
            {
                brackets.push((pe, ce));
            }
            prev = here;
        }
        brackets
    }
}

/// Trace the elevation fan once for one tracer. Reproduces the engine's scan
/// loop - same start, same `while e <= elev_max` accumulation, same treatment
/// of escapes and failures - so the intervals it reports are the ones the
/// engine's own scan would have reported.
pub(super) fn scan_elevations<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    from: &SphericalPoint,
    to: &SphericalPoint,
    config: &HomingConfig,
) -> ElevationScan
where
    D: ElectronDensity + Sync + ?Sized,
    B: MagneticField + Sync + ?Sized,
    C: CollisionFrequency + Sync + ?Sized,
{
    let az0 = bearing(from, to);
    let (e0, e1, de) = (
        config.elev_min.get(),
        config.elev_max.get(),
        config.elev_step.get(),
    );
    // The elevations are enumerated by the same accumulation the engine's own
    // scan uses, so the values handed back are the ones its narrowed re-scan
    // would produce; only the tracing of them is fanned out. Rays are entirely
    // independent - the tracer holds no mutable state - and `map` preserves
    // input order, so the scan is bit-for-bit what the serial loop produced.
    let mut elevations = Vec::new();
    let mut e = e0;
    while e <= e1 {
        elevations.push(e);
        e += de;
    }
    let points = elevations
        .par_iter()
        .map(|&e| {
            let along = match tracer.trace(from, Radians::new(e), az0) {
                Ok(res) if res.outcome == Outcome::Landed => {
                    Some(track_errors(from, az0, &res.end).0.get())
                }
                Ok(_) | Err(_) => None,
            };
            (e, along)
        })
        .collect();
    ElevationScan { points }
}

pub(super) fn homing_config(use_field: bool) -> HomingConfig {
    let mut c = HomingConfig {
        miss_tolerance_m: PRACTICAL_MISS_TOLERANCE_M,
        ..HomingConfig::default()
    };
    // Without a field the near-vertical Spitze cannot occur, so the scan can
    // reach NVIS geometries. With a field, keep the engine's default cap.
    if !use_field {
        c.elev_max = Radians::from_degrees(88.0);
    }
    c
}

/// Propagate `hops` hops from `tx` at the given launch angles, reflecting
/// specularly off the ground between them.
pub(super) fn propagate<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    elev: Radians,
    az: Radians,
    hops: u32,
    f_hz: f64,
    ground: GroundModel,
) -> (Vec<HopDetail>, Vec<SphericalPoint>, Option<String>)
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let mut details = Vec::new();
    let mut ends = Vec::new();
    let mut point = *tx;
    let (mut e, mut a) = (elev, az);

    for i in 0..hops {
        let cap = match trace_capture(tracer, &point, e, a) {
            Ok(c) => c,
            Err(err) => {
                return (details, ends, Some(format!("hop {} failed: {err}", i + 1)));
            }
        };
        let res = &cap.result;
        let (arr_elev, arr_az) = arrival_angles(res.end_m);
        // A ground reflection happens where this hop lands only if another hop
        // follows; the final hop arrives at the receiver with no reflection.
        // The surface is looked up at the landing point, so with auto-detection
        // each bounce gets the ground it actually happens over.
        let end_lat_lon = to_lat_lon(&res.end);
        let reflects = res.outcome == Outcome::Landed && i + 1 < hops;
        let (ground_type, ground_reason) = if reflects {
            let (g, why) = ground.at(end_lat_lon.0, end_lat_lon.1);
            (Some(g), why)
        } else {
            (None, None)
        };
        let ground_loss_db = match ground_type {
            Some(g) => {
                let (eps_r, sigma) = g.constants();
                ground_reflection_loss_db(arr_elev.to_radians(), f_hz, eps_r, sigma)
            }
            None => 0.0,
        };
        let range_km = central_angle(&point, &res.end).get() * EARTH_RADIUS_M / 1e3;
        let apex_x = res.apexes.first().map_or(f64::NAN, |ap| ap.x);
        let apex_from_engine = res
            .apexes
            .first()
            .map_or(cap.apex_alt_km, |ap| (ap.r.get() - EARTH_RADIUS_M) / 1e3);

        details.push(HopDetail {
            index: i + 1,
            launch_elev_deg: e.to_degrees(),
            launch_az_deg: a.to_degrees().rem_euclid(360.0),
            arrival_elev_deg: arr_elev,
            arrival_az_deg: arr_az,
            apex_alt_km: apex_from_engine,
            apex_x,
            apex_lat_lon: cap.apex_lat_lon,
            ground_range_km: range_km,
            group_km: res.group_path.get() / 1e3,
            phase_km: res.phase_path.get() / 1e3,
            arc_km: res.arc_length.get() / 1e3,
            absorption_db: res.absorption.get() * NEPERS_TO_DB,
            ground_loss_db,
            ground_label: ground_type.map(GroundType::label),
            ground_reason,
            steps: res.steps,
            hamiltonian_drift: res.hamiltonian_drift,
            outcome: outcome_label(res.outcome),
            polyline: cap.polyline,
            end_lat_lon,
        });
        ends.push(res.end);

        if res.outcome != Outcome::Landed {
            return (
                details,
                ends,
                Some(format!("hop {} escaped instead of landing", i + 1)),
            );
        }
        let (ne, na) = reflect(res.end_m);
        e = ne;
        a = na;
        point = res.end;
    }
    (details, ends, None)
}

// Bundling these into a struct would only move the argument list to the call
// site; this is an internal helper with a stable, one-caller signature.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble(
    mode: Mode,
    layer: LayerMode,
    probability: f64,
    hops: u32,
    details: Vec<HopDetail>,
    ends: &[SphericalPoint],
    rx: &SphericalPoint,
    homing_miss_m: f64,
    note: Option<String>,
    f_mhz: f64,
    link_settings: LinkSettings<'_>,
) -> Solution {
    let total_group_km: f64 = details.iter().map(|h| h.group_km).sum();
    let total_phase_km: f64 = details.iter().map(|h| h.phase_km).sum();
    let total_arc_km: f64 = details.iter().map(|h| h.arc_km).sum();
    let total_absorption_db: f64 = details.iter().map(|h| h.absorption_db).sum();
    let total_ground_km: f64 = details.iter().map(|h| h.ground_range_km).sum();
    let max_hamiltonian_drift = details
        .iter()
        .map(|h| h.hamiltonian_drift)
        .fold(0.0_f64, f64::max);
    let total_steps: usize = details.iter().map(|h| h.steps).sum();
    let terminal_miss_km = ends.last().map_or(f64::NAN, |e| {
        central_angle(e, rx).get() * EARTH_RADIUS_M / 1e3
    });

    // Link budget: spreading over the whole ray path + ionospheric absorption +
    // Fresnel loss at each intermediate ground reflection.
    let free_space_loss_db = free_space_loss_db(total_arc_km.max(1e-3), f_mhz);
    let ground_reflection_loss_db: f64 = details.iter().map(|h| h.ground_loss_db).sum();
    let num_ground_reflections =
        u32::try_from(details.iter().filter(|h| h.ground_loss_db > 0.0).count()).unwrap_or(0);
    let total_system_loss_db = free_space_loss_db + total_absorption_db + ground_reflection_loss_db;

    // Antenna gain. Each end is read at the angle the ray actually uses: the
    // transmitter at the launch elevation of the first hop, the receiver at the
    // arrival elevation of the last. This is what makes the pattern change
    // which mode wins rather than scaling every solution equally - a steep
    // 4-hop path and a shallow 2-hop path see different parts of the pattern.
    let tx_elev_deg = details.first().map_or(0.0, |h| h.launch_elev_deg);
    let rx_elev_deg = details.last().map_or(0.0, |h| h.arrival_elev_deg);
    let tx_gain_dbi = link_settings.tx_antenna.gain_dbi(tx_elev_deg.to_radians());
    let rx_gain_dbi = link_settings.rx_antenna.gain_dbi(rx_elev_deg.to_radians());
    let total_gain_db = tx_gain_dbi + rx_gain_dbi;

    // Judgment layer only: transmitter power, antenna gain and noise floor
    // applied to the propagation loss just computed. Nothing above the
    // `total_system_loss_db` line is affected by any of it, which is why that
    // figure remains a pure propagation loss the UI can show on its own.
    let link = LinkBudget::from_settings(link_settings, total_system_loss_db - total_gain_db);

    Solution {
        mode,
        layer,
        probability,
        hops,
        hop_details: details,
        total_group_km,
        total_phase_km,
        total_arc_km,
        total_absorption_db,
        free_space_loss_db,
        ground_reflection_loss_db,
        // Set by the Es pass when a solution is attributed to the sheet; a
        // deterministic path has no sheet reflection to charge for.
        es_reflection_loss_db: 0.0,
        num_ground_reflections,
        total_system_loss_db,
        tx_gain_dbi,
        rx_gain_dbi,
        tx_elev_deg,
        rx_elev_deg,
        total_gain_db,
        link,
        total_ground_km,
        terminal_miss_km,
        homing_miss_m,
        max_hamiltonian_drift,
        total_steps,
        group_delay_ms: total_group_km * 1e3 / 299_792_458.0 * 1e3,
        note,
    }
}

/// Sweep elevations to find how close the engine can get when nothing homes.
#[allow(clippy::too_many_arguments)]
pub(super) fn near_miss_sweep<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    brng: Radians,
    total_arc: Radians,
    mode: Mode,
    max_hops: u32,
    freq_mhz: f64,
    errors: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Vec<NearMiss>
where
    D: ElectronDensity + Sync + ?Sized,
    B: MagneticField + Sync + ?Sized,
    C: CollisionFrequency + Sync + ?Sized,
{
    // The sweep itself does not depend on the hop count - only the target range
    // each landing is measured against does - so it is traced once and then
    // reduced per hop count, instead of once per hop count as it used to be.
    let mut elevations = Vec::new();
    let mut elev = 3.0_f64;
    while elev <= 88.0 {
        elevations.push(elev);
        elev += 1.0;
    }
    // Independent rays, fanned out and folded back in elevation order so both
    // the sweep and its diagnostics are what the serial loop produced.
    let traced: Vec<(f64, Result<Option<f64>, String>)> = elevations
        .par_iter()
        .map(|&elev| {
            let outcome = match tracer.trace(tx, Radians::from_degrees(elev), brng) {
                Ok(res) => {
                    let landed = res.outcome == Outcome::Landed;
                    let range_km = central_angle(tx, &res.end).get() * EARTH_RADIUS_M / 1e3;
                    Ok(landed.then_some(range_km))
                }
                Err(e) => Err(format!(
                    "{} mode, sweep elev {elev:.1} deg: {e}",
                    mode_label(mode)
                )),
            };
            (elev, outcome)
        })
        .collect();
    let mut swept: Vec<(f64, Option<f64>)> = Vec::new();
    for (elev, outcome) in traced {
        match outcome {
            Ok(range) => swept.push((elev, range)),
            Err(msg) => {
                if !errors.contains(&msg) {
                    errors.push(msg);
                }
            }
        }
    }

    let mut out = Vec::new();
    for hops in 1..=max_hops {
        let target_arc = total_arc.get() / f64::from(hops);
        let target_km = target_arc * EARTH_RADIUS_M / 1e3;
        let mut best: Option<NearMiss> = None;
        let mut landed_count = 0u32;
        let mut escaped_count = 0u32;
        for &(elev, landed_range_km) in &swept {
            let Some(range_km) = landed_range_km else {
                escaped_count += 1;
                continue;
            };
            landed_count += 1;
            let miss = (range_km - target_km).abs();
            if best.as_ref().is_none_or(|b| miss < b.miss_km) {
                best = Some(NearMiss {
                    mode,
                    hops,
                    elevation_deg: elev,
                    landed_range_km: range_km,
                    target_range_km: target_km,
                    miss_km: miss,
                    note: "landed".to_string(),
                });
            }
        }
        if let Some(b) = best {
            out.push(b);
        } else if landed_count == 0 && escaped_count > 0 {
            // Nothing reflected anywhere, so there is no "closest landing".
            // Say so explicitly instead of returning an empty table.
            notes.push(format!(
                "{} mode, {hops} hop(s): no elevation between 3 and 88 deg reflects - the ray \
                 penetrates the layer at every angle, so {:.2} MHz is above this ionosphere's \
                 maximum usable frequency for any geometry ({escaped_count} elevations swept, \
                 all escaped). Target range was {target_km:.0} km.",
                mode_label(mode),
                freq_mhz,
            ));
        }
    }
    out
}
