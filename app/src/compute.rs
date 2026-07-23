//! General-purpose parallel compute layer for the app.
//!
//! The engine crate is single-threaded by design and stays untouched. This
//! module is a thin, reusable *execution* wrapper on top of it: given a batch of
//! independent work items (candidate frequencies today; anything else tomorrow),
//! it runs a closure over each one across all CPU cores using `rayon`, and hands
//! back the results *in input order* plus timing for the run.
//!
//! It implements no physics and shares no mutable state between items: each call
//! into the engine is independent, so parallelism needs no locks. The only thing
//! this layer decides is *where* each closure runs.
//!
//! Two properties make it drop-in swappable:
//!   * [`Execution::Sequential`] runs the exact same closures on the calling
//!     thread, so any parallel result can be checked against the single-threaded
//!     engine for free (see the equivalence test in `sweep`).
//!   * Every map returns a [`Timing`] with per-item and total wall-clock, so a
//!     claimed speedup can be measured rather than assumed.

use std::time::{Duration, Instant};

use rayon::prelude::*;

/// Where a [`ComputePool`] runs its work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Execution {
    /// Fan the batch out across the pool's worker threads.
    Parallel,
    /// Run every item on the calling thread, in order. The swappable fallback
    /// used for testing, comparison, and reproducing the old behaviour exactly.
    Sequential,
}

/// Construction options for a [`ComputePool`].
#[derive(Clone, Copy, Debug)]
pub struct PoolConfig {
    pub execution: Execution,
    /// Upper bound on worker threads. `None` uses every logical core the machine
    /// reports. Present so a future setting can cap parallelism (e.g. to leave
    /// cores for the UI) without touching call sites.
    pub max_threads: Option<usize>,
}

impl Default for PoolConfig {
    /// All cores, in parallel: the intended production configuration.
    fn default() -> Self {
        Self {
            execution: Execution::Parallel,
            max_threads: None,
        }
    }
}

/// Wall-clock timing for one [`ComputePool::map`] call.
///
/// `total` is the elapsed time of the whole batch. `per_item[i]` is how long the
/// closure took on item `i` (in input order). Comparing the sum of `per_item`
/// against `total` is exactly the parallel speedup: the sum is the serial work
/// performed, `total` is how long it actually took.
#[derive(Clone, Debug)]
pub struct Timing {
    pub total: Duration,
    pub per_item: Vec<Duration>,
    /// Worker threads the batch was allowed to use (1 when sequential).
    pub threads: usize,
}

impl Timing {
    /// Serial-equivalent work: the sum of every item's own runtime.
    #[must_use]
    pub fn work(&self) -> Duration {
        self.per_item.iter().copied().sum()
    }

    /// Mean per-item runtime, or zero for an empty batch.
    #[must_use]
    pub fn mean_item(&self) -> Duration {
        if self.per_item.is_empty() {
            Duration::ZERO
        } else {
            self.work() / u32::try_from(self.per_item.len()).unwrap_or(u32::MAX)
        }
    }

    /// Measured speedup, `work / total`. ~1.0 when sequential, up to the core
    /// count when the batch is large and well balanced. Zero total yields 0.0.
    #[must_use]
    pub fn speedup(&self) -> f64 {
        let total = self.total.as_secs_f64();
        if total > 0.0 {
            self.work().as_secs_f64() / total
        } else {
            0.0
        }
    }
}

/// A reusable execution context: a `rayon` thread pool sized once, plus the
/// chosen execution mode. Build it once and keep it alive; each `map` reuses the
/// same worker threads rather than paying pool-spawn cost per batch.
///
/// It is deliberately generic (not sweep-specific) so any batch of independent
/// engine solves — a coverage grid, a multi-path scan — can share one pool.
pub struct ComputePool {
    /// `None` in sequential mode: no worker threads are spawned at all.
    pool: Option<rayon::ThreadPool>,
    execution: Execution,
    threads: usize,
}

impl ComputePool {
    /// Build a pool from `config`. In parallel mode this spawns a dedicated
    /// `rayon` pool of `min(max_threads, cores)` threads (all cores by default),
    /// isolated from rayon's global pool so it never fights other users of it.
    ///
    /// # Errors
    /// Propagates a `rayon` thread-pool build failure as a human-readable string.
    pub fn new(config: PoolConfig) -> Result<Self, String> {
        let cores = available_cores();
        match config.execution {
            Execution::Sequential => Ok(Self {
                pool: None,
                execution: Execution::Sequential,
                threads: 1,
            }),
            Execution::Parallel => {
                let threads = config.max_threads.map_or(cores, |cap| cap.clamp(1, cores));
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .thread_name(|i| format!("skipzone-compute-{i}"))
                    .build()
                    .map_err(|e| format!("compute pool build failed: {e}"))?;
                Ok(Self {
                    pool: Some(pool),
                    execution: Execution::Parallel,
                    threads,
                })
            }
        }
    }

    /// Convenience: an all-cores parallel pool.
    ///
    /// # Errors
    /// See [`ComputePool::new`].
    // Part of the reusable pool API; exercised by tests and future batch callers
    // beyond the sweep, so the non-test build sees it as unused.
    #[allow(dead_code)]
    pub fn all_cores() -> Result<Self, String> {
        Self::new(PoolConfig::default())
    }

