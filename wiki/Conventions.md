# Conventions

The rules this codebase is written to. Most of them exist because breaking one
produced a wrong answer that looked right.

## Physics and coordinates

Fixed crate-wide, derived in `docs/derivations/conventions.md`, assumed by every
module.

- **Time dependence `exp(-i omega t)`.** A lossy medium has `Im(n) > 0` and the
  field attenuates as `exp(-(omega/c) Im(n) s)`. Budden and Davies use
  `exp(+i omega t)`; their formulas are the complex conjugates of these. Check
  this first when porting an equation out of a textbook.
- **Geocentric spherical `(r, theta, phi)`:** radius from Earth's centre,
  colatitude from the geographic north pole, east longitude. Local right-handed
  orthonormal basis `(r_hat, theta_hat, phi_hat)` is (up, south, east), so north
  is `-theta_hat`.
- **Field Jacobians are plain coordinate partials**, not covariant derivatives.
  The basis-rotation terms are handled once, exactly, in the ray equations. A
  field model that pre-applies them will be wrong twice.
- **SI units and `f64` throughout.** Newtypes guard public boundaries; the
  integrator state is raw `f64` in SI, converted at the edges.
- **`sigma` is not arc length.** The ray parameter has units of metres but arc
  length is carried separately as state element 9.

## Code

### Derive before you implement

Every equation in the engine is derived from first principles in
`docs/derivations/` before it is written, and the code comment cites the file
and section. This is the project's actual method, not a documentation
aspiration. Where the derivation and the code disagree, the derivation wins
until someone argues otherwise in writing. A derivation has already caught one
specification error and won.

### Analytic derivatives, finite differences as oracle

Never the reverse. Every model that reports a gradient has a test named
`*_matches_finite_differences` or `*_matches_fd`. Shipping finite differences
because the analytic form was hard is not an option here.

### No `unsafe`

`unsafe_code = "forbid"` at the engine crate level. The stated bar for changing
that is a measured benchmark justifying an isolated, documented exception.

### Errors are for numerical failure only

A ray that lands or escapes the domain is a physical **outcome**, not an error.
Errors are reserved for cases where the computation itself can no longer be
trusted, such as the adaptive controller driving the step below its minimum.

The same distinction runs through `LayerStatus`: `Failed` is a numerical
outcome, never to be rendered as a physical one. "The model could not answer" is
a different statement from "nothing arrives", and conflating them is how a bug
gets shown to an operator as a propagation result.

### No silent fallbacks

If a guessed value gets used, the run says so. Three examples of the pattern:

- `iono_check` warns on stderr when it cannot read the daily SSN series or the
  corpus, and names the flag that would fix it.
- `Assumptions::fof2_source` records which foF2 backend ran, verbatim, including
  when the grid failed to load and the constant took over.
- `Bounded::clamped` returns whether it clamped, and a caller that discards the
  flag has thrown away the finding.

### Distinguish "no data" from "zero"

A cut that found nothing reports "no error", not an error of zero. Cells thinner
than `MIN_QUOTABLE` (30 samples) print no error at all, because a median from a
handful of points is that handful's own noise.

### Say what a number is not

Every unverified anchor carries a docstring saying what it is, its unit, and how
far it can be trusted. Modules whose values are the author's own construction
rather than a citable table say so in capitals in the module documentation, and
the UI surfaces it on every run. `noise.rs`, `fof2.rs` and `sporadic_e.rs` are
the three to read as examples.

### The parallelism seam is outside the ODE loop

Always. A batch is a set of whole independent solves, never individual
integrator steps. Sequential and parallel results are bit-identical and that
property is tested, not assumed.

### Layers depend downward only

The app's five-layer stack is in
[Architecture](Architecture.md#the-apps-internal-layers). Two invariants:
nothing in `ui` computes a physical quantity, and nothing below `state` mentions
egui. Both are greppable, so check them after touching that boundary.

### Widgets return actions, they do not mutate

Menu items, overlay buttons and dialog buttons produce a `ui::Action`. The
action is applied once, centrally, after the frame is drawn.

## Naming and style

- `rustfmt` defaults, enforced in CI. No `rustfmt.toml`.
- Physics notation follows the derivations, which is why `similar_names` and
  `many_single_char_names` are allowed in the engine. `Y_L` is `Y_L` because
  that is what the derivation calls it.
- Comments explain **why**, not what. The codebase has a high comment density
  and almost all of it is rationale, trade-off or a measured fact. Match that
  when adding to it.
- Where a decision was measured, record the measurement in the comment. "Es
  spots were 41 percent of the solved spots at +21 dB" is worth more than "Es
  spots skew the fit."

## Documentation comments

- Wrap unit brackets in backticks or a ` ```text ` fence. `r [m]` in a doc
  comment is parsed as an intra-doc link to an item named `m`, and CI builds
  rustdoc with `-D warnings`.
- Do not start a wrapped line with `- `. Clippy reads it as a list item and
  demands the continuation be indented.

## What gets committed

- Not build output. Not `corpus/`. Not editor configuration.
- Generated data only when a test proves it still matches its generator. There
  is exactly one such case, `fof2_grid.tsv`.
- See [Data and Assets](Data-and-Assets.md#what-is-deliberately-not-committed).
