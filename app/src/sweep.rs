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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use egui::Context;
use skipzone::magnetoionic::Mode;

use crate::scenario::{self, Assumptions, Inputs, ProfileRow};
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
#[derive(Clone, Copy)]
pub struct SweepPoint {
    pub freq_mhz: f64,
    pub connects: bool,
    /// Lowest total absorption [dB] among connecting modes; `+inf` if none.
    pub absorption_db: f64,
    /// Smallest near-miss [km] when nothing connects; 0 when it connects.
    pub miss_km: f64,
    pub hops: u32,
    pub mode: Option<Mode>,
}

impl SweepPoint {
    /// 0 (best) to 1 (worst), for the green->red chart: connecting points are
    /// graded by absorption over [0, `ABS_RED`] dB and kept in the green->amber
    /// half; non-connecting points fill the amber->red half by near-miss.
    #[must_use]
    pub fn badness(self) -> f32 {
        const ABS_RED_DB: f64 = 24.0;
        const MISS_RED_KM: f64 = 4000.0;
        if self.connects {
            #[allow(clippy::cast_possible_truncation)]
            let t = (self.absorption_db / ABS_RED_DB).clamp(0.0, 1.0) as f32;
            0.5 * t
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let t = (self.miss_km / MISS_RED_KM).clamp(0.0, 1.0) as f32;
            0.5 + 0.5 * t
        }
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
}

/// Reduce a full solve to the cached summary for one frequency.
fn summarize(freq_mhz: f64, out: &SolveOutcome) -> SweepPoint {
    if let Some(best) = out
        .solutions
        .iter()
        .min_by(|a, b| a.total_absorption_db.total_cmp(&b.total_absorption_db))
    {
        SweepPoint {
            freq_mhz,
            connects: true,
            absorption_db: best.total_absorption_db,
            miss_km: 0.0,
            hops: best.hops,
            mode: Some(best.mode),
        }
    } else {
        // near_misses are sorted ascending by miss_km in solve().
        let nm = out.near_misses.first();
        SweepPoint {
            freq_mhz,
            connects: false,
            absorption_db: f64::INFINITY,
            miss_km: nm.map_or(f64::INFINITY, |m| m.miss_km),
            hops: nm.map_or(0, |m| m.hops),
            mode: nm.map(|m| m.mode),
        }
    }
}

/// Best-frequency rule: any connecting frequency beats any non-connecting one;
/// among connectors the lowest absorption wins; among misses the smallest miss.
#[must_use]
pub fn better(a: SweepPoint, b: SweepPoint) -> SweepPoint {
    match (a.connects, b.connects) {
        (true, false) => a,
        (false, true) => b,
        (true, true) => {
            if a.absorption_db <= b.absorption_db {
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
    job_tx: Sender<(u64, Job, Arc<AtomicBool>)>,
    msg_rx: Receiver<(u64, Msg)>,
    current_cancel: Option<Arc<AtomicBool>>,
    epoch: u64,
}

impl SolverService {
    #[must_use]
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

    /// Queue a job. Any in-flight job is asked to stop first (the sweep checks
    /// the flag between frequencies; a single main solve is atomic and simply
    /// finishes, its result then dropped by the epoch filter).
    pub fn dispatch(&mut self, job: Job) {
        if let Some(c) = self.current_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.epoch += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let _ = self.job_tx.send((self.epoch, job, Arc::clone(&cancel)));
        self.current_cancel = Some(cancel);
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

fn worker(
    job_rx: &Receiver<(u64, Job, Arc<AtomicBool>)>,
    msg_tx: &Sender<(u64, Msg)>,
    ctx: &Context,
) {
    while let Ok((epoch, job, cancel)) = job_rx.recv() {
        match job {
            Job::Main(inputs) => run_main(epoch, &inputs, msg_tx, ctx),
            Job::Sweep(inputs) => run_sweep(epoch, &inputs, &cancel, msg_tx, ctx),
        }
    }
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

fn run_sweep(
    epoch: u64,
    inputs: &Inputs,
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
    let coarse = coarse_freqs();
    // Optimistic total for the bar: coarse pass plus one fine window. Each
    // progress message carries its own (possibly revised) total.
    let mut total = coarse.len() + fine_freqs(0.5 * (SWEEP_MIN_MHZ + SWEEP_MAX_MHZ)).len();
    let _ = msg_tx.send((epoch, Msg::SweepStart { total }));
    ctx.request_repaint();

    // A point is "better" exactly when its badness is lower (badness is built
    // to agree with `better`), so badness alone drives both the winner and the
    // uphill early-stop.
    let solve_point = |f: f64| -> Option<SweepPoint> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let mut fi = inputs.clone();
        fi.freq_mhz = f;
        Some(summarize(f, &solve::solve(&fi, &a, &models)))
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
        let Some(point) = solve_point(f) else { return };
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
            let Some(point) = solve_point(f) else { return };
            done += 1;
            best = Some(best.map_or(point, |b| better(b, point)));
            let _ = msg_tx.send((epoch, Msg::SweepProgress { done, total, point }));
            ctx.request_repaint();
        }
    }

    let _ = msg_tx.send((
        epoch,
        Msg::SweepDone {
            best: best.map(|point| SweepBest { point }),
        },
    ));
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn pt(freq: f64, connects: bool, abs: f64, miss: f64) -> SweepPoint {
        SweepPoint {
            freq_mhz: freq,
            connects,
            absorption_db: abs,
            miss_km: miss,
            hops: 1,
            mode: None,
        }
    }

    #[test]
    fn better_prefers_connecting_then_lowest_absorption() {
        let connect_hi = pt(14.0, true, 12.0, 0.0);
        let connect_lo = pt(10.0, true, 3.0, 0.0);
        let miss_small = pt(28.0, false, f64::INFINITY, 200.0);
        // Connecting beats missing regardless of the miss size.
        assert!(better(connect_hi, miss_small).connects);
        assert!(better(miss_small, connect_hi).connects);
        // Among connectors, lowest absorption wins.
        assert!((better(connect_hi, connect_lo).absorption_db - 3.0).abs() < 1e-12);
        // Among misses, smallest miss wins.
        let miss_big = pt(29.0, false, f64::INFINITY, 900.0);
        assert!((better(miss_small, miss_big).miss_km - 200.0).abs() < 1e-12);
    }

    #[test]
    fn badness_orders_best_to_worst() {
        let clean = pt(10.0, true, 0.0, 0.0);
        let lossy = pt(10.0, true, 24.0, 0.0);
        let missed = pt(28.0, false, f64::INFINITY, 3000.0);
        assert!(clean.badness() < lossy.badness());
        assert!(lossy.badness() <= missed.badness());
        assert!((0.0..=1.0).contains(&clean.badness()));
        assert!((0.0..=1.0).contains(&missed.badness()));
    }

    /// summarize reads a real solve: the default scenario connects (finite
    /// absorption), and a 45 MHz solve does not (a finite near-miss instead).
    #[test]
    fn summarize_reads_real_solves() {
        let inputs = Inputs::default();
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).expect("models");
        let connect = summarize(inputs.freq_mhz, &solve::solve(&inputs, &a, &models));
        assert!(connect.connects && connect.absorption_db.is_finite());

        let mut hi = inputs;
        hi.freq_mhz = 45.0;
        let missed = summarize(45.0, &solve::solve(&hi, &a, &models));
        assert!(!missed.connects);
    }
}
