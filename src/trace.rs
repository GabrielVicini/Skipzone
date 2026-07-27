//! Ray trace driver: adaptive stepping, event location, observables.
//!
//! Events (ground landing, domain-top escape, apex passage) are located by
//! re-integrating a partial step from the last accepted state and root
//! finding on it; each trial is a single full-order RK step no larger than
//! an already-accepted one, so event states carry the integrator's own
//! accuracy rather than an interpolant's.

use crate::collision::CollisionFrequency;
use crate::density::ElectronDensity;
use crate::error::TraceError;
use crate::geo::SphericalPoint;
use crate::hamiltonian::{RayEquations, STATE_DIM, State};
use crate::integrate::Dopri5;
use crate::mag::MagneticField;
use crate::magnetoionic::Mode;
use crate::units::{Hertz, Meters, Nepers, Radians};

/// Relative tolerance default. At 1e-10 the validated field-free scenarios
/// land within ~10 cm over 1000+ km paths (tests); looser degrades landing
/// accuracy roughly linearly, tighter approaches roundoff with more steps.
pub const DEFAULT_RTOL: f64 = 1e-10;

/// Per-component absolute error floors, in the state's own units. Chosen at
/// the precision that is physically meaningless to resolve further:
/// 1e-3 m in position/paths (mm), 1e-12 rad in angles (~6 um of ground),
/// 1e-12 in the dimensionless momentum (n resolved to 1e-12), 1e-9 Np in
/// absorption. Too small only costs steps; too large lets the component
/// drift unchecked when it passes near zero.
pub const ATOL: [f64; STATE_DIM] = [
    1e-3, 1e-12, 1e-12, 1e-12, 1e-12, 1e-12, 1e-3, 1e-3, 1e-9, 1e-3,
];

/// Hairer's standard controller constants: accept if the scaled RMS error
/// is <= 1, propose h * SAFETY * err^(-1/5) clamped to [FAC_MIN, FAC_MAX]
/// (growth also capped at 1.0 right after a rejection to prevent
/// accept/reject oscillation). The exponent is 1/(embedded order + 1).
const SAFETY: f64 = 0.9;
const FAC_MIN: f64 = 0.2;
const FAC_MAX: f64 = 5.0;
const ERR_EXP: f64 = -0.2;

/// Event refinement: landing/escape radius resolved to 1 um (far below
/// physical meaning, a few extra partial steps), apex ray-parameter bracket
/// to 1e-6 m. Iteration cap is a hard stop against pathological brackets.
const EVENT_R_TOL: f64 = 1e-6;
const EVENT_SIGMA_TOL: f64 = 1e-6;
const EVENT_MAX_ITER: usize = 80;

pub struct TraceConfig {
    /// Ground radius: landing surface, m.
    pub r_ground: Meters,
    /// Domain top: a ray above this radius moving outward has escaped, m.
    pub r_top: Meters,
    pub rtol: f64,
    /// First trial step, m of ray parameter. 100 m is small enough that the
    /// controller's first error estimate is trustworthy in any HF scenario;
    /// the controller grows it immediately in smooth regions.
    pub initial_step: f64,
    /// Step cap, m. Bounds how far an event can overshoot before detection;
    /// error control alone sets accuracy.
    pub max_step: f64,
    /// Below this step the medium is unresolvable at the tolerance
    /// (typically a profile kink): typed failure, never a silent clamp.
    pub min_step: f64,
    /// Runaway guard; a multi-hop HF ray needs ~1e4 steps, so 5e5 means
    /// something is wrong.
    pub max_steps: usize,
    /// Measure `hamiltonian_drift` along the ray.
    ///
    /// The drift is a pure DIAGNOSTIC: `H = 0` on the true trajectory, so
    /// `max |H|` over the accepted steps reports how far the integrator has
    /// wandered off shell. It is not fed back into the solution in any way.
    ///
    /// Measuring it costs one full evaluation of the density, field and
    /// collision models per accepted step, on top of the six the
    /// Dormand-Prince stages already need - measured at 1 in every 7.2
    /// evaluations a trace performs, i.e. about 14 % of all model work. That is
    /// worth paying on a ray whose numbers are going to be reported, and pure
    /// waste on the hundreds of rays a homing search throws away. Callers that
    /// are searching turn it off and get `hamiltonian_drift = 0.0`; callers
    /// that report leave it on and get exactly the figure they always did.
    pub measure_drift: bool,
}

