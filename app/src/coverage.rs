//! Area coverage: one transmitter, a grid of receiver positions, and the
//! point-to-point calculation run once per grid point.
//!
//! There is no coverage-specific physics here. A grid point is evaluated by
//! moving the receiver to it and calling the same [`crate::solve::solve`] as
//! the RUN TRACE button, so path loss, antenna gain, noise floor and per-hop
//! ground detection all come from that one chain. This module chooses the
//! receiver positions and reduces each solve to the number the map colours by.
//!
//! # Why the engine models can be built once
//!
//! [`crate::scenario::build_models`] depends on the scenario only through the
//! sunspot number, hmF2, scale height, the collision profile, the IGRF epoch and
//! the solar *declination* - and declination (Cooper 1969) is a function of the
//! day of year alone. None depend on the receiver position, so one `Models`
//! serves every grid point, as the frequency sweep reuses one across
//! frequencies. [`crate::scenario::resolve`] does depend on the receiver (its
//! day/night and season feed the noise floor) and is re-run per grid point.
//! `cell_matches_a_full_point_to_point_solve` checks the equivalence.

use skipzone::magnetoionic::Mode;

use crate::noise::PathState;
use crate::scenario::{self, EARTH_RADIUS_M, Inputs, Models};
use crate::solve::{self, LayerMode, LayerStatus};

/// Upper bound on grid points in one run, a guard against an accidentally
/// enormous job: a full solve costs about a second.
pub const MAX_POINTS: usize = 19_881; // 141 x 141

/// Grid points nearer the transmitter than this are skipped. At zero range the
/// great-circle bearing is undefined and there is no hop geometry to home.
pub const MIN_RANGE_KM: f64 = 25.0;

/// Highest latitude a grid row may sit at.
///
/// At exactly +/-90 the great-circle bearing from the transmitter is undefined;
/// one degree short of the pole keeps it defined.
///
/// Public because the map clamps a tile's drawn edges to the same limit: a row
/// centred just inside it still extends half a step past it, and nothing may be
/// painted past the pole.
pub const LAT_LIMIT_DEG: f64 = 89.0;

/// The user-controlled grid: how far out from the transmitter to go, and how
/// finely to sample.
///
/// `points_per_deg` sets how many grid points are computed. Nothing
/// interpolates between them, so a coarse setting looks blocky.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CoverageConfig {
    /// Half-width of the lat/lon box centred on the transmitter, degrees.
    pub half_span_deg: f64,
    /// Grid points per degree, in both latitude and longitude.
    pub points_per_deg: f64,
}

impl Default for CoverageConfig {
    /// A grid that finishes in a couple of minutes on a desktop: 40 degrees out
    /// in each direction at one point every 4 degrees.
    fn default() -> Self {
        Self {
            half_span_deg: 40.0,
            points_per_deg: 0.25,
        }
    }
}

impl CoverageConfig {
    /// Spacing between adjacent grid points, degrees. This is also the edge
    /// length of the square each computed point is drawn as.
    #[must_use]
    pub fn step_deg(self) -> f64 {
        (1.0 / self.points_per_deg.max(1e-3)).clamp(0.05, 30.0)
    }

    /// Grid rows/columns either side of the transmitter, clamped so the full
    /// square can never exceed [`MAX_POINTS`].
    #[must_use]
    pub fn half_steps(self) -> i32 {
        #[allow(clippy::cast_possible_truncation)]
        let n = (self.half_span_deg / self.step_deg()).floor() as i32;
        n.clamp(1, 70)
    }

    /// The half-span the run will cover, which is the requested one until the
    /// [`MAX_POINTS`] cap applies.
    ///
    /// [`half_steps`](Self::half_steps) clamps the lattice at 70 steps either
    /// way, so a fine resolution shrinks the box (180 degrees at 4 pts/deg is
    /// really 17.5). Exposed so the control can show it.
    #[must_use]
    pub fn effective_half_span_deg(self) -> f64 {
        f64::from(self.half_steps()) * self.step_deg()
    }

    /// Longitude columns to generate, centred on the transmitter.
    ///
    /// `2 * half_steps() + 1` columns can span more than 360 degrees (380 at the
    /// widest extent and coarsest resolution). With longitudes wrapped into
    /// [-180, 180), the surplus would be solved and painted twice, showing as a
    /// dark stripe. Capping at one full turn removes the overlap without a gap.
    #[must_use]
    pub fn columns(self) -> i32 {
        let step = self.step_deg();
        #[allow(clippy::cast_possible_truncation)]
        let full_turn = (360.0 / step).floor() as i32;
        (2 * self.half_steps() + 1).min(full_turn.max(1))
    }

