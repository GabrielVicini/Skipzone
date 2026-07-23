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
use crate::scenario::{Assumptions, Inputs, ProfileRow};
use crate::solve::{Solution, SolveOutcome};
use crate::sweep::{Job, Msg, SolverService, SweepBest, SweepPoint};

/// What the background solver is currently doing, for the status readout.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Idle,
    Solving,
    Sweeping { done: usize, total: usize },
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
            Self::Sweeping { done, total } if total > 0 =>
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

pub struct Session {
    pub inputs: Inputs,
    pub solve: SolveResults,
    pub sweep: SweepResults,
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
            busy: Busy::Idle,
            error: None,
            solver: SolverService::new(ctx),
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
        }
    }
}