impl TraceConfig {
    #[must_use]
    pub fn new(r_ground: Meters, r_top: Meters) -> Self {
        Self {
            r_ground,
            r_top,
            rtol: DEFAULT_RTOL,
            initial_step: 100.0,
            max_step: 25_000.0,
            min_step: 1e-3,
            max_steps: 500_000,
            measure_drift: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Ray returned to the ground radius (descending).
    Landed,
    /// Ray crossed the domain top moving outward (layer penetration).
    Escaped,
}

/// One apex (turning point) of the ray path.
#[derive(Clone, Copy, Debug)]
pub struct Apex {
    pub r: Meters,
    /// X = (f_p/f)^2 at the apex; the reflection-condition tests assert
    /// n^2 r^2 = C^2 here (field-free) via this and the state.
    pub x: f64,
}

#[derive(Debug)]
pub struct TraceResult {
    pub outcome: Outcome,
    pub end: SphericalPoint,
    /// Momentum m = (c/omega)k at the end state (physical components).
    pub end_m: [f64; 3],
    pub group_path: Meters,
    pub phase_path: Meters,
    pub absorption: Nepers,
    pub arc_length: Meters,
    pub apexes: Vec<Apex>,
    pub steps: usize,
    /// max |H| over accepted steps: on-shell drift, an integrator-quality
    /// diagnostic (H = 0 exactly on the true trajectory).
    pub hamiltonian_drift: f64,
    /// Total ray parameter at the end state, m.
    pub sigma: f64,
}

pub struct Tracer<'a, D: ?Sized, B: ?Sized, C: ?Sized> {
    pub eqs: RayEquations<'a, D, B, C>,
    pub config: TraceConfig,
}

impl<'a, D, B, C> Tracer<'a, D, B, C>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    pub fn new(
        density: &'a D,
        field: &'a B,
        collisions: &'a C,
        f: Hertz,
        mode: Mode,
        config: TraceConfig,
    ) -> Self {
        Self {
            eqs: RayEquations::new(density, field, collisions, f, mode),
            config,
        }
    }

    /// Trace from a launch point until landing, escape, or failure.
    ///
    /// # Errors
    /// All `TraceError` variants: evanescent launch, pole proximity, step
    /// collapse, step budget, non-finite state.
    pub fn trace(
        &self,
        launch: &SphericalPoint,
        elevation: Radians,
        azimuth: Radians,
    ) -> Result<TraceResult, TraceError> {
        self.trace_with_observer(launch, elevation, azimuth, &mut |_, _| {})
    }

