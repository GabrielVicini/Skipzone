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
//!
//! The work is split by concern across the submodules: [`types`] holds the
//! result structs the UI renders, [`link_budget`] the free-space and ground
//! reflection loss terms, and [`tracing`] the per-hop tracing/homing helpers.
//! This module keeps only the top-level [`solve`] driver that stitches them
//! together.

mod link_budget;
mod tracing;
mod types;

pub use types::{Solution, SolveOutcome, mode_label};

use skipzone::geo::{bearing, central_angle};
use skipzone::homing::{Homing, HomingError};
use skipzone::magnetoionic::Mode;
use skipzone::units::Radians;

use crate::noise::LinkSettings;
use crate::scenario::{
    self, Assumptions, EARTH_RADIUS_M, Inputs, Models, destination_point, ground_point,
};

use tracing::{assemble, homing_config, make_tracer, near_miss_sweep, propagate};

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

    // The judgment layer's inputs, fixed for this solve. Computed at THIS
    // frequency (not the tuned one) so the frequency sweep, which re-solves
    // against one `Assumptions`, gets the right floor at every candidate.
    let noise = scenario::noise_floor_at(inputs, a, inputs.freq_mhz);

    // Antenna patterns, built once per solve. Each curve costs a hemispherical
    // power integral (see `crate::antenna::image`), so it is computed here and
    // sampled per solution rather than recomputed per angle. Both ends stand
    // over the scenario's ground type.
    let antenna_ground = inputs.ground_type.as_antenna_ground();
    let tx_antenna = inputs.tx_antenna.curve(antenna_ground, f_hz);
    let rx_antenna = inputs.rx_antenna.curve(antenna_ground, f_hz);

    let link_settings = LinkSettings {
        tx_power_w: inputs.tx_power_w,
        noise,
        threshold_db: inputs.snr_threshold_db,
        tx_antenna: &tx_antenna,
        rx_antenna: &rx_antenna,
    };

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
                            link_settings,
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
