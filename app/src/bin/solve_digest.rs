//! A stable fingerprint of everything `solve()` produces, over a scenario grid.
//!
//! # Why this exists
//!
//! The solver is about to be made faster by NOT tracing rays that cannot
//! possibly reach the target. That is only safe if it changes which rays are
//! skipped and nothing else - and "nothing else" has to be demonstrated, not
//! asserted, because every calibrated number in this project was fitted against
//! solver output. If the solution set moves, the calibration silently stops
//! describing the model it was fitted to.
//!
//! So: run this BEFORE the change, keep the output, run it AFTER, and diff. A
//! byte-identical diff is a proof that no reported ray moved, which means the
//! WSPR calibration does not need re-running to stay valid. That is the whole
//! point - a 40-minute re-fit is a poor substitute for an exact comparison.
//!
//! The grid deliberately covers the geometries where an elevation window is most
//! likely to clip something real: very short paths that need steep launches, very
//! long paths that need grazing ones, the skip zone where the low and high rays
//! meet, multi-hop, both magnetoionic modes, day and night, and the sporadic-E
//! stack, whose thin sheet reflects at an altitude no F-layer window would expect.
//!
//! Run:
//! ```text
//! cargo run --release -p skipzone-app --bin solve_digest > before.txt
//! ```

use std::process::ExitCode;

use skipzone_app::antenna::{AntennaConfig, AntennaKind};
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve;

/// Transmitter position for the whole grid. Mid-latitude, non-zero longitude so
/// no term degenerates on a special value.
const TX_LAT_DEG: f64 = 52.0;
const TX_LON_DEG: f64 = -3.0;

/// Place the receiver `range_km` from the transmitter on a given bearing, by
/// direct spherical construction so the range is exact rather than approximated.
fn place(inputs: Inputs, range_km: f64, bearing_deg: f64) -> Inputs {
    let d = range_km * 1e3 / scenario::EARTH_RADIUS_M;
    let (lat1, lon1) = (TX_LAT_DEG.to_radians(), TX_LON_DEG.to_radians());
    let brg = bearing_deg.to_radians();
    let lat2 = (lat1.sin() * d.cos() + lat1.cos() * d.sin() * brg.cos()).asin();
    let lon2 = lon1 + (brg.sin() * d.sin() * lat1.cos()).atan2(d.cos() - lat1.sin() * lat2.sin());
    Inputs {
        tx_lat: TX_LAT_DEG,
        tx_lon: TX_LON_DEG,
        rx_lat: lat2.to_degrees(),
        rx_lon: lon2.to_degrees(),
        ..inputs
    }
}

fn main() -> ExitCode {
    let antenna = AntennaConfig {
        kind: AntennaKind::Isotropic,
        ..AntennaConfig::default()
    };

    println!("# solve() digest - every reported solution, over a scenario grid.");
    println!("# Fields are printed at fixed precision so the file diffs cleanly.");
    println!(
        "# range_km bearing utc freq_mhz es | layer hops elev_deg arc_km absorb free_sp ground snr apex_km"
    );

    let mut n_scenarios = 0usize;
    let mut n_solutions = 0usize;

    // Ranges chosen around the awkward places: NVIS, the skip zone, the
    // single-hop horizon, and well beyond it where only multi-hop can serve.
    for &range_km in &[
        120.0, 300.0, 547.0, 900.0, 1500.0, 2200.0, 3000.0, 3800.0, 4500.0, 6000.0, 7540.0, 9000.0,
        12000.0,
    ] {
        // Two bearings so the field geometry (and the magnetoionic split) is not
        // sampled at one orientation only.
        for &bearing in &[70.0, 250.0] {
            // Local noon, dusk and midnight at the transmitter.
            for &utc in &[12.0, 18.0, 0.0] {
                for &freq in &[1.838, 3.570, 7.040, 14.097, 21.096, 28.126] {
                    for &es in &[false, true] {
                        let inputs = place(
                            Inputs {
                                freq_mhz: freq,
                                utc_hours: utc,
                                ssn: 98.0,
                                month: 7,
                                day_of_month: 5,
                                es_enabled: es,
                                tx_antenna: antenna,
                                rx_antenna: antenna,
                                ..Inputs::default()
                            },
                            range_km,
                            bearing,
                        );
                        let a = scenario::resolve(&inputs);
                        let Ok(models) = scenario::build_models(&inputs, &a) else {
                            continue;
                        };
                        let out = solve::solve(&inputs, &a, &models);
                        n_scenarios += 1;

                        // Every solution, in the order the solver reports them.
                        // Order is part of the contract: `best_by_snr` and the Es
                        // fallback both break ties by position in this list.
                        for s in &out.solutions {
                            n_solutions += 1;
                            println!(
                                "{range_km:.0} {bearing:.0} {utc:.1} {freq:.3} {es} \
                                 | {:<2} {} {:.6} {:.4} {:.6} {:.6} {:.6} {:.6} {:.4}",
                                s.layer.label(),
                                s.hops,
                                s.hop_details
                                    .first()
                                    .map_or(f64::NAN, |h| h.launch_elev_deg),
                                s.total_arc_km,
                                s.total_absorption_db,
                                s.free_space_loss_db,
                                s.ground_reflection_loss_db,
                                s.link.snr_db,
                                // The apex is the sharpest single check that a ray
                                // is the SAME ray: an elevation window that clipped
                                // the high ray and left the low one would keep the
                                // count and the layer and move this.
                                s.hop_details.first().map_or(f64::NAN, |h| h.apex_alt_km),
                            );
                        }
                        // The absence of a solution is part of the fingerprint too:
                        // a "faster" solver that quietly stops finding a ray would
                        // otherwise show up as a shorter file rather than a diff.
                        if out.solutions.is_empty() {
                            println!("{range_km:.0} {bearing:.0} {utc:.1} {freq:.3} {es} | NONE");
                        }
                    }
                }
            }
        }
    }
    println!("# {n_scenarios} scenarios, {n_solutions} solutions");
    ExitCode::SUCCESS
}