    /// `trace`, additionally calling `observer(sigma, state)` at every
    /// accepted step (validation hooks: invariant checks along the path).
    ///
    /// # Errors
    /// As `trace`.
    #[allow(clippy::too_many_lines)] // one driver loop; splitting scatters the event logic
    pub fn trace_with_observer(
        &self,
        launch: &SphericalPoint,
        elevation: Radians,
        azimuth: Radians,
        observer: &mut dyn FnMut(f64, &State),
    ) -> Result<TraceResult, TraceError> {
        let mut y = self.eqs.initial_state(launch, elevation, azimuth)?;
        let mut rk = Dopri5::<STATE_DIM>::new();
        let mut f = |s: &State, ds: &mut State| self.eqs.rhs(s, ds);
        let mut k1 = [0.0; STATE_DIM];
        f(&y, &mut k1)?;

        // Degenerate launches that no step will ever bracket: already at or
        // below the ground heading down, or above the top heading up.
        if y[0] <= self.config.r_ground.get() && k1[0] < 0.0 {
            return Ok(Self::finish(Outcome::Landed, y, 0.0, 0, Vec::new(), 0.0));
        }
        if y[0] >= self.config.r_top.get() && k1[0] > 0.0 {
            return Ok(Self::finish(Outcome::Escaped, y, 0.0, 0, Vec::new(), 0.0));
        }

        let mut h = self.config.initial_step;
        let mut sigma = 0.0;
        let mut steps = 0usize;
        let mut rejected_last = false;
        let mut apexes = Vec::new();
        let mut drift = 0.0f64;

        let (mut y5, mut k7, mut err) = ([0.0; STATE_DIM], [0.0; STATE_DIM], [0.0; STATE_DIM]);
        loop {
            if steps >= self.config.max_steps {
                return Err(TraceError::MaxStepsExceeded {
                    max_steps: steps,
                    sigma,
                });
            }
            if h < self.config.min_step {
                return Err(TraceError::StepSizeCollapse {
                    sigma,
                    step: h,
                    min_step: self.config.min_step,
                });
            }
            h = h.min(self.config.max_step);
            rk.try_step(&mut f, &y, &k1, h, &mut y5, &mut k7, &mut err)?;
            steps += 1;

            let mut err_norm_sq = 0.0;
            for i in 0..STATE_DIM {
                let sc = ATOL[i] + self.config.rtol * y[i].abs().max(y5[i].abs());
                err_norm_sq += (err[i] / sc) * (err[i] / sc);
            }
            #[allow(clippy::cast_precision_loss)]
            let err_norm = (err_norm_sq / STATE_DIM as f64).sqrt();

            if err_norm > 1.0 {
                rejected_last = true;
                h *= (SAFETY * err_norm.powf(ERR_EXP)).max(FAC_MIN);
                continue;
            }

            // Accepted. Growth capped after a rejection (controller notes).
            let grow_cap = if rejected_last { 1.0 } else { FAC_MAX };
            rejected_last = false;
            let h_next = h * (SAFETY * err_norm.powf(ERR_EXP)).clamp(FAC_MIN, grow_cap);

            if y5.iter().any(|v| !v.is_finite()) {
                return Err(TraceError::NonFiniteState { sigma });
            }

            // Apex: radial motion turned downward inside this step.
            if k1[0] > 0.0 && k7[0] < 0.0 {
                let (ya, _) = Self::refine(&mut rk, &mut f, &y, &k1, h, RefineTarget::Apex)?;
                apexes.push(Apex {
                    r: Meters::new(ya[0]),
                    x: self.x_at(&ya),
                });
            }
            // Landing: crossed the ground radius going down.
            if y5[0] <= self.config.r_ground.get() && y[0] > self.config.r_ground.get() {
                let (ye, se) = Self::refine(
                    &mut rk,
                    &mut f,
                    &y,
                    &k1,
                    h,
                    RefineTarget::Radius(self.config.r_ground.get()),
                )?;
                return Ok(Self::finish(
                    Outcome::Landed,
                    ye,
                    sigma + se,
                    steps,
                    apexes,
                    drift,
                ));
            }
            // Escape through the top, moving outward.
            if y5[0] >= self.config.r_top.get() && k7[0] > 0.0 {
                let (ye, se) = Self::refine(
                    &mut rk,
                    &mut f,
                    &y,
                    &k1,
                    h,
                    RefineTarget::Radius(self.config.r_top.get()),
                )?;
                return Ok(Self::finish(
                    Outcome::Escaped,
                    ye,
                    sigma + se,
                    steps,
                    apexes,
                    drift,
                ));
            }

            sigma += h;
            y = y5;
            k1 = k7;
            h = h_next;
            if self.config.measure_drift {
                drift = drift.max(self.eqs.hamiltonian(&y)?.abs());
            }
            observer(sigma, &y);
        }
    }

    /// Fixed-step integration over an exact ray-parameter span with no event
    /// handling: the convergence-order harness (validation C).
    ///
    /// # Errors
    /// RHS failures propagate.
    pub fn integrate_fixed(
        &self,
        launch: &SphericalPoint,
        elevation: Radians,
        azimuth: Radians,
        sigma_span: f64,
        n_steps: usize,
    ) -> Result<State, TraceError> {
        let mut y = self.eqs.initial_state(launch, elevation, azimuth)?;
        let mut rk = Dopri5::<STATE_DIM>::new();
        let mut f = |s: &State, ds: &mut State| self.eqs.rhs(s, ds);
        let mut k1 = [0.0; STATE_DIM];
        f(&y, &mut k1)?;
        #[allow(clippy::cast_precision_loss)]
        let h = sigma_span / n_steps as f64;
        let (mut y5, mut k7, mut err) = ([0.0; STATE_DIM], [0.0; STATE_DIM], [0.0; STATE_DIM]);
        for _ in 0..n_steps {
            rk.try_step(&mut f, &y, &k1, h, &mut y5, &mut k7, &mut err)?;
            y = y5;
            k1 = k7;
        }
        Ok(y)
    }

    fn x_at(&self, y: &State) -> f64 {
        // X from the density model at the state's position; used for the
        // apex plasma-condition observable.
        let p = SphericalPoint::new(Meters::new(y[0]), Radians::new(y[1]), Radians::new(y[2]));
        self.eqs.x_at(&p)
    }

