//! The one place [`AppState`] changes.
//!
//! [`update`] applies an [`Action`] a view emitted, starting jobs on [`Jobs`]
//! where the action asks for one. [`absorb`] applies a [`Response`] a job sent
//! back. Each frame the app absorbs the responses, draws, then updates with
//! the actions the draw produced, in order.

use crate::jobs::{Jobs, Request, Response};
use crate::state::{AppState, Busy};
use crate::ui::actions::{Action, InputChange};
use crate::ui::map::view::ZOOM_STEP;

/// Apply one action.
pub fn update(state: &mut AppState, action: Action, jobs: &Jobs) {
    match action {
        Action::SetInput(change) => change.apply(&mut state.inputs),
        Action::PlaceStation { lat, lon } => {
            let station = state.ui.place;
            InputChange::Position { station, lat, lon }.apply(&mut state.inputs);
        }
        Action::SetPlaceMode(place) => state.ui.place = place,
        Action::Calculate => {
            // Re-running replaces what the panel is showing; the panel
            // itself stays where the operator sized it.
            calculate(state, jobs);
            state.ui.trace_open = true;
        }
        Action::BestFrequency => {
            find_best_frequency(state, jobs);
            state.ui.modals.best_freq = true;
        }
        Action::FitPath => state.ui.map.needs_fit = true,
        Action::ZoomIn => state.ui.map.pending_zoom *= ZOOM_STEP,
        Action::ZoomOut => state.ui.map.pending_zoom /= ZOOM_STEP,
        Action::TuneTo(freq_mhz) => state.inputs.scenario.freq_mhz = freq_mhz,
        Action::RunCoverage => {
            run_coverage(state, jobs);
            state.ui.modals.coverage = true;
        }
        Action::CancelCoverage => cancel_coverage(state, jobs),
        Action::ResetCoverage => state.results.coverage.clear(),
        Action::ShowCoverage => state.ui.modals.coverage = true,
        Action::ShowSettings => state.ui.modals.settings = true,
        Action::ShowAntennas => state.ui.modals.antennas = true,
        Action::ShowAbout => state.ui.modals.about = true,
    }
}

/// Kick off the point-to-point solve.
fn calculate(state: &mut AppState, jobs: &Jobs) {
    close_running_job(state);
    state.results.errors.main = None;
    state.results.busy = Busy::Solving;
    jobs.submit(Request::Main(state.inputs.scenario.clone()));
}

/// Kick off the frequency sweep. The current solution stays on screen; this
/// is a separate query alongside it.
fn find_best_frequency(state: &mut AppState, jobs: &Jobs) {
    close_running_job(state);
    state.results.errors.sweep = None;
    state.results.sweep.clear();
    state.results.busy = Busy::Sweeping { done: 0, total: 0 };
    jobs.submit(Request::Sweep(state.inputs.scenario.clone()));
}

/// Kick off the area coverage grid. Tiles from a previous run are cleared
/// first.
fn run_coverage(state: &mut AppState, jobs: &Jobs) {
    close_running_job(state);
    state.results.errors.coverage = None;
    state.results.coverage.clear();
    state.results.busy = Busy::Covering {
        done: 0,
        total: 0,
        threads: 0,
    };
    jobs.submit(Request::Coverage(
        state.inputs.scenario.clone(),
        state.inputs.coverage,
    ));
}

/// Stop the running coverage grid. Everything already computed stays on the
/// map; only the points still queued are abandoned.
fn cancel_coverage(state: &mut AppState, jobs: &Jobs) {
    if matches!(state.results.busy, Busy::Covering { .. }) {
        jobs.cancel();
        close_running_job(state);
    }
}

/// Close out the running job before it is cancelled or replaced. The job's
/// own closing response will carry a stale generation and be dropped, so a
/// coverage grid cut short is marked cancelled here.
fn close_running_job(state: &mut AppState) {
    if matches!(state.results.busy, Busy::Covering { .. }) {
        state.results.coverage.cancelled = true;
    }
    state.results.busy = Busy::Idle;
}

