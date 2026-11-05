//! Synchronous runners for the three jobs the app offers: the point-to-point
//! solve, the frequency sweep and the area coverage grid.
//!
//! Each runner works on the calling thread and returns when the job is done.
//! It owns no thread and no channel: the caller passes a cancel predicate,
//! which is polled between units of work (returning true asks the runner to
//! stop), and a callback that receives each progress event as it happens. Both
//! are `Sync` because the parallel runners call them from several pool threads
//! at once. A caller that wants the work off its own
//! thread puts the call on a thread of its own choosing.
//!
//! The parallel work inside a runner (a sweep's frequencies, a coverage grid's
//! points) runs on the [`ComputePool`](crate::compute::ComputePool) passed in.
//! [`crate::compute::background_pool_config`] and
//! [`crate::compute::coverage_pool_config`] give the sizes the app uses.

pub use crate::coverage::{CoverageEvent, run_coverage};
pub use crate::sweep::{SweepEvent, run_sweep};

use crate::scenario::{self, Assumptions, Inputs, ProfileRow};
use crate::solve::{self, SolveOutcome};

/// Why a runner did not return a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// The cancel predicate returned true before the job finished. Any
    /// progress events already delivered stay valid.
    Cancelled,
    /// The scenario's engine models could not be built.
    Failed(String),
}

/// A completed point-to-point solve: everything the UI needs to display it.
pub struct MainResult {
    pub assumptions: Assumptions,
    pub profile: Vec<ProfileRow>,
    pub outcome: SolveOutcome,
}

/// The single point-to-point solve.
///
/// A solve cannot be stopped part-way, so `cancel` is checked once, after the
/// models are built and before the solve starts. The solve parallelises
/// internally on whatever rayon pool the call runs in.
///
/// # Errors
/// [`RunError::Failed`] when the models cannot be built, and
/// [`RunError::Cancelled`] when `cancel` was set before the solve began.
pub fn run_main(
    inputs: &Inputs,
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<MainResult, RunError> {
    let a = scenario::resolve(inputs);
    let models = scenario::build_models(inputs, &a).map_err(RunError::Failed)?;
    if cancel() {
        return Err(RunError::Cancelled);
    }
    let profile = scenario::sample_profile(&models, &a);
    let outcome = solve::solve(inputs, &a, &models);
    Ok(MainResult {
        assumptions: a,
        profile,
        outcome,
    })
}
