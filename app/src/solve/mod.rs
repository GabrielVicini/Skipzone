//! Drives the engine's existing homing and tracer to produce every mode that
//! connects, plus full per-hop geometry for drawing and a near-miss report
//! when nothing connects. Calls public API only; no physics is implemented
//! here.
//!
//! Multi-hop handling: the engine's homing solves a single hop. For an N-hop
//! path we home one hop of 1/N the great-circle arc (exact when the medium is
//! height-only), then actually propagate N hops by specular ground reflection
//! and report where the ray lands. With a magnetic field the medium is not
//! spherically symmetric, so the terminal miss measures the error of the
//! equal-hop assumption.
//!
//! The work is split by concern across the submodules: `types` holds the
//! result structs the UI renders, `link_budget` the free-space and ground
//! reflection loss terms, and `tracing` the per-hop tracing/homing helpers.
//! This module keeps only the top-level [`solve`] driver that stitches them
//! together.

mod link_budget;
mod tracing;
mod types;

pub use types::{LayerMode, LayerStatus, ModeReport, Solution, SolveOutcome, mode_label};

/// The mode a listener would hear over a deterministic path: the strongest SNR
/// among the F2/E solutions, or `None` when none connected.
///
/// Shared by the frequency sweep and the coverage grid so they judge a
/// scenario by the same mode. Sporadic E is excluded; see
/// [`best_with_es_fallback`].
#[must_use]
pub fn best_by_snr(out: &SolveOutcome) -> Option<&Solution> {
    out.solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db))
}

/// The strongest Es-supported path, if any. Separate from [`best_by_snr`]
/// because it comes with a probability attached and must not be compared with a
/// deterministic path as though it were one.
#[must_use]
pub fn best_es(out: &SolveOutcome) -> Option<&Solution> {
    out.es_solutions
        .iter()
        .max_by(|a, b| a.link.snr_db.total_cmp(&b.link.snr_db))
}

/// The path to report when a single answer is needed: the best deterministic
/// path if one closed, otherwise the best Es-supported one.
///
/// # Why this is not the strongest SNR of the two lists
///
/// An Es reflection at 100 km has a shorter ray path than the F2 alternative,
/// less spreading loss and a shorter slant through the absorbing D region, so
/// on raw SNR it always wins where it is geometrically possible. Ranking both
/// lists by SNR would just prefer the lower layer and ignore that Es is present
/// only [`SporadicE::probability`](crate::sporadic_e::SporadicE) of the time.
/// Folding that probability into a dB figure would mix a likelihood with a
/// power. So the rule is ordinal: a path that is always there outranks one
/// that might be, and Es is used only when nothing deterministic closed (for
/// example a 17 m signal at 400 km, where F2 has no solution).
///
/// Callers must carry the winner's `probability`, since an Es answer is a
/// "maybe".
#[must_use]
pub fn best_with_es_fallback(out: &SolveOutcome) -> Option<&Solution> {
    best_by_snr(out).or_else(|| best_es(out))
}

use std::cmp::Ordering;

use rayon::prelude::*;

use skipzone::density::ElectronDensity;
use skipzone::geo::{bearing, central_angle};
use skipzone::magnetoionic::Mode;
use skipzone::units::Radians;

use crate::noise::LinkSettings;
use crate::scenario::{
    self, Assumptions, E_ATTRIBUTION_TOP_KM, EARTH_RADIUS_M, Inputs, Models, destination_point,
    ground_point,
};

use tracing::{
    GroundModel, StepTuning, assemble, home_terminal, homing_config, hop_elevation_window_deg,
    make_tracer, near_miss_sweep, propagate, scan_elevations, terminal_tolerance_m,
};

/// What one pass over the homing produced, per layer.
struct StackOutcome {
    solutions: Vec<Solution>,
    /// True when at least one (mode, hop count) combination reported
    /// `NoBracket`, i.e. rays reflect but none lands at the target.
    saw_no_bracket: bool,
    /// True when a trace failed outright inside homing refinement. Tracked
    /// separately so a numerical failure is not reported as a physical
    /// "nothing reflects".
    saw_trace_failure: bool,
    /// Diagnostics from this pass, kept per outcome so that the modes can be
    /// run in parallel and their messages still merged in a fixed order.
    errors: Vec<String>,
}

/// What one candidate ray produced. Carried out of the parallel map so the
/// fold back into the solution and error lists happens in a fixed order.
enum Candidate {
    /// Boxed because a `Solution` carries every hop's polyline, which would
    /// otherwise set the size of the enum at every use site.
    Solved(Box<Solution>),
    /// Nothing from this bracket reaches the receiver at this hop count.
    NoBracket,
    /// The propagation itself failed, which is not a physical answer.
    Failed(String),
}

/// Two launches this close in elevation are the same ray found twice: real
/// multipath branches (low ray / high ray, E / F2) are degrees apart, and the
/// terminal homing converges to well under a milli-degree.
const SAME_RAY_ELEV_DEG: f64 = 1e-3;

/// Do these two solutions describe the same propagation mode?
fn is_same_ray(a: &Solution, b: &Solution) -> bool {
    a.mode == b.mode
        && a.hops == b.hops
        && match (a.hop_details.first(), b.hop_details.first()) {
            (Some(x), Some(y)) => (x.launch_elev_deg - y.launch_elev_deg).abs() < SAME_RAY_ELEV_DEG,
            _ => false,
        }
}

/// Attribute a deterministic solution to a layer from the apex altitude the
/// engine reported for its first hop. Es is never a candidate here: the
/// deterministic stack has no Es sheet in it, so a reflection at 100 km is an
/// E-region reflection.
fn classify_deterministic(apex_alt_km: f64) -> LayerMode {
    if apex_alt_km.is_finite() && apex_alt_km <= E_ATTRIBUTION_TOP_KM {
        LayerMode::E
    } else {
        LayerMode::F2
    }
}

