//! Background jobs: the point-to-point solve, the frequency sweep and the
//! coverage grid, run off the UI thread so the interface never freezes.
//!
//! One worker thread runs predict's synchronous runners (see
//! [`skipzone_predict::run`]). Requests go in over one channel and responses
//! come back over another. Every request is stamped with a generation id, and
//! submitting a request or calling [`Jobs::cancel`] bumps the current
//! generation. A running job polls "is my generation still current?" between
//! units of work, so it stops once superseded, and [`Jobs::poll`] drops any
//! response whose generation is no longer current.
//!
//! [`Jobs::inline`] runs each request on the calling thread instead, so tests
//! get their results without waiting on another thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use skipzone_predict::compute::{
    ComputePool, background_pool_config, build_pool, coverage_pool_config,
};
use skipzone_predict::coverage::{CoverageCell, CoverageConfig};
use skipzone_predict::run::{
    self, CoverageEvent, MainResult, RunError, SweepEvent, run_coverage, run_sweep,
};
use skipzone_predict::scenario::Inputs;
use skipzone_predict::sweep::{SweepBest, SweepPoint};

/// A job to run.
pub enum Request {
    /// The point-to-point solve.
    Main(Inputs),
    /// The frequency sweep.
    Sweep(Inputs),
    /// The area coverage grid.
    Coverage(Inputs, CoverageConfig),
}

/// What a job reports back, in the order it happens.
pub enum Response {
    MainDone(Box<MainResult>),
    MainFailed(String),
    SweepStart {
        total: usize,
    },
    SweepPoint {
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
    /// One finished grid point, sent as soon as it is computed so the map
    /// fills in progressively.
    CoverageCell {
        done: usize,
        total: usize,
        threads: usize,
        cell: Box<CoverageCell>,
    },
    CoverageDone,
    CoverageFailed(String),
}

/// Called after every response is sent. The app passes one that asks egui to
/// redraw, since a response from the worker arrives between frames.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// Runs requests. Owns the compute pools and the sending end of the response
/// channel. Lives on the worker thread, or inside [`Jobs`] in inline mode.
struct Executor {
    /// For the main solve and the sweep: holds two cores back for the UI.
    background: ComputePool,
    /// For the coverage grid: every core.
    coverage: ComputePool,
    current: Arc<AtomicU64>,
    out: Sender<(u64, Response)>,
    repaint: Repaint,
}

impl Executor {
    fn new(current: Arc<AtomicU64>, out: Sender<(u64, Response)>, repaint: Repaint) -> Self {
        // Built once: sizing a rayon pool costs a thread spawn per worker.
        Self {
            background: build_pool("background", background_pool_config()),
            coverage: build_pool("coverage", coverage_pool_config()),
            current,
            out,
            repaint,
        }
    }

    fn run(&self, generation: u64, request: Request) {
        let cancelled = || self.current.load(Ordering::Relaxed) != generation;
        // A request superseded while it waited in the queue is skipped.
        if cancelled() {
            return;
        }
        let send = |r: Response| {
            let _ = self.out.send((generation, r));
            (self.repaint)();
        };
        // A cancelled job sends nothing at the end: its generation is stale,
        // so poll would drop the message anyway.
        match request {
            Request::Main(inputs) => {
                // Inside the pool, so the solve's own parallel work keeps the
                // same two cores free as the sweep.
                let result = self
                    .background
                    .install(|| run::run_main(&inputs, &cancelled));
                match result {
                    Ok(r) => send(Response::MainDone(Box::new(r))),
                    Err(RunError::Failed(e)) => send(Response::MainFailed(e)),
                    Err(RunError::Cancelled) => {}
                }
            }
            Request::Sweep(inputs) => {
                let on_event = |e: SweepEvent| {
                    send(match e {
                        SweepEvent::Start { total } => Response::SweepStart { total },
                        SweepEvent::Point { done, total, point } => {
                            Response::SweepPoint { done, total, point }
                        }
                    });
                };
                match run_sweep(&inputs, &self.background, &cancelled, &on_event) {
                    Ok(best) => send(Response::SweepDone { best }),
                    Err(RunError::Failed(e)) => send(Response::SweepFailed(e)),
                    Err(RunError::Cancelled) => {}
                }
            }
            Request::Coverage(inputs, config) => {
                let on_event = |e: CoverageEvent| {
                    send(match e {
                        CoverageEvent::Start { total, threads } => {
                            Response::CoverageStart { total, threads }
                        }
                        CoverageEvent::Cell {
                            done,
                            total,
                            threads,
                            cell,
                        } => Response::CoverageCell {
                            done,
                            total,
                            threads,
                            cell: Box::new(cell),
                        },
                    });
                };
                match run_coverage(&inputs, config, &self.coverage, &cancelled, &on_event) {
                    Ok(()) => send(Response::CoverageDone),
                    Err(RunError::Failed(e)) => send(Response::CoverageFailed(e)),
                    Err(RunError::Cancelled) => {}
                }
            }
        }
    }
}

enum Mode {
    /// Requests go to the worker thread.
    Worker(Sender<(u64, Request)>),
    /// Requests run on the caller's thread inside `submit`.
    Inline(Box<Executor>),
}

/// Handle to the background job service. Submit requests, poll responses once
/// per frame.
pub struct Jobs {
    current: Arc<AtomicU64>,
    responses: Receiver<(u64, Response)>,
    mode: Mode,
}

impl Jobs {
    /// Start the worker thread. `repaint` is called from it after every
    /// response.
    #[must_use]
    pub fn spawn(repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let current = Arc::new(AtomicU64::new(0));
        let (out, responses) = mpsc::channel();
        let (requests, inbox) = mpsc::channel::<(u64, Request)>();
        let executor = Executor::new(Arc::clone(&current), out, Arc::new(repaint));
        std::thread::Builder::new()
            .name("skipzone-jobs".to_string())
            .spawn(move || {
                // Ends when the Jobs handle, and with it the sender, is dropped.
                while let Ok((generation, request)) = inbox.recv() {
                    executor.run(generation, request);
                }
            })
            .expect("spawn job worker thread");
        Self {
            current,
            responses,
            mode: Mode::Worker(requests),
        }
    }

