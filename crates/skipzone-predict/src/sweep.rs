//! The frequency sweep behind "Find best frequency": `solve()` run at each
//! candidate frequency across the HF band, reduced to one [`SweepPoint`] per
//! frequency and a best pick.
//!
//! It reuses `solve()` verbatim at each frequency (no duplicated ray logic)
//! and reports one [`SweepEvent::Point`] per frequency, so a caller can advance
//! a progress bar and redraw the live band chart.
//!
//! [`run_sweep`] runs on the calling thread; it owns no thread or channel. See
//! [`crate::run`] for the other job runners.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use skipzone::magnetoionic::Mode;

use crate::compute::{ComputePool, Execution, Timing};
use crate::noise::PathState;
use crate::run::RunError;
use crate::scenario::{self, Assumptions, Inputs, Models};
use crate::solve::{self, SolveOutcome};

/// HF band swept by "Find best frequency". The scan is coarse-to-fine: a cheap
/// 1 MHz pass locates the good region (and stops early once it is clearly past
/// the optimum and getting worse), then a fine pass refines around the best.
pub const SWEEP_MIN_MHZ: f64 = 2.0;
pub const SWEEP_MAX_MHZ: f64 = 30.0;
const COARSE_STEP_MHZ: f64 = 1.0;
const FINE_STEP_MHZ: f64 = 0.2;
const FINE_HALF_WINDOW_MHZ: f64 = 1.5;
/// Stop the coarse pass after this many consecutive non-improving steps past
/// the best-so-far: "if it is only going uphill, don't fill the whole parabola".
const UPHILL_PATIENCE: usize = 4;

/// Inclusive `[lo, hi]` grid at `step`, with the last point snapped to `hi`.
fn frange(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = ((hi - lo) / step).round().max(0.0) as usize;
    #[allow(clippy::cast_precision_loss)]
    (0..=n).map(|i| (lo + i as f64 * step).min(hi)).collect()
}

/// Coarse pass frequencies, low to high.
#[must_use]
pub fn coarse_freqs() -> Vec<f64> {
    frange(SWEEP_MIN_MHZ, SWEEP_MAX_MHZ, COARSE_STEP_MHZ)
}

/// Fine refinement frequencies around `center`, clamped to the band.
#[must_use]
pub fn fine_freqs(center: f64) -> Vec<f64> {
    let lo = (center - FINE_HALF_WINDOW_MHZ).max(SWEEP_MIN_MHZ);
    let hi = (center + FINE_HALF_WINDOW_MHZ).min(SWEEP_MAX_MHZ);
    frange(lo, hi, FINE_STEP_MHZ)
}

/// One frequency's outcome, cached for the chart and the best-frequency pick.
///
/// `state` is a three-way verdict, because a path that closes geometrically may
/// still be too weak to hear.
#[derive(Clone, Copy)]
pub struct SweepPoint {
    pub freq_mhz: f64,
    pub state: PathState,
    /// Lowest total absorption (dB) among the modes found; `+inf` if none.
    pub absorption_db: f64,
    /// Total system loss (dB) of the picked mode; `+inf` if no path.
    pub system_loss_db: f64,
    /// Received power (dBm) of the picked mode; `-inf` if no path.
    pub rx_power_dbm: f64,
    /// Noise floor (dBm) at this frequency. Defined even with no path.
    pub noise_dbm: f64,
    /// SNR (dB) of the picked mode; `-inf` if no path.
    pub snr_db: f64,
    /// `snr_db - threshold`; `-inf` if no path.
    pub margin_db: f64,
    /// Smallest near-miss (km) when no path was found; 0 when one was.
    pub miss_km: f64,
    pub hops: u32,
    pub mode: Option<Mode>,
}

impl SweepPoint {
    /// 0 (best) to 1 (worst), used for the shade within a state's colour band.
    /// Usable points are graded by how far above the threshold they sit,
    /// below-threshold points by how far under it, and no-path points by
    /// near-miss, so the chart shows structure inside each band.
    #[must_use]
    pub fn badness(self) -> f32 {
        const MARGIN_FULL_DB: f64 = 20.0;
        const SHORTFALL_FULL_DB: f64 = 25.0;
        const MISS_RED_KM: f64 = 4000.0;
        #[allow(clippy::cast_possible_truncation)]
        match self.state {
            // Best (0.0) at >= MARGIN_FULL_DB of margin, worst (1.0) at 0 dB.
            PathState::Usable => (1.0 - (self.margin_db / MARGIN_FULL_DB).clamp(0.0, 1.0)) as f32,
            // 0.0 just under the threshold, 1.0 hopelessly under it.
            PathState::BelowThreshold => {
                ((-self.margin_db) / SHORTFALL_FULL_DB).clamp(0.0, 1.0) as f32
            }
            PathState::NoPath => (self.miss_km / MISS_RED_KM).clamp(0.0, 1.0) as f32,
        }
    }

