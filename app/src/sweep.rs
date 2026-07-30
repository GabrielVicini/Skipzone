//! Background solver service: runs every `solve()` off the UI thread so the
//! interface never freezes, and drives the frequency sweep.
//!
//! Two job kinds share one worker thread and one result channel:
//!   * `Job::Main`  - the single point-to-point solve behind RUN TRACE.
//!   * `Job::Sweep` - the frequency sweep behind FIND BEST FREQUENCY: it
//!     reuses `solve()` verbatim at each candidate frequency (no duplicated
//!     ray logic) and streams one `SweepProgress` per frequency so the UI can
//!     advance a progress bar and redraw the live band chart.
//!
//! Nothing here implements physics: it clones the `Inputs`, rebuilds the engine
//! models once per job, and calls the existing `scenario`/`solve` API.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use egui::Context;
use skipzone::magnetoionic::Mode;

use crate::compute::{ComputePool, Execution, PoolConfig, Timing, available_cores};
use crate::coverage::{CoverageCell, CoverageConfig};
use crate::noise::PathState;
use crate::scenario::{self, Assumptions, Inputs, Models, ProfileRow};
use crate::solve::{self, SolveOutcome};

/// HF band swept by FIND BEST FREQUENCY. The scan is coarse-to-fine: a cheap
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
/// `state` is the three-way verdict that replaced the old `connects` boolean:
/// a path that closes geometrically is not the same claim as a path anyone can
/// hear, and the sweep now distinguishes them.
#[derive(Clone, Copy)]
pub struct SweepPoint {
    pub freq_mhz: f64,
    pub state: PathState,
    /// Lowest total absorption [dB] among the modes found; `+inf` if none.
    pub absorption_db: f64,
    /// Total system loss [dB] of the picked mode; `+inf` if no path.
    pub system_loss_db: f64,
    /// Received power [dBm] of the picked mode; `-inf` if no path.
    pub rx_power_dbm: f64,
    /// Noise floor [dBm] at this frequency. Defined even with no path.
    pub noise_dbm: f64,
    /// SNR [dB] of the picked mode; `-inf` if no path.
    pub snr_db: f64,
    /// `snr_db - threshold`; `-inf` if no path.
    pub margin_db: f64,
    /// Smallest near-miss [km] when no path was found; 0 when one was.
    pub miss_km: f64,
    pub hops: u32,
    pub mode: Option<Mode>,
}

