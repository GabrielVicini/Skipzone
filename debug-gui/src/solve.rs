//! Drives the engine's existing homing and tracer to produce every mode that
//! connects, plus full per-hop geometry for drawing and a near-miss report
//! when nothing connects. Calls public API only; no physics is implemented
//! here.
//!
//! Multi-hop handling: the engine's homing solves a single hop. For an N-hop
//! path we home one hop of 1/N the great-circle arc (exact when the medium is
//! height-only), then actually propagate N hops by specular ground reflection
//! and report where the ray really lands. With a magnetic field the medium is
//! not spherically symmetric, so the terminal miss is a genuine diagnostic of
//! the equal-hop assumption rather than something to hide.

use skipzone::collision::CollisionFrequency;
use skipzone::density::ElectronDensity;
use skipzone::error::TraceError;
use skipzone::geo::{SphericalPoint, bearing, central_angle};
use skipzone::homing::{Homing, HomingConfig, HomingError};
use skipzone::mag::MagneticField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};

use crate::scenario::{
    Assumptions, EARTH_RADIUS_M, Inputs, Models, destination_point, ground_point, to_lat_lon,
};

/// Nepers -> dB for field amplitude: 20/ln(10).
pub const NEPERS_TO_DB: f64 = 8.685_889_638_065_035;
/// Cap on drawn points per hop; the ray polyline is decimated to this.
const MAX_POLY_POINTS: usize = 400;

pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Ordinary => "O",
        Mode::Extraordinary => "X",
    }
}

/// One traced hop, with everything the UI wants to show about it.
#[derive(Clone)]
pub struct HopDetail {
    pub index: u32,
    pub launch_elev_deg: f64,
    pub launch_az_deg: f64,
    pub arrival_elev_deg: f64,
    pub arrival_az_deg: f64,
    pub apex_alt_km: f64,
    /// X = (fp/f)^2 at the apex, from the engine's own apex record. At an
    /// isotropic reflection this should sit at the plasma condition.
    pub apex_x: f64,
    pub apex_lat_lon: (f64, f64),
    pub ground_range_km: f64,
    pub group_km: f64,
    pub phase_km: f64,
    pub arc_km: f64,
    pub absorption_db: f64,
    pub steps: usize,
    pub hamiltonian_drift: f64,
    pub outcome: &'static str,
    /// Ground-track polyline for this hop, decimated, (lat, lon).
    pub polyline: Vec<(f64, f64)>,
    /// Landing point of this hop.
    pub end_lat_lon: (f64, f64),
}

#[derive(Clone)]
pub struct Solution {
    pub mode: Mode,
    pub hops: u32,
    pub hop_details: Vec<HopDetail>,
    pub total_group_km: f64,
    pub total_phase_km: f64,
    pub total_arc_km: f64,
    pub total_absorption_db: f64,
    pub total_ground_km: f64,
    /// Distance from the final landing point to the requested receiver.
    pub terminal_miss_km: f64,
    /// Miss reported by the single-hop homing that produced the launch angles.
    pub homing_miss_m: f64,
    pub max_hamiltonian_drift: f64,
    pub total_steps: usize,
    /// Time of flight from the group path, ms.
    pub group_delay_ms: f64,
    /// Non-fatal note, e.g. a later hop failing after the first succeeded.
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct NearMiss {
    pub mode: Mode,
    pub hops: u32,
    pub elevation_deg: f64,
    pub landed_range_km: f64,
    pub target_range_km: f64,
    pub miss_km: f64,
    pub note: String,
}

pub struct SolveOutcome {
    pub solutions: Vec<Solution>,
    pub near_misses: Vec<NearMiss>,
    /// Every typed engine error encountered, verbatim, with context.
    pub errors: Vec<String>,
    pub great_circle_km: f64,
    pub bearing_deg: f64,
    pub reverse_bearing_deg: f64,
    pub elapsed_ms: f64,
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
    if let Some(last) = points.last() {
        if out.last() != Some(last) {
            out.push(*last);
        }
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

fn make_tracer<'a>(
    models: &'a Models,
    freq_mhz: f64,
    mode: Mode,
    a: &Assumptions,
) -> Tracer<'a, dyn ElectronDensity + 'a, dyn MagneticField + 'a, dyn CollisionFrequency + 'a> {
    let config = TraceConfig::new(Meters::new(a.r_ground_m), Meters::new(a.r_top_m));
    Tracer::new(
        models.density_dyn(),
        models.field_dyn(),
        models.collisions_dyn(),
        Hertz::new(freq_mhz * 1e6),
        mode,
        config,
    )
}

fn homing_config(use_field: bool) -> HomingConfig {
    let mut c = HomingConfig::default();
    // Without a field the near-vertical Spitze cannot occur, so the scan can
    // reach NVIS geometries. With a field, keep the engine's default cap.
    if !use_field {
        c.elev_max = Radians::from_degrees(88.0);
    }
    c
}

/// Propagate `hops` hops from `tx` at the given launch angles, reflecting
/// specularly off the ground between them.
fn propagate<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    elev: Radians,
    az: Radians,
    hops: u32,
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
                return (
                    details,
                    ends,
                    Some(format!("hop {} failed: {err}", i + 1)),
                );
            }
        };
        let res = &cap.result;
        let (arr_elev, arr_az) = arrival_angles(res.end_m);
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
            steps: res.steps,
            hamiltonian_drift: res.hamiltonian_drift,
            outcome: outcome_label(res.outcome),
            polyline: cap.polyline,
            end_lat_lon: to_lat_lon(&res.end),
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

