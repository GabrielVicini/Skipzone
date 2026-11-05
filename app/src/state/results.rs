//! What the background jobs computed, and what they are doing now.
//!
//! Each result records the job generation it came from (see
//! [`crate::jobs::Jobs`]), so it can be told apart from a later run.

use skipzone_predict::coverage::CoverageCell;
use skipzone_predict::scenario::{Assumptions, ProfileRow};
use skipzone_predict::solve::{Solution, SolveOutcome};
use skipzone_predict::sweep::{SweepBest, SweepPoint};

/// What the background jobs are currently doing, for the status readout.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Busy {
    #[default]
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

/// Everything one point-to-point solve produced.
#[derive(Default)]
pub struct SolveResults {
    pub outcome: Option<SolveOutcome>,
    pub assumptions: Option<Assumptions>,
    pub profile: Vec<ProfileRow>,
    /// The job generation these came from, 0 before the first solve.
    pub generation: u64,
}

impl SolveResults {
    /// The solutions to draw on the map; empty when nothing has been solved.
    #[must_use]
    pub fn solutions(&self) -> &[Solution] {
        self.outcome
            .as_ref()
            .map_or(&[], |o| o.solutions.as_slice())
    }

    pub fn clear(&mut self) {
        self.outcome = None;
        self.profile.clear();
    }
}

/// Everything the frequency sweep produced.
#[derive(Default)]
pub struct SweepResults {
    /// Every frequency tried so far, in the order they completed.
    pub points: Vec<SweepPoint>,
    /// Winner of the last completed sweep.
    pub best: Option<SweepBest>,
    pub generation: u64,
}

impl SweepResults {
    /// Points sorted by frequency, which is what every chart wants.
    #[must_use]
    pub fn sorted(&self) -> Vec<SweepPoint> {
        let mut sorted = self.points.clone();
        sorted.sort_by(|a, b| a.freq_mhz.total_cmp(&b.freq_mhz));
        sorted
    }

    pub fn clear(&mut self) {
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
    /// The last run ended early, cancelled or superseded by another job.
    pub cancelled: bool,
    pub generation: u64,
}

impl CoverageResults {
    /// Whether there is anything on the map to clear, which shows the Reset
    /// button: true after a finished or cancelled run.
    #[must_use]
    pub fn has_tiles(&self) -> bool {
        !self.cells.is_empty()
    }

    pub fn clear(&mut self) {
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

/// Everything computed from the inputs, and the state of the job computing it.
#[derive(Default)]
pub struct Results {
    pub main: SolveResults,
    pub sweep: SweepResults,
    pub coverage: CoverageResults,
    pub busy: Busy,
    /// Model-build failures, one slot per job kind.
    pub errors: JobErrors,
}

impl Results {
    #[must_use]
    pub fn is_busy(&self) -> bool {
        !self.busy.is_idle()
    }
}
