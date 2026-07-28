//! Area coverage: one transmitter, a grid of receiver positions, and the
//! *existing* point-to-point calculation run once per grid point.
//!
//! There is deliberately no coverage-specific physics, link budget or
//! propagation shortcut in this module. A grid point is evaluated by moving the
//! receiver to it and calling the same [`crate::solve::solve`] the RUN TRACE
//! button calls - path loss, TX power to received power, antenna gain by takeoff
//! angle, noise floor and per-hop ground/coastline detection all come from that
//! one chain. This file only decides *which* receiver positions to try and
//! reduces each solve to the one number the map colours by.
//!
//! # Why the engine models can be built once
//!
//! [`crate::scenario::build_models`] depends on the scenario only through the
//! sunspot number, hmF2, scale height, the collision profile, the IGRF epoch and
//! the solar *declination* - and declination (Cooper 1969) is a function of the
//! day of year alone. None of those depend on where the receiver is, so one
//! `Models` is valid for every grid point, exactly as the frequency sweep reuses
//! one `Models` across every candidate frequency. [`crate::scenario::resolve`],
//! which *does* vary with the receiver (day/night and season at the receiving
//! site feed the noise floor), is re-run per grid point. The
//! `cell_matches_a_full_point_to_point_solve` test pins that equivalence.

use skipzone::magnetoionic::Mode;

use crate::noise::PathState;
use crate::scenario::{self, EARTH_RADIUS_M, Inputs, Models};
use crate::solve::{self, LayerMode, LayerStatus};

/// Upper bound on grid points in one run. This is a guard against an
/// accidentally enormous job, not a quality knob: a full solve costs on the
/// order of a second, so a grid this size is already a deliberate, long run.
pub const MAX_POINTS: usize = 19_881; // 141 x 141

/// Grid points nearer the transmitter than this are skipped. At zero range the
/// great-circle bearing is undefined and there is no hop geometry to home; this
/// is a domain limit of the point-to-point solver, not a coverage heuristic.
pub const MIN_RANGE_KM: f64 = 25.0;

/// Highest latitude a grid row may sit at. Web Mercator diverges at the poles
/// and the tile would be unrenderable, so the row is simply not computed.
///
/// Public because the map plugin clamps a *tile's* drawn edges to the same
/// limit: a row centred just inside it still extends half a step past it, and
/// Web Mercator's `tan(lat).asinh()` changes sign beyond the pole, which throws
/// the rectangle to the far side of the world.
pub const LAT_LIMIT_DEG: f64 = 85.0;