fn assemble(
    mode: Mode,
    hops: u32,
    details: Vec<HopDetail>,
    ends: &[SphericalPoint],
    rx: &SphericalPoint,
    homing_miss_m: f64,
    note: Option<String>,
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
    let terminal_miss_km = ends
        .last()
        .map_or(f64::NAN, |e| central_angle(e, rx).get() * EARTH_RADIUS_M / 1e3);

    Solution {
        mode,
        hops,
        hop_details: details,
        total_group_km,
        total_phase_km,
        total_arc_km,
        total_absorption_db,
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
fn near_miss_sweep<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    tx: &SphericalPoint,
    brng: Radians,
    total_arc: Radians,
    mode: Mode,
    max_hops: u32,
    errors: &mut Vec<String>,
) -> Vec<NearMiss>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let mut out = Vec::new();
    for hops in 1..=max_hops {
        let target_arc = total_arc.get() / f64::from(hops);
        let target_km = target_arc * EARTH_RADIUS_M / 1e3;
        let mut best: Option<NearMiss> = None;
        let mut elev = 3.0_f64;
        while elev <= 88.0 {
            let r = tracer.trace(tx, Radians::from_degrees(elev), brng);
            match r {
                Ok(res) => {
                    let landed = res.outcome == Outcome::Landed;
                    let range_km =
                        central_angle(tx, &res.end).get() * EARTH_RADIUS_M / 1e3;
                    if landed {
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
                }
                Err(e) => {
                    let msg = format!("{} mode, sweep elev {elev:.1} deg: {e}", mode_label(mode));
                    if !errors.contains(&msg) {
                        errors.push(msg);
                    }
                }
            }
            elev += 1.0;
        }
        if let Some(b) = best {
            out.push(b);
        }
    }
    out
}

pub fn solve(inputs: &Inputs, a: &Assumptions, models: &Models) -> SolveOutcome {
    let started = std::time::Instant::now();
    let tx = ground_point(inputs.tx_lat, inputs.tx_lon);
    let rx = ground_point(inputs.rx_lat, inputs.rx_lon);
    let total_arc = central_angle(&tx, &rx);
    let brng = bearing(&tx, &rx);
    let great_circle_km = total_arc.get() * EARTH_RADIUS_M / 1e3;

    let mut solutions = Vec::new();
    let mut errors = Vec::new();

    // Without a field, O and X are bit-identical by construction, so tracing
    // both would just draw the same path twice.
    let modes: &[Mode] = if inputs.use_field {
        &[Mode::Ordinary, Mode::Extraordinary]
    } else {
        &[Mode::Ordinary]
    };

    for &mode in modes {
        let tracer = make_tracer(models, inputs.freq_mhz, mode, a);
        let homing = Homing {
            tracer: &tracer,
            config: homing_config(inputs.use_field),
        };
        for hops in 1..=inputs.max_hops {
            let target = if hops == 1 {
                rx
            } else {
                destination_point(&tx, brng, Radians::new(total_arc.get() / f64::from(hops)))
            };
            match homing.home_scan(&tx, &target) {
                Ok(rays) => {
                    for ray in rays {
                        let (details, ends, note) =
                            propagate(&tracer, &tx, ray.elevation, ray.azimuth, hops);
                        if details.is_empty() {
                            if let Some(n) = note {
                                errors.push(format!("{} mode, {hops} hop(s): {n}", mode_label(mode)));
                            }
                            continue;
                        }
                        solutions.push(assemble(
                            mode, hops, details, &ends, &rx, ray.miss_m, note,
                        ));
                    }
                }
                Err(HomingError::NoBracket { .. }) => {}
                Err(e) => {
                    errors.push(format!("{} mode, {hops} hop(s): {e}", mode_label(mode)));
                }
            }
        }
    }

    let mut near_misses = Vec::new();
    if solutions.is_empty() {
        for &mode in modes {
            let tracer = make_tracer(models, inputs.freq_mhz, mode, a);
            near_misses.extend(near_miss_sweep(
                &tracer,
                &tx,
                brng,
                total_arc,
                mode,
                inputs.max_hops,
                &mut errors,
            ));
        }
        near_misses.sort_by(|x, y| x.miss_km.total_cmp(&y.miss_km));
    }

    // Shortest total group path first: the most likely dominant mode.
    solutions.sort_by(|x, y| x.total_group_km.total_cmp(&y.total_group_km));

    SolveOutcome {
        solutions,
        near_misses,
        errors,
        great_circle_km,
        bearing_deg: brng.to_degrees().rem_euclid(360.0),
        reverse_bearing_deg: bearing(&rx, &tx).to_degrees().rem_euclid(360.0),
        elapsed_ms: started.elapsed().as_secs_f64() * 1e3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario;

    fn run(inputs: &Inputs) -> SolveOutcome {
        let a = scenario::resolve(inputs);
        let models = scenario::build_models(inputs, &a).expect("models build");
        solve(inputs, &a, &models)
    }

    /// The default scenario (Denver -> London, 14.1 MHz) must produce real
    /// solutions with physically sane geometry. This exercises the whole
    /// GUI-to-engine wiring without needing a display.
    #[test]
    fn default_scenario_connects_with_sane_geometry() {
        let out = run(&Inputs::default());
        assert!(
            !out.solutions.is_empty(),
            "expected at least one mode; errors: {:?}",
            out.errors
        );
        assert!(
            (out.great_circle_km - 7541.0).abs() < 60.0,
            "great circle {}",
            out.great_circle_km
        );

        for s in &out.solutions {
            assert!(s.hops >= 1 && s.hops <= Inputs::default().max_hops);
            // Group path spans at least the ground distance covered.
            assert!(
                s.total_group_km >= s.total_ground_km - 1.0,
                "group {} < ground {}",
                s.total_group_km,
                s.total_ground_km
            );
            // Phase path is shorter than group path inside a plasma.
            assert!(
                s.total_phase_km < s.total_group_km,
                "phase {} >= group {}",
                s.total_phase_km,
                s.total_group_km
            );
            assert!(s.group_delay_ms > 0.0 && s.group_delay_ms < 200.0);
            assert!(
                s.max_hamiltonian_drift < 1e-6,
                "solver drift {}",
                s.max_hamiltonian_drift
            );
            for h in &s.hop_details {
                assert!(
                    h.apex_alt_km > 50.0 && h.apex_alt_km < 600.0,
                    "apex {}",
                    h.apex_alt_km
                );
                assert!(h.launch_elev_deg > 0.0 && h.launch_elev_deg < 90.0);
                assert!(!h.polyline.is_empty(), "no polyline captured for drawing");
            }
        }
    }

    /// Absorption must be genuinely non-zero: the collision wiring is real,
    /// not the ZeroCollisions path.
    #[test]
    fn absorption_is_nonzero_with_real_collisions() {
        let out = run(&Inputs::default());
        let s = out.solutions.first().expect("a solution");
        assert!(
            s.total_absorption_db > 0.0,
            "absorption should be positive, got {}",
            s.total_absorption_db
        );
        assert!(s.total_absorption_db < 500.0, "absorption implausibly large");
    }

    /// Far above the MUF nothing connects, and the near-miss sweep must still
    /// give the operator something rather than an empty screen.
    #[test]
    fn above_muf_reports_near_misses_not_silence() {
        let inputs = Inputs {
            freq_mhz: 45.0,
            ..Inputs::default()
        };
        let out = run(&inputs);
        assert!(out.solutions.is_empty(), "45 MHz should not connect");
        assert!(
            !out.near_misses.is_empty(),
            "near-miss sweep should report closest approaches"
        );
        for w in out.near_misses.windows(2) {
            assert!(w[0].miss_km <= w[1].miss_km, "near misses not sorted");
        }
    }

    /// With no field, O and X are degenerate, so only one mode is traced.
    #[test]
    fn zero_field_traces_single_mode() {
        let inputs = Inputs {
            use_field: false,
            ..Inputs::default()
        };
        let out = run(&inputs);
        assert!(
            out.solutions.iter().all(|s| s.mode == Mode::Ordinary),
            "zero field should not emit a separate X mode"
        );
    }
}
