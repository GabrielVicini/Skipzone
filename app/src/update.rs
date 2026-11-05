//! The one place [`AppState`] changes.
//!
//! [`update`] applies an [`Action`] a view emitted, starting jobs on [`Jobs`]
//! where the action asks for one. [`absorb`] applies a [`Response`] a job sent
//! back. Each frame the app absorbs the responses, draws, then updates with
//! the actions the draw produced, in order.

use skipzone_iono::clock;

use crate::jobs::{Jobs, Request, Response};
use crate::state::{AntennaPatterns, AppState, Busy, PatternKey};
use crate::ui::actions::{Action, InputChange};
use crate::ui::map::view::ZOOM_STEP;

/// Apply a frame's actions in order, then bring derived results up to date.
pub fn update_all(state: &mut AppState, actions: Vec<Action>, jobs: &Jobs) {
    for action in actions {
        update(state, action, jobs);
    }
    refresh_patterns(state);
}

/// Recompute the antenna gain curves if the Antennas dialog is open and an
/// input they depend on changed. Once per frame at most, after the actions.
fn refresh_patterns(state: &mut AppState) {
    if !state.ui.modals.antennas {
        return;
    }
    let inputs = &state.inputs.scenario;
    let key = PatternKey::of(inputs);
    if state
        .results
        .antenna_patterns
        .as_ref()
        .is_some_and(|p| p.key == key)
    {
        return;
    }
    state.results.antenna_patterns = Some(AntennaPatterns::compute(inputs));
}

