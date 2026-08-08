# App Crate (`skipzone-app`, `app/src/`)

The point-to-point application, and everything that is not the ray tracer. It is
a library with a thin `main.rs` on top, so the headless harnesses in
`app/src/bin/` drive exactly the same model and solver code the GUI does rather
than a parallel copy.

The layer stack and its two invariants are in
[Architecture](Architecture.md#the-apps-internal-layers). This page walks the
modules.

## Conversion layer

Depends on nothing else in the crate.

### `clock`

Civil date and time arithmetic plus the system UTC clock. Implements the
proleptic Gregorian conversion from its closed form rather than taking a
date-time dependency: `days_from_civil` and `civil_from_days` are Howard
Hinnant's algorithms, which shift the year to start in March so the leap day
falls at the end of a 146097-day, 400-year era. Exact over the whole range of
`i64` days, pinned against known dates by test.

Nothing here feeds the physics. `solar` derives declination from day of year
alone and deliberately ignores leap years, which is far below the accuracy of
the climatology it drives. The year exists so the operator sees a real date.

### `grid`

Maidenhead locator conversion, both directions, with validation. Invalid text is
reported rather than clamped.

### `solar`

Solar geometry for the path midpoint: declination, hour angle, solar zenith
angle, local solar time. This is what makes the ionosphere day/night aware.

### `coastline`

Decides whether a point is sea, fresh water or land, from the Natural Earth
1:50m `land` and `lakes` polygon datasets bundled under `app/src/assets/`.

Two things to know. First, it classifies **water versus land only**; it does not
distinguish sea state, ice or terrain. Second, at 1:50m resolution small
features are absent, so a point a few kilometres from a coast can classify as
the wrong side. Known reference points are pinned by test, and holes and islands
are checked to invert correctly.

The result feeds the ground reflection term, per hop, from each bounce's own
position rather than one ground type for the whole path.

## Model layer

May know about the engine. This is the one place the application carries physics
of its own, and [Ionosphere Model](Ionosphere-Model.md) covers it in full.

- `chapman` - horizontally varying Chapman layers, including the grazing-
  incidence D region.
- `fof2` - where the F2 and E layers get their peak densities, from a bundled
  climatology grid.
- `sporadic_e` - the thin intermittent layer near 100 km, kept structurally
  apart from the deterministic verdict.
- `scenario` - turns UI inputs into engine model objects and records every
  assumed value so the interface can display it. No physics of its own.
- `noise` - the radio noise floor and the received-signal / SNR judgment layer.
  It answers the question the ray tracer cannot: given that a path exists, can
  anyone hear it.
- `antenna` - elevation patterns in dBi as a function of take-off angle.

### `antenna`

Closed-form patterns from image theory over a flat lossy ground
(`antenna/image.rs`): horizontal dipole, end-fed half wave, vertical monopole,
each with a stated `provenance` string saying where the model comes from and
what it excludes. The dipole is checked against NEC-4 over real ground, and the
end-fed half wave against its harmonic behaviour.

`antenna/table.rs` is an **extension point, deliberately not wired up**. A Yagi,
log-periodic, trap dipole or loop cannot be written in closed form, and the
honest way to carry one is tabulated gain against elevation per frequency and
per height, from a NEC run or a measurement. The machinery is implemented and
tested so that adding one is a data file plus an `AntennaKind` variant rather
than a redesign. Nothing ships with the app yet, which is why the module carries
a crate-level `allow(dead_code)`. Interpolation is bilinear in frequency and
height, linear in elevation, all in dB; outside the tabulated range the nearest
block is used and `in_range` reports false, so the UI can say the pattern is
being extrapolated instead of presenting an edge value as data.

## Computation layer

May know about the model. Never mentions egui.

### `compute`

The general-purpose parallel execution layer. `ComputePool` wraps a private
rayon pool, `map` and `map_reporting` run a batch and return results in input
order plus a `Timing`. `map_reporting` additionally invokes a callback as each
item finishes, in completion order, which is how progress streams to the UI
while order is still preserved in the result.

Sequential and parallel execution are bit-identical. That is the property the
whole layer is built around, and it is what lets any parallel result be checked
against the single-threaded engine for free.

### `solve`

Drives the engine's homing and tracer to produce every mode that connects, plus
full per-hop geometry for drawing and a near-miss report when nothing connects.
Calls the engine's public API only. Split by concern:

- `solve/types.rs` - the result structs the UI renders.
- `solve/link_budget.rs` - free-space spreading and Fresnel ground reflection.
  Pure functions, no engine calls.
- `solve/tracing.rs` - per-hop tracing and homing helpers.
- `solve/mod.rs` - the top-level driver that stitches them together.

See [Solver and Link Budget](Solver-and-Link-Budget.md).

### `sweep`

The background solver service. Every `solve()` runs off the UI thread so the
interface never freezes. Three job kinds share one worker thread and one result
channel: a single point-to-point solve, the frequency sweep, and the coverage
grid.

Each dispatch bumps an epoch and cancels the previous job. Results carry their
job's epoch and `drain` returns only current-epoch messages, so straggler
progress from a superseded job can never flip the UI state. `cancel`
deliberately does **not** bump the epoch, because work already finished was
delivered under the current one and the job's closing message must still arrive.

The frequency sweep is coarse to fine: a 1 MHz pass locates the good region and
stops early once it is clearly past the optimum and getting worse, then a 0.2
MHz pass refines a 1.5 MHz window around the best.

This module owns the `Wake` callback type that keeps egui out of the computation
layer.

### `coverage`

Area coverage: one transmitter, a grid of receiver positions, and the *existing*
point-to-point calculation run once per grid point. There is no interpolation
and no separate area model. A cell is a real trace, and the test
`cell_matches_a_full_point_to_point_solve` pins that.

## State layer

- `state/session.rs` - the scenario being worked on, everything computed from
  it, and the handle to the background solver. The UI never talks to
  `SolverService` directly; it calls `Session::calculate`.
- `state/ui.rs` - view state that is not part of the scenario or its results.
- `state/location.rs` - editable state for one station's position. The
  authoritative position always lives in `scenario::Inputs`; this is the edit
  buffer, and invalid text is reported rather than clamped.

## View layer

See [User Interface](User-Interface.md).

## Harness-only modules

These exist for validation and calibration and are not reachable from the GUI.

| Module | Purpose |
|---|---|
| `net` | The app's only outbound network access. One file, one function, so "what does this program talk to" has a single answer. |
| `spaceweather` | Observed solar indices, fetched rather than assumed. |
| `wsprlive` | Live WSPR spot retrieval from the wspr.live ClickHouse endpoint. |
| `wspr` | WSPR spot ingest and model-versus-measurement scoring. |
| `wspr_report` | Breaks a validation run down into the places the model is weakest. |
| `corpus` | A saved, reproducible WSPR corpus: positives, negatives, per-day sunspot number. |
| `calib` | The unverified anchors, each with the range it is defensible over. |
| `fit` | Two-way fixed-effects calibration against measured WSPR SNRs. |

See [Validation Harnesses](Validation-Harnesses.md) and
[Calibration](Calibration.md).

## Lint policy

The app is deliberately **not** clippy-pedantic, unlike the engine. This is an
egui application crate and pedantic lint churn on UI layout code buys nothing.
The engine keeps its strict lints. Both are held to `-D warnings` in CI for the
default lint set.
