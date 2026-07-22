//! Homing: find launch (elevation, azimuth) landing a ray at a target.
//!
//! Two independent methods, cross-checked by the validation suite:
//! 1. `home_scan`: elevation scan at the great-circle bearing, bracketing
//!    sign changes of the along-track range error, then alternating 1D
//!    bisection (elevation for along-track, azimuth for cross-track).
//!    Slow, derivative-free, robust.
//! 2. `home_newton`: 2D Newton on (elevation, azimuth) -> (along, cross)
//!    from a scan seed, Jacobian by finite differences of full traces.
//!    FD is used here deliberately (documented exception to the
//!    analytic-gradients rule): the analytic alternative is integrating
//!    6x6 variational equations along each ray; the landing map is smooth
//!    away from caustics, an FD Jacobian at 1e-4 rad resolves it to ~1e-6
//!    relative, and Newton tolerates approximate Jacobians. Near caustics
//!    (skip-zone edge) Newton legitimately fails and reports; it never
//!    silently accepts a bad miss.
//!
//! Multipath is first-class: a reachable target inside the maximum range
//! generally has a low and a high ray; all brackets found are refined and
//! returned sorted by elevation.

use crate::collision::CollisionFrequency;
use crate::density::ElectronDensity;
use crate::error::TraceError;
use crate::geo::{SphericalPoint, bearing, central_angle, track_errors};
use crate::mag::MagneticField;
use crate::trace::{Outcome, TraceResult, Tracer};
use crate::units::Radians;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum HomingError {
    /// No elevation in the scanned range lands at the target range: the
    /// target is inside the skip zone, beyond the maximum range, or every
    /// scanned ray escaped/failed.
    #[error(
        "no launch elevation in [{scan_min_deg:.1}, {scan_max_deg:.1}] deg brackets the target range {target_km:.1} km"
    )]
    NoBracket {
        scan_min_deg: f64,
        scan_max_deg: f64,
        target_km: f64,
    },

    /// Refinement did not reach the miss tolerance within the iteration
    /// budget (typically a caustic / skip-zone-edge target).
    #[error("homing did not converge: best miss {best_miss_m:.1} m after {iters} iterations")]
    NoConvergence { best_miss_m: f64, iters: usize },

    /// A trace inside refinement failed; scan-phase failures are tolerated
    /// (they bound brackets), refinement-phase ones are not.
    #[error("trace failed during homing refinement: {0}")]
    Trace(#[from] TraceError),
}

pub struct HomingConfig {
    /// Elevation scan range. Default 4..=80 deg: below ~4 deg ground
    /// interaction and tropospheric effects this crate does not model
    /// dominate; above 80 deg near-vertical geometry gives no useful
    /// range and risks the Spitze configuration.
    pub elev_min: Radians,
    pub elev_max: Radians,
    /// Scan step. 1 deg resolves the range-vs-elevation curve of F-layer
    /// scenarios (its scale is several degrees); halve it if a scenario's
    /// high-ray branch is missed.
    pub elev_step: Radians,
    /// Accept when the great-circle miss is below this. 30 m is far below
    /// any HF application's meaning and safely above the tracer's own
    /// landing accuracy (~sub-metre at the default rtol).
    pub miss_tolerance_m: f64,
    /// Newton budget; the landing map is nearly linear at these scales, so
    /// convergence takes ~3-6 iterations or it will not converge at all.
    pub max_iters: usize,
    /// FD step for the Newton Jacobian, rad. 1e-4 (~6 mdeg) balances
    /// truncation against landing-position noise (~1 m / FD step).
    pub fd_step: f64,
}

impl Default for HomingConfig {
    fn default() -> Self {
        Self {
            elev_min: Radians::from_degrees(4.0),
            elev_max: Radians::from_degrees(80.0),
            elev_step: Radians::from_degrees(1.0),
            miss_tolerance_m: 30.0,
            max_iters: 25,
            fd_step: 1e-4,
        }
    }
}

#[derive(Debug)]
pub struct HomedRay {
    pub elevation: Radians,
    pub azimuth: Radians,
    pub result: TraceResult,
    /// Great-circle distance from the landing point to the target, m.
    pub miss_m: f64,
}

