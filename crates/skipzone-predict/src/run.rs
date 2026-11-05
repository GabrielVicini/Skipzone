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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::{ComputePool, Execution, PoolConfig};

    /// The numbers `solve_digest` prints for each solution, as raw bits.
    fn fingerprint(r: &MainResult) -> Vec<(u32, [u64; 7])> {
        r.outcome
            .solutions
            .iter()
            .map(|s| {
                let first = s.hop_details.first();
                (
                    s.hops,
                    [
                        first.map_or(0, |h| h.launch_elev_deg.to_bits()),
                        first.map_or(0, |h| h.apex_alt_km.to_bits()),
                        s.total_arc_km.to_bits(),
                        s.total_absorption_db.to_bits(),
                        s.free_space_loss_db.to_bits(),
                        s.ground_reflection_loss_db.to_bits(),
                        s.link.snr_db.to_bits(),
                    ],
                )
            })
            .collect()
    }

    /// The pool a solve runs in only decides where its rays are traced, so
    /// one thread, a few threads and rayon's global pool give the same bits.
    #[test]
    fn main_solve_is_identical_in_any_pool() {
        let inputs = Inputs::default();
        let run_in = |threads: usize| {
            let pool = ComputePool::new(PoolConfig {
                execution: Execution::Parallel,
                max_threads: Some(threads),
            })
            .unwrap();
            pool.install(|| run_main(&inputs, &|| false).map(|r| fingerprint(&r)))
                .expect("default scenario solves")
        };
        let global = run_main(&inputs, &|| false)
            .map(|r| fingerprint(&r))
            .expect("default scenario solves");
        assert!(!global.is_empty(), "the default scenario finds a path");
        assert_eq!(run_in(1), global);
        assert_eq!(run_in(3), global);
    }

    #[test]
    fn main_solve_stops_when_cancelled() {
        assert!(matches!(
            run_main(&Inputs::default(), &|| true),
            Err(RunError::Cancelled)
        ));
    }
}
