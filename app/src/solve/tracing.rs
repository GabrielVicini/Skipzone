//! Ray-tracing helpers that drive the engine's tracer and homing to build
//! full multi-hop solutions and, when nothing homes, a near-miss report.
//! Calls the engine's public API only; no physics is implemented here.

use skipzone::collision::CollisionFrequency;
use skipzone::density::ElectronDensity;
use skipzone::error::TraceError;
use skipzone::geo::{SphericalPoint, central_angle};
use skipzone::homing::HomingConfig;
use skipzone::mag::MagneticField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};

use crate::noise::{LinkBudget, LinkSettings};
use crate::scenario::{Assumptions, EARTH_RADIUS_M, Models, to_lat_lon};

use super::link_budget::{NEPERS_TO_DB, free_space_loss_db, ground_reflection_loss_db};
use super::types::{HopDetail, NearMiss, Solution, mode_label};

/// Cap on drawn points per hop; the ray polyline is decimated to this.
const MAX_POLY_POINTS: usize = 400;

/// Practical homing miss tolerance for interactive HF prediction, m. The
/// engine default (30 m) is set for its own validation and, near a skip-zone
/// edge / caustic, the bisection legitimately stalls at a few hundred metres
/// after the iteration budget - a miss that is already a match for any HF use
/// (< ~0.1 % of path length). Accepting it here turns those "practically a
/// match" cases into the connections they are, instead of reporting no path.
const PRACTICAL_MISS_TOLERANCE_M: f64 = 2000.0;

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

pub(super) fn make_tracer<'a>(
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

// Bundling these into a struct would only move the argument list to the call
// site; this is an internal helper with a stable, one-caller signature.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble(
    mode: Mode,
    hops: u32,
    details: Vec<HopDetail>,
    ends: &[SphericalPoint],
    rx: &SphericalPoint,
    homing_miss_m: f64,
    note: Option<String>,
    f_mhz: f64,
    link_settings: LinkSettings,
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

    // Judgment layer only: transmitter power and noise floor applied to the
    // loss just computed. Nothing above this line is affected by it.
    let link = LinkBudget::from_settings(link_settings, total_system_loss_db);

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