/// Landing metrics of one launch relative to the target track frame:
/// along-track error (rad, positive = overshoot), cross-track (rad).
struct Miss {
    along_err: f64,
    cross: f64,
    result: TraceResult,
}

pub struct Homing<'a, 'b, D: ?Sized, B: ?Sized, C: ?Sized> {
    pub tracer: &'b Tracer<'a, D, B, C>,
    pub config: HomingConfig,
}

impl<D, B, C> Homing<'_, '_, D, B, C>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    fn shoot(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
        elev: f64,
        az: f64,
    ) -> Result<Option<Miss>, TraceError> {
        let res = self
            .tracer
            .trace(from, Radians::new(elev), Radians::new(az))?;
        if res.outcome != Outcome::Landed {
            return Ok(None);
        }
        let track = bearing(from, to);
        let target = central_angle(from, to).get();
        let (along, cross) = track_errors(from, track, &res.end);
        Ok(Some(Miss {
            along_err: along.get() - target,
            cross: cross.get(),
            result: res,
        }))
    }

    /// Method 1: scan + alternating bisection. Returns all solutions.
    ///
    /// # Errors
    /// `NoBracket` if the scan finds no sign change; `NoConvergence` /
    /// `Trace` from refinement.
    pub fn home_scan(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
    ) -> Result<Vec<HomedRay>, HomingError> {
        let brackets = self.scan_brackets(from, to)?;
        let az0 = bearing(from, to).get();
        let mut out = Vec::new();
        for (e_lo, e_hi) in brackets {
            let ray = self.refine_bisect(from, to, e_lo, e_hi, az0)?;
            out.push(ray);
        }
        Ok(out)
    }

    /// Method 2: scan for seeds, then 2D Newton with an FD Jacobian.
    ///
    /// # Errors
    /// As `home_scan`.
    pub fn home_newton(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
    ) -> Result<Vec<HomedRay>, HomingError> {
        let brackets = self.scan_brackets(from, to)?;
        let az0 = bearing(from, to).get();
        let mut out = Vec::new();
        for (e_lo, e_hi) in brackets {
            let ray = self.refine_newton(from, to, 0.5 * (e_lo + e_hi), az0)?;
            out.push(ray);
        }
        Ok(out)
    }

    /// Elevation intervals over which the along-track error changes sign
    /// (traced at the direct great-circle bearing). Per-ray trace failures
    /// and escapes act as bracket boundaries, not errors.
    fn scan_brackets(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
    ) -> Result<Vec<(f64, f64)>, HomingError> {
        let az0 = bearing(from, to).get();
        let (e0, e1, de) = (
            self.config.elev_min.get(),
            self.config.elev_max.get(),
            self.config.elev_step.get(),
        );
        let mut brackets = Vec::new();
        let mut prev: Option<(f64, f64)> = None;
        let mut e = e0;
        while e <= e1 {
            let here = match self.shoot(from, to, e, az0) {
                Ok(Some(m)) => Some((e, m.along_err)),
                Ok(None) | Err(_) => None,
            };
            if let (Some((pe, pv)), Some((ce, cv))) = (prev, here) {
                if pv.signum() != cv.signum() {
                    brackets.push((pe, ce));
                }
            }
            if here.is_some() {
                prev = here;
            } else {
                prev = None;
            }
            e += de;
        }
        if brackets.is_empty() {
            return Err(HomingError::NoBracket {
                scan_min_deg: self.config.elev_min.to_degrees(),
                scan_max_deg: self.config.elev_max.to_degrees(),
                target_km: central_angle(from, to).get() * from.r.get() / 1e3,
            });
        }
        Ok(brackets)
    }

    fn refine_bisect(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
        mut e_lo: f64,
        mut e_hi: f64,
        az0: f64,
    ) -> Result<HomedRay, HomingError> {
        let r0 = from.r.get();
        let mut az = az0;
        let mut best: Option<(f64, f64, Miss)> = None;
        let mut v_lo = match self.shoot(from, to, e_lo, az)? {
            Some(m) => m.along_err,
            None => {
                return Err(HomingError::NoConvergence {
                    best_miss_m: f64::INFINITY,
                    iters: 0,
                });
            }
        };
        for it in 0..self.config.max_iters {
            // 1D bisection in elevation on the along-track error at fixed
            // azimuth, to the angular scale of the miss tolerance.
            for _ in 0..60 {
                let mid = 0.5 * (e_lo + e_hi);
                let Some(m) = self.shoot(from, to, mid, az)? else {
                    // A gap inside the bracket (escape): shrink toward lo.
                    e_hi = mid;
                    continue;
                };
                let vm = m.along_err;
                if vm.signum() == v_lo.signum() {
                    e_lo = mid;
                    v_lo = vm;
                } else {
                    e_hi = mid;
                }
                if (e_hi - e_lo) * r0 < 0.3 * self.config.miss_tolerance_m {
                    break;
                }
            }
            let e = 0.5 * (e_lo + e_hi);
            let m = self
                .shoot(from, to, e, az)?
                .ok_or(HomingError::NoConvergence {
                    best_miss_m: f64::INFINITY,
                    iters: it,
                })?;
            let miss = (m.along_err.hypot(m.cross)) * r0;
            let done = miss < self.config.miss_tolerance_m;
            // Azimuth correction: rotate the launch bearing by the negative
            // cross-track angle (exact for a spherically symmetric medium,
            // first-order otherwise).
            az -= m.cross;
            best = Some((e, az, m));
            if done {
                break;
            }
            // Re-bracket around the current elevation for the next pass.
            let half = (e_hi - e_lo).max(1e-6);
            e_lo = e - half;
            e_hi = e + half;
            v_lo = match self.shoot(from, to, e_lo, az)? {
                Some(m2) => m2.along_err,
                None => {
                    return Err(HomingError::NoConvergence {
                        best_miss_m: miss,
                        iters: it,
                    });
                }
            };
        }
        let (e, az, m) = best.ok_or(HomingError::NoConvergence {
            best_miss_m: f64::INFINITY,
            iters: self.config.max_iters,
        })?;
        let miss_m = m.along_err.hypot(m.cross) * r0;
        if miss_m >= self.config.miss_tolerance_m {
            return Err(HomingError::NoConvergence {
                best_miss_m: miss_m,
                iters: self.config.max_iters,
            });
        }
        Ok(HomedRay {
            elevation: Radians::new(e),
            azimuth: Radians::new(az),
            result: m.result,
            miss_m,
        })
    }

    fn refine_newton(
        &self,
        from: &SphericalPoint,
        to: &SphericalPoint,
        mut e: f64,
        mut az: f64,
    ) -> Result<HomedRay, HomingError> {
        let r0 = from.r.get();
        let h = self.config.fd_step;
        let mut best_miss = f64::INFINITY;
        for it in 0..self.config.max_iters {
            let m = self
                .shoot(from, to, e, az)?
                .ok_or(HomingError::NoConvergence {
                    best_miss_m: best_miss,
                    iters: it,
                })?;
            let miss = m.along_err.hypot(m.cross) * r0;
            best_miss = best_miss.min(miss);
            if miss < self.config.miss_tolerance_m {
                return Ok(HomedRay {
                    elevation: Radians::new(e),
                    azimuth: Radians::new(az),
                    result: m.result,
                    miss_m: miss,
                });
            }
            // FD Jacobian of (along_err, cross) w.r.t. (elev, az).
            let me = self
                .shoot(from, to, e + h, az)?
                .ok_or(HomingError::NoConvergence {
                    best_miss_m: best_miss,
                    iters: it,
                })?;
            let ma = self
                .shoot(from, to, e, az + h)?
                .ok_or(HomingError::NoConvergence {
                    best_miss_m: best_miss,
                    iters: it,
                })?;
            let j = [
                [
                    (me.along_err - m.along_err) / h,
                    (ma.along_err - m.along_err) / h,
                ],
                [(me.cross - m.cross) / h, (ma.cross - m.cross) / h],
            ];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-12 {
                // Singular landing map: caustic; a smaller step cannot fix it.
                return Err(HomingError::NoConvergence {
                    best_miss_m: best_miss,
                    iters: it,
                });
            }
            let de = (-m.along_err * j[1][1] + m.cross * j[0][1]) / det;
            let da = (m.along_err * j[1][0] - m.cross * j[0][0]) / det;
            e += de;
            az += da;
        }
        Err(HomingError::NoConvergence {
            best_miss_m: best_miss,
            iters: self.config.max_iters,
        })
    }
}
