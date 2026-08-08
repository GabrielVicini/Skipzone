//! Session state: the scenario being worked on, the results computed from it,
//! and the handle to the background solver.
//!
//! This is the layer between the UI and the compute layer. It owns no egui
//! types beyond the `Context` the solver needs to request repaints, and the UI
//! never talks to [`SolverService`] directly - it calls [`Session::calculate`]
//! or [`Session::find_best_frequency`] and reads the fields back. That keeps
//! "what has been computed" in one place instead of scattered across widgets.

use egui::Context;

use crate::clock::{self, CivilDate};
use crate::coverage::{CoverageCell, CoverageConfig};
use crate::scenario::{Assumptions, Inputs, ProfileRow};
use crate::solve::{Solution, SolveOutcome};
use crate::sweep::{Job, Msg, SolverService, SweepBest, SweepPoint};

/// What the background solver is currently doing, for the status readout.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Idle,
    Solving,
    Sweeping {
        done: usize,
        total: usize,
    },
    /// The area coverage grid. `threads` is carried so the progress readout can
    /// state how much of the machine the run is using.
    Covering {
        done: usize,
        total: usize,
        threads: usize,
    },
}

impl Busy {
    #[must_use]
    pub fn is_idle(self) -> bool {
        self == Self::Idle
    }