/// Apply one action.
pub fn update(state: &mut AppState, action: Action, jobs: &Jobs) {
    match action {
        Action::SetInput(change) => change.apply(&mut state.inputs),
        Action::PlaceStation { lat, lon } => {
            let station = state.ui.place;
            InputChange::Position { station, lat, lon }.apply(&mut state.inputs);
        }
        Action::SetPlaceMode(place) => state.ui.place = place,
        Action::SetSolutionVisible { index, visible } => {
            if let Some(v) = state.ui.selection.visible.get_mut(index) {
                *v = visible;
            }
        }
        Action::SelectSolution(index) => state.ui.selection.selected = Some(index),
        Action::CloseTrace => state.ui.trace_open = false,
        // A job of the same kind already running is left to finish, and the
        // request only shows its readout. The views disable these buttons
        // then too.
        Action::Calculate => {
            // Re-running replaces what the panel is showing; the panel
            // itself stays where the operator sized it.
            if state.results.busy != Busy::Solving {
                calculate(state, jobs);
            }
            state.ui.trace_open = true;
        }
        Action::BestFrequency => {
            if !matches!(state.results.busy, Busy::Sweeping { .. }) {
                find_best_frequency(state, jobs);
            }
            state.ui.modals.best_freq = true;
        }
        Action::SetCamera { camera, fitted } => {
            let map = &mut state.ui.map;
            map.camera = camera;
            map.pending_zoom = 1.0;
            if fitted {
                map.needs_fit = false;
            }
        }
        Action::FitPath => state.ui.map.needs_fit = true,
        Action::ZoomIn => state.ui.map.pending_zoom *= ZOOM_STEP,
        Action::ZoomOut => state.ui.map.pending_zoom /= ZOOM_STEP,
        Action::SetDateToday => {
            let (today, _) = clock::utc_now();
            InputChange::Date(today).apply(&mut state.inputs);
        }
        Action::RunCoverage => {
            if !matches!(state.results.busy, Busy::Covering { .. }) {
                run_coverage(state, jobs);
            }
            state.ui.modals.coverage = true;
        }
        Action::CancelCoverage => cancel_coverage(state, jobs),
        Action::ResetCoverage => state.results.coverage.clear(),
        Action::OpenModal(modal) => state.ui.modals.set(modal, true),
        Action::CloseModal(modal) => state.ui.modals.set(modal, false),
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
    use crate::state::{Modal, Station};

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

    /// Every action of a frame is applied, in order, not just the first. The
    /// old sink kept only the first action a frame produced.
    #[test]
    fn every_action_in_a_frame_is_applied() {
        let (mut s, jobs) = setup();
        let frame = vec![
            Action::SetInput(InputChange::Freq(7.1)),
            Action::OpenModal(Modal::Settings),
            Action::SetInput(InputChange::Freq(10.1)),
        ];
        update_all(&mut s, frame, &jobs);
        assert!(s.ui.modals.settings, "the second action was applied");
        assert!(
            (s.inputs.scenario.freq_mhz - 10.1).abs() < 1e-12,
            "the last change wins"
        );
    }

    /// The camera the map reports is kept, and it consumes the pending zoom
    /// and, when it says so, the pending fit.
    #[test]
    fn the_map_camera_is_set_through_actions() {
        use crate::ui::map::projection::Camera;

        let (mut s, jobs) = setup();
        assert!(s.ui.map.needs_fit, "fits on the first frame");
        apply(&mut s, &jobs, [Action::ZoomIn]);
        assert!(s.ui.map.pending_zoom > 1.0);

        let camera = Camera {
            lat: 10.0,
            lon: 20.0,
            ppd: 5.0,
        };
        let not_fitted = Action::SetCamera {
            camera,
            fitted: false,
        };
        apply(&mut s, &jobs, [not_fitted]);
        assert_eq!(s.ui.map.camera, camera);
        assert!((s.ui.map.pending_zoom - 1.0).abs() < f64::EPSILON);
        assert!(s.ui.map.needs_fit, "a fit is still owed");

        apply(
            &mut s,
            &jobs,
            [Action::SetCamera {
                camera,
                fitted: true,
            }],
        );
        assert!(!s.ui.map.needs_fit);
        apply(&mut s, &jobs, [Action::FitPath]);
        assert!(s.ui.map.needs_fit);
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

    /// A mode change seeds the bandwidth and threshold from its preset.
    #[test]
    fn choosing_a_mode_seeds_its_bandwidth_and_threshold() {
        let (mut s, jobs) = setup();
        let mode = skipzone_predict::noise::OperatingMode::ALL
            .into_iter()
            .find(|m| *m != s.inputs.scenario.op_mode)
            .expect("more than one mode");
        apply(&mut s, &jobs, [Action::SetInput(InputChange::OpMode(mode))]);
        let inputs = &s.inputs.scenario;
        assert_eq!(inputs.op_mode, mode);
        assert_eq!(
            (inputs.bandwidth_hz, inputs.snr_threshold_db),
            mode.defaults()
        );
    }

    /// The time and date controls write the scenario's clock fields, and the
    /// night toggle only the display preference.
    #[test]
    fn time_date_and_night_edits_land_in_the_inputs() {
        let (mut s, jobs) = setup();
        let date = skipzone_iono::clock::CivilDate::new(2027, 3, 21);
        apply(
            &mut s,
            &jobs,
            [
                Action::SetInput(InputChange::UtcHours(6.5)),
                Action::SetInput(InputChange::Date(date)),
                Action::SetInput(InputChange::ShowTerminator(false)),
            ],
        );
        assert!((s.inputs.scenario.utc_hours - 6.5).abs() < 1e-12);
        assert_eq!(s.inputs.date(), date);
        assert!(!s.inputs.display.show_terminator);
    }

    /// Opening and closing dialogs touches only the dialog named.
    #[test]
    fn dialogs_open_and_close_one_at_a_time() {
        let (mut s, jobs) = setup();
        apply(
            &mut s,
            &jobs,
            [
                Action::OpenModal(Modal::Antennas),
                Action::OpenModal(Modal::Coverage),
                Action::CloseModal(Modal::Antennas),
            ],
        );
        assert!(!s.ui.modals.is_open(Modal::Antennas));
        assert!(s.ui.modals.is_open(Modal::Coverage));
        assert!(!s.ui.modals.is_open(Modal::Settings));
    }

    /// Settings edits land on their own field and nowhere else.
    #[test]
    fn settings_edits_change_only_their_field() {
        use skipzone_predict::scenario::GroundType;

        let (mut s, jobs) = setup();
        let before = s.inputs.scenario.clone();
        apply(
            &mut s,
            &jobs,
            [
                Action::SetInput(InputChange::Ssn(150.0)),
                Action::SetInput(InputChange::Ground(GroundType::SeaWater)),
                Action::SetInput(InputChange::MaxHops(3)),
                Action::SetInput(InputChange::ShowCoastlines(true)),
            ],
        );
        let expected = Inputs {
            ssn: 150.0,
            ground_type: GroundType::SeaWater,
            max_hops: 3,
            ..before
        };
        assert_eq!(s.inputs.scenario, expected);
        assert!(s.inputs.display.show_coastlines);
    }

    /// The antenna curves are computed when the dialog opens and again only
    /// when an input they depend on changes.
    #[test]
    fn antenna_patterns_follow_their_inputs_only() {
        use skipzone_predict::antenna::{AntennaConfig, AntennaKind};

        let (mut s, jobs) = setup();
        update_all(&mut s, vec![], &jobs);
        assert!(s.results.antenna_patterns.is_none(), "not while closed");

        update_all(&mut s, vec![Action::OpenModal(Modal::Antennas)], &jobs);
        let key = |s: &AppState| s.results.antenna_patterns.as_ref().map(|p| p.key);
        assert_eq!(key(&s), Some(PatternKey::of(&s.inputs.scenario)));

        // Plant isotropic curves under the current key: a recompute would
        // replace them with the real dipole curves.
        let isotropic = AntennaConfig {
            kind: AntennaKind::Isotropic,
            ..AntennaConfig::default()
        };
        let planted = AntennaPatterns::compute(&Inputs {
            tx_antenna: isotropic,
            rx_antenna: isotropic,
            ..s.inputs.scenario.clone()
        });
        let iso_label = planted.tx.label().to_string();
        s.results.antenna_patterns = Some(AntennaPatterns {
            key: PatternKey::of(&s.inputs.scenario),
            ..planted
        });
        let tx_label = |s: &AppState| {
            s.results
                .antenna_patterns
                .as_ref()
                .map(|p| p.tx.label().to_string())
        };

        update_all(
            &mut s,
            vec![Action::SetInput(InputChange::Ssn(10.0))],
            &jobs,
        );
        assert_eq!(tx_label(&s), Some(iso_label.clone()), "kept");

        let config = AntennaConfig {
            kind: AntennaKind::VerticalMonopole,
            ..s.inputs.scenario.rx_antenna
        };
        let change = InputChange::Antenna {
            station: Station::Rx,
            config,
        };
        update_all(&mut s, vec![Action::SetInput(change)], &jobs);
        assert_eq!(s.inputs.scenario.rx_antenna, config);
        assert_eq!(key(&s), Some(PatternKey::of(&s.inputs.scenario)));
        assert_ne!(tx_label(&s), Some(iso_label), "recomputed");
    }

    /// The coverage controls change the next run's grid and the tile
    /// opacity, and leave the scenario alone.
    #[test]
    fn coverage_controls_set_the_grid_and_opacity() {
        let (mut s, jobs) = setup();
        let scenario = s.inputs.scenario.clone();
        let grid = CoverageConfig {
            half_span_deg: 12.0,
            points_per_deg: 0.5,
        };
        apply(
            &mut s,
            &jobs,
            [
                Action::SetInput(InputChange::CoverageGrid(grid)),
                Action::SetInput(InputChange::CoverageAlpha(80)),
            ],
        );
        assert_eq!(s.inputs.coverage, grid);
        assert_eq!(s.inputs.display.coverage_alpha, 80);
        assert_eq!(s.inputs.scenario, scenario);
    }

    /// A solve resets the legend; toggles and picks then change only the
    /// selection, never the solve's results.
    #[test]
    fn legend_toggles_and_picks_change_the_selection() {
        let (mut s, jobs) = setup();
        // The full default scenario, which finds several modes.
        s.inputs.scenario = Inputs::default();
        apply(&mut s, &jobs, [Action::Calculate]);
        absorb_all(&mut s, &jobs);
        let n = s.results.main.solutions().len();
        assert!(n >= 1, "the default path has a mode");
        assert_eq!(s.ui.selection.visible, vec![true; n]);
        assert_eq!(s.ui.selection.selected, Some(0));

        apply(
            &mut s,
            &jobs,
            [
                Action::SetSolutionVisible {
                    index: 0,
                    visible: false,
                },
                Action::SelectSolution(n - 1),
                // Out of range is ignored, not a panic.
                Action::SetSolutionVisible {
                    index: n,
                    visible: false,
                },
                Action::CloseTrace,
            ],
        );
        assert!(!s.ui.selection.visible[0]);
        assert_eq!(s.ui.selection.selected, Some(n - 1));
        assert_eq!(s.results.main.solutions().len(), n);
        assert!(!s.ui.trace_open);
    }

    /// A job of the same kind is not restarted while it runs; starting the
    /// other kind supersedes it.
    #[test]
    fn a_running_job_is_not_restarted_by_its_own_kind() {
        let (mut s, jobs) = setup();
        apply(&mut s, &jobs, [Action::RunCoverage]);
        let running = jobs.generation();
        apply(&mut s, &jobs, [Action::RunCoverage]);
        assert_eq!(jobs.generation(), running, "no second grid");
        absorb_all(&mut s, &jobs);
        assert!(s.results.coverage.has_tiles(), "the first grid finished");
        assert!(!s.results.coverage.cancelled);

        apply(&mut s, &jobs, [Action::Calculate]);
        let solving = jobs.generation();
        apply(&mut s, &jobs, [Action::Calculate]);
        assert_eq!(jobs.generation(), solving, "no second solve");
        absorb_all(&mut s, &jobs);
        assert!(s.results.busy.is_idle());
    }
}
