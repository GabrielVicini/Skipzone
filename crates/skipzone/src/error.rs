//! Typed errors for numerical failure modes. A ray that lands or escapes is a
//! physical outcome, not an error; errors are reserved for situations where the
//! computation itself cannot be trusted to continue.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum TraceError {
    /// The adaptive controller drove the step below the configured minimum.
    /// Cause: a gradient the tolerance cannot resolve (e.g. a discontinuous
    /// profile edge) or an inconsistent state. Clamping instead of failing
    /// would silently produce an unconverged ray.
    #[error("step collapsed to {step:.3e} m (min {min_step:.3e} m) at ray parameter {sigma:.6e} m")]
    StepSizeCollapse {
        sigma: f64,
        step: f64,
        min_step: f64,
    },

    /// The step budget ran out before a termination condition was met.
    #[error("exceeded {max_steps} integration steps at ray parameter {sigma:.6e} m")]
    MaxStepsExceeded { max_steps: usize, sigma: f64 },

    /// The ray came too close to a coordinate pole, where the spherical
    /// Haselgrove equations have cot(theta) and 1/sin(theta) singularities.
    /// This is a coordinate-system limitation, not physics.
    #[error("ray approached the coordinate pole: sin(colatitude) = {sin_colat:.3e}")]
    PoleProximity { sin_colat: f64 },

    /// The requested mode does not propagate at the launch point
    /// (`n^2 <= 0` there), so no ray exists.
    #[error("launch point is evanescent for this mode: n^2 = {n_squared:.6e}")]
    EvanescentLaunch { n_squared: f64 },

    /// A state component became NaN or infinite; the ray cannot be continued
    /// or trusted.
    #[error("non-finite state during integration at ray parameter {sigma:.6e} m")]
    NonFiniteState { sigma: f64 },
}
