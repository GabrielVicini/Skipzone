//! Session state: the scenario being worked on, the results computed from it,
//! and the handle to the background jobs.
//!
//! This is the layer between the UI and the compute layer. The UI never talks
//! to [`Jobs`] directly - it calls [`Session::calculate`] or
//! [`Session::find_best_frequency`] and reads the fields back. That keeps
//! "what has been computed" in one place instead of scattered across widgets.

use skipzone_iono::clock::{self, CivilDate};
use skipzone_predict::coverage::{CoverageCell, CoverageConfig};
use skipzone_predict::scenario::{Assumptions, Inputs, ProfileRow};
use skipzone_predict::solve::{Solution, SolveOutcome};
use skipzone_predict::sweep::{SweepBest, SweepPoint};

use crate::jobs::{Jobs, Request, Response};

/// What the background jobs are currently doing, for the status readout.
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
/// cancelled run stops appending, so its cells stay put.
#[derive(Default)]
pub struct CoverageResults {
    pub cells: Vec<CoverageCell>,
    /// The last run ended early because Cancel was pressed.
    pub cancelled: bool,
}

impl CoverageResults {
    /// Whether there is anything on the map to clear, which shows the Reset
    /// button: true after a finished or cancelled run.
    #[must_use]
    pub fn has_tiles(&self) -> bool {
        !self.cells.is_empty()
    }

    fn clear(&mut self) {
        self.cells.clear();
        self.cancelled = false;
    }
}

/// The last failure of each job kind, so a failed sweep is reported as a
/// sweep failure and not in the trace panel.
#[derive(Default)]
pub struct JobErrors {
    pub main: Option<String>,
    pub sweep: Option<String>,
    pub coverage: Option<String>,
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
    /// Model-build failures, one slot per job kind.
    pub errors: JobErrors,
    jobs: Jobs,
}

impl Session {
    /// Start from the default scenario, but on today's UTC date and time: the
    /// operator almost always wants "now", and `Inputs::default()` stays a
    /// fixed, test-reproducible scenario rather than a moving target.
    #[must_use]
    pub fn new(jobs: Jobs) -> Self {
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
            errors: JobErrors::default(),
            jobs,
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
        self.close_running_job();
        self.errors.main = None;
        self.busy = Busy::Solving;
        self.jobs.submit(Request::Main(self.inputs.clone()));
    }

    /// Kick off the frequency sweep. The current solution stays on screen;
    /// this is a separate query alongside it.
    pub fn find_best_frequency(&mut self) {
        self.close_running_job();
        self.errors.sweep = None;
        self.sweep.clear();
        self.busy = Busy::Sweeping { done: 0, total: 0 };
        self.jobs.submit(Request::Sweep(self.inputs.clone()));
    }

    /// Kick off the area coverage grid. Any tiles from a previous run are
    /// cleared first.
    pub fn run_coverage(&mut self) {
        self.close_running_job();
        self.errors.coverage = None;
        self.coverage.clear();
        self.busy = Busy::Covering {
            done: 0,
            total: 0,
            threads: 0,
        };
        self.jobs
            .submit(Request::Coverage(self.inputs.clone(), self.coverage_config));
    }

    /// Stop the running coverage grid. Everything already computed stays on the
    /// map; only the calculations still queued are abandoned.
    pub fn cancel_coverage(&mut self) {
        if matches!(self.busy, Busy::Covering { .. }) {
            self.jobs.cancel();
            self.close_running_job();
        }
    }

    /// Clear the coverage tiles from the map.
    pub fn reset_coverage(&mut self) {
        self.coverage.clear();
    }

    /// Close out the running job before it is cancelled or replaced. The job's
    /// own closing response will carry a stale generation and be dropped, so
    /// a coverage grid cut short is marked cancelled here.
    fn close_running_job(&mut self) {
        if matches!(self.busy, Busy::Covering { .. }) {
            self.coverage.cancelled = true;
        }
        self.busy = Busy::Idle;
    }

    /// Absorb any results the worker has posted since the last frame.
    pub fn pump(&mut self) {
        for response in self.jobs.poll() {
            self.apply(response);
        }
    }

    fn apply(&mut self, response: Response) {
        match response {
            Response::MainDone(result) => {
                let result = *result;
                self.solve.visible = vec![true; result.outcome.solutions.len()];
                self.solve.selected = (!result.outcome.solutions.is_empty()).then_some(0);
                self.solve.outcome = Some(result.outcome);
                self.solve.assumptions = Some(result.assumptions);
                self.solve.profile = result.profile;
                self.errors.main = None;
                self.busy = Busy::Idle;
            }
            Response::MainFailed(e) => {
                self.solve.clear();
                self.errors.main = Some(e);
                self.busy = Busy::Idle;
            }
            Response::SweepStart { total } => {
                self.sweep.clear();
                self.sweep.points.reserve(total);
                self.busy = Busy::Sweeping { done: 0, total };
            }
            Response::SweepPoint { done, total, point } => {
                self.sweep.points.push(point);
                self.busy = Busy::Sweeping { done, total };
            }
            Response::SweepDone { best } => {
                self.sweep.best = best;
                self.busy = Busy::Idle;
            }
            Response::SweepFailed(e) => {
                self.errors.sweep = Some(e);
                self.busy = Busy::Idle;
            }
            Response::CoverageStart { total, threads } => {
                self.coverage.clear();
                self.coverage.cells.reserve(total);
                self.busy = Busy::Covering {
                    done: 0,
                    total,
                    threads,
                };
            }
            Response::CoverageCell {
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
            Response::CoverageDone => {
                self.coverage.cancelled = false;
                self.busy = Busy::Idle;
            }
            Response::CoverageFailed(e) => {
                self.errors.coverage = Some(e);
                self.busy = Busy::Idle;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session on inline jobs with a tiny coverage grid. Inline jobs finish
    /// inside `submit`, but `busy` only changes when `pump` applies the
    /// responses, so a run looks in progress until then.
    fn session() -> Session {
        let mut s = Session::new(Jobs::inline(|| {}));
        s.inputs = Inputs {
            max_hops: 1,
            es_enabled: false,
            ..Inputs::default()
        };
        s.coverage_config = CoverageConfig {
            half_span_deg: 4.0,
            points_per_deg: 0.25,
        };
        s
    }

    #[test]
    fn cancelling_coverage_marks_it_cancelled() {
        let mut s = session();
        s.run_coverage();
        s.cancel_coverage();
        // The job's own responses are stale now and dropped.
        s.pump();
        assert!(s.coverage.cancelled);
        assert!(s.busy.is_idle());
        assert!(s.coverage.cells.is_empty());
    }

    #[test]
    fn a_new_job_marks_running_coverage_cancelled() {
        let mut s = session();
        s.run_coverage();
        s.calculate();
        s.pump();
        assert!(s.coverage.cancelled);
        assert!(s.solve.outcome.is_some(), "the Calculate result arrives");
        assert!(s.busy.is_idle());
    }

    #[test]
    fn a_finished_coverage_run_is_not_cancelled() {
        let mut s = session();
        s.run_coverage();
        s.pump();
        assert!(!s.coverage.cancelled);
        assert!(s.coverage.has_tiles());
        assert!(s.busy.is_idle());
    }
}