    /// The receiver positions to solve, in row-major order from the south-west
    /// corner. Positions past the latitude limit or inside [`MIN_RANGE_KM`] of
    /// the transmitter are omitted, so this is the exact count of solves the run
    /// will perform.
    #[must_use]
    pub fn grid(self, tx_lat: f64, tx_lon: f64) -> Vec<(f64, f64)> {
        let step = self.step_deg();
        let n = self.half_steps();
        let cols = self.columns();
        // Keep the columns centred on the transmitter when one has been dropped
        // to close the ring: the extra column comes off the eastern end.
        let (j0, j1) = (-(cols / 2), -(cols / 2) + cols - 1);
        let tx = scenario::ground_point(tx_lat, tx_lon);
        let mut points = Vec::new();
        for i in -n..=n {
            let lat = tx_lat + f64::from(i) * step;
            if lat.abs() > LAT_LIMIT_DEG {
                continue;
            }
            for j in j0..=j1 {
                let lon = wrap_lon(tx_lon + f64::from(j) * step);
                let rx = scenario::ground_point(lat, lon);
                let km = skipzone::geo::central_angle(&tx, &rx).get() * EARTH_RADIUS_M / 1e3;
                if km < MIN_RANGE_KM {
                    continue;
                }
                points.push((lat, lon));
            }
        }
        points
    }
}

/// Longitude wrapped to [-180, 180).
fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// One solved grid point: where it is, and what the receiver there would hear.
///
/// Every field is read straight off the solve.
///
/// # Deterministic and probabilistic verdicts
///
/// A cell carries both verdicts. Folding them together would paint a position
/// the F2 layer cannot reach like one nothing reaches, even when sporadic E puts
/// a signal there a third of the time. `snr_db` is the strongest mode of any
/// kind and `probability` says how often it is present.
#[derive(Clone, Copy)]
pub struct CoverageCell {
    pub lat: f64,
    pub lon: f64,
    /// Edge length of the square this point stands for, degrees. Carried on the
    /// cell so tiles from a finished run keep their own size even if the
    /// resolution control is moved afterwards.
    pub step_deg: f64,
    pub state: PathState,
    /// SNR (dB) of the strongest mode of any kind; `-inf` when nothing arrives.
    /// A continuous field: the threshold is applied only for display.
    pub snr_db: f64,
    /// SNR (dB) of the strongest deterministic (F2 or E) mode; `-inf` when
    /// there is none. Equal to `snr_db` unless a sporadic-E path wins.
    pub deterministic_snr_db: f64,
    /// Probability the mode behind `snr_db` is present at all: 1 for F2 and E,
    /// the occurrence probability for Es.
    pub probability: f64,
    /// Which layer `snr_db` came from.
    pub layer: Option<LayerMode>,
    /// Why the deterministic stack produced nothing, when it produced nothing.
    /// This separates "inside the F2 skip zone" from "nothing arrives at all".
    pub deterministic_status: LayerStatus,
    pub rx_power_dbm: f64,
    pub noise_dbm: f64,
    /// `snr_db - threshold`; `-inf` when no path was found.
    pub margin_db: f64,
    /// Great-circle range from the transmitter, km.
    pub range_km: f64,
    pub hops: u32,
    pub mode: Option<Mode>,
}

impl CoverageCell {
    #[must_use]
    pub fn found_path(self) -> bool {
        self.state.found_path()
    }

    /// True when the only thing reaching this position needs sporadic E.
    #[must_use]
    pub fn es_only(self) -> bool {
        self.layer == Some(LayerMode::Es)
    }

    /// True when a deterministic path exists.
    #[must_use]
    pub fn has_deterministic_path(self) -> bool {
        self.deterministic_snr_db.is_finite()
    }
}

