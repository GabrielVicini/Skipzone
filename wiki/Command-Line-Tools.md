# Command Line Tools

Ten binaries besides the GUI. None is required to use the application. They
exist so that every claim the model makes can be checked, and so that the
checking is reproducible rather than something someone did once.

One lives in the engine crate; the rest are app harnesses. All of them drive the
same library code the GUI does, which is why `app/` is a library with a thin
`main.rs` on top rather than a plain binary.

## Engine

### `hfpredict`

A first-order HF path predictor built only from engine code. Given a
transmitter, a receiver, a frequency and a time of day, it assumes a single
Chapman F2 layer whose foF2 and hmF2 come from a coarse midlatitude climatology
table, then uses the validated homing to decide whether a reflected ray path
exists and reports its geometry.

```bash
cargo run --release --bin hfpredict -- --help
```

It states its own honesty boundaries in full, and they are worth repeating
because they are narrower than the GUI's:

- The foF2/hmF2 table is order-of-magnitude climatology. It is consistent with
  published midlatitude ranges but is not a measured or forecast value for the
  specific path and time. The assumed numbers are printed so they can be
  checked, and `--fof2` / `--hmf2` override them with real ionosonde or
  prediction data. That is the defensible path for real work.
- A single Chapman F2 layer only. No E or F1 layers, no horizontal gradients, no
  sporadic E, no tilts.
- Field-free O-mode by default. O and X are bit-identical without a field, so
  there is no magnetoionic splitting here.
- "Connects" means a ray reflects and reaches the receiver **geometrically**.
  Absorption and signal strength are not modelled, so this is an MUF-style
  reachability check, not a link budget.
- Multi-hop uses the equal-hop assumption. Because the assumed ionosphere
  depends only on height, every hop is geometrically identical, so an N-hop path
  exists exactly when a single hop of 1/N the ground distance reflects.

## Corpus and calibration

### `wspr_corpus`

Fetches a reproducible WSPR corpus once and writes it to disk, so that every
later calibration run scores the same spots.

```bash
cargo run --release -p skipzone-app --bin wspr_corpus -- \
    --from 2026-07-02 --days 7 --out corpus/fit.tsv --neg corpus/fit_neg.tsv
```

Writes positives (spots that were decoded) and negatives (receivers that were
listening on that band at that time and did not decode the transmitter). The
negatives are what make a false-positive rate computable at all.

### `wspr_calibrate`

The calibration driver. Fits the unverified anchors against the saved corpus
with transmitter and receiver effects treated as nuisance parameters.

```bash
cargo run --release -p skipzone-app --bin wspr_calibrate -- \
    --fit corpus/fit.tsv --holdout corpus/holdout.tsv --negatives corpus/fit_neg.tsv
```

It is the largest tool in the project and is split into modules under
`app/src/bin/wspr_calibrate/`:

| Module | Contents |
|---|---|
| `main.rs` | Entry point only |
| `args.rs` | The command line surface and its documented defaults |
| `solving.rs` | Reading the corpus and solving it into cached predictions |
| `driver.rs` | The fit itself, plus the anchor scans |
| `report.rs` | Everything printed; no fitting happens there |
| `negatives.rs` | Negatives, decode probability, false-positive rate |
| `jackknife.rs` | Day-level refits |

Read [Calibration](Calibration.md) before believing any number it prints.

Run `--help` for the full flag list. The defaults point at `corpus/`, and a
missing file is a hard error, not a silent fallback.

## Validation

### `wspr_validate`

Scores the model against a saved list of WSPR spots.

```bash
cargo run --release -p skipzone-app --bin wspr_validate -- spots.tsv [--ssn 70] [--quiet]
```

### `wspr_live_check`

Fetches real WSPR spots and the observed sunspot number, scores the model
against them, and reports where it falls behind.

```bash
cargo run --release -p skipzone-app --bin wspr_live_check
cargo run --release -p skipzone-app --bin wspr_live_check -- --minutes 20 --limit 300
cargo run --release -p skipzone-app --bin wspr_live_check -- --band 14 --at "2026-07-24 03:22"
cargo run --release -p skipzone-app --bin wspr_live_check -- --file spots.tsv --ssn 119
```

This is the only tool that reaches the network by default. It goes through
`app/src/net.rs`, which is one file with one function so that "what does this
program talk to" has a single answer.

### `iono_check`

Scores the ionosphere model against measured ionosonde foF2 and foE. Unlike
WSPR, an ionosonde has no antenna, no noise floor and no station effect for
error to hide in, so the comparison is a straight measurement of model error in
MHz with nothing absorbed and nothing fitted.

```bash
cargo run --release -p skipzone-app --bin iono_check
cargo run --release -p skipzone-app --bin iono_check -- path/to/obs.tsv --ssn=corpus/ssn_daily.tsv --fit=corpus/fit.tsv
```

Inputs and their defaults:

| Flag | Default | What it is |
|---|---|---|
| first positional | `corpus/ionosonde.tsv` | The GIRO observations |
| `--ssn=PATH` | `corpus/ssn_daily.tsv` | SILSO daily sunspot number |
| `--fit=PATH` | `corpus/fit.tsv` | WSPR corpus, used only for its median SSN |
| `--propose` | off | Print suggested anchor changes rather than only the error |

If either auxiliary file is missing the run continues but says so on stderr,
because falling back to a median or to a mid-cycle SSN of 90 changes what the
run measures.

It does **not** fit anything. It reports error. Anything that looks like a
correction belongs in a separate deliberate change with its own test.

## Diagnostics

### `mode_audit`

Answers "why did the solver admit this path, and what did it charge it". Prints
the D-region decomposition and the night-floor leverage for a chosen scenario.

```bash
cargo run --release -p skipzone-app --bin mode_audit
```

### `solve_digest`

A stable fingerprint of everything `solve()` produces, over a scenario grid.
This is the tool that makes a performance change safe: capture a digest before,
make the change, capture one after, and diff.

```bash
cargo run --release -p skipzone-app --bin solve_digest > before.txt
# make the change
cargo run --release -p skipzone-app --bin solve_digest > after.txt
diff before.txt after.txt
```

An optimisation that skips rays which cannot reach the target is only safe if it
changes which rays are *traced* and not which are *found*. An empty diff is that
proof.

### `profile_solve`

Counts where the time in one solve actually goes: traces, integrator steps,
density, field and collision evaluations, and the raw cost of one evaluation of
each model.

```bash
cargo run --release -p skipzone-app --bin profile_solve
```

The instrumentation is entirely app-side. The engine models are wrapped in
counting decorators, so nothing in the engine is modified to be measurable.

## Generators

### `gen_fof2_grid`

Regenerates `app/src/assets/fof2_grid.tsv`, the bundled foF2 climatology table.

```bash
cargo run -p skipzone-app --bin gen_fof2_grid
```

The table is data, bundled and checked in, so nothing is computed or fetched at
runtime. The generator exists so the data's provenance is a checked-in program
rather than a pasted blob. Run it and the file is reproduced exactly, and the
test `fof2::tests::bundled_grid_matches_its_generator` fails if the checked-in
file and the generator ever disagree.