    /// Did ray tracing find a path at all, whatever its strength?
    #[must_use]
    pub fn found_path(self) -> bool {
        self.state.found_path()
    }

    /// Full per-frequency readout: the existing link-budget numbers plus the
    /// received power, noise floor and SNR that decide the verdict. Used
    /// both for the sweep's stderr log and the chart's hover tooltip, so the
    /// two can never drift apart.
    #[must_use]
    pub fn debug_line(self) -> String {
        if !self.found_path() {
            return format!(
                "{:>5.2} MHz  NO PATH        near-miss {:>7.0} km  {} hop(s)  \
                 noise {:>7.1} dBm",
                self.freq_mhz, self.miss_km, self.hops, self.noise_dbm,
            );
        }
        format!(
            "{:>5.2} MHz  {:<14} {}-mode {} hop(s)  abs {:>6.2} dB  loss {:>6.1} dB  \
             Prx {:>7.1} dBm  noise {:>7.1} dBm  SNR {:>6.1} dB  margin {:>+6.1} dB",
            self.freq_mhz,
            if self.state == PathState::Usable {
                "USABLE"
            } else {
                "BELOW THRESH"
            },
            self.mode.map_or("?", crate::solve::mode_label),
            self.hops,
            self.absorption_db,
            self.system_loss_db,
            self.rx_power_dbm,
            self.noise_dbm,
            self.snr_db,
            self.margin_db,
        )
    }
}

/// The winner and enough context to render a one-line verdict.
#[derive(Clone, Copy)]
pub struct SweepBest {
    pub point: SweepPoint,
}

/// Progress from [`run_sweep`], delivered as it happens.
#[derive(Clone, Copy)]
pub enum SweepEvent {
    /// The sweep is about to start. `total` is an optimistic estimate: the
    /// whole coarse grid plus one fine window.
    Start { total: usize },
    /// One solved frequency, in completion order. `total` is revised once the
    /// fine window is known.
    Point {
        done: usize,
        total: usize,
        point: SweepPoint,
    },
}

/// Reduce a full solve to the cached summary for one frequency.
///
/// The mode with the strongest SNR is kept, since that is what an operator
/// would hear.
fn summarize(freq_mhz: f64, out: &SolveOutcome) -> SweepPoint {
    if let Some(best) = solve::best_by_snr(out) {
        SweepPoint {
            freq_mhz,
            state: best.link.state(),
            absorption_db: best.total_absorption_db,
            system_loss_db: best.total_system_loss_db,
            rx_power_dbm: best.link.rx_power_dbm,
            noise_dbm: best.link.noise.power_dbm,
            snr_db: best.link.snr_db,
            margin_db: best.link.margin_db(),
            miss_km: 0.0,
            hops: best.hops,
            mode: Some(best.mode),
        }
    } else {
        // near_misses are sorted ascending by miss_km in solve().
        let nm = out.near_misses.first();
        SweepPoint {
            freq_mhz,
            state: PathState::NoPath,
            absorption_db: f64::INFINITY,
            system_loss_db: f64::INFINITY,
            rx_power_dbm: f64::NEG_INFINITY,
            noise_dbm: out.noise.power_dbm,
            snr_db: f64::NEG_INFINITY,
            margin_db: f64::NEG_INFINITY,
            miss_km: nm.map_or(f64::INFINITY, |m| m.miss_km),
            hops: nm.map_or(0, |m| m.hops),
            mode: nm.map(|m| m.mode),
        }
    }
}