pub fn solve(inputs: &Inputs, a: &Assumptions, models: &Models) -> SolveOutcome {
    let started = std::time::Instant::now();
    let tx = ground_point(inputs.tx_lat, inputs.tx_lon);
    let rx = ground_point(inputs.rx_lat, inputs.rx_lon);
    let total_arc = central_angle(&tx, &rx);
    let brng = bearing(&tx, &rx);
    let great_circle_km = total_arc.get() * EARTH_RADIUS_M / 1e3;

    let mut errors = Vec::new();
    let f_hz = inputs.freq_mhz * 1e6;
    let terminal_tol = terminal_tolerance_m(great_circle_km);
    let ground = if inputs.ground_type.is_auto() {
        GroundModel::Auto {
            land_fallback: inputs.ground_land_fallback,
        }
    } else {
        GroundModel::Fixed(inputs.ground_type)
    };

    // The judgment layer's inputs, computed at this solve's frequency so the
    // frequency sweep, which re-solves against one `Assumptions`, gets the
    // right floor at every candidate.
    let noise = scenario::noise_floor_at(inputs, a, inputs.freq_mhz);

    // Antenna patterns, built once per solve. Each curve costs a hemispherical
    // power integral (see `crate::antenna::image`), so it is computed here and
    // sampled per solution. Both ends stand over the scenario's ground type,
    // or when auto-detected, over the surface at each station's coordinates
    // using the same lookup as the bounces.
    let ground_under = |lat: f64, lon: f64| match ground {
        GroundModel::Fixed(g) => g,
        GroundModel::Auto { land_fallback } => crate::coastline::get().map_or(land_fallback, |c| {
            c.classify(lat, lon, land_fallback).ground
        }),
    };
    let tx_antenna = inputs.tx_antenna.curve(
        ground_under(inputs.tx_lat, inputs.tx_lon).as_antenna_ground(),
        f_hz,
    );
    let rx_antenna = inputs.rx_antenna.curve(
        ground_under(inputs.rx_lat, inputs.rx_lon).as_antenna_ground(),
        f_hz,
    );

    let link_settings = LinkSettings {
        tx_power_w: inputs.tx_power_w,
        noise,
        threshold_db: inputs.snr_threshold_db,
        model_bias_db: inputs.model_bias_db,
        tx_antenna: &tx_antenna,
        rx_antenna: &rx_antenna,
    };

    // Without a field, O and X are bit-identical, so trace only one.
    let modes: &[Mode] = if inputs.use_field {
        &[Mode::Ordinary, Mode::Extraordinary]
    } else {
        &[Mode::Ordinary]
    };

    // One pass over the homing against one density stack. It runs once against
    // the deterministic layers and once with a sporadic-E sheet added, which
    // keeps the probabilistic answer separate from the deterministic one.
    let run_stack = |density: &(dyn ElectronDensity + Sync),
                     tuning: StepTuning,
                     errors: &mut Vec<String>|
     -> StackOutcome {
        // The two magnetoionic modes are separate rays through separate
        // refractive indices with nothing shared but the read-only models, so
        // they run side by side and are merged back in mode order.
        let per_mode: Vec<StackOutcome> = modes
            .par_iter()
            .map(|&mode| {
                let mut solutions = Vec::new();
                let mut saw_no_bracket = false;
                let mut saw_trace_failure = false;
                let mut errors: Vec<String> = Vec::new();
                {
                    let tracer = make_tracer(
                        density,
                        models.field_dyn(),
                        models.collisions_dyn(),
                        inputs.freq_mhz,
                        mode,
                        a,
                        tuning,
                    );
                    // The search tracer: same tolerances as the reporting one, drift
                    // diagnostic off. Terminal homing runs hundreds of traces through
                    // it and reads only the landing point off each.
                    let search_tracer = make_tracer(
                        density,
                        models.field_dyn(),
                        models.collisions_dyn(),
                        inputs.freq_mhz,
                        mode,
                        a,
                        tuning.for_search(),
                    );
                    let base_config = homing_config(inputs.use_field);
                    // The elevation fan, traced once for this (mode, stack), on its own
                    // tracer at the bracketing tolerance. Every hop count below brackets
                    // against the same rays; see `ElevationScan` and `StepTuning::for_scan`.
                    let scan_tracer = make_tracer(
                        density,
                        models.field_dyn(),
                        models.collisions_dyn(),
                        inputs.freq_mhz,
                        mode,
                        a,
                        tuning.for_scan(),
                    );
                    // Narrow the scan to the elevations that could serve any hop
                    // count here. The fan is 4..=80 deg in 1 deg steps regardless of
                    // geometry: a 7540 km target cannot be reached with a 60 deg
                    // launch off any modelled layer, nor a 300 km one with 5 deg.
                    //
                    // Only the scan is narrowed. `base_config` keeps the Newton
                    // limits below at full range, so a mis-sized window can at worst
                    // miss a bracket, which `solve_digest` checks for.
                    let mut scan_config = homing_config(inputs.use_field);
                    if let Some((lo, hi)) = (1..=inputs.max_hops)
                        .filter_map(|hops| {
                            hop_elevation_window_deg(total_arc.get() / f64::from(hops))
                        })
                        .fold(None, |acc: Option<(f64, f64)>, (lo, hi)| {
                            Some(acc.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))))
                        })
                    {
                        // Snap outward to the full fan's grid. Brackets come from sign
                        // changes between consecutive samples, so shifting the samples
                        // (say starting at 16.0 deg instead of 4.0) changes which
                        // brackets are found. Whole steps from `elev_min` keep the
                        // scanned elevations a subset of the full fan's, so every
                        // bracket inside the padded window is found identically.
                        let full_lo = base_config.elev_min.get().to_degrees();
                        let full_hi = base_config.elev_max.get().to_degrees();
                        let step = base_config.elev_step.get().to_degrees();
                        let snap = |v: f64, up: bool| {
                            if step <= 0.0 {
                                return v;
                            }
                            let k = (v - full_lo) / step;
                            let k = if up { k.ceil() } else { k.floor() };
                            (full_lo + k * step).clamp(full_lo, full_hi)
                        };
                        scan_config.elev_min = Radians::from_degrees(snap(lo.max(full_lo), false));
                        scan_config.elev_max = Radians::from_degrees(snap(hi.min(full_hi), true));
                    }
                    let scan = scan_elevations(&scan_tracer, &tx, &rx, &scan_config);

                    // Every (hop count, bracket) pair is one independent candidate
                    // ray: its own terminal homing search and its own propagation, with
                    // nothing shared but the read-only models. They are enumerated
                    // first, run across the pool, and folded back in order so the
                    // solution and error lists do not depend on thread timing. The
                    // engine itself traces one ray at a time.
                    let mut work = Vec::new();
                    for hops in 1..=inputs.max_hops {
                        // A hop count whose per-hop arc is over the horizon for every
                        // modelled layer cannot produce a ray. This does not set
                        // `saw_no_bracket`, which means "rays reflect but none lands at
                        // this range" in the per-layer report.
                        if hop_elevation_window_deg(total_arc.get() / f64::from(hops)).is_none() {
                            continue;
                        }
                        let target = if hops == 1 {
                            rx
                        } else {
                            destination_point(
                                &tx,
                                brng,
                                Radians::new(total_arc.get() / f64::from(hops)),
                            )
                        };
                        // The scan brackets against one hop of 1/N of the arc, a cheap
                        // way to enumerate the distinct rays (low/high, E/F2) that could
                        // serve this hop count. `home_terminal` decides which of them
                        // reaches the receiver.
                        let brackets = scan.brackets(central_angle(&tx, &target).get());
                        if brackets.is_empty() {
                            // "Rays reflect but none lands here" differs from "nothing
                            // reflects", and the per-layer report says which.
                            saw_no_bracket = true;
                            continue;
                        }
                        for bracket in brackets {
                            work.push((hops, bracket));
                        }
                    }

                    let limits = (base_config.elev_min.get(), base_config.elev_max.get());
                    let outcomes: Vec<Candidate> = work
                        .par_iter()
                        .map(|&(hops, bracket)| {
                            // The search runs on the loose tracer and only returns a
                            // launch elevation. Acceptance is judged on the terminal miss
                            // of the path `propagate` traces at the engine's tolerance,
                            // which is the path the operator sees.
                            let Some(homed) = home_terminal(
                                &search_tracer,
                                &tx,
                                &rx,
                                hops,
                                // The bracket midpoint. See `ElevationScan::brackets`:
                                // seeding this any closer to the root changes which
                                // ray Newton converges to.
                                0.5 * (bracket.0 + bracket.1),
                                limits,
                                terminal_tol,
                            ) else {
                                return Candidate::NoBracket;
                            };
                            let (details, ends, note) = propagate(
                                &tracer,
                                &tx,
                                homed.elevation,
                                homed.azimuth,
                                hops,
                                f_hz,
                                ground,
                            );
                            // `propagate` returns a note exactly when a hop escaped or
                            // the trace failed, so the ray never reached the receiver
                            // and is not a solution.
                            if let Some(n) = &note {
                                return Candidate::Failed(format!(
                                    "{} mode, {hops} hop(s): {n}",
                                    mode_label(mode)
                                ));
                            }
                            // The terminal miss of the reported path at the engine's
                            // tolerance, re-measured because the search used a looser
                            // integrator.
                            let terminal_miss_m = ends.last().map_or(f64::INFINITY, |e| {
                                central_angle(e, &rx).get() * EARTH_RADIUS_M
                            });
                            if terminal_miss_m.partial_cmp(&terminal_tol) != Some(Ordering::Less) {
                                return Candidate::NoBracket;
                            }
                            let apex_km = details.first().map_or(f64::NAN, |h| h.apex_alt_km);
                            Candidate::Solved(Box::new(assemble(
                                mode,
                                classify_deterministic(apex_km),
                                1.0,
                                hops,
                                details,
                                &ends,
                                &rx,
                                terminal_miss_m,
                                note,
                                inputs.freq_mhz,
                                link_settings,
                            )))
                        })
                        .collect();

                    for outcome in outcomes {
                        match outcome {
                            Candidate::Solved(candidate) => {
                                // Two brackets of the equal-hop scan can converge onto
                                // the same ray once the terminal point is what is being
                                // homed, because the scan brackets a quantity the
                                // refinement no longer targets. List that mode once,
                                // keeping whichever landed closer.
                                match solutions.iter_mut().find(|s| is_same_ray(s, &candidate)) {
                                    Some(existing)
                                        if candidate.homing_miss_m < existing.homing_miss_m =>
                                    {
                                        *existing = *candidate;
                                    }
                                    Some(_) => {}
                                    None => solutions.push(*candidate),
                                }
                            }
                            Candidate::NoBracket => saw_no_bracket = true,
                            Candidate::Failed(msg) => {
                                saw_trace_failure = true;
                                errors.push(msg);
                            }
                        }
                    }
                }
                StackOutcome {
                    solutions,
                    saw_no_bracket,
                    saw_trace_failure,
                    errors,
                }
            })
            .collect();

        let mut merged = StackOutcome {
            solutions: Vec::new(),
            saw_no_bracket: false,
            saw_trace_failure: false,
            errors: Vec::new(),
        };
        for out in per_mode {
            merged.solutions.extend(out.solutions);
            merged.saw_no_bracket |= out.saw_no_bracket;
            merged.saw_trace_failure |= out.saw_trace_failure;
            errors.extend(out.errors);
        }
        merged
    };

    // The two density stacks share nothing but the read-only models, so the
    // deterministic pass and the sporadic-E pass run side by side. Their
    // diagnostics stay in separate lists and are merged below in a fixed order,
    // with the Es pass's messages tagged.
    let es_tuning = StepTuning::for_thin_sheet(a.sporadic_e.semi_thickness_km * 1e3);
    let (deterministic, es_pass) = rayon::join(
        || {
            let mut errs = Vec::new();
            let out = run_stack(models.density_dyn(), StepTuning::DEFAULT, &mut errs);
            (out, errs)
        },
        || {
            models.density_with_es_dyn().map(|d| {
                let mut errs = Vec::new();
                let out = run_stack(d, es_tuning, &mut errs);
                (out, errs)
            })
        },
    );
    let (deterministic, det_errors) = deterministic;
    errors.extend(det_errors);
    let mut solutions = deterministic.solutions;

    // The probabilistic pass. Only reflections that turn in the Es sheet count;
    // anything else duplicates a deterministic solution.
    let (es_band_lo, es_band_hi) = a.sporadic_e.attribution_band_km();
    let mut es_solutions = Vec::new();
    let mut es_saw_no_bracket = false;
    let mut es_saw_trace_failure = false;
    if let Some((out, es_errors)) = es_pass {
        es_saw_no_bracket = out.saw_no_bracket;
        es_saw_trace_failure = out.saw_trace_failure;
        for mut s in out.solutions {
            let apex = s.hop_details.first().map_or(f64::NAN, |h| h.apex_alt_km);
            if apex >= es_band_lo && apex <= es_band_hi {
                s.layer = LayerMode::Es;
                s.probability = a.sporadic_e.probability;
                // Every hop of this path bounced off the sheet once, and each
                // bounce leaks the fraction that tunnels through it. Charged per
                // hop from that hop's own turning point, because a 4-hop path
                // reflects at a steeper incidence and so turns deeper.
                s.es_reflection_loss_db = s
                    .hop_details
                    .iter()
                    .map(|h| a.sporadic_e.reflection_loss_db(h.apex_alt_km))
                    .sum();
                // Re-derive the two figures that depend on the loss total. The
                // link budget is a pure function of the system loss, so this is
                // a recomputation, not a second model.
                s.total_system_loss_db += s.es_reflection_loss_db;
                s.link = crate::noise::LinkBudget::from_settings(
                    link_settings,
                    s.total_system_loss_db - s.total_gain_db,
                );
                es_solutions.push(s);
            }
        }
        // Es-pass errors are tagged because they concern the Es sheet rather
        // than the deterministic ionosphere.
        errors.extend(
            es_errors
                .into_iter()
                .map(|e| format!("sporadic-E pass: {e}")),
        );
    }

    let mut near_misses = Vec::new();
    let mut sweep_notes = Vec::new();
    if solutions.is_empty() && es_solutions.is_empty() {
        for &mode in modes {
            let tracer = make_tracer(
                models.density_dyn(),
                models.field_dyn(),
                models.collisions_dyn(),
                inputs.freq_mhz,
                mode,
                a,
                StepTuning::DEFAULT,
            );
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
    es_solutions.sort_by(|x, y| x.total_group_km.total_cmp(&y.total_group_km));

    // "Every elevation escaped" is reported by the near-miss sweep, and only
    // runs when nothing at all connected; using it here is what separates
    // "above the MUF" from "inside the skip zone".
    let everything_penetrated =
        !sweep_notes.is_empty() && sweep_notes.iter().all(|n| n.contains("no elevation"));

    let mode_reports = build_mode_reports(
        &solutions,
        &es_solutions,
        StackDiagnosis {
            deterministic_no_bracket: deterministic.saw_no_bracket,
            deterministic_trace_failure: deterministic.saw_trace_failure,
            // The sweep runs only on the deterministic stack, so its "everything
            // penetrated" verdict says nothing about Es.
            deterministic_penetrates: everything_penetrated,
            es_no_bracket: es_saw_no_bracket,
            es_trace_failure: es_saw_trace_failure,
        },
        a,
        inputs.snr_threshold_db,
    );

    SolveOutcome {
        solutions,
        es_solutions,
        mode_reports,
        noise,
        snr_threshold_db: inputs.snr_threshold_db,
        near_misses,
        sweep_notes,
        errors,
        great_circle_km,
        bearing_deg: brng.to_degrees().rem_euclid(360.0),
        reverse_bearing_deg: bearing(&rx, &tx).to_degrees().rem_euclid(360.0),
        elapsed_ms: started.elapsed().as_secs_f64() * 1e3,
    }
}

/// What the report builder needs to explain a layer that produced nothing,
/// kept per stack so evidence about one is not attributed to the other.
struct StackDiagnosis {
    deterministic_no_bracket: bool,
    deterministic_trace_failure: bool,
    /// The elevation sweep found that every ray escapes. Only ever measured on
    /// the deterministic stack.
    deterministic_penetrates: bool,
    es_no_bracket: bool,
    es_trace_failure: bool,
}

/// Reduce the solutions to one report per layer.
///
/// A layer that produced a path reports its SNR whether or not it clears the
/// threshold; `ModeReport::state` applies the threshold when colouring. A layer
/// that produced nothing says why, so the caller can tell a skip zone from a
/// frequency that reaches nothing.
fn build_mode_reports(
    solutions: &[Solution],
    es_solutions: &[Solution],
    diag: StackDiagnosis,
    a: &Assumptions,
    threshold_db: f64,
) -> Vec<ModeReport> {
    LayerMode::ALL
        .into_iter()
        .map(|layer| {
            let is_es = layer == LayerMode::Es;
            let pool: &[Solution] = if is_es { es_solutions } else { solutions };
            let best = pool
                .iter()
                .filter(|s| s.layer == layer)
                .max_by(|x, y| x.link.snr_db.total_cmp(&y.link.snr_db));
            let probability = if is_es { a.sporadic_e.probability } else { 1.0 };

            let (status, hops, best_snr_db) = match best {
                Some(s) => (LayerStatus::Solved, s.hops, s.link.snr_db),
                None if is_es && !a.es_solved => (LayerStatus::NotAttempted, 0, f64::NEG_INFINITY),
                None => {
                    let (no_bracket, trace_failure) = if is_es {
                        (diag.es_no_bracket, diag.es_trace_failure)
                    } else {
                        (
                            diag.deterministic_no_bracket,
                            diag.deterministic_trace_failure,
                        )
                    };
                    // Order matters. A ray that reflects but lands short is a
                    // skip zone; a ray that escapes at every elevation is above
                    // the MUF; a trace that failed numerically is neither. Only
                    // the deterministic stack has sweep evidence, so an Es layer
                    // with no bracket and no failure stays NoBracket.
                    let s = if trace_failure && !no_bracket {
                        LayerStatus::Failed
                    } else if no_bracket {
                        LayerStatus::NoBracket
                    } else if !is_es && diag.deterministic_penetrates {
                        LayerStatus::Penetrates
                    } else if is_es {
                        LayerStatus::NoBracket
                    } else {
                        LayerStatus::Penetrates
                    };
                    (s, 0, f64::NEG_INFINITY)
                }
            };

            let note = match (&status, layer) {
                (LayerStatus::Solved, LayerMode::Es) => format!(
                    "{hops}-hop sporadic-E path at {best_snr_db:.1} dB SNR, but only when a \
                     sheet is present: {:.0} % occurrence (foEs {:.1} MHz). This is NOT a \
                     deterministic opening",
                    100.0 * probability,
                    a.sporadic_e.foes_mhz,
                ),
                (LayerStatus::Solved, _) => {
                    format!(
                        "{hops}-hop {} path at {best_snr_db:.1} dB SNR",
                        layer.label()
                    )
                }
                (LayerStatus::NotAttempted, LayerMode::Es) => format!(
                    "sporadic E not solved: occurrence {:.0} % is below the {:.0} % worth a \
                     second pass, or Es is switched off",
                    100.0 * probability,
                    100.0 * crate::sporadic_e::ES_NEGLIGIBLE_PROBABILITY,
                ),
                (LayerStatus::NoBracket, _) => format!(
                    "no {} path: rays reflect, but no launch elevation lands one at this range \
                     - the target is inside this layer's skip zone or past its maximum range. \
                     That is NOT the same as nothing arriving at all",
                    layer.label()
                ),
                (LayerStatus::Penetrates, _) => format!(
                    "no {} path: the ray penetrates at every elevation, so this frequency is \
                     above this layer's maximum usable frequency for any geometry",
                    layer.label()
                ),
                (LayerStatus::Failed, _) => format!(
                    "no {} verdict: the tracer failed on this stack (see the errors list). This \
                     is a NUMERICAL failure, not a statement that nothing arrives",
                    layer.label()
                ),
                (LayerStatus::NotAttempted, _) => format!("{} not attempted", layer.label()),
            };

            ModeReport {
                layer,
                status,
                best_snr_db,
                threshold_db,
                probability,
                hops,
                note,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::link_budget::{free_space_loss_db, ground_reflection_loss_db};
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

    /// Absorption must be non-zero, i.e. the collision model is wired in.
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

    /// Past the engine's 85 deg plane-parallel limit, the Chapman grazing layer
    /// keeps the D region producing, so both the sampled density and the solved
    /// absorption stay well above the F2-only floor (~1e-3 dB).
    #[test]
    fn terminator_d_region_is_not_cut() {
        use skipzone::density::ElectronDensity;
        use skipzone::geo::SphericalPoint;
        use skipzone::units::{Meters, Radians};

        // Denver -> London in January, walked in UTC to a midpoint zenith angle
        // just past 85 deg. The frequency is searched alongside the hour so the
        // test does not depend on which band the foF2 model lets close.
        let mut chosen: Option<(Inputs, scenario::Assumptions)> = None;
        let mut utc = 17.5_f64;
        'search: while utc <= 20.0 {
            for freq in [14.1_f64, 10.1, 7.1, 5.357, 3.57] {
                let inputs = Inputs {
                    utc_hours: utc,
                    freq_mhz: freq,
                    ..Inputs::default()
                };
                let a = scenario::resolve(&inputs);
                if !(85.0..88.0).contains(&a.solar.zenith_angle_deg) {
                    break;
                }
                let models = scenario::build_models(&inputs, &a).expect("models build");
                if !solve(&inputs, &a, &models).solutions.is_empty() {
                    chosen = Some((inputs, a));
                    break 'search;
                }
            }
            utc += 0.1;
        }
        let (inputs, a) =
            chosen.expect("a terminator geometry in 17.5..=20 UTC with a solved path on some band");
        assert!(a.solar.is_day(), "sun is still up just past 85 deg");

        // The D region must still be producing at the midpoint: sample the
        // density model fed to the tracer at its realised peak height.
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

        // The solved path must absorb well above the ~1e-3 dB F2-only floor.
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
    /// active. Checked because the configuration is easy to break.
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
    /// signal under a 5 MHz layer does not reflect at any elevation, so there
    /// is no closest landing to report and the sweep must say so.
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

    /// A frequency that reflects but misses the target range must still
    /// produce ranked near-misses.
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
        // The near-miss sweep runs only when neither stack answered (see the
        // `solutions.is_empty() && es_solutions.is_empty()` gate in `solve`).
        if out.solutions.is_empty() && out.es_solutions.is_empty() {
            assert!(
                !out.near_misses.is_empty() || !out.sweep_notes.is_empty(),
                "expected near-misses or an explanation"
            );
        }
        for w in out.near_misses.windows(2) {
            assert!(w[0].miss_km <= w[1].miss_km, "near misses not sorted");
        }
    }

    /// Daytime absorption reference case.
    /// Denver -> New York at 10.1 MHz, 21 June, 19:00 UTC (local mid-afternoon at
    /// the path midpoint). Compares an F2-only stack with hand-picked collision
    /// numbers against the full stack (D region driven by solar zenith angle,
    /// neutral-atmosphere nu).
    ///
    /// 10.1 MHz rather than 7.1 MHz because a midday foE of ~3.5 MHz screens
    /// 7.1 MHz off F2 below about 28 deg (see
    /// `daytime_e_layer_screens_the_low_bands`).
    #[test]
    fn daytime_absorption_before_after() {
        use skipzone::collision::ExponentialCollisions;
        use skipzone::density::{ChapmanLayer, MultiLayer};
        use skipzone::mag::Igrf;
        use skipzone::units::{Meters, PerCubicMeter, PerSecond};

        // Denver -> New York, 2620 km, 21 June 19:00 UTC: local mid-afternoon
        // at the path midpoint, so the D region is at full strength. 10.1 MHz
        // is above the E-screening frequency for this geometry, so the F2 path
        // exists.
        let inputs = Inputs {
            freq_mhz: 10.1,
            month: 6,
            day_of_month: 21,
            utc_hours: 19.0,
            rx_lat: 40.7,
            rx_lon: -74.0,
            max_hops: 3,
            ..Inputs::default()
        };
        let a = scenario::resolve(&inputs);

        let after_models = scenario::build_models(&inputs, &a).expect("after models");
        let after = solve(&inputs, &a, &after_models);

        // Reference: F2 alone, no D or E region, and a hand-picked collision
        // profile (1e5 /s at 100 km, H = 30 km).
        let field = || {
            Some(
                Igrf::from_embedded()
                    .unwrap()
                    .model_at(inputs.igrf_epoch)
                    .unwrap(),
            )
        };
        let f2_layer = || {
            ChapmanLayer::new(
                PerCubicMeter::new(a.nm_per_m3),
                Meters::new(scenario::EARTH_RADIUS_M + a.hmf2_km * 1e3),
                Meters::new(a.scale_height_km * 1e3),
            )
            .unwrap()
        };
        let before_models = scenario::Models {
            density: MultiLayer::new(vec![Box::new(f2_layer())]),
            density_with_es: None,
            field: field(),
            collisions: ExponentialCollisions::new(
                PerSecond::new(1.0e5),
                Meters::new(scenario::EARTH_RADIUS_M + 100e3),
                Meters::new(30e3),
            )
            .unwrap(),
        };
        let before = solve(&inputs, &a, &before_models);

        // Isolation run: F2 only, with the neutral-atmosphere nu. Absorption
        // here is F2/deviative; the rest of the full-stack figure is D region.
        let isolate_models = scenario::Models {
            density: MultiLayer::new(vec![Box::new(f2_layer())]),
            density_with_es: None,
            field: field(),
            collisions: ExponentialCollisions::new(
                PerSecond::new(scenario::NU_REF_PER_S),
                Meters::new(scenario::EARTH_RADIUS_M + scenario::NU_REF_ALT_KM * 1e3),
                Meters::new(scenario::NU_SCALE_HEIGHT_KM * 1e3),
            )
            .unwrap(),
        };
        let isolate = solve(&inputs, &a, &isolate_models);

        // Same path and date, but local midnight at the midpoint (which sits
        // near 89.5 W, so LST = UTC - 5.97 h).
        let night_inputs = Inputs {
            utc_hours: 6.0,
            ..inputs.clone()
        };
        let night_a = scenario::resolve(&night_inputs);
        let night_models = scenario::build_models(&night_inputs, &night_a).expect("night models");
        let night = solve(&night_inputs, &night_a, &night_models);

        // Use the strongest path: the full stack may list an E-mode solution
        // first, while the other two stacks are F2-only.
        let first_abs = |o: &SolveOutcome| best_by_snr(o).map_or(0.0, |s| s.total_absorption_db);
        let (a_before, a_after) = (first_abs(&before), first_abs(&after));
        let (a_isolate, a_night) = (first_abs(&isolate), first_abs(&night));
        println!(
            "midpoint {:.2},{:.2} chi = {:.2} deg; BEFORE {a_before:.3} dB, \
             ISOLATE {a_isolate:.5} dB, AFTER {a_after:.3} dB, NIGHT {a_night:.3} dB",
            a.midpoint_lat, a.midpoint_lon, a.solar.zenith_angle_deg
        );

        assert!(a.d_region_active, "21 June local noon must be daylight");
        assert!(!after.solutions.is_empty(), "errors: {:?}", after.errors);

        // With nu falling off with neutral density, the F2 region contributes
        // almost nothing. The broad hand-picked profile only got a plausible
        // magnitude by putting collisions at F2 heights.
        assert!(
            a_isolate < 0.05,
            "F2-only with neutral nu should be negligible, got {a_isolate} dB"
        );
        assert!(
            a_after > 20.0 * a_isolate,
            "absorption should now be dominated by the D region: {a_after} vs {a_isolate}"
        );

        // Absorption must respond to solar zenith angle: night must be far
        // quieter than local noon.
        assert!(
            !night_a.d_region_active,
            "local midnight should be past the Chapman limit"
        );
        assert!(
            a_night < 0.5 * a_after,
            "night absorption {a_night} dB should collapse relative to day {a_after} dB"
        );
    }

    /// E-layer screening. A daytime E layer of foE ~ 3.5 MHz reflects 7 MHz at
    /// every incidence beyond about 62 deg, so the shallow launches a long
    /// multi-hop F2 path needs never reach F2. The same daytime path closes on
    /// F2 at 10.1 MHz but not at 7.1 MHz.
    #[test]
    fn daytime_e_layer_screens_the_low_bands() {
        // Denver -> New York in local mid-afternoon: short enough that both
        // frequencies have a geometry to work with, so the only thing that
        // differs between them is whether the E layer lets them through.
        let base = Inputs {
            month: 6,
            day_of_month: 21,
            utc_hours: 19.0,
            rx_lat: 40.7,
            rx_lon: -74.0,
            max_hops: 3,
            ..Inputs::default()
        };
        let a = scenario::resolve(&base);
        assert!(
            (3.0..4.2).contains(&a.foe_midpoint_mhz),
            "midday foE should be 3-4 MHz, got {}",
            a.foe_midpoint_mhz
        );

        let low = run(&Inputs {
            freq_mhz: 7.1,
            ..base.clone()
        });
        let high = run(&Inputs {
            freq_mhz: 10.1,
            ..base.clone()
        });

        // The high band gets its F2 path.
        assert!(
            high.mode_reports
                .iter()
                .any(|r| r.layer == LayerMode::F2 && r.status == LayerStatus::Solved),
            "10.1 MHz should still reach F2"
        );

        // Screening is about launch angle. foE sec(i) lets 10.1 MHz through at
        // a shallow incidence but not 7.1 MHz, so the low band reaches F2 only
        // steeply, and a steep multi-hop path is heavily absorbed. Expect no
        // shallow F2 path at 7.1 MHz, a shallow one at 10.1 MHz, and an
        // unusable F2 SNR at 7.1 MHz. Steep F2 paths at 7.1 MHz do exist, so
        // `LayerStatus::NoBracket` would be the wrong assertion.
        let shallowest = |o: &SolveOutcome| {
            o.solutions
                .iter()
                .filter(|s| s.layer == LayerMode::F2)
                .filter_map(|s| s.hop_details.first())
                .map(|h| h.launch_elev_deg)
                .fold(f64::INFINITY, f64::min)
        };
        let (low_f2, high_f2) = (shallowest(&low), shallowest(&high));
        assert!(
            high_f2 < 25.0,
            "10.1 MHz should get a shallow F2 path through the E layer, got {high_f2} deg"
        );
        assert!(
            low_f2 > 30.0,
            "7.1 MHz should be screened off every shallow F2 geometry, got {low_f2} deg"
        );

        let best_f2 = |o: &SolveOutcome| {
            o.solutions
                .iter()
                .filter(|s| s.layer == LayerMode::F2)
                .map(|s| s.link.snr_db)
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let (low_snr, high_snr) = (best_f2(&low), best_f2(&high));
        assert!(
            low_snr < high_snr - 40.0,
            "the screened band's only F2 route is the absorbed steep one:              7.1 MHz {low_snr:.1} dB vs 10.1 MHz {high_snr:.1} dB"
        );
        assert!(
            low_snr < base.snr_threshold_db,
            "7.1 MHz F2 must not be usable at midday, got {low_snr:.1} dB"
        );
    }

    /// Auto-detect must classify each bounce from where it actually lands, and
    /// must leave every other manual selection alone. Denver -> London crosses
    /// North America and then the Atlantic, so a multi-hop path should pick up
    /// both land and sea bounces.
    #[test]
    fn auto_detect_classifies_each_bounce_from_its_own_position() {
        use crate::scenario::GroundType;

        let inputs = Inputs {
            ground_type: GroundType::AutoDetect,
            ground_land_fallback: GroundType::DryGround,
            ..Inputs::default()
        };
        let out = run(&inputs);
        assert!(!out.solutions.is_empty(), "errors: {:?}", out.errors);

        let mut seen = Vec::new();
        for s in &out.solutions {
            for h in &s.hop_details {
                // A reflection carries a surface and a reason; the final hop,
                // which arrives at the receiver, carries neither.
                assert_eq!(h.ground_label.is_some(), h.ground_reason.is_some());
                if let (Some(label), Some(reason)) = (h.ground_label, h.ground_reason.as_ref()) {
                    println!(
                        "{}-mode {} hop(s), hop {}: {label} - {reason}",
                        mode_label(s.mode),
                        s.hops,
                        h.index
                    );
                    assert!(reason.contains("reflection point"), "reason: {reason}");
                    // Only water, or the operator's land fallback, may appear:
                    // auto-detect never invents a soil type.
                    assert!(
                        label == GroundType::SeaWater.label()
                            || label == GroundType::FreshWater.label()
                            || label == GroundType::DryGround.label(),
                        "unexpected surface {label}"
                    );
                    seen.push(label);
                }
                // No reflection, no ground loss.
                if h.ground_label.is_none() {
                    assert_eq!(h.ground_loss_db, 0.0);
                }
            }
        }
        assert!(!seen.is_empty(), "a multi-hop path should have bounces");
        assert!(
            seen.contains(&GroundType::SeaWater.label()),
            "a transatlantic path must bounce off the sea somewhere: {seen:?}"
        );
    }

    /// A manual selection applies one surface to every bounce.
    #[test]
    fn manual_ground_still_applies_one_surface_to_the_whole_path() {
        use crate::scenario::GroundType;

        let inputs = Inputs {
            ground_type: GroundType::DryGround,
            // Different on purpose; it must be ignored.
            ground_land_fallback: GroundType::WetGround,
            ..Inputs::default()
        };
        let out = run(&inputs);
        assert!(!out.solutions.is_empty(), "errors: {:?}", out.errors);
        for s in &out.solutions {
            for h in &s.hop_details {
                assert!(h.ground_reason.is_none(), "manual needs no explanation");
                if let Some(label) = h.ground_label {
                    assert_eq!(label, GroundType::DryGround.label());
                }
            }
        }
    }

    /// The per-layer report is complete, consistent with the solutions it
    /// describes, and keeps the probabilistic layer separate from the
    /// deterministic ones.
    #[test]
    fn mode_reports_cover_every_layer_and_agree_with_the_solutions() {
        for inputs in [
            Inputs::default(),
            // A short summer-afternoon path, where Es is the interesting layer.
            Inputs {
                freq_mhz: 18.1,
                month: 7,
                day_of_month: 24,
                utc_hours: 3.37,
                tx_lat: 40.0,
                tx_lon: -105.0,
                rx_lat: 43.6,
                rx_lon: -105.0,
                max_hops: 2,
                ..Inputs::default()
            },
            // Far above every MUF.
            Inputs {
                freq_mhz: 45.0,
                ..Inputs::default()
            },
        ] {
            let out = run(&inputs);

            // Exactly one report per layer, in a fixed order.
            assert_eq!(out.mode_reports.len(), LayerMode::ALL.len());
            for (r, want) in out.mode_reports.iter().zip(LayerMode::ALL) {
                assert_eq!(r.layer, want);
                assert_eq!(r.threshold_db, inputs.snr_threshold_db);
                // Deterministic layers are certain; Es carries a probability
                // and never claims to be certain.
                if want.is_deterministic() {
                    assert_eq!(r.probability, 1.0, "{} must be deterministic", want.label());
                } else {
                    assert!((0.0..=1.0).contains(&r.probability));
                }
                assert!(!r.note.is_empty());

                // A report that says "solved" must have a solution behind it,
                // with exactly that SNR; one that does not must carry -inf,
                // never a stale number.
                let pool = if want == LayerMode::Es {
                    &out.es_solutions
                } else {
                    &out.solutions
                };
                let best = pool
                    .iter()
                    .filter(|s| s.layer == want)
                    .map(|s| s.link.snr_db)
                    .fold(f64::NEG_INFINITY, f64::max);
                if r.status == LayerStatus::Solved {
                    assert_eq!(r.best_snr_db, best, "{} SNR disagrees", want.label());
                    assert!(r.hops >= 1);
                } else {
                    assert_eq!(r.best_snr_db, f64::NEG_INFINITY);
                    assert_eq!(r.hops, 0);
                    assert_eq!(best, f64::NEG_INFINITY);
                }
            }

            // Es solutions never leak into the deterministic list, and every
            // one of them carries the occurrence probability rather than 1.
            assert!(out.solutions.iter().all(|s| s.layer != LayerMode::Es));
            assert!(out.solutions.iter().all(|s| s.probability == 1.0));
            for s in &out.es_solutions {
                assert_eq!(s.layer, LayerMode::Es);
                assert_eq!(s.probability, a_probability(&inputs));
            }

            // `best_by_snr` is deterministic-only.
            assert!(best_by_snr(&out).is_none_or(|s| s.layer != LayerMode::Es));
            // `best_with_es_fallback` is ordinal: a deterministic path outranks
            // an Es one however strong, and Es appears only when nothing
            // deterministic closed.
            match (best_by_snr(&out), best_with_es_fallback(&out)) {
                (Some(det), Some(any)) => {
                    assert_eq!(
                        any.link.snr_db.to_bits(),
                        det.link.snr_db.to_bits(),
                        "a deterministic path existed, so it must be the reported one"
                    );
                    assert!(any.layer != LayerMode::Es);
                }
                (None, Some(any)) => assert_eq!(
                    any.layer,
                    LayerMode::Es,
                    "with nothing deterministic, only Es can answer"
                ),
                (None, None) => assert!(out.es_solutions.is_empty()),
                (Some(_), None) => panic!("a deterministic path must never be dropped"),
            }
        }
    }

    fn a_probability(inputs: &Inputs) -> f64 {
        scenario::resolve(inputs).sporadic_e.probability
    }

    /// A 17 m signal at 400 km: F2 has no solution at that geometry but Es
    /// does, and the output must report both.
    #[test]
    fn es_supported_path_is_reported_separately_from_a_dead_one() {
        // 2026-07-24, 03:22 UTC, 18.1 MHz, ~400 km: the WSPR geometry.
        let inputs = Inputs {
            freq_mhz: 18.1,
            month: 7,
            day_of_month: 24,
            utc_hours: 3.37,
            tx_lat: 40.0,
            tx_lon: -105.0,
            rx_lat: 43.6,
            rx_lon: -105.0,
            tx_power_w: 0.2,
            max_hops: 2,
            bandwidth_hz: 2500.0,
            snr_threshold_db: -29.0,
            ..Inputs::default()
        };
        let out = run(&inputs);

        let f2 = &out.mode_reports[0];
        let es = &out.mode_reports[2];
        assert_eq!(f2.layer, LayerMode::F2);
        assert_eq!(es.layer, LayerMode::Es);

        // No deterministic path...
        assert!(
            out.solutions.is_empty(),
            "F2/E should not close at 400 km on 17 m here"
        );
        assert_ne!(f2.status, LayerStatus::Solved);
        // ...but a real, reported, probabilistic one.
        assert_eq!(es.status, LayerStatus::Solved, "{}", es.note);
        assert!(!out.es_solutions.is_empty());
        assert!(es.probability > 0.1, "occurrence {}", es.probability);
        assert!(es.best_snr_db.is_finite());
        assert!(
            es.note.contains("NOT a deterministic opening"),
            "the Es note must not read like a certainty: {}",
            es.note
        );

        // The reflection happened in the sheet.
        let (lo, hi) = scenario::resolve(&inputs).sporadic_e.attribution_band_km();
        for s in &out.es_solutions {
            let apex = s.hop_details[0].apex_alt_km;
            assert!(
                (lo..=hi).contains(&apex),
                "Es apex {apex} km outside {lo}..{hi}"
            );
        }

        // The near-miss sweep did not run, since a path was found.
        assert!(out.near_misses.is_empty());
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