/// The user-controlled grid: how far out from the transmitter to go, and how
/// finely to sample.
///
/// `points_per_deg` is the resolution setting, and it is real: raising it
/// computes more grid points. Nothing anywhere interpolates or smooths between
/// them, so a coarse setting looks blocky - that blockiness is an honest picture
/// of how much was actually computed.
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

    /// The half-span the run will actually cover, which is the requested one
    /// only until the [`MAX_POINTS`] cap bites.
    ///
    /// [`half_steps`](Self::half_steps) clamps the lattice at 70 steps either
    /// way, so a fine resolution silently shrinks the box: 180 degrees at 4
    /// pts/deg is really 17.5. Exposed so the control can say so rather than
    /// leaving the operator to wonder why most of the map came back empty.
    #[must_use]
    pub fn effective_half_span_deg(self) -> f64 {
        f64::from(self.half_steps()) * self.step_deg()
    }

    /// Longitude columns to generate, centred on the transmitter.
    ///
    /// The naive `2 * half_steps() + 1` columns can span more than 360 degrees -
    /// at the widest extent and coarsest resolution it spans 380 - and because
    /// column longitudes are wrapped into [-180, 180), the surplus lands back on
    /// top of columns already generated. Those duplicate positions are solved
    /// twice and, since the tiles are drawn with alpha, painted twice: the
    /// doubled column reads as a dark stripe across the map. Capping the count
    /// at one full turn of longitude removes the overlap without leaving a gap,
    /// because a whole number of steps short of 360 still closes the ring.
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
/// Every field is read straight off the solve; nothing is rescaled or blended.
///
/// # Deterministic and probabilistic, side by side
///
/// A cell carries BOTH verdicts, because they are different kinds of answer and
/// a map that folds them together is exactly what produced the reported "hard
/// dead zone": a position the F2 layer cannot reach was painted the same grey
/// as a position nothing can reach, even when a sporadic-E opening puts a real
/// signal there a third of the time. `snr_db` is the strongest mode of any
/// kind - what a listener would hear on a day when everything present is
/// present - and `probability` says how often that day comes.
#[derive(Clone, Copy)]
pub struct CoverageCell {
    pub lat: f64,
    pub lon: f64,
    /// Edge length of the square this point stands for, degrees. Carried on the
    /// cell so tiles from a finished run keep their own size even if the
    /// resolution control is moved afterwards.
    pub step_deg: f64,
    pub state: PathState,
    /// SNR [dB] of the strongest mode of ANY kind; `-inf` when nothing arrives.
    /// A continuous field: the threshold is applied only for display.
    pub snr_db: f64,
    /// SNR [dB] of the strongest DETERMINISTIC (F2 or E) mode; `-inf` when
    /// there is none. Equal to `snr_db` unless a sporadic-E path wins.
    pub deterministic_snr_db: f64,
    /// Probability the mode behind `snr_db` is present at all: 1 for F2 and E,
    /// the occurrence probability for Es.
    pub probability: f64,
    /// Which layer `snr_db` came from.
    pub layer: Option<LayerMode>,
    /// Why the deterministic stack produced nothing, when it produced nothing.
    /// This is what separates "inside the F2 skip zone" from "nothing arrives
    /// at all" - the map's colour key promised that distinction long before the
    /// solver could make it.
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

/// Solve one grid point: move the receiver there and run the whole existing
/// point-to-point chain, then keep the strongest mode - the signal an operator
/// at that position would actually hear.
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
                // Nothing deterministic closed: report the F2 layer's own
                // reason, which is the one an operator looking at a dead zone
                // is actually asking about.
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

    /// The load-bearing claim of this module: a coverage cell is the point-to-
    /// point answer, not an approximation of it. Solving a grid position through
    /// `solve_cell` with the shared models must reproduce, bit for bit, what the
    /// app produces when the operator drags the receiver to that position and
    /// presses RUN TRACE (models rebuilt from scratch for that scenario).
    #[test]
    fn cell_matches_a_full_point_to_point_solve() {
        let inputs = Inputs::default();
        let shared_a = scenario::resolve(&inputs);
        let shared_models = scenario::build_models(&inputs, &shared_a).expect("models");

        for (lat, lon) in [(51.5, -0.13), (35.0, -40.0), (60.0, 20.0), (-10.0, -70.0)] {
            let cell = solve_cell(lat, lon, 4.0, &inputs, &shared_models);

            // The independent reference: the whole chain rebuilt for a scenario
            // whose receiver IS that grid point.
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

    /// Raising the resolution must produce more genuinely computed points, and
    /// the run can never exceed the hard cap.
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

    /// The widest extent at the coarsest resolution used to generate 19 columns
    /// of 20 degrees - 380 degrees - so one column wrapped back onto another and
    /// was solved and painted twice. No grid may contain a duplicate position,
    /// and the ring must still close.
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
                // ...and never a gap left in one, when a full turn was both
                // asked for and actually granted. A run the point cap has
                // truncated covers less than the requested extent by design,
                // and `effective_half_span_deg` is what reports that.
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
    /// centre. Clamped to the Mercator limit that band must stay a real,
    /// non-empty piece of the map - the plugin relies on it.
    #[test]
    fn tile_bands_stay_inside_the_mercator_limit() {
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
