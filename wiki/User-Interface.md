# User Interface

`app/src/ui/`. Everything egui, arranged by where it appears on screen.

Two rules hold throughout and both are checkable:

1. **Nothing here computes a physical quantity.** There is no `use skipzone::`
   in this tree. The handful of float operations are chart scaling and map
   projection.
2. **Nothing here mutates state where it is drawn.** Every widget returns an
   `Action`; the action is applied once, centrally, after the frame is drawn.

## The `Action` indirection

Menu items, overlay buttons and dialog buttons all produce an `ui::Action`
rather than mutating state at the point of drawing. That keeps a command's
meaning in one place. "Calculate" means "dispatch a solve and show the trace"
whether it was invoked from the menu, the overlay or a dialog, and the widgets
themselves stay free of side effects.

`shell::draw` collects the action from each region with `.or(...)`, so the first
region that produced one wins, and applies it at the end.

## Layout

`shell.rs` is the one function that says where everything goes, and reading it
top to bottom **is** the layout:

```
header            solid top bar (menus, status, TX/RX entry rows)
  trace panel     docks right, taking width off the map before it lays out
    map           fills the remaining window
      overlays    float over the map, anchored to screen corners
        modals    dialogs on top of everything
```

The trace panel docking before the map is laid out is deliberate: opening it
pushes the map and the floating controls aside rather than covering them.

## Regions

### `header/`

The only opaque chrome in the layout. Menus and status on the first row, the TX
and RX entry rows beneath.

- `menus.rs` - the menu bar.
- `stations.rs` - TX and RX position entry.
- `status.rs` - run state, progress, thread count.

### `map/`

- `view.rs` - `MapView`, the walkers map widget, zoom and framing.
- `plugins.rs` - the drawing overlays that sit on the tiles: the great-circle
  path, the traced ray, hop markers, the coverage raster.

### `overlays/`

Controls that float directly over the map, anchored to screen corners.

- `controls.rs` - the primary run controls.
- `map_tools.rs` - placement mode, framing.
- `time_date.rs` - the scenario clock.

### `panels/`

The trace readouts, one file per panel. Deliberately dense and complete rather
than tidy: this is an instrument panel, not a product screen.

| Panel | Shows |
|---|---|
| `verdict.rs` | The headline result, per-mode breakdown, and the no-path explanation |
| `solution.rs` | The picked solution's full geometry and link budget |
| `profile.rs` | The sampled electron-density profile along the path |
| `assumptions.rs` | Every assumed value the scenario resolved to |
| `diagnostics.rs` | Errors and the near-miss report |
| `reference.rs` | Units, conventions and what the numbers mean |

The assumptions panel is load bearing. Every unverified anchor and every backend
choice that was made for the operator is displayed there, including when a
fallback took over, so no number in the interface is unattributed.

### `modals/`

Dialog windows. Each is one file and one entry point; `chrome::dialog` gives
them all the same draggable, resizable window with a scrolling body.

`about.rs`, `antennas.rs`, `best_freq.rs`, `coverage.rs`, `settings.rs`.

### `widgets/`

Reusable pieces, none of which know anything about the scenario they are
displaying. Every one takes plain data or a `&mut` to state owned elsewhere.

`band.rs`, `calendar.rs`, `chart.rs`, `fields.rs`, `layout.rs`, `menu.rs`.

### `theme.rs`

The one place colours, spacing and container chrome are defined, plus DPI
scaling.

## State ownership

The UI owns no scenario state. `app.rs` holds three pieces and hands them to the
shell each frame:

- `Session` - the scenario, everything computed from it, and the handle to the
  background solver.
- `UiState` - view state that is not part of the scenario or its results.
- `MapView` - map tile and camera resources.

The UI never talks to `SolverService` directly. It calls `Session::calculate`,
which dispatches the job, and `Session` drains results each frame.

## How a background result reaches the screen

The solver worker thread has no `egui::Context`. It holds a `sweep::Wake`
callback, and the view layer constructs it in `Session::new` as
`Arc::new(move || ctx.request_repaint())`. When the worker has something new it
calls `wake()`, egui schedules a frame, and `Session::drain` picks up the
current-epoch messages on the next one.

That indirection is what keeps the computation layer free of egui, and it is
what lets the harnesses in `app/src/bin/` drive the same solver headlessly with
`sweep::no_wake()`.
