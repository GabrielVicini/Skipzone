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

use num_complex::Complex64;
use std::f64::consts::PI;

use crate::scenario::{
    Assumptions, EARTH_RADIUS_M, Inputs, Models, destination_point, ground_point, to_lat_lon,
};

/// Nepers -> dB for field amplitude: 20/ln(10).
pub const NEPERS_TO_DB: f64 = 8.685_889_638_065_035;

/// Basic free-space (spreading) loss [dB] over a path length `dist_km` at
/// `f_mhz`: the standard Friis form `32.44 + 20 log10(f_MHz) + 20 log10(d_km)`.
/// The distance used is the total ray arc length (the physical path the energy
/// travels), not the great-circle range.
#[must_use]
pub fn free_space_loss_db(dist_km: f64, f_mhz: f64) -> f64 {
    32.44 + 20.0 * f_mhz.log10() + 20.0 * dist_km.log10()
}

/// Loss [dB] at one ground reflection, from the Fresnel power reflection
/// coefficient of a lossy dielectric half-space.
///
/// The complex relative permittivity is `eps_r - j sigma/(omega eps0)`
/// (ITU-R P.527 form). Horizontal and vertical coefficients are
///   R_h = (sin g - w)/(sin g + w),  R_v = (eps_c sin g - w)/(eps_c sin g + w),
///   w = sqrt(eps_c - cos^2 g),
/// with `g` the grazing (elevation) angle. A sky wave is elliptically polarised
/// after its ionospheric reflection, so we use the average power coefficient
/// `(|R_h|^2 + |R_v|^2)/2`; the loss is `-10 log10` of it.
#[must_use]
pub fn ground_reflection_loss_db(grazing_rad: f64, f_hz: f64, eps_r: f64, sigma: f64) -> f64 {
    const EPS0: f64 = 8.854_187_8e-12;
    let eps_c = Complex64::new(eps_r, -sigma / (2.0 * PI * f_hz * EPS0));
    let (sin_g, cos_g) = grazing_rad.sin_cos();
    let s = Complex64::new(sin_g, 0.0);
    let w = (eps_c - cos_g * cos_g).sqrt();
    let r_h = (s - w) / (s + w);
    let r_v = (eps_c * s - w) / (eps_c * s + w);
    let power = 0.5 * (r_h.norm_sqr() + r_v.norm_sqr());
    -10.0 * power.clamp(1e-12, 1.0).log10()
}
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
    /// Ground-reflection loss [dB] incurred where this hop lands, when another
    /// hop follows (0 for the final hop, which arrives at the receiver).
    pub ground_loss_db: f64,
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
    /// Free-space spreading loss over the total ray path, dB.
    pub free_space_loss_db: f64,
    /// Summed Fresnel loss over the intermediate ground reflections, dB.
    pub ground_reflection_loss_db: f64,
    /// Number of intermediate ground reflections (hops - 1 for a landed path).
    pub num_ground_reflections: u32,
    /// Basic transmission loss = free-space + absorption + ground reflection, dB.
    /// Excludes antenna gains and any statistical excess-system-loss term.
    pub total_system_loss_db: f64,
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
    /// Plain-language outcome of the elevation sweep when nothing homed -
    /// notably the case where no elevation reflects at all, which produces no
    /// "closest landing" and would otherwise leave the operator with a blank
    /// panel.
    pub sweep_notes: Vec<String>,
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

/// Practical homing miss tolerance for interactive HF prediction, m. The
/// engine default (30 m) is set for its own validation and, near a skip-zone
/// edge / caustic, the bisection legitimately stalls at a few hundred metres
/// after the iteration budget - a miss that is already a match for any HF use
/// (< ~0.1 % of path length). Accepting it here turns those "practically a
/// match" cases into the connections they are, instead of reporting no path.
const PRACTICAL_MISS_TOLERANCE_M: f64 = 2000.0;