/// Solve one grid point: move the receiver there, run the point-to-point chain,
/// and keep the strongest mode.
///
/// `models` is the shared, receiver-independent model set (see the module
/// documentation); `step_deg` is recorded on the cell so a tile keeps the size
/// of the run that produced it.
#[must_use]
pub fn solve_cell(
    lat: f64,
    lon: f64,
    step_deg: f64,
    inputs: &Inputs,
    models: &Models,
) -> CoverageCell {
    let mut point_inputs = inputs.clone();
    point_inputs.rx_lat = lat;
    point_inputs.rx_lon = lon;

    let a = scenario::resolve(&point_inputs);
    let out = solve::solve(&point_inputs, &a, models);
    let range_km = out.great_circle_km;

    let deterministic_snr_db =
        solve::best_by_snr(&out).map_or(f64::NEG_INFINITY, |s| s.link.snr_db);
    let deterministic_status = out
        .mode_reports
        .iter()
        .filter(|r| r.layer.is_deterministic())
        .find(|r| r.status == LayerStatus::Solved)
        .map_or_else(
            || {
                // Nothing deterministic closed: report the F2 layer's reason,
                // which is what a dead zone on the map is asking about.
                out.mode_reports
                    .iter()
                    .find(|r| r.layer == LayerMode::F2)
                    .map_or(LayerStatus::NotAttempted, |r| r.status)
            },
            |r| r.status,
        );

    if let Some(best) = solve::best_with_es_fallback(&out) {
        CoverageCell {
            lat,
            lon,
            step_deg,
            state: best.link.state(),
            snr_db: best.link.snr_db,
            deterministic_snr_db,
            probability: best.probability,
            layer: Some(best.layer),
            deterministic_status,
            rx_power_dbm: best.link.rx_power_dbm,
            noise_dbm: best.link.noise.power_dbm,
            margin_db: best.link.margin_db(),
            range_km,
            hops: best.hops,
            mode: Some(best.mode),
        }
    } else {
        CoverageCell {
            lat,
            lon,
            step_deg,
            state: PathState::NoPath,
            snr_db: f64::NEG_INFINITY,
            deterministic_snr_db,
            probability: 0.0,
            layer: None,
            deterministic_status,
            rx_power_dbm: f64::NEG_INFINITY,
            noise_dbm: out.noise.power_dbm,
            margin_db: f64::NEG_INFINITY,
            range_km,
            hops: 0,
            mode: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A coverage cell is the point-to-point answer: solving a grid position
    /// through `solve_cell` with the shared models must match, bit for bit, a
    /// RUN TRACE with the receiver at that position (models rebuilt from
    /// scratch).
    #[test]
    fn cell_matches_a_full_point_to_point_solve() {
        let inputs = Inputs::default();
        let shared_a = scenario::resolve(&inputs);
        let shared_models = scenario::build_models(&inputs, &shared_a).expect("models");

        for (lat, lon) in [(51.5, -0.13), (35.0, -40.0), (60.0, 20.0), (-10.0, -70.0)] {
            let cell = solve_cell(lat, lon, 4.0, &inputs, &shared_models);

            // The independent reference: the whole chain rebuilt for a scenario
            // whose receiver is that grid point.
            let mut reference_inputs = inputs.clone();
            reference_inputs.rx_lat = lat;
            reference_inputs.rx_lon = lon;
            let a = scenario::resolve(&reference_inputs);
            let models = scenario::build_models(&reference_inputs, &a).expect("models");
            let out = solve::solve(&reference_inputs, &a, &models);

            let (state, snr, prx, hops) = solve::best_with_es_fallback(&out).map_or(
                (
                    PathState::NoPath,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                    0u32,
                ),
                |s| (s.link.state(), s.link.snr_db, s.link.rx_power_dbm, s.hops),
            );
            assert_eq!(cell.state, state, "state differs at {lat},{lon}");
            assert_eq!(
                cell.snr_db.to_bits(),
                snr.to_bits(),
                "SNR differs at {lat},{lon}: {} vs {snr}",
                cell.snr_db
            );
            assert_eq!(cell.rx_power_dbm.to_bits(), prx.to_bits());
            assert_eq!(cell.hops, hops);
        }
    }

    #[test]
    fn grid_is_centred_stepped_and_bounded() {
        let cfg = CoverageConfig {
            half_span_deg: 10.0,
            points_per_deg: 0.5, // 2 deg steps
        };
        assert!((cfg.step_deg() - 2.0).abs() < 1e-12);
        assert_eq!(cfg.half_steps(), 5);

        let points = cfg.grid(40.0, -105.0);
        assert!(!points.is_empty());
        // Every point sits on the lattice through the transmitter.
        for (lat, lon) in &points {
            assert!(((lat - 40.0) / 2.0).fract().abs() < 1e-9, "lat {lat}");
            assert!(*lat <= LAT_LIMIT_DEG && *lat >= -LAT_LIMIT_DEG);
            assert!((-180.0..180.0).contains(lon), "lon {lon}");
        }
        // The transmitter's own cell is inside MIN_RANGE_KM and must be omitted.
        assert!(
            !points
                .iter()
                .any(|&(lat, lon)| (lat - 40.0).abs() < 1e-9 && (lon + 105.0).abs() < 1e-9),
            "the transmitter's own position has no path to solve"
        );
        // 11 x 11 lattice minus the excluded centre.
        assert_eq!(points.len(), 11 * 11 - 1);
    }

    /// Raising the resolution must produce more computed points, and the run
    /// can never exceed the hard cap.
    #[test]
    fn resolution_adds_real_points_and_is_capped() {
        let coarse = CoverageConfig {
            half_span_deg: 20.0,
            points_per_deg: 0.25,
        };
        let fine = CoverageConfig {
            half_span_deg: 20.0,
            points_per_deg: 1.0,
        };
        assert!(fine.grid(0.0, 0.0).len() > 4 * coarse.grid(0.0, 0.0).len());

        let absurd = CoverageConfig {
            half_span_deg: 180.0,
            points_per_deg: 8.0,
        };
        assert!(absurd.grid(0.0, 0.0).len() <= MAX_POINTS);
    }

    /// At the widest extent and coarsest resolution, 19 columns of 20 degrees
    /// would span 380 degrees. No grid may contain a duplicate position, and
    /// the ring must still close.
    #[test]
    fn columns_never_wrap_onto_themselves() {
        for &(half_span, res) in &[
            (180.0, 0.05),
            (180.0, 0.25),
            (180.0, 1.0),
            (90.0, 0.1),
            (170.0, 0.06),
        ] {
            let cfg = CoverageConfig {
                half_span_deg: half_span,
                points_per_deg: res,
            };
            for &(tx_lat, tx_lon) in &[(39.74, -104.99), (0.0, 178.0), (55.0, 0.0)] {
                let points = cfg.grid(tx_lat, tx_lon);
                let mut seen: Vec<(u64, u64)> = points
                    .iter()
                    .map(|&(lat, lon)| (lat.to_bits(), lon.to_bits()))
                    .collect();
                let before = seen.len();
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(
                    seen.len(),
                    before,
                    "duplicate grid position at half_span {half_span}, res {res}, tx {tx_lat},{tx_lon}"
                );
                // Never more than one full turn of longitude...
                let span = f64::from(cfg.columns()) * cfg.step_deg();
                assert!(span <= 360.0 + 1e-9, "span {span} at res {res}");
                // ...and no gap when a full turn was asked for and granted. A run
                // truncated by the point cap covers less, as
                // `effective_half_span_deg` reports.
                if 2.0 * cfg.effective_half_span_deg() >= 360.0 {
                    assert!(
                        span > 360.0 - cfg.step_deg() - 1e-9,
                        "gap left in the ring: {span} at res {res}"
                    );
                }
            }
        }
    }

    /// Every computed row is drawn as a band half a step tall either side of its
    /// centre. Clamped to the latitude limit, that band must stay non-empty;
    /// the drawing layer relies on it.
    #[test]
    fn tile_bands_stay_inside_the_latitude_limit() {
        let cfg = CoverageConfig {
            half_span_deg: 180.0,
            points_per_deg: 0.05,
        };
        let half = 0.5 * cfg.step_deg();
        for &tx_lat in &[39.74, 55.0, 5.0, -60.0] {
            for (lat, _) in cfg.grid(tx_lat, 0.0) {
                let south = (lat - half).max(-LAT_LIMIT_DEG);
                let north = (lat + half).min(LAT_LIMIT_DEG);
                assert!(north > south, "empty band at {lat} (tx {tx_lat})");
                assert!(south >= -LAT_LIMIT_DEG && north <= LAT_LIMIT_DEG);
            }
        }
    }

    #[test]
    fn effective_half_span_reports_the_point_cap() {
        let uncapped = CoverageConfig {
            half_span_deg: 180.0,
            points_per_deg: 0.05,
        };
        assert!((uncapped.effective_half_span_deg() - 180.0).abs() < 1e-9);

        let capped = CoverageConfig {
            half_span_deg: 180.0,
            points_per_deg: 4.0,
        };
        assert!(capped.effective_half_span_deg() < 180.0);
        assert!((capped.effective_half_span_deg() - 17.5).abs() < 1e-9);
    }

    #[test]
    fn longitudes_wrap_across_the_antimeridian() {
        let cfg = CoverageConfig {
            half_span_deg: 10.0,
            points_per_deg: 0.2, // 5 deg steps
        };
        let points = cfg.grid(0.0, 178.0);
        assert!(points.iter().any(|&(_, lon)| lon < 0.0), "must wrap west");
        assert!(
            points
                .iter()
                .all(|&(_, lon)| (-180.0..180.0).contains(&lon))
        );
    }
}
