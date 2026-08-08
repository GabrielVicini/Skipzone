# Architecture

## Two crates, one workspace

```
skipzone            (root Cargo.toml, src/)     the engine
skipzone-app        (app/Cargo.toml, app/src/)  everything else
```

The root `Cargo.toml` declares `members = ["app"]` and
`default-members = ["app"]`, so a bare `cargo run` builds and launches the
desktop application while `cargo build -p skipzone --lib` builds the engine
alone.

The split exists for exactly one reason, stated in the root manifest: the app's
`eframe`/`winit`/HTTP dependency tree must never enter the engine crate. The
engine has to stay buildable with nothing but `std` and two crates, on any
target, with no system libraries. A workspace build alone would not prove that,
because Cargo would happily let an engine module reach for something the app
pulled in. CI therefore has a dedicated step that builds the engine on its own,
outside the default members.

## Size, and what that means

| | Lines | What is in it |
|---|---|---|
| `src/` | ~4,200 | Ray tracer, magnetic field, dispersion relation, integrator, homing |
| `app/src/` | ~30,000 | Ionosphere, noise, antennas, link budget, solver, GUI, harnesses |
| `tests/` | ~1,300 | Analytic and invariant suites against the engine |

The engine is 12 percent of the codebase, and it is easy to read the naming and
assume otherwise. Be explicit about it: `src/` is a ray tracer, not a
propagation predictor. It has no ionosphere. Everything that makes Skipzone
answer "will this path work at 14 MHz at 0300Z" lives in `app/`.

This is a deliberate boundary, not an accident of growth. Coupling electron
production to solar geometry and to a climatology map is a scenario concern, not
a reusable engine primitive. An embedder who wants a different ionosphere gets
the tracer without having to tear one out.

## The app's internal layers

`app/src/lib.rs` declares a five-layer stack. Each layer may depend only on the
ones above it.

| Layer | Modules | May know about |
|---|---|---|
| conversion | `clock`, `grid`, `solar`, `coastline` | nothing else |
| model | `chapman`, `fof2`, `sporadic_e`, `scenario`, `noise` | the engine |
| computation | `compute`, `solve`, `sweep`, `coverage` | the model |
| state | `state` | the computation |
| view | `ui`, `app` | the state |

Two invariants hold and are worth checking whenever this area is touched:

1. **Nothing in `ui` computes a physical quantity.** There is no `use skipzone::`
   anywhere under `app/src/ui/` or `app/src/state/`. The handful of float
   operations in the UI are chart scaling and map projection.
2. **Nothing below `state` mentions egui.** The computation layer is the
   interesting case, because the background solver genuinely needs to tell the
   window to redraw. It does that through a `sweep::Wake` callback
   (`Arc<dyn Fn() + Send + Sync>`), not an `egui::Context`. The view layer
   supplies one that calls `request_repaint`; a headless caller supplies
   `sweep::no_wake()`.

That second point is why the harnesses in `app/src/bin/` can drive exactly the
same solver and model code the GUI does, rather than a parallel copy of it. It
is also why `app/src/main.rs` is thin: the app is a library with a binary on
top, not a binary with some helpers.

## Threading

The engine is single-threaded and has no thread-pool dependency. All parallelism
lives in `app/src/compute.rs`, which owns a reusable `ComputePool` built on a
private `rayon` pool, isolated from rayon's global pool so it never fights other
users of it.

The parallelism seam is always *outside* the ODE loop. A batch is a set of
whole independent solves, never individual integrator steps. Two properties fall
out of that and both are load bearing:

- Results are **bit-identical** between sequential and parallel execution, which
  is what makes the substitution safe and what the equivalence test in `sweep`
  checks.
- Every map returns a `Timing` with per-item and total wall clock, so a claimed
  speedup can be measured rather than assumed.

The two job kinds are sized differently on purpose. The frequency sweep holds
two cores back, because it runs while the operator is still panning the map and
the tile fetcher wants CPU. The coverage grid takes every core, because it is a
deliberate one-off run and the operator is doing nothing but waiting for it.
Both are overridable without a rebuild:

```
SKIPZONE_COMPUTE=sequential     switch the parallel layer off entirely
SKIPZONE_COMPUTE_THREADS=N      cap worker threads at N
```

## Data flow of one prediction

```
UI inputs (Inputs)
      |
      v
scenario::resolve      -> Assumptions   (every assumed value, recorded for display)
scenario::build_models -> Models        (Chapman/foF2/sporadic-E layers, IGRF, collisions)
      |
      v
solve::solve
      |-- enumerate (hop count, elevation bracket) candidates
      |-- ComputePool: home and trace each candidate            <- engine
      |-- link_budget: free-space + Fresnel ground reflection
      |-- noise: noise floor, received power, SNR, margin
      v
SolveOutcome  ->  state::Session  ->  ui panels
```

Nothing in that chain implements physics twice. `solve` calls the engine's
public API only; `scenario` builds model objects and records what it assumed;
the UI renders what it is handed.

## The removed web target

A WebAssembly proof of concept existed until it was removed. It is documented
here so the decision is not relitigated from scratch.

It worked, but it carried its own code paths throughout the computation layer,
all of them consequences of `wasm32-unknown-unknown` having no threads: the
solver service ran jobs inline on the browser main thread instead of on a
worker, the compute pool was forced sequential, `Instant` and `SystemTime` came
from a shim crate rather than `std`, and the crate emitted a `cdylib` nothing
native used. On top of that, a 12.7 MB build artifact and its generated JS glue
were committed to the repository, went stale the moment `app/src/` changed, and
nothing regenerated or verified them.

The cost was a permanent second execution model in the layer where correctness
is hardest to reason about, in exchange for a target that was explicitly not the
focus. It was removed rather than maintained. If a browser build is ever wanted
again, the honest version starts from cross-origin isolation and real worker
threads, not from cfg-gating the sequential path back in.