    /// Run every request synchronously inside [`Jobs::submit`]. For tests: the
    /// responses are ready to [`Jobs::poll`] as soon as `submit` returns.
    #[must_use]
    pub fn inline(repaint: impl Fn() + Send + Sync + 'static) -> Self {
        let current = Arc::new(AtomicU64::new(0));
        let (out, responses) = mpsc::channel();
        let executor = Executor::new(Arc::clone(&current), out, Arc::new(repaint));
        Self {
            current,
            responses,
            mode: Mode::Inline(Box::new(executor)),
        }
    }

    /// Start a job, superseding any job still running. Returns the job's
    /// generation.
    pub fn submit(&self, request: Request) -> u64 {
        let generation = self.current.fetch_add(1, Ordering::Relaxed) + 1;
        match &self.mode {
            Mode::Worker(requests) => {
                let _ = requests.send((generation, request));
            }
            Mode::Inline(executor) => executor.run(generation, request),
        }
        generation
    }

    /// Stop the running job. Its remaining work is abandoned at the next
    /// check, and anything it still sends is dropped by [`Jobs::poll`].
    pub fn cancel(&self) {
        self.current.fetch_add(1, Ordering::Relaxed);
    }

    /// The generation a response must carry to be delivered.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.current.load(Ordering::Relaxed)
    }

    /// Responses from the current job that arrived since the last call, without
    /// blocking. Responses from superseded or cancelled jobs are dropped.
    #[must_use]
    pub fn poll(&self) -> Vec<Response> {
        let current = self.generation();
        self.responses
            .try_iter()
            .filter_map(|(generation, r)| (generation == current).then_some(r))
            .collect()
    }
}

impl Drop for Jobs {
    /// Stop the running job so the worker thread can exit promptly.
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// A scenario that solves quickly: one hop, no sporadic E, short path.
    fn cheap_inputs() -> Inputs {
        Inputs {
            rx_lat: 45.0,
            rx_lon: -95.0,
            max_hops: 1,
            es_enabled: false,
            ..Inputs::default()
        }
    }

    fn is_main_done(r: &Response) -> bool {
        matches!(r, Response::MainDone(_))
    }

    /// A response stamped with a superseded generation never reaches the
    /// caller, even if it was sent before the newer request.
    #[test]
    fn stale_generation_responses_are_dropped() {
        let jobs = Jobs::inline(|| {});
        let first = jobs.submit(Request::Main(cheap_inputs()));
        let second = jobs.submit(Request::Main(cheap_inputs()));
        assert!(second > first);
        let responses = jobs.poll();
        // The first job's MainDone is already in the channel, but stale.
        assert_eq!(responses.iter().filter(|r| is_main_done(r)).count(), 1);
        assert_eq!(responses.len(), 1);

        // A cancel makes the next job's responses stale too.
        jobs.submit(Request::Main(cheap_inputs()));
        jobs.cancel();
        assert!(jobs.poll().is_empty());
    }

    /// Cancel stops a long job part-way: the worker becomes free for the next
    /// request long before the cancelled grid could have finished, and nothing
    /// from the cancelled job is delivered.
    #[test]
    fn cancel_stops_a_long_job() {
        let jobs = Jobs::spawn(|| {});
        // A grid far too large to finish in this test: every point is a solve.
        let big = CoverageConfig {
            half_span_deg: 60.0,
            points_per_deg: 1.0,
        };
        jobs.submit(Request::Coverage(cheap_inputs(), big));

        // Wait until the grid has started streaming cells.
        let started = Instant::now();
        let mut cells = 0usize;
        while cells == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "grid never started"
            );
            cells += jobs
                .poll()
                .iter()
                .filter(|r| matches!(r, Response::CoverageCell { .. }))
                .count();
            std::thread::sleep(Duration::from_millis(10));
        }

        jobs.cancel();
        // One worker thread: the next request runs only once the cancelled
        // grid has returned.
        jobs.submit(Request::Main(cheap_inputs()));
        let mut done = false;
        while !done {
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "the cancelled grid kept the worker busy"
            );
            for r in jobs.poll() {
                assert!(
                    !matches!(r, Response::CoverageCell { .. } | Response::CoverageDone),
                    "a cancelled job's response was delivered"
                );
                done |= is_main_done(&r);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