    /// Worker threads this pool will use (1 when sequential).
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
    }

    #[must_use]
    pub fn execution(&self) -> Execution {
        self.execution
    }

    /// Run `f` over every item, returning results in input order plus [`Timing`].
    ///
    /// Each item is timed individually; there is no shared mutable state, so the
    /// closure needs only `Sync` (it is called from several threads at once in
    /// parallel mode). Order is preserved even though completion order is not.
    // Progress-free convenience wrapper; used by tests and future batch callers.
    #[allow(dead_code)]
    pub fn map<T, R, F>(&self, items: &[T], f: F) -> (Vec<R>, Timing)
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Sync,
    {
        self.map_reporting(items, f, |_, _| {})
    }

    /// Like [`ComputePool::map`], but invokes `on_each(index, &result)` as each
    /// item finishes — used to stream progress to the UI. `on_each` fires in
    /// completion order (arbitrary under parallelism), not input order, and must
    /// be `Sync` because several threads may call it concurrently; the returned
    /// results are still ordered by input index.
    pub fn map_reporting<T, R, F, P>(&self, items: &[T], f: F, on_each: P) -> (Vec<R>, Timing)
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Sync,
        P: Fn(usize, &R) + Sync,
    {
        // One timed unit of work. Kept identical between the parallel and
        // sequential arms so the two can never diverge in what they measure.
        let run_one = |index: usize, item: &T| -> (Duration, R) {
            let started = Instant::now();
            let result = f(item);
            let elapsed = started.elapsed();
            on_each(index, &result);
            (elapsed, result)
        };

        let batch_start = Instant::now();
        let timed: Vec<(Duration, R)> = match &self.pool {
            Some(pool) => pool.install(|| {
                items
                    .par_iter()
                    .enumerate()
                    .map(|(i, item)| run_one(i, item))
                    .collect()
            }),
            None => items
                .iter()
                .enumerate()
                .map(|(i, item)| run_one(i, item))
                .collect(),
        };
        let total = batch_start.elapsed();

        let mut per_item = Vec::with_capacity(timed.len());
        let mut results = Vec::with_capacity(timed.len());
        for (dur, res) in timed {
            per_item.push(dur);
            results.push(res);
        }
        (
            results,
            Timing {
                total,
                per_item,
                threads: self.threads,
            },
        )
    }
}

/// Logical cores reported by the OS, floored at 1. The default width of a
/// parallel batch.
#[must_use]
pub fn available_cores() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_preserves_input_order_in_both_modes() {
        let input: Vec<u64> = (0..1000).collect();
        for execution in [Execution::Parallel, Execution::Sequential] {
            let pool = ComputePool::new(PoolConfig {
                execution,
                max_threads: None,
            })
            .unwrap();
            let (out, timing) = pool.map(&input, |&n| n * n);
            let expected: Vec<u64> = input.iter().map(|&n| n * n).collect();
            assert_eq!(out, expected, "order must be stable in {execution:?}");
            assert_eq!(timing.per_item.len(), input.len());
        }
    }

    #[test]
    fn parallel_uses_all_cores_by_default() {
        let pool = ComputePool::all_cores().unwrap();
        assert_eq!(pool.threads(), available_cores());
        assert_eq!(pool.execution(), Execution::Parallel);
    }

    #[test]
    fn thread_cap_is_honoured_and_clamped() {
        let capped = ComputePool::new(PoolConfig {
            execution: Execution::Parallel,
            max_threads: Some(2),
        })
        .unwrap();
        assert_eq!(capped.threads(), 2.min(available_cores()));

        // A cap of zero is nonsense; it must clamp up to one, not panic.
        let zero = ComputePool::new(PoolConfig {
            execution: Execution::Parallel,
            max_threads: Some(0),
        })
        .unwrap();
        assert!(zero.threads() >= 1);
    }

    #[test]
    fn sequential_pool_spawns_no_threads() {
        let pool = ComputePool::new(PoolConfig {
            execution: Execution::Sequential,
            max_threads: None,
        })
        .unwrap();
        assert_eq!(pool.threads(), 1);
        assert!(pool.pool.is_none());
    }

    #[test]
    fn reporting_callback_fires_once_per_item() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let pool = ComputePool::all_cores().unwrap();
        let input: Vec<u64> = (0..500).collect();
        let calls = AtomicUsize::new(0);
        let (_out, _t) = pool.map_reporting(
            &input,
            |&n| n + 1,
            |_i, _r| {
                calls.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert_eq!(calls.load(Ordering::Relaxed), input.len());
    }

    #[test]
    fn timing_speedup_and_work_are_sane() {
        let pool = ComputePool::all_cores().unwrap();
        let input: Vec<u64> = (0..64).collect();
        // Give each item real, roughly-equal work so the numbers mean something.
        let (_out, timing) = pool.map(&input, |&n| {
            let mut acc = n;
            for i in 0..200_000u64 {
                acc = acc.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(i);
            }
            acc
        });
        assert_eq!(timing.per_item.len(), input.len());
        assert!(timing.work() >= timing.total, "work is the serial sum");
        assert!(timing.speedup() > 0.0);
    }
}