/// Best-frequency rule: any frequency with a path beats any without one; among
/// those with a path the strongest SNR wins; among the rest the smallest miss.
///
/// Ranking is by SNR rather than absorption, which is only one loss term and
/// ignores the noise the signal has to beat.
#[must_use]
pub fn better(a: SweepPoint, b: SweepPoint) -> SweepPoint {
    match (a.found_path(), b.found_path()) {
        (true, false) => a,
        (false, true) => b,
        (true, true) => {
            if a.snr_db >= b.snr_db {
                a
            } else {
                b
            }
        }
        (false, false) => {
            if a.miss_km <= b.miss_km {
                a
            } else {
                b
            }
        }
    }
}

/// The frequency sweep behind "Find best frequency": build the scenario models
/// once, then call `solve()` at each candidate frequency, coarse pass first
/// and then a fine window around the coarse best.
///
/// The pool's execution mode picks the path. In parallel each phase's whole
/// grid runs at once; sequentially the coarse pass stops early once it is
/// clearly past the optimum. Both give the same per-frequency results.
/// `cancel` is checked before each frequency.
///
/// Returns the winning frequency, which is `None` only if nothing was solved.
///
/// # Errors
/// [`RunError::Failed`] when the models cannot be built, and
/// [`RunError::Cancelled`] when `cancel` stopped the sweep. Points already
/// delivered through `on_event` stay valid.
pub fn run_sweep(
    inputs: &Inputs,
    pool: &ComputePool,
    cancel: &(dyn Fn() -> bool + Sync),
    on_event: &(dyn Fn(SweepEvent) + Sync),
) -> Result<Option<SweepBest>, RunError> {
    let a = scenario::resolve(inputs);
    let models = scenario::build_models(inputs, &a).map_err(RunError::Failed)?;
    let best = match pool.execution() {
        Execution::Parallel => run_sweep_parallel(inputs, &a, &models, pool, cancel, on_event),
        Execution::Sequential => run_sweep_sequential(inputs, &a, &models, cancel, on_event),
    }?;
    Ok(best.map(|point| SweepBest { point }))
}

/// One candidate frequency -> its cached `SweepPoint`. Returns `None` only when
/// the job has been cancelled, so a superseded sweep stops doing engine work as
/// soon as each in-flight task next checks the flag. This is the single unit of
/// work both the parallel and sequential paths dispatch.
fn solve_point(
    freq_mhz: f64,
    inputs: &Inputs,
    a: &Assumptions,
    models: &Models,
    cancel: &(dyn Fn() -> bool + Sync),
) -> Option<SweepPoint> {
    if cancel() {
        return None;
    }
    let mut fi = inputs.clone();
    fi.freq_mhz = freq_mhz;
    let point = summarize(freq_mhz, &solve::solve(&fi, a, models));
    eprintln!("[sweep] {}", point.debug_line());
    Some(point)
}

/// Fold `better` over a batch of already-computed points, continuing from
/// `seed`. Order-independent in the values it selects (absorption / miss), so it
/// agrees with the sequential fold over the same points.
fn fold_best(seed: Option<SweepPoint>, points: &[SweepPoint]) -> Option<SweepPoint> {
    let mut best = seed;
    for &p in points {
        best = Some(best.map_or(p, |b| better(b, p)));
    }
    best
}

/// Parallel path: evaluate each phase's whole frequency grid at once across the
/// pool. With every candidate dispatched together there is no uphill early
/// stop, which only saves sequential time; the complete coarse grid is a
/// superset of what the sequential path evaluates, so per-frequency results are
/// identical and the winner is never worse.
fn run_sweep_parallel(
    inputs: &Inputs,
    a: &Assumptions,
    models: &Models,
    pool: &ComputePool,
    cancel: &(dyn Fn() -> bool + Sync),
    on_event: &(dyn Fn(SweepEvent) + Sync),
) -> Result<Option<SweepPoint>, RunError> {
    let coarse = coarse_freqs();
    // Optimistic total for the bar: whole coarse grid plus one fine window.
    let total_estimate = coarse.len() + fine_freqs(0.5 * (SWEEP_MIN_MHZ + SWEEP_MAX_MHZ)).len();
    on_event(SweepEvent::Start {
        total: total_estimate,
    });

    // Shared by every pool thread's progress callback.
    let done = AtomicUsize::new(0);

    let evaluate = |f: &f64| solve_point(*f, inputs, a, models, cancel);

    // Run one grid in parallel, reporting each solved frequency.
    let run_phase = |freqs: &[f64], total: usize| -> (Vec<SweepPoint>, Timing) {
        let (opt_points, timing) =
            pool.map_reporting(freqs, evaluate, |_, point: &Option<SweepPoint>| {
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(point) = point {
                    on_event(SweepEvent::Point {
                        done: n,
                        total,
                        point: *point,
                    });
                }
            });
        (opt_points.into_iter().flatten().collect(), timing)
    };

    // Phase 1: the full coarse grid.
    let (coarse_points, coarse_timing) = run_phase(&coarse, total_estimate);
    if cancel() {
        return Err(RunError::Cancelled);
    }
    let mut best = fold_best(None, &coarse_points);

    // Phase 2: fine refinement around the coarse best.
    let fine_timing = if let Some(seed) = best {
        let fine = fine_freqs(seed.freq_mhz);
        let total = done.load(Ordering::Relaxed) + fine.len();
        let (fine_points, timing) = run_phase(&fine, total);
        if cancel() {
            return Err(RunError::Cancelled);
        }
        best = fold_best(best, &fine_points);
        timing
    } else {
        Timing {
            total: Duration::ZERO,
            per_item: Vec::new(),
            threads: pool.threads(),
        }
    };

    log_sweep_timing("parallel", &coarse_timing, &fine_timing);
    Ok(best)
}