/// Apply one response from the job of `generation`.
pub fn absorb(state: &mut AppState, response: Response, generation: u64) {
    let results = &mut state.results;
    match response {
        Response::MainDone(result) => {
            let result = *result;
            let n = result.outcome.solutions.len();
            state.ui.selection.visible = vec![true; n];
            state.ui.selection.selected = (n > 0).then_some(0);
            results.main.outcome = Some(result.outcome);
            results.main.assumptions = Some(result.assumptions);
            results.main.profile = result.profile;
            results.main.generation = generation;
            results.errors.main = None;
            results.busy = Busy::Idle;
        }
        Response::MainFailed(e) => {
            results.main.clear();
            results.main.generation = generation;
            state.ui.selection.visible.clear();
            state.ui.selection.selected = None;
            results.errors.main = Some(e);
            results.busy = Busy::Idle;
        }
        Response::SweepStart { total } => {
            results.sweep.clear();
            results.sweep.points.reserve(total);
            results.sweep.generation = generation;
            results.busy = Busy::Sweeping { done: 0, total };
        }
        Response::SweepPoint { done, total, point } => {
            results.sweep.points.push(point);
            results.busy = Busy::Sweeping { done, total };
        }
        Response::SweepDone { best } => {
            results.sweep.best = best;
            results.busy = Busy::Idle;
        }
        Response::SweepFailed(e) => {
            results.errors.sweep = Some(e);
            results.busy = Busy::Idle;
        }
        Response::CoverageStart { total, threads } => {
            results.coverage.clear();
            results.coverage.cells.reserve(total);
            results.coverage.generation = generation;
            results.busy = Busy::Covering {
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
            results.coverage.cells.push(*cell);
            results.busy = Busy::Covering {
                done,
                total,
                threads,
            };
        }
        Response::CoverageDone => {
            results.coverage.cancelled = false;
            results.busy = Busy::Idle;
        }
        Response::CoverageFailed(e) => {
            results.errors.coverage = Some(e);
            results.busy = Busy::Idle;
        }
    }
}

/// Absorb every response waiting on `jobs`.
pub fn absorb_all(state: &mut AppState, jobs: &Jobs) {
    // Poll delivers only the current generation, and only this thread moves
    // it, so reading it first names the job every response came from.
    let generation = jobs.generation();
    for response in jobs.poll() {
        absorb(state, response, generation);
    }
}

#[cfg(test)]
mod tests {
    use skipzone_predict::coverage::CoverageConfig;
    use skipzone_predict::scenario::Inputs;

    use super::*;
    use crate::state::Station;

    /// A scenario that solves quickly: one hop, no sporadic E.
    fn cheap_scenario() -> Inputs {
        Inputs {
            max_hops: 1,
            es_enabled: false,
            ..Inputs::default()
        }
    }

    /// State on inline jobs with a tiny coverage grid. Inline jobs finish
    /// inside `submit`, but `busy` only changes when the responses are
    /// absorbed, so a run looks in progress until then.
    fn setup() -> (AppState, Jobs) {
        let mut state = AppState::with_scenario(cheap_scenario());
        state.inputs.coverage = CoverageConfig {
            half_span_deg: 4.0,
            points_per_deg: 0.25,
        };
        (state, Jobs::inline(|| {}))
    }

    fn apply(state: &mut AppState, jobs: &Jobs, actions: impl IntoIterator<Item = Action>) {
        for action in actions {
            update(state, action, jobs);
        }
    }

    #[test]
    fn cancelling_coverage_marks_it_cancelled() {
        let (mut s, jobs) = setup();
        apply(&mut s, &jobs, [Action::RunCoverage, Action::CancelCoverage]);
        // The job's own responses are stale now and dropped.
        absorb_all(&mut s, &jobs);
        assert!(s.results.coverage.cancelled);
        assert!(s.results.busy.is_idle());
        assert!(s.results.coverage.cells.is_empty());
    }

    #[test]
    fn a_new_job_marks_running_coverage_cancelled() {
        let (mut s, jobs) = setup();
        apply(&mut s, &jobs, [Action::RunCoverage, Action::Calculate]);
        absorb_all(&mut s, &jobs);
        assert!(s.results.coverage.cancelled);
        assert!(
            s.results.main.outcome.is_some(),
            "the Calculate result arrives"
        );
        assert!(s.results.busy.is_idle());
        assert!(s.ui.trace_open);
    }

    #[test]
    fn a_finished_coverage_run_is_not_cancelled() {
        let (mut s, jobs) = setup();
        apply(&mut s, &jobs, [Action::RunCoverage]);
        absorb_all(&mut s, &jobs);
        assert!(!s.results.coverage.cancelled);
        assert!(s.results.coverage.has_tiles());
        assert!(s.results.busy.is_idle());
        assert_eq!(s.results.coverage.generation, jobs.generation());
    }

    /// Every action of a frame is applied, in order, not just the first.
    #[test]
    fn every_action_in_a_frame_is_applied() {
        let (mut s, jobs) = setup();
        apply(
            &mut s,
            &jobs,
            [
                Action::ShowSettings,
                Action::TuneTo(7.1),
                Action::ShowAbout,
                Action::TuneTo(10.1),
            ],
        );
        assert!(s.ui.modals.settings);
        assert!(s.ui.modals.about);
        assert!((s.inputs.scenario.freq_mhz - 10.1).abs() < 1e-12);
    }

    /// A map click moves whichever station is being placed, and only that one.
    #[test]
    fn a_map_click_moves_the_station_being_placed() {
        let (mut s, jobs) = setup();
        let tx = (s.inputs.scenario.tx_lat, s.inputs.scenario.tx_lon);
        apply(
            &mut s,
            &jobs,
            [
                Action::SetPlaceMode(Station::Rx),
                Action::PlaceStation {
                    lat: 10.0,
                    lon: 20.0,
                },
            ],
        );
        assert_eq!(s.ui.place, Station::Rx);
        assert_eq!(
            (s.inputs.scenario.rx_lat, s.inputs.scenario.rx_lon),
            (10.0, 20.0)
        );
        assert_eq!((s.inputs.scenario.tx_lat, s.inputs.scenario.tx_lon), tx);

        let typed = InputChange::Position {
            station: Station::Tx,
            lat: -5.0,
            lon: 30.0,
        };
        apply(&mut s, &jobs, [Action::SetInput(typed)]);
        assert_eq!(
            (s.inputs.scenario.tx_lat, s.inputs.scenario.tx_lon),
            (-5.0, 30.0)
        );
    }
}