    fn finish(
        outcome: Outcome,
        y: State,
        sigma: f64,
        steps: usize,
        apexes: Vec<Apex>,
        drift: f64,
    ) -> TraceResult {
        TraceResult {
            outcome,
            end: SphericalPoint::new(Meters::new(y[0]), Radians::new(y[1]), Radians::new(y[2])),
            end_m: [y[3], y[4], y[5]],
            group_path: Meters::new(y[6]),
            phase_path: Meters::new(y[7]),
            absorption: Nepers::new(y[8]),
            arc_length: Meters::new(y[9]),
            apexes,
            steps,
            hamiltonian_drift: drift,
            sigma,
        }
    }

    /// Locate an event inside an accepted step [0, h] from `y0` by bisection
    /// with re-integration: each trial is one RK step of size d <= h, so its
    /// local error is no worse than the accepted step's. Returns the refined
    /// state and its partial ray parameter.
    fn refine<F>(
        rk: &mut Dopri5<STATE_DIM>,
        f: &mut F,
        y0: &State,
        k1: &State,
        h: f64,
        target: RefineTarget,
    ) -> Result<(State, f64), TraceError>
    where
        F: FnMut(&State, &mut State) -> Result<(), TraceError>,
    {
        let (mut y5, mut k7, mut err) = ([0.0; STATE_DIM], [0.0; STATE_DIM], [0.0; STATE_DIM]);
        let eval = |rk: &mut Dopri5<STATE_DIM>,
                    f: &mut F,
                    d: f64,
                    y5: &mut State,
                    k7: &mut State,
                    err: &mut State|
         -> Result<f64, TraceError> {
            rk.try_step(f, y0, k1, d, y5, k7, err)?;
            Ok(match target {
                RefineTarget::Radius(rt) => y5[0] - rt,
                RefineTarget::Apex => k7[0],
            })
        };
        let g0 = match target {
            RefineTarget::Radius(rt) => y0[0] - rt,
            RefineTarget::Apex => k1[0],
        };
        let (mut lo, mut hi) = (0.0f64, h);
        let mut g_lo = g0;
        let mut g_hi = eval(rk, f, h, &mut y5, &mut k7, &mut err)?;
        debug_assert!(
            g_lo == 0.0 || g_lo.signum() != g_hi.signum(),
            "refine called without a bracket"
        );
        for _ in 0..EVENT_MAX_ITER {
            // Bisection with a secant candidate when it falls inside the
            // bracket: robust, and each iteration costs one RK step.
            let mid = {
                let sec = lo - g_lo * (hi - lo) / secant_guard(g_hi, g_lo);
                if sec > lo && sec < hi {
                    sec
                } else {
                    0.5 * (lo + hi)
                }
            };
            let g_mid = eval(rk, f, mid, &mut y5, &mut k7, &mut err)?;
            let done = match target {
                RefineTarget::Radius(_) => g_mid.abs() < EVENT_R_TOL,
                RefineTarget::Apex => hi - lo < EVENT_SIGMA_TOL,
            };
            if done {
                return Ok((y5, mid));
            }
            if g_mid.signum() == g_lo.signum() {
                lo = mid;
                g_lo = g_mid;
            } else {
                hi = mid;
                g_hi = g_mid;
            }
        }
        // Bracket did not converge in the budget: report as step collapse at
        // the event rather than returning an unconverged state.
        Err(TraceError::StepSizeCollapse {
            sigma: lo,
            step: hi - lo,
            min_step: EVENT_SIGMA_TOL,
        })
    }
}

/// Trace a fan of launches in parallel, one rayon task per ray (the spec's
/// parallelism seam: never inside the ODE loop). Models must be `Sync`;
/// results keep launch order.
#[must_use]
pub fn trace_fan<D, B, C>(
    tracer: &Tracer<'_, D, B, C>,
    launches: &[(SphericalPoint, Radians, Radians)],
) -> Vec<Result<TraceResult, TraceError>>
where
    D: ElectronDensity + Sync + ?Sized,
    B: MagneticField + Sync + ?Sized,
    C: CollisionFrequency + Sync + ?Sized,
{
    use rayon::prelude::*;
    launches
        .par_iter()
        .map(|(p, elev, az)| tracer.trace(p, *elev, *az))
        .collect()
}

#[derive(Clone, Copy)]
enum RefineTarget {
    /// Root of r(sigma) - target.
    Radius(f64),
    /// Root of v_r(sigma) (radial turning point).
    Apex,
}

/// Secant denominator guard: if the bracket endpoints have equal g the
/// secant is invalid; return something that pushes the candidate outside
/// the bracket so bisection is used.
fn secant_guard(g_hi: f64, g_lo: f64) -> f64 {
    let d = g_hi - g_lo;
    if d == 0.0 { f64::MIN_POSITIVE } else { d }
}