impl SweepPoint {
    /// 0 (best) to 1 (worst), used for the shade WITHIN a state's colour band.
    /// Usable points are graded by how far above the threshold they sit,
    /// below-threshold points by how far under it, and no-path points by
    /// near-miss - so the chart still shows structure inside each of the three
    /// bands rather than three flat colours.
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
    /// received power, noise floor and SNR that now decide the verdict. Used
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

/// A completed main solve, everything the UI needs to display it.
pub struct MainResult {
    pub assumptions: Assumptions,
    pub profile: Vec<ProfileRow>,
    pub outcome: SolveOutcome,
}

pub enum Job {
    Main(Inputs),
    Sweep(Inputs),
    Coverage(Inputs, CoverageConfig),
}

pub enum Msg {
    MainDone(Box<MainResult>),
    MainFailed(String),
    SweepStart {
        total: usize,
    },
    SweepProgress {
        done: usize,
        total: usize,
        point: SweepPoint,
    },
    SweepDone {
        best: Option<SweepBest>,
    },
    SweepFailed(String),
    CoverageStart {
        total: usize,
        threads: usize,
    },
    /// One finished grid point, streamed the instant it is computed so the map
    /// fills in progressively rather than appearing at the end.
    CoverageProgress {
        done: usize,
        total: usize,
        threads: usize,
        cell: Box<CoverageCell>,
    },
    CoverageDone {
        /// True when the run stopped early because CANCEL was pressed. The
        /// cells already streamed stay on the map either way.
        cancelled: bool,
    },
    CoverageFailed(String),
}

/// Reduce a full solve to the cached summary for one frequency.
///
/// Among the modes found, the one with the strongest SNR is kept: that is the
/// signal an operator would actually hear, and it is what the three-state
/// verdict must be based on.
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
/// SNR replaced absorption as the ranking key here because absorption is only
/// one term of the loss and says nothing about the noise the signal has to beat
/// - ranking by it could name a "best" frequency that is inaudible.
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

/// Handle to the worker thread. Dispatch jobs, drain results each frame.
///
/// Each dispatch bumps an epoch and cancels the previous job; results carry
/// their job's epoch and `drain` returns only current-epoch messages, so
/// straggler progress from a superseded job can never flip the UI state.
pub struct SolverService {
    /// Web build: nothing reads this, because `dispatch` runs the job inline
    /// rather than queuing it. Kept so the two builds share one struct shape.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    job_tx: Sender<(u64, Job, Arc<AtomicBool>)>,
    msg_rx: Receiver<(u64, Msg)>,
    /// Web build only: the job runs on the caller's stack, so the service holds
    /// the sending half and the pool the worker thread would otherwise own.
    #[cfg(target_arch = "wasm32")]
    msg_tx: Sender<(u64, Msg)>,
    #[cfg(target_arch = "wasm32")]
    ctx: Context,
    #[cfg(target_arch = "wasm32")]
    pool: ComputePool,
    current_cancel: Option<Arc<AtomicBool>>,
    epoch: u64,
}

impl SolverService {
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(ctx: Context) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<(u64, Job, Arc<AtomicBool>)>();
        let (msg_tx, msg_rx) = mpsc::channel::<(u64, Msg)>();
        std::thread::Builder::new()
            .name("skipzone-solver".to_string())
            .spawn(move || worker(&job_rx, &msg_tx, &ctx))
            .expect("spawn solver thread");
        Self {
            job_tx,
            msg_rx,
            current_cancel: None,
            epoch: 0,
        }
    }

    /// Web build: no worker thread, because wasm32-unknown-unknown has none to
    /// spawn. The channels and the epoch protocol are kept exactly as they are
    /// natively - only *when* the job runs changes, and [`Self::dispatch`] runs
    /// it inline. Everything downstream, including `drain`, is unaware.
    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn new(ctx: Context) -> Self {
        let (job_tx, _job_rx) = mpsc::channel::<(u64, Job, Arc<AtomicBool>)>();
        let (msg_tx, msg_rx) = mpsc::channel::<(u64, Msg)>();
        Self {
            job_tx,
            msg_rx,
            msg_tx,
            ctx,
            pool: build_pool("coverage", coverage_pool_config()),
            current_cancel: None,
            epoch: 0,
        }
    }

    /// Queue a job. Any in-flight job is asked to stop first (the sweep checks
    /// the flag between frequencies; a single main solve is atomic and simply
    /// finishes, its result then dropped by the epoch filter).
    pub fn dispatch(&mut self, job: Job) {
        if let Some(c) = self.current_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.epoch += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.job_tx.send((self.epoch, job, Arc::clone(&cancel)));
        // Web build: run it here and now. This blocks the browser's main thread
        // for the length of the job, so a long sweep or a coverage grid freezes
        // the tab until it finishes and its progress lands in one go instead of
        // streaming - the honest consequence of having no worker thread. CANCEL
        // cannot interrupt a job that is already running for the same reason.
        #[cfg(target_arch = "wasm32")]
        run_job(
            self.epoch,
            job,
            &self.pool,
            &cancel,
            &self.msg_tx,
            &self.ctx,
        );
        self.current_cancel = Some(cancel);
    }