    /// Progress as a fraction, or `None` when there is nothing measurable to
    /// report (idle, or a single solve with no intermediate steps).
    #[must_use]
    pub fn fraction(self) -> Option<f32> {
        match self {
            Self::Sweeping { done, total } | Self::Covering { done, total, .. } if total > 0 =>
            {
                #[allow(clippy::cast_precision_loss)]
                Some((done as f32 / total as f32).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn label(self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Solving => Some("calculating".to_string()),
            Self::Sweeping { done, total } => Some(format!("sweeping {done}/{total}")),
            Self::Covering {
                done,
                total,
                threads,
            } => Some(format!("coverage {done}/{total} on {threads} threads")),
        }
    }
}

/// Everything one point-to-point solve produced, plus which of its modes the
/// operator is currently looking at.
#[derive(Default)]
pub struct SolveResults {
    pub outcome: Option<SolveOutcome>,
    pub assumptions: Option<Assumptions>,
    pub profile: Vec<ProfileRow>,
    /// Per-solution draw flag, parallel to `outcome.solutions`.
    pub visible: Vec<bool>,
    /// Index into `outcome.solutions` of the mode shown in detail.
    pub selected: Option<usize>,
}

impl SolveResults {
    /// The solutions to draw on the map; empty when nothing has been solved.
    #[must_use]
    pub fn solutions(&self) -> &[Solution] {
        self.outcome
            .as_ref()
            .map_or(&[], |o| o.solutions.as_slice())
    }

    /// The mode currently selected for the detail readouts.
    #[must_use]
    pub fn selected_solution(&self) -> Option<&Solution> {
        self.selected.and_then(|i| self.solutions().get(i))
    }

    fn clear(&mut self) {
        self.outcome = None;
        self.profile.clear();
        self.visible.clear();
        self.selected = None;
    }
}

/// Everything the frequency sweep produced.
#[derive(Default)]
pub struct SweepResults {
    /// Every frequency tried so far, in the order they completed.
    pub points: Vec<SweepPoint>,
    /// Winner of the last completed sweep.
    pub best: Option<SweepBest>,
}

impl SweepResults {
    /// Points sorted by frequency, which is what every chart wants.
    #[must_use]
    pub fn sorted(&self) -> Vec<SweepPoint> {
        let mut sorted = self.points.clone();
        sorted.sort_by(|a, b| a.freq_mhz.total_cmp(&b.freq_mhz));
        sorted
    }

    fn clear(&mut self) {
        self.points.clear();
        self.best = None;
    }
}

/// Everything the area coverage run has produced so far.
///
/// `cells` is append-only while a run streams in, which is what makes the map
/// fill in progressively: the plugin draws whatever is in here each frame. A
/// cancelled run simply stops appending, so its cells stay put.
#[derive(Default)]
pub struct CoverageResults {
    pub cells: Vec<CoverageCell>,
    /// The last run ended early because CANCEL was pressed.
    pub cancelled: bool,
}

impl CoverageResults {
    /// Is there anything on the map to clear? This is what puts the RESET
    /// button on screen - true after a finished run and after a cancelled one.
    #[must_use]
    pub fn has_tiles(&self) -> bool {
        !self.cells.is_empty()
    }

    fn clear(&mut self) {
        self.cells.clear();
        self.cancelled = false;
    }
}

pub struct Session {
    pub inputs: Inputs,
    pub solve: SolveResults,
    pub sweep: SweepResults,
    /// Grid settings for the area coverage map. A run parameter rather than
    /// part of the scenario, so it lives here and not in `Inputs`.
    pub coverage_config: CoverageConfig,
    pub coverage: CoverageResults,
    pub busy: Busy,
    /// Model-build or solver failure from the last dispatched job.
    pub error: Option<String>,
    solver: SolverService,
}

impl Session {
    /// Start from the default scenario, but on today's UTC date and time: the
    /// operator almost always wants "now", and `Inputs::default()` stays a
    /// fixed, test-reproducible scenario rather than a moving target.
    #[must_use]
    pub fn new(ctx: Context) -> Self {
        let (today, hours) = clock::utc_now();
        let inputs = Inputs {
            year: today.year,
            month: today.month,
            day_of_month: today.day,
            utc_hours: hours,
            ..Inputs::default()
        };
        Self {
            inputs,
            solve: SolveResults::default(),
            sweep: SweepResults::default(),
            coverage_config: CoverageConfig::default(),
            coverage: CoverageResults::default(),
            busy: Busy::Idle,
            error: None,
            // The computation layer knows nothing about egui; all it needs from
            // the view is a way to say "redraw, there is something new".
            solver: SolverService::new(std::sync::Arc::new(move || ctx.request_repaint())),
        }
    }

    #[must_use]
    pub fn date(&self) -> CivilDate {
        CivilDate::new(
            self.inputs.year,
            self.inputs.month,
            self.inputs.day_of_month,
        )
    }

    pub fn set_date(&mut self, date: CivilDate) {
        self.inputs.year = date.year;
        self.inputs.month = date.month;
        self.inputs.day_of_month = date.day;
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        !self.busy.is_idle()
    }

    /// Kick off the point-to-point solve on the worker thread.
    pub fn calculate(&mut self) {
        self.error = None;
        self.busy = Busy::Solving;
        self.solver.dispatch(Job::Main(self.inputs.clone()));
    }

    /// Kick off the frequency sweep. The current solution stays on screen;
    /// this is a separate query alongside it.
    pub fn find_best_frequency(&mut self) {
        self.error = None;
        self.sweep.clear();
        self.busy = Busy::Sweeping { done: 0, total: 0 };
        self.solver.dispatch(Job::Sweep(self.inputs.clone()));
    }

    /// Kick off the area coverage grid. Any tiles from a previous run are
    /// cleared first: a new grid is a new answer, not an overlay on the old one.
    pub fn run_coverage(&mut self) {
        self.error = None;
        self.coverage.clear();
        self.busy = Busy::Covering {
            done: 0,
            total: 0,
            threads: 0,
        };
        self.solver
            .dispatch(Job::Coverage(self.inputs.clone(), self.coverage_config));
    }

    /// Stop the running coverage grid. Everything already computed stays on the
    /// map; only the calculations still queued are abandoned.
    pub fn cancel_coverage(&mut self) {
        self.solver.cancel();
    }

    /// Clear the coverage tiles from the map.
    pub fn reset_coverage(&mut self) {
        self.coverage.clear();
    }

    /// Absorb any results the worker has posted since the last frame.
    pub fn pump(&mut self) {
        for msg in self.solver.drain() {
            self.apply(msg);
        }
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::MainDone(result) => {
                let result = *result;
                self.solve.visible = vec![true; result.outcome.solutions.len()];
                self.solve.selected = (!result.outcome.solutions.is_empty()).then_some(0);
                self.solve.outcome = Some(result.outcome);
                self.solve.assumptions = Some(result.assumptions);
                self.solve.profile = result.profile;
                self.error = None;
                self.busy = Busy::Idle;
            }
            Msg::MainFailed(e) => {
                self.solve.clear();
                self.error = Some(e);
                self.busy = Busy::Idle;
            }
            Msg::SweepStart { total } => {
                self.sweep.clear();
                self.sweep.points.reserve(total);
                self.busy = Busy::Sweeping { done: 0, total };
            }
            Msg::SweepProgress { done, total, point } => {
                self.sweep.points.push(point);
                self.busy = Busy::Sweeping { done, total };
            }
            Msg::SweepDone { best } => {
                self.sweep.best = best;
                self.busy = Busy::Idle;
            }
            Msg::SweepFailed(e) => {
                self.error = Some(e);
                self.busy = Busy::Idle;
            }
            Msg::CoverageStart { total, threads } => {
                self.coverage.clear();
                self.coverage.cells.reserve(total);
                self.busy = Busy::Covering {
                    done: 0,
                    total,
                    threads,
                };
            }
            Msg::CoverageProgress {
                done,
                total,
                threads,
                cell,
            } => {
                self.coverage.cells.push(*cell);
                self.busy = Busy::Covering {
                    done,
                    total,
                    threads,
                };
            }
            Msg::CoverageDone { cancelled } => {
                self.coverage.cancelled = cancelled;
                self.busy = Busy::Idle;
            }
            Msg::CoverageFailed(e) => {
                self.error = Some(e);
                self.busy = Busy::Idle;
            }
        }
    }
}