/// Single-threaded path: a coarse scan with an uphill early stop, then a fine
/// scan. The fallback when parallelism is off, and the equivalence baseline.
fn run_sweep_sequential(
    inputs: &Inputs,
    a: &Assumptions,
    models: &Models,
    cancel: &(dyn Fn() -> bool + Sync),
    on_event: &(dyn Fn(SweepEvent) + Sync),
) -> Result<Option<SweepPoint>, RunError> {
    let sweep_started = std::time::Instant::now();
    let mut per_item: Vec<Duration> = Vec::new();

    let coarse = coarse_freqs();
    // Optimistic total for the bar: coarse pass plus one fine window. Each
    // progress event carries its own (possibly revised) total.
    let mut total = coarse.len() + fine_freqs(0.5 * (SWEEP_MIN_MHZ + SWEEP_MAX_MHZ)).len();
    on_event(SweepEvent::Start { total });

    let mut timed_solve = |f: f64| -> Result<SweepPoint, RunError> {
        let started = std::time::Instant::now();
        let point = solve_point(f, inputs, a, models, cancel).ok_or(RunError::Cancelled)?;
        per_item.push(started.elapsed());
        Ok(point)
    };

    // The winner uses the exact `better` rule (which resolves ties badness
    // cannot, e.g. two heavily-absorbed connectors); the uphill early-stop uses
    // the badness direction, where saturation past the worst-case cap is fine.
    let mut best: Option<SweepPoint> = None;
    let mut best_badness = f32::INFINITY;
    let mut done = 0usize;
    let mut worse_streak = 0usize;

    // Phase 1: coarse scan, low to high, stopping once clearly past the optimum.
    for &f in &coarse {
        let point = timed_solve(f)?;
        done += 1;
        best = Some(best.map_or(point, |b| better(b, point)));
        if point.badness() < best_badness - 1e-4 {
            best_badness = point.badness();
            worse_streak = 0;
        } else {
            worse_streak += 1;
        }
        on_event(SweepEvent::Point { done, total, point });
        if worse_streak >= UPHILL_PATIENCE {
            break; // going uphill: no need to fill out the rest of the band
        }
    }

    // Phase 2: fine refinement around the coarse best.
    if let Some(seed) = best {
        let fine = fine_freqs(seed.freq_mhz);
        total = done + fine.len();
        for &f in &fine {
            let point = timed_solve(f)?;
            done += 1;
            best = Some(best.map_or(point, |b| better(b, point)));
            on_event(SweepEvent::Point { done, total, point });
        }
    }

    let timing = Timing {
        total: sweep_started.elapsed(),
        per_item,
        threads: 1,
    };
    log_sweep_timing_single("sequential", &timing);
    Ok(best)
}

/// One-line timing summary combining the coarse and fine phases, so the
/// speedup can be measured.
fn log_sweep_timing(label: &str, coarse: &Timing, fine: &Timing) {
    let mut per_item = coarse.per_item.clone();
    per_item.extend_from_slice(&fine.per_item);
    let combined = Timing {
        total: coarse.total + fine.total,
        per_item,
        threads: coarse.threads,
    };
    log_sweep_timing_single(label, &combined);
}