    /// Ask the in-flight job to stop without queuing anything in its place.
    ///
    /// The epoch is deliberately NOT bumped: work already finished has been
    /// delivered under the current epoch, and the job's own closing message must
    /// still arrive so the UI learns the run ended. Remaining items stop at the
    /// next flag check, which for a coverage grid is before each grid point.
    pub fn cancel(&mut self) {
        if let Some(c) = self.current_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// Current-epoch messages that have arrived since the last call
    /// (non-blocking); stale messages from superseded jobs are discarded.
    pub fn drain(&self) -> Vec<Msg> {
        let epoch = self.epoch;
        self.msg_rx
            .try_iter()
            .filter_map(|(e, m)| (e == epoch).then_some(m))
            .collect()
    }
}

/// Run one job to completion, whatever thread we happen to be on.
///
/// Split out of [`worker`] so the web build, which has no worker thread, can
/// call exactly the same code path from `dispatch`. On wasm both job kinds share
/// one sequential pool, since the sweep/coverage thread-budget split that
/// `sweep_pool_config` and `coverage_pool_config` express has nothing to divide.
fn run_job(
    epoch: u64,
    job: Job,
    pool: &ComputePool,
    cancel: &AtomicBool,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    match job {
        Job::Main(inputs) => run_main(epoch, &inputs, msg_tx, ctx),
        Job::Sweep(inputs) => run_sweep(epoch, &inputs, pool, cancel, msg_tx, ctx),
        Job::Coverage(inputs, config) => {
            run_coverage(epoch, &inputs, config, pool, cancel, msg_tx, ctx);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn worker(
    job_rx: &Receiver<(u64, Job, Arc<AtomicBool>)>,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    // One pool per job kind for the worker's whole lifetime: sizing a rayon pool
    // costs a thread spawn per worker, so we pay it once and reuse it. The two
    // are separate because they are deliberately sized differently (see
    // `sweep_pool_config` and `coverage_pool_config`).
    let pool = build_pool("sweep", sweep_pool_config());
    let coverage_pool = build_pool("coverage", coverage_pool_config());
    while let Ok((epoch, job, cancel)) = job_rx.recv() {
        // The pool a job gets is the only thing the worker decides; the job
        // itself runs identically here and in the web build's inline path.
        let pool = match job {
            Job::Coverage(..) => &coverage_pool,
            Job::Main(_) | Job::Sweep(_) => &pool,
        };
        run_job(epoch, job, pool, &cancel, msg_tx, ctx);
    }
}

/// Execution config for the sweep's compute pool, overridable by environment so
/// the parallel layer can be capped or switched off entirely without a rebuild
/// (for A/B timing or debugging a suspected parallelism bug):
///   * `SKIPZONE_COMPUTE=sequential` - single-threaded fallback (the old path).
///   * `SKIPZONE_COMPUTE_THREADS=N`   - cap worker threads at N.
///
/// Default: parallel, but holding two cores back.
///
/// The sweep is the *incidental* background job - it runs while the operator
/// carries on panning the map, dragging stations and reading panels, and the
/// tile fetcher and the UI thread both want CPU while it does. Reserving two
/// cores keeps the interface responsive for the whole minute-plus it takes. The
/// coverage grid is the opposite case and is sized accordingly.
// Only the worker thread splits the thread budget between the two job kinds,
// and the web build has no worker thread and no budget to split.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn sweep_pool_config() -> PoolConfig {
    let cores = available_cores();
    PoolConfig {
        execution: execution_from_env(),
        max_threads: Some(threads_from_env().unwrap_or(cores.saturating_sub(2).max(1))),
    }
}

/// The coverage grid gets every core.
///
/// It is a deliberate, bounded, one-off run the operator starts and then
/// watches fill in - there is no other work competing for the machine, and the
/// only thing they are waiting on is this. So unlike the sweep it holds nothing
/// back.
fn coverage_pool_config() -> PoolConfig {
    PoolConfig {
        execution: execution_from_env(),
        max_threads: Some(threads_from_env().unwrap_or_else(available_cores)),
    }
}

/// `SKIPZONE_COMPUTE=sequential` switches the parallel layer off entirely, so a
/// suspected parallelism bug can be A/B'd without a rebuild.
fn execution_from_env() -> Execution {
    match std::env::var("SKIPZONE_COMPUTE").as_deref() {
        Ok("sequential" | "seq" | "off" | "0") => Execution::Sequential,
        _ => Execution::Parallel,
    }
}

/// `SKIPZONE_COMPUTE_THREADS=N` caps both pools at N threads.
fn threads_from_env() -> Option<usize> {
    std::env::var("SKIPZONE_COMPUTE_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
}

/// Build a pool, degrading to the single-threaded fallback if the thread pool
/// cannot be created (which must never take the app down).
fn build_pool(label: &str, config: PoolConfig) -> ComputePool {
    ComputePool::new(config).unwrap_or_else(|e| {
        eprintln!("[{label}] compute pool build failed ({e}); using sequential fallback");
        ComputePool::new(PoolConfig {
            execution: Execution::Sequential,
            max_threads: None,
        })
        .expect("sequential pool construction is infallible")
    })
}

fn run_main(epoch: u64, inputs: &Inputs, msg_tx: &Sender<(u64, Msg)>, ctx: &Context) {
    let a = scenario::resolve(inputs);
    match scenario::build_models(inputs, &a) {
        Ok(models) => {
            let profile = scenario::sample_profile(&models, &a);
            let outcome = solve::solve(inputs, &a, &models);
            let _ = msg_tx.send((
                epoch,
                Msg::MainDone(Box::new(MainResult {
                    assumptions: a,
                    profile,
                    outcome,
                })),
            ));
        }
        Err(e) => {
            let _ = msg_tx.send((epoch, Msg::MainFailed(e)));
        }
    }
    ctx.request_repaint();
}

/// Build the scenario models once, then run the sweep on the chosen execution
/// path. Model construction and the coarse-then-fine search structure are shared
/// by both paths; only *where* each frequency's `solve()` runs differs.
fn run_sweep(
    epoch: u64,
    inputs: &Inputs,
    pool: &ComputePool,
    cancel: &AtomicBool,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    let a = scenario::resolve(inputs);
    let models = match scenario::build_models(inputs, &a) {
        Ok(m) => m,
        Err(e) => {
            let _ = msg_tx.send((epoch, Msg::SweepFailed(e)));
            ctx.request_repaint();
            return;
        }
    };
    match pool.execution() {
        Execution::Parallel => {
            run_sweep_parallel(epoch, inputs, &a, &models, pool, cancel, msg_tx, ctx);
        }
        Execution::Sequential => {
            run_sweep_sequential(epoch, inputs, &a, &models, cancel, msg_tx, ctx);
        }
    }
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
    cancel: &AtomicBool,
) -> Option<SweepPoint> {
    if cancel.load(Ordering::Relaxed) {
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
/// pool. Because every candidate is dispatched together there is no uphill
/// early-stop (that was purely a way to save *sequential* time); the parallel
/// path instead evaluates the complete coarse grid, which is a superset of what
/// the sequential path would, so the per-frequency results are identical and the
/// winner is never worse.
#[allow(clippy::too_many_arguments)]
fn run_sweep_parallel(
    epoch: u64,
    inputs: &Inputs,
    a: &Assumptions,
    models: &Models,
    pool: &ComputePool,
    cancel: &AtomicBool,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    let coarse = coarse_freqs();
    // Optimistic total for the bar: whole coarse grid plus one fine window.
    let total_estimate = coarse.len() + fine_freqs(0.5 * (SWEEP_MIN_MHZ + SWEEP_MAX_MHZ)).len();
    let _ = msg_tx.send((
        epoch,
        Msg::SweepStart {
            total: total_estimate,
        },
    ));
    ctx.request_repaint();

    // Shared, lock-free progress counter; the mpsc Sender is !Sync so a clone is
    // wrapped in a Mutex (progress is one message per solve, so contention is
    // nil). Both are captured by the per-phase progress callbacks below.
    let done = AtomicUsize::new(0);
    let prog = Mutex::new(msg_tx.clone());

    let evaluate = |f: &f64| solve_point(*f, inputs, a, models, cancel);

    // Run one grid in parallel, streaming a SweepProgress per solved frequency.
    let run_phase = |freqs: &[f64], total: usize| -> (Vec<SweepPoint>, Timing) {
        let (opt_points, timing) =
            pool.map_reporting(freqs, evaluate, |_, point: &Option<SweepPoint>| {
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(point) = point {
                    if let Ok(tx) = prog.lock() {
                        let _ = tx.send((
                            epoch,
                            Msg::SweepProgress {
                                done: n,
                                total,
                                point: *point,
                            },
                        ));
                    }
                    ctx.request_repaint();
                }
            });
        (opt_points.into_iter().flatten().collect(), timing)
    };

    // Phase 1: the full coarse grid.
    let (coarse_points, coarse_timing) = run_phase(&coarse, total_estimate);
    let mut best = fold_best(None, &coarse_points);

    // Phase 2: fine refinement around the coarse best.
    let fine_timing = if let Some(seed) = best {
        let fine = fine_freqs(seed.freq_mhz);
        let total = done.load(Ordering::Relaxed) + fine.len();
        let (fine_points, timing) = run_phase(&fine, total);
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

    let _ = msg_tx.send((
        epoch,
        Msg::SweepDone {
            best: best.map(|point| SweepBest { point }),
        },
    ));
    ctx.request_repaint();
}

/// Single-threaded path: the original coarse-with-uphill-early-stop then fine
/// scan, preserved verbatim as the swappable fallback and the equivalence
/// baseline. Kept here so turning parallelism off reproduces the old behaviour
/// exactly, including which frequencies the early-stop skips.
fn run_sweep_sequential(
    epoch: u64,
    inputs: &Inputs,
    a: &Assumptions,
    models: &Models,
    cancel: &AtomicBool,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    let sweep_started = web_time::Instant::now();
    let mut per_item: Vec<Duration> = Vec::new();

    let coarse = coarse_freqs();
    // Optimistic total for the bar: coarse pass plus one fine window. Each
    // progress message carries its own (possibly revised) total.
    let mut total = coarse.len() + fine_freqs(0.5 * (SWEEP_MIN_MHZ + SWEEP_MAX_MHZ)).len();
    let _ = msg_tx.send((epoch, Msg::SweepStart { total }));
    ctx.request_repaint();

    let mut timed_solve = |f: f64| -> Option<SweepPoint> {
        let started = web_time::Instant::now();
        let point = solve_point(f, inputs, a, models, cancel);
        if point.is_some() {
            per_item.push(started.elapsed());
        }
        point
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
        let Some(point) = timed_solve(f) else { return };
        done += 1;
        best = Some(best.map_or(point, |b| better(b, point)));
        if point.badness() < best_badness - 1e-4 {
            best_badness = point.badness();
            worse_streak = 0;
        } else {
            worse_streak += 1;
        }
        let _ = msg_tx.send((epoch, Msg::SweepProgress { done, total, point }));
        ctx.request_repaint();
        if worse_streak >= UPHILL_PATIENCE {
            break; // going uphill: no need to fill out the rest of the band
        }
    }

    // Phase 2: fine refinement around the coarse best.
    if let Some(seed) = best {
        let fine = fine_freqs(seed.freq_mhz);
        total = done + fine.len();
        for &f in &fine {
            let Some(point) = timed_solve(f) else { return };
            done += 1;
            best = Some(best.map_or(point, |b| better(b, point)));
            let _ = msg_tx.send((epoch, Msg::SweepProgress { done, total, point }));
            ctx.request_repaint();
        }
    }

    let timing = Timing {
        total: sweep_started.elapsed(),
        per_item,
        threads: 1,
    };
    log_sweep_timing_single("sequential", &timing);

    let _ = msg_tx.send((
        epoch,
        Msg::SweepDone {
            best: best.map(|point| SweepBest { point }),
        },
    ));
    ctx.request_repaint();
}

/// Area coverage: build the models once, then run the existing point-to-point
/// solve at every grid point across the coverage pool, streaming each finished
/// cell the moment it lands.
///
/// This is a loop of traces and nothing more - there is no coverage-specific
/// physics anywhere in it. `crate::coverage::solve_cell` is the whole unit of
/// work; see that module for why one `Models` is valid for every grid point.
#[allow(clippy::too_many_arguments)]
fn run_coverage(
    epoch: u64,
    inputs: &Inputs,
    config: CoverageConfig,
    pool: &ComputePool,
    cancel: &AtomicBool,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    let a = scenario::resolve(inputs);
    let models = match scenario::build_models(inputs, &a) {
        Ok(m) => m,
        Err(e) => {
            let _ = msg_tx.send((epoch, Msg::CoverageFailed(e)));
            ctx.request_repaint();
            return;
        }
    };

    let step_deg = config.step_deg();
    let points = config.grid(inputs.tx_lat, inputs.tx_lon);
    let total = points.len();
    let threads = pool.threads();
    let _ = msg_tx.send((epoch, Msg::CoverageStart { total, threads }));
    ctx.request_repaint();

    let done = AtomicUsize::new(0);
    let prog = Mutex::new(msg_tx.clone());

    let (_cells, timing) = pool.map_reporting(
        &points,
        |&(lat, lon)| {
            // Checked per grid point, so pressing CANCEL stops the remaining
            // calculations rather than letting the whole grid drain.
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            Some(crate::coverage::solve_cell(
                lat, lon, step_deg, inputs, &models,
            ))
        },
        |_, cell: &Option<CoverageCell>| {
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(cell) = cell
                && let Ok(tx) = prog.lock()
            {
                let _ = tx.send((
                    epoch,
                    Msg::CoverageProgress {
                        done: n,
                        total,
                        threads,
                        cell: Box::new(*cell),
                    },
                ));
                ctx.request_repaint();
            }
        },
    );

    let cancelled = cancel.load(Ordering::Relaxed);
    eprintln!(
        "[coverage] {} of {total} grid point(s) on {threads} thread(s) in {:.3} s \
         (serial work {:.3} s, {:.1}x, mean {:.0} ms/point){}",
        timing.per_item.len(),
        timing.total.as_secs_f64(),
        timing.work().as_secs_f64(),
        timing.speedup(),
        timing.mean_item().as_secs_f64() * 1e3,
        if cancelled { " [CANCELLED]" } else { "" },
    );

    let _ = msg_tx.send((epoch, Msg::CoverageDone { cancelled }));
    ctx.request_repaint();
}

/// One-line timing summary combining the coarse and fine phases. This is the
/// instrumentation that confirms where sweep time actually goes and lets the
/// measured speedup be checked, rather than assumed.
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
    /// reproduce the sequential path exactly - not just approximately.
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

    /// The load-bearing check for this whole change: running the *same* set of
    /// candidate frequencies across all cores must produce bit-identical
    /// per-frequency results to running them one at a time, and it prints the
    /// before/after timing and the thread count so the speedup is measured, not
    /// assumed. Run with:
    ///   cargo test -p skipzone-app --release parallel_matches_sequential -- --nocapture
    #[test]
    fn parallel_matches_sequential_and_reports_timing() {
        use crate::compute::{ComputePool, Execution, PoolConfig, available_cores};
        use std::sync::atomic::AtomicBool;

        let inputs = Inputs::default();
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).expect("models build");

        // The exact frequencies the app's parallel sweep would touch: the whole
        // coarse grid plus a fine window around a plausible optimum. Both paths
        // evaluate this identical list, isolating the execution layer from the
        // search heuristic (early-stop) that differs between them.
        let mut freqs = coarse_freqs();
        freqs.extend(fine_freqs(14.0));

        let never_cancel = AtomicBool::new(false);
        let evaluate =
            |f: &f64| solve_point(*f, &inputs, &a, &models, &never_cancel).expect("not cancelled");

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

    /// The coverage run's three behavioural promises, checked without a window:
    /// every grid point is streamed as its own message (that is what makes the
    /// map fill in progressively rather than appearing at the end), the run
    /// reports the thread count it is using, and a cancel mid-run stops the
    /// remaining calculations while keeping everything already delivered.
    #[test]
    fn coverage_streams_each_point_and_cancel_keeps_what_was_computed() {
        use crate::coverage::CoverageConfig;

        // A small grid: enough points to cancel part-way through.
        let config = CoverageConfig {
            half_span_deg: 12.0,
            points_per_deg: 0.25, // 4 deg steps => 7 x 7 minus the centre
        };
        let inputs = Inputs::default();
        let expected = config.grid(inputs.tx_lat, inputs.tx_lon).len();
        assert!(expected > 8, "need a grid worth cancelling, got {expected}");

        let pool = build_pool("coverage-test", coverage_pool_config());
        let ctx = Context::default();

        // Run 1: to completion. One message per grid point, in the order they
        // finished, each carrying the pool's thread count.
        let (tx, rx) = mpsc::channel();
        let never = AtomicBool::new(false);
        run_coverage(1, &inputs, config, &pool, &never, &tx, &ctx);
        drop(tx);
        let msgs: Vec<Msg> = rx.into_iter().map(|(_, m)| m).collect();

        let mut streamed = 0usize;
        let mut started = None;
        let mut finished = None;
        for m in &msgs {
            match m {
                Msg::CoverageStart { total, threads } => started = Some((*total, *threads)),
                Msg::CoverageProgress { threads, .. } => {
                    streamed += 1;
                    assert_eq!(*threads, pool.threads());
                }
                Msg::CoverageDone { cancelled } => finished = Some(*cancelled),
                _ => panic!("unexpected message kind from a coverage run"),
            }
        }
        assert_eq!(started, Some((expected, pool.threads())));
        assert_eq!(streamed, expected, "one message per computed grid point");
        assert_eq!(finished, Some(false));

        // The coverage pool must be the more aggressive of the two: the sweep
        // deliberately holds cores back, this run does not.
        let sweep_threads = build_pool("sweep-test", sweep_pool_config()).threads();
        if available_cores() > 2 && threads_from_env().is_none() {
            assert!(
                pool.threads() > sweep_threads,
                "coverage ({}) should use more threads than the sweep ({sweep_threads})",
                pool.threads(),
            );
        }

        // Run 2: cancelled before it starts. Nothing further is computed, the
        // run still closes itself out, and it says it was cancelled.
        let (tx, rx) = mpsc::channel();
        let cancelled_flag = AtomicBool::new(true);
        run_coverage(2, &inputs, config, &pool, &cancelled_flag, &tx, &ctx);
        drop(tx);
        let msgs: Vec<Msg> = rx.into_iter().map(|(_, m)| m).collect();
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m, Msg::CoverageProgress { .. })),
            "a cancelled run must not solve any more grid points"
        );
        assert!(matches!(
            msgs.last(),
            Some(Msg::CoverageDone { cancelled: true })
        ));
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
        // Among found paths, the strongest SNR wins - even though the weak one
        // would have tied on absorption under the old rule.
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
        // SNR is `Prx - Pnoise` LESS the calibrated model bias, and the gap must be
        // exactly that bias and nothing else. This is the invariant that keeps the
        // correction honest: it is applied to the SNR alone, so `rx_power_dbm` and
        // every loss term still report what the propagation produced and a measured
        // fudge cannot hide inside the budget panel. Before the bias was applied by
        // default this read as a plain equality; the difference between the two is
        // the whole point, so it is asserted rather than relaxed to a tolerance.
        assert!(
            (hit.snr_db
                - (hit.rx_power_dbm - hit.noise_dbm - scenario::MEASURED_MODEL_BIAS_DB))
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