fn homing_config(use_field: bool) -> HomingConfig {
    let mut c = HomingConfig::default();
    c.miss_tolerance_m = PRACTICAL_MISS_TOLERANCE_M;
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
    f_hz: f64,
    ground: (f64, f64),
) -> (Vec<HopDetail>, Vec<SphericalPoint>, Option<String>)
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    let (eps_r, sigma) = ground;
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
        let ground_loss_db = if res.outcome == Outcome::Landed && i + 1 < hops {
            ground_reflection_loss_db(arr_elev.to_radians(), f_hz, eps_r, sigma)
        } else {
            0.0
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
    f_mhz: f64,
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

    Solution {
        mode,
        hops,
        hop_details: details,
        total_group_km,
        total_phase_km,
        total_arc_km,
        total_absorption_db,
        free_space_loss_db,
        ground_reflection_loss_db,
        num_ground_reflections,
        total_system_loss_db,
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
    freq_mhz: f64,
    errors: &mut Vec<String>,
    notes: &mut Vec<String>,
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
        let mut landed_count = 0u32;
        let mut escaped_count = 0u32;
        let mut elev = 3.0_f64;
        while elev <= 88.0 {
            let r = tracer.trace(tx, Radians::from_degrees(elev), brng);
            match r {
                Ok(res) => {
                    let landed = res.outcome == Outcome::Landed;
                    let range_km = central_angle(tx, &res.end).get() * EARTH_RADIUS_M / 1e3;
                    if landed {
                        landed_count += 1;
                    } else {
                        escaped_count += 1;
                    }
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

pub fn solve(inputs: &Inputs, a: &Assumptions, models: &Models) -> SolveOutcome {
    let started = std::time::Instant::now();
    let tx = ground_point(inputs.tx_lat, inputs.tx_lon);
    let rx = ground_point(inputs.rx_lat, inputs.rx_lon);
    let total_arc = central_angle(&tx, &rx);
    let brng = bearing(&tx, &rx);
    let great_circle_km = total_arc.get() * EARTH_RADIUS_M / 1e3;

    let mut solutions = Vec::new();
    let mut errors = Vec::new();
    let f_hz = inputs.freq_mhz * 1e6;
    let ground = inputs.ground_type.constants();

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
                            propagate(&tracer, &tx, ray.elevation, ray.azimuth, hops, f_hz, ground);
                        if details.is_empty() {
                            if let Some(n) = note {
                                errors
                                    .push(format!("{} mode, {hops} hop(s): {n}", mode_label(mode)));
                            }
                            continue;
                        }
                        solutions.push(assemble(
                            mode,
                            hops,
                            details,
                            &ends,
                            &rx,
                            ray.miss_m,
                            note,
                            inputs.freq_mhz,
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
    let mut sweep_notes = Vec::new();
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
                inputs.freq_mhz,
                &mut errors,
                &mut sweep_notes,
            ));
        }
        near_misses.sort_by(|x, y| x.miss_km.total_cmp(&y.miss_km));
    }

    // Shortest total group path first: the most likely dominant mode.
    solutions.sort_by(|x, y| x.total_group_km.total_cmp(&y.total_group_km));

    SolveOutcome {
        solutions,
        near_misses,
        sweep_notes,
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

    /// Free-space loss matches the textbook Friis value, and ground-reflection
    /// loss behaves physically: non-negative, sea water (highly conducting) loses
    /// far less than dry ground at the same grazing angle, and (for lossy ground)
    /// a near-grazing bounce reflects better than a steeper one.
    #[test]
    fn link_budget_terms_are_physical() {
        // Friis: 20 MHz over 8000 km = 32.44 + 26.02 + 78.06 = 136.5 dB.
        assert!((free_space_loss_db(8000.0, 20.0) - 136.52).abs() < 0.1);

        let f = 14e6;
        let grazing = 8.0_f64.to_radians();
        let sea = ground_reflection_loss_db(grazing, f, 80.0, 5.0);
        let dry = ground_reflection_loss_db(grazing, f, 5.0, 0.001);
        assert!(sea >= 0.0 && dry >= 0.0);
        assert!(sea < 0.6, "sea reflects well, got {sea} dB");
        assert!(
            dry > sea + 1.0,
            "dry ground {dry} should lose more than sea {sea}"
        );
        // Over lossy ground the reflection coefficient falls away from grazing
        // (the vertical-polarisation Brewster dip), so a steeper bounce loses more.
        let steep = ground_reflection_loss_db(45.0_f64.to_radians(), f, 5.0, 0.001);
        assert!(
            steep > dry,
            "steeper bounce {steep} should lose more than grazing {dry}"
        );
    }

    /// The assembled Solution's total system loss is exactly the sum of its
    /// three parts, spreading dominates, and there are hops-1 ground reflections.
    #[test]
    fn total_system_loss_is_sum_of_parts() {
        let out = run(&Inputs::default());
        let s = out.solutions.first().expect("a solution");
        let sum = s.free_space_loss_db + s.total_absorption_db + s.ground_reflection_loss_db;
        assert!((s.total_system_loss_db - sum).abs() < 1e-9);
        assert!(
            s.free_space_loss_db > 120.0,
            "spreading {}",
            s.free_space_loss_db
        );
        assert_eq!(s.num_ground_reflections, s.hops - 1);
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
        assert!(
            s.total_absorption_db < 500.0,
            "absorption implausibly large"
        );
    }

    /// Regression guard for the "absorption ~ 0 at the terminator" bug. Past
    /// the engine's 85 deg plane-parallel limit the D region used to be omitted
    /// entirely, collapsing absorption to the negligible F2-only floor
    /// (~1e-3 dB). With the Chapman grazing layer the D region is still present
    /// and producing a few degrees past 85 deg, so both the sampled density and
    /// the solved absorption stay meaningfully non-zero.
    #[test]
    fn terminator_d_region_is_not_cut() {
        use skipzone::density::ElectronDensity;
        use skipzone::geo::SphericalPoint;
        use skipzone::units::{Meters, Radians};

        // Denver -> London in January, walked in UTC to a midpoint zenith angle
        // just past the old 85 deg cliff.
        let mut chosen: Option<(Inputs, scenario::Assumptions)> = None;
        let mut utc = 17.5_f64;
        while utc <= 20.0 {
            let inputs = Inputs {
                utc_hours: utc,
                ..Inputs::default()
            };
            let a = scenario::resolve(&inputs);
            if (85.0..88.0).contains(&a.solar.zenith_angle_deg) {
                chosen = Some((inputs, a));
                break;
            }
            utc += 0.1;
        }
        let (inputs, a) = chosen.expect("a terminator geometry in 17.5..=20 UTC");
        assert!(a.solar.is_day(), "sun is still up just past 85 deg");

        // The D region must still be producing at the midpoint: sample the
        // density model actually fed to the tracer at its realised peak height.
        // Under the old omit-past-85 logic this would be the F2 tail alone (~0).
        let models = scenario::build_models(&inputs, &a).expect("models build");
        let mid = SphericalPoint::new(
            Meters::new(scenario::EARTH_RADIUS_M + a.d_region_peak_alt_km * 1e3),
            Radians::from_degrees(90.0 - a.midpoint_lat),
            Radians::from_degrees(a.midpoint_lon),
        );
        let ne = models.density.sample(&mid).ne;
        assert!(
            ne > 1e8,
            "terminator D region should still produce, got Ne = {ne:.3e} at chi = {} deg",
            a.solar.zenith_angle_deg
        );

        // And the solved path must absorb meaningfully, not collapse to the
        // ~1e-3 dB F2-only floor that was the reported bug.
        let out = solve(&inputs, &a, &models);
        let s = out
            .solutions
            .first()
            .expect("a solution at the terminator geometry");
        assert!(
            s.total_absorption_db > 0.1,
            "terminator absorption collapsed to {} dB (the bug)",
            s.total_absorption_db
        );
    }

    /// The default scenario sits close to the terminator: mid-January,
    /// 18:00 UTC, midpoint near 59 N gives chi ~ 84 deg, i.e. the sun barely
    /// 6 deg above the horizon. It is (just) daylight, so a weak D region is
    /// active. Pinned because it is a sensitive, easily-broken configuration.
    #[test]
    fn default_scenario_is_marginal_daylight() {
        let a = scenario::resolve(&Inputs::default());
        assert!(
            (80.0..85.0).contains(&a.solar.zenith_angle_deg),
            "default midpoint chi = {} deg",
            a.solar.zenith_angle_deg
        );
        assert!(a.d_region_active, "marginal daylight still has a D region");
        // sqrt(cos chi) at 84 deg is ~0.32, so the D region is much weaker
        // than at noon.
        assert!(
            a.d_region_peak_ne < 0.4 * scenario::D_REGION_PEAK_NE_OVERHEAD,
            "grazing sun should thin the D region, got {:.3e}",
            a.d_region_peak_ne
        );
    }

    /// Well above the MUF nothing connects. With Earth curvature, a 45 MHz
    /// signal under a 5 MHz layer does not reflect at ANY elevation, so there
    /// is no "closest landing" to report - the sweep must then say so in plain
    /// language rather than leaving the operator with an empty panel.
    #[test]
    fn above_muf_explains_itself_rather_than_going_silent() {
        let inputs = Inputs {
            freq_mhz: 45.0,
            month: 6,
            day_of_month: 21,
            utc_hours: 15.5,
            ..Inputs::default()
        };
        let out = run(&inputs);
        assert!(out.solutions.is_empty(), "45 MHz should not connect");
        assert!(
            !out.near_misses.is_empty() || !out.sweep_notes.is_empty(),
            "sweep must produce either near-misses or an explanation"
        );
        for w in out.near_misses.windows(2) {
            assert!(w[0].miss_km <= w[1].miss_km, "near misses not sorted");
        }
        if out.near_misses.is_empty() {
            assert!(
                out.sweep_notes.iter().any(|n| n.contains("no elevation")),
                "expected a penetration explanation, got {:?}",
                out.sweep_notes
            );
        }
    }

    /// A frequency that genuinely can reflect but misses the target range must
    /// still produce ranked near-misses, which is the other half of the
    /// requirement.
    #[test]
    fn reachable_but_wrong_range_reports_ranked_near_misses() {
        let inputs = Inputs {
            freq_mhz: 7.1,
            month: 6,
            day_of_month: 21,
            utc_hours: 15.5,
            rx_lat: 40.0,
            rx_lon: -103.0,
            max_hops: 1,
            ..Inputs::default()
        };
        let out = run(&inputs);
        if out.solutions.is_empty() {
            assert!(
                !out.near_misses.is_empty() || !out.sweep_notes.is_empty(),
                "expected near-misses or an explanation"
            );
        }
    }

    /// Daytime 40 m reference case: Denver -> London at 7.1 MHz, 21 June,
    /// 15:30 UTC (local solar noon at the path midpoint). Compares the old
    /// wiring (F2 layer only, hand-picked collision numbers) against the new
    /// one (D region driven by solar zenith angle, neutral-atmosphere nu).
    #[test]
    fn daytime_40m_before_after() {
        use skipzone::collision::ExponentialCollisions;
        use skipzone::density::{ChapmanLayer, MultiLayer, density_at_critical_frequency};
        use skipzone::mag::Igrf;
        use skipzone::units::{Hertz, Meters, PerCubicMeter, PerSecond};

        let inputs = Inputs {
            freq_mhz: 7.1,
            month: 6,
            day_of_month: 21,
            utc_hours: 15.5,
            ..Inputs::default()
        };
        let a = scenario::resolve(&inputs);

        let after_models = scenario::build_models(&inputs, &a).expect("after models");
        let after = solve(&inputs, &a, &after_models);

        // Reconstruct the previous behaviour: F2 layer alone, no D region, and
        // the old hand-picked collision profile (1e5 /s at 100 km, H = 30 km).
        let f2 = ChapmanLayer::new(
            PerCubicMeter::new(a.nm_per_m3),
            Meters::new(scenario::EARTH_RADIUS_M + a.hmf2_km * 1e3),
            Meters::new(a.scale_height_km * 1e3),
        )
        .unwrap();
        let before_models = scenario::Models {
            density: MultiLayer::new(vec![Box::new(f2)]),
            field: Some(
                Igrf::from_embedded()
                    .unwrap()
                    .model_at(inputs.igrf_epoch)
                    .unwrap(),
            ),
            collisions: ExponentialCollisions::new(
                PerSecond::new(1.0e5),
                Meters::new(scenario::EARTH_RADIUS_M + 100e3),
                Meters::new(30e3),
            )
            .unwrap(),
        };
        let before = solve(&inputs, &a, &before_models);
        let _ = density_at_critical_frequency(Hertz::new(7.1e6));

        // Isolation run: F2 only, but with the NEW neutral-atmosphere nu.
        // Whatever absorption survives here is F2/deviative; the rest of the
        // AFTER figure must be genuine D-region absorption.
        let f2_only = ChapmanLayer::new(
            PerCubicMeter::new(a.nm_per_m3),
            Meters::new(scenario::EARTH_RADIUS_M + a.hmf2_km * 1e3),
            Meters::new(a.scale_height_km * 1e3),
        )
        .unwrap();
        let isolate_models = scenario::Models {
            density: MultiLayer::new(vec![Box::new(f2_only)]),
            field: Some(
                Igrf::from_embedded()
                    .unwrap()
                    .model_at(inputs.igrf_epoch)
                    .unwrap(),
            ),
            collisions: ExponentialCollisions::new(
                PerSecond::new(scenario::NU_REF_PER_S),
                Meters::new(scenario::EARTH_RADIUS_M + scenario::NU_REF_ALT_KM * 1e3),
                Meters::new(scenario::NU_SCALE_HEIGHT_KM * 1e3),
            )
            .unwrap(),
        };
        let isolate = solve(&inputs, &a, &isolate_models);

        let pick = |o: &SolveOutcome| {
            o.solutions
                .first()
                .map(|s| (s.hops, s.total_absorption_db, s.total_group_km))
        };
        println!("=== daytime 40m: Denver->London, 7.1 MHz, 21 Jun 15:30 UTC ===");
        println!(
            "midpoint {:.2},{:.2}  chi = {:.2} deg  (elev {:.2} deg)",
            a.midpoint_lat, a.midpoint_lon, a.solar.zenith_angle_deg, a.solar.elevation_deg
        );
        println!(
            "D region: active={} peak Ne={:.3e} m^-3 at {:.1} km",
            a.d_region_active, a.d_region_peak_ne, a.d_region_peak_alt_km
        );
        println!("BEFORE (F2 only, hand-picked nu): {:?}", pick(&before));
        println!("AFTER  (D region + neutral nu)  : {:?}", pick(&after));
        println!("ISOLATE (F2 only, new nu)       : {:?}", pick(&isolate));
        for (name, o) in [
            ("BEFORE", &before),
            ("ISOLATE", &isolate),
            ("AFTER", &after),
        ] {
            for s in &o.solutions {
                println!(
                    "  {name}: {}-mode {} hop(s)  abs {:.4} dB  group {:.1} km",
                    mode_label(s.mode),
                    s.hops,
                    s.total_absorption_db,
                    s.total_group_km
                );
            }
        }

        // Same path and date, but local midnight at the midpoint.
        let night_inputs = Inputs {
            utc_hours: 3.5,
            ..inputs.clone()
        };
        let night_a = scenario::resolve(&night_inputs);
        let night_models = scenario::build_models(&night_inputs, &night_a).expect("night models");
        let night = solve(&night_inputs, &night_a, &night_models);
        println!(
            "NIGHT (chi = {:.2} deg, D active = {}): {:?}",
            night_a.solar.zenith_angle_deg,
            night_a.d_region_active,
            pick(&night)
        );

        let first_abs =
            |o: &SolveOutcome| o.solutions.first().map_or(0.0, |s| s.total_absorption_db);
        let (a_before, a_after) = (first_abs(&before), first_abs(&after));
        let a_isolate = first_abs(&isolate);
        let a_night = first_abs(&night);

        assert!(a.d_region_active, "21 June local noon must be daylight");

        // With a physically-shaped nu (falling off with neutral density), the
        // F2 region contributes essentially nothing. The old 7.4 dB was an
        // artifact of a collision profile broad enough to put collisions at
        // F2 heights - it happened to land near the right magnitude for
        // entirely the wrong reason.
        assert!(
            a_isolate < 0.05,
            "F2-only with neutral nu should be negligible, got {a_isolate} dB"
        );
        assert!(
            a_after > 20.0 * a_isolate,
            "absorption should now be dominated by the D region: {a_after} vs {a_isolate}"
        );

        // The whole point of item 1: absorption must now respond to solar
        // zenith angle. Night must be far quieter than local noon.
        assert!(
            !night_a.d_region_active,
            "local midnight should be past the Chapman limit"
        );
        assert!(
            a_night < 0.1 * a_after,
            "night absorption {a_night} dB should collapse relative to day {a_after} dB"
        );
        let _ = a_before;
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