fn log_sweep_timing_single(label: &str, timing: &Timing) {
    let n = timing.per_item.len();
    eprintln!(
        "[sweep] {label}: {n} freqs on {} thread(s) in {:.3} s \
         (serial work {:.3} s, {:.1}x, mean {:.1} ms/freq)",
        timing.threads,
        timing.total.as_secs_f64(),
        timing.work().as_secs_f64(),
        timing.speedup(),
        timing.mean_item().as_secs_f64() * 1e3,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two `SweepPoint`s are the same result when every field is bit-identical.
    /// The per-frequency solve is a pure function, so the parallel path must
    /// reproduce the sequential path exactly.
    fn same_point(a: SweepPoint, b: SweepPoint) -> bool {
        a.freq_mhz.to_bits() == b.freq_mhz.to_bits()
            && a.state == b.state
            && a.absorption_db.to_bits() == b.absorption_db.to_bits()
            && a.system_loss_db.to_bits() == b.system_loss_db.to_bits()
            && a.rx_power_dbm.to_bits() == b.rx_power_dbm.to_bits()
            && a.noise_dbm.to_bits() == b.noise_dbm.to_bits()
            && a.snr_db.to_bits() == b.snr_db.to_bits()
            && a.miss_km.to_bits() == b.miss_km.to_bits()
            && a.hops == b.hops
            && a.mode == b.mode
    }

    /// Running the same candidate frequencies across all cores must give
    /// bit-identical per-frequency results to running them one at a time. It
    /// prints both timings and the thread count. Run with:
    ///   cargo test -p skipzone-predict --release parallel_matches_sequential -- --nocapture
    #[test]
    fn parallel_matches_sequential_and_reports_timing() {
        use crate::compute::{ComputePool, Execution, PoolConfig, available_cores};

        let inputs = Inputs::default();
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).expect("models build");

        // The exact frequencies the app's parallel sweep would touch: the whole
        // coarse grid plus a fine window around a plausible optimum. Both paths
        // evaluate this identical list, isolating the execution layer from the
        // search heuristic (early-stop) that differs between them.
        let mut freqs = coarse_freqs();
        freqs.extend(fine_freqs(14.0));

        let evaluate =
            |f: &f64| solve_point(*f, &inputs, &a, &models, &|| false).expect("not cancelled");

        let seq = ComputePool::new(PoolConfig {
            execution: Execution::Sequential,
            max_threads: None,
        })
        .unwrap();
        let par = ComputePool::new(PoolConfig {
            execution: Execution::Parallel,
            max_threads: None,
        })
        .unwrap();

        let (seq_points, seq_timing) = seq.map(&freqs, evaluate);
        let (par_points, par_timing) = par.map(&freqs, evaluate);

        assert_eq!(seq_points.len(), par_points.len());
        let mut mismatches = 0usize;
        for (s, p) in seq_points.iter().zip(&par_points) {
            if !same_point(*s, *p) {
                mismatches += 1;
                eprintln!(
                    "  MISMATCH @ {:.2} MHz: seq(state={}, snr={:.6}, miss={:.3}) \
                     vs par(state={}, snr={:.6}, miss={:.3})",
                    s.freq_mhz,
                    s.state.label(),
                    s.snr_db,
                    s.miss_km,
                    p.state.label(),
                    p.snr_db,
                    p.miss_km,
                );
            }
        }

        eprintln!("=== frequency sweep: parallel vs single-threaded ===");
        eprintln!("cores detected            : {}", available_cores());
        eprintln!(
            "frequencies evaluated     : {} (coarse {} + fine {})",
            freqs.len(),
            coarse_freqs().len(),
            freqs.len() - coarse_freqs().len(),
        );
        eprintln!(
            "single-threaded (1 thread): {:.3} s total, {:.1} ms/freq",
            seq_timing.total.as_secs_f64(),
            seq_timing.mean_item().as_secs_f64() * 1e3,
        );
        eprintln!(
            "parallel ({:>2} threads)     : {:.3} s total, {:.1} ms/freq, {:.1}x speedup",
            par_timing.threads,
            par_timing.total.as_secs_f64(),
            par_timing.mean_item().as_secs_f64() * 1e3,
            seq_timing.total.as_secs_f64() / par_timing.total.as_secs_f64().max(1e-9),
        );
        eprintln!("frequencies differing     : {mismatches}");

        assert_eq!(
            mismatches, 0,
            "parallel results diverged from single-threaded"
        );
    }

    /// A cancelled sweep says so on both execution paths, and solves nothing.
    #[test]
    fn cancelled_sweep_reports_cancelled_on_both_paths() {
        use crate::compute::{ComputePool, Execution, PoolConfig};
        use std::sync::Mutex;

        for execution in [Execution::Parallel, Execution::Sequential] {
            let pool = ComputePool::new(PoolConfig {
                execution,
                max_threads: Some(2),
            })
            .unwrap();
            let points = Mutex::new(0usize);
            let count = |e: SweepEvent| {
                if matches!(e, SweepEvent::Point { .. }) {
                    *points.lock().unwrap() += 1;
                }
            };
            let end = run_sweep(&Inputs::default(), &pool, &|| true, &count);
            assert!(matches!(end, Err(RunError::Cancelled)), "{execution:?}");
            assert_eq!(*points.lock().unwrap(), 0, "{execution:?}");
        }
    }

    #[test]
    fn coarse_grid_spans_hf() {
        let f = coarse_freqs();
        assert_eq!(f.first().copied(), Some(SWEEP_MIN_MHZ));
        assert!((f.last().copied().unwrap() - SWEEP_MAX_MHZ).abs() < 1e-9);
        assert!(
            f.windows(2)
                .all(|w| (w[1] - w[0] - COARSE_STEP_MHZ).abs() < 1e-9)
        );
    }

    #[test]
    fn fine_window_is_centred_and_clamped() {
        let f = fine_freqs(15.0);
        assert!((f.first().copied().unwrap() - (15.0 - FINE_HALF_WINDOW_MHZ)).abs() < 1e-9);
        assert!((f.last().copied().unwrap() - (15.0 + FINE_HALF_WINDOW_MHZ)).abs() < 1e-9);
        // Finer than the coarse pass, and clamped at the band edges.
        assert!(f.windows(2).all(|w| w[1] - w[0] < COARSE_STEP_MHZ));
        assert_eq!(
            fine_freqs(SWEEP_MIN_MHZ).first().copied(),
            Some(SWEEP_MIN_MHZ)
        );
        assert!(fine_freqs(SWEEP_MAX_MHZ).last().copied().unwrap() <= SWEEP_MAX_MHZ + 1e-9);
    }

    /// A point with a found path, described by its SNR against a 10 dB
    /// threshold (so `snr - 10` is the margin that decides the state).
    fn found(freq: f64, snr: f64) -> SweepPoint {
        let margin = snr - 10.0;
        SweepPoint {
            freq_mhz: freq,
            state: if margin >= 0.0 {
                PathState::Usable
            } else {
                PathState::BelowThreshold
            },
            absorption_db: 6.0,
            system_loss_db: 140.0,
            rx_power_dbm: -90.0,
            noise_dbm: -90.0 - snr,
            snr_db: snr,
            margin_db: margin,
            miss_km: 0.0,
            hops: 1,
            mode: None,
        }
    }

    fn no_path(freq: f64, miss: f64) -> SweepPoint {
        SweepPoint {
            freq_mhz: freq,
            state: PathState::NoPath,
            absorption_db: f64::INFINITY,
            system_loss_db: f64::INFINITY,
            rx_power_dbm: f64::NEG_INFINITY,
            noise_dbm: -100.0,
            snr_db: f64::NEG_INFINITY,
            margin_db: f64::NEG_INFINITY,
            miss_km: miss,
            hops: 1,
            mode: None,
        }
    }

    #[test]
    fn better_prefers_a_found_path_then_the_strongest_snr() {
        let weak = found(14.0, 2.0);
        let strong = found(10.0, 18.0);
        let miss_small = no_path(28.0, 200.0);
        // Any found path beats a miss regardless of how near the miss was.
        assert!(better(weak, miss_small).found_path());
        assert!(better(miss_small, weak).found_path());
        // Among found paths the strongest SNR wins, even when absorption ties.
        assert!((better(weak, strong).snr_db - 18.0).abs() < 1e-12);
        assert!((better(strong, weak).snr_db - 18.0).abs() < 1e-12);
        // Among misses, the smallest miss wins.
        let miss_big = no_path(29.0, 900.0);
        assert!((better(miss_small, miss_big).miss_km - 200.0).abs() < 1e-12);
    }

    /// The point of the whole change: a path that closes but sits under the
    /// threshold must not be ranked as if it were a usable one.
    #[test]
    fn below_threshold_is_a_distinct_state_from_usable() {
        let usable = found(14.0, 11.0);
        let below = found(14.2, 9.0);
        assert_eq!(usable.state, PathState::Usable);
        assert_eq!(below.state, PathState::BelowThreshold);
        // Both found a path, so both beat a no-path point...
        assert!(below.found_path());
        assert!(better(below, no_path(21.0, 50.0)).found_path());
        // ...but the usable one still wins between them.
        assert_eq!(better(below, usable).state, PathState::Usable);
    }

    #[test]
    fn badness_orders_best_to_worst_within_each_state() {
        // Inside "usable": a big margin is better than a bare one.
        assert!(found(10.0, 35.0).badness() < found(10.0, 11.0).badness());
        // Inside "below threshold": just under is better than hopeless.
        assert!(found(10.0, 9.0).badness() < found(10.0, -40.0).badness());
        // Inside "no path": a near miss is better than a huge one.
        assert!(no_path(28.0, 100.0).badness() < no_path(28.0, 3000.0).badness());
        for p in [
            found(10.0, 35.0),
            found(10.0, 9.0),
            no_path(28.0, 3000.0),
            no_path(28.0, f64::INFINITY),
        ] {
            assert!((0.0..=1.0).contains(&p.badness()), "{}", p.badness());
        }
    }

    /// summarize reads a real solve: the default scenario finds a path (finite
    /// absorption and a real SNR), and a 45 MHz solve does not.
    #[test]
    fn summarize_reads_real_solves() {
        let inputs = Inputs::default();
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).expect("models");
        let hit = summarize(inputs.freq_mhz, &solve::solve(&inputs, &a, &models));
        assert!(hit.found_path() && hit.absorption_db.is_finite());
        assert!(
            hit.snr_db.is_finite() && hit.noise_dbm.is_finite(),
            "a found path must carry a real SNR and noise floor"
        );
        // SNR is `Prx - Pnoise` less the calibrated model bias, and the gap must
        // be exactly that bias. The correction applies to the SNR alone, so
        // `rx_power_dbm` and the loss terms still report pure propagation.
        assert!(
            (hit.snr_db - (hit.rx_power_dbm - hit.noise_dbm - scenario::MEASURED_MODEL_BIAS_DB))
                .abs()
                < 1e-9,
            "SNR must be Prx - Pnoise - model_bias; got {} vs {}",
            hit.snr_db,
            hit.rx_power_dbm - hit.noise_dbm - scenario::MEASURED_MODEL_BIAS_DB
        );

        let mut hi = inputs;
        hi.freq_mhz = 45.0;
        let missed = summarize(45.0, &solve::solve(&hi, &a, &models));
        assert_eq!(missed.state, PathState::NoPath);
        // Even with no path the noise floor is real - it is a property of the
        // receiver, not of whether anything arrived.
        assert!(missed.noise_dbm.is_finite());
    }

    /// Raising the SNR threshold can only ever move frequencies from usable to
    /// below-threshold, never the reverse, and never invents or destroys paths.
    #[test]
    fn raising_the_threshold_only_demotes_points() {
        let base = Inputs::default();
        let a = scenario::resolve(&base);
        let models = scenario::build_models(&base, &a).expect("models");

        let mut strict = base.clone();
        strict.snr_threshold_db = base.snr_threshold_db + 40.0;

        for f in [7.0, 10.0, 14.1, 21.0] {
            let mut lenient_i = base.clone();
            lenient_i.freq_mhz = f;
            let mut strict_i = strict.clone();
            strict_i.freq_mhz = f;
            let lenient = summarize(f, &solve::solve(&lenient_i, &a, &models));
            let harsh = summarize(f, &solve::solve(&strict_i, &a, &models));
            assert_eq!(
                lenient.found_path(),
                harsh.found_path(),
                "the threshold must not change whether a path exists at {f} MHz"
            );
            if harsh.state == PathState::Usable {
                assert_eq!(
                    lenient.state,
                    PathState::Usable,
                    "usable under a strict threshold implies usable under a lenient one"
                );
            }
        }
    }
}
