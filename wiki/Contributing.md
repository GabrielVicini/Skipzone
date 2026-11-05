# Contributing

To send a change, fork the repository, make the change, and run the checks in
[Before you push](#before-you-push). If you only read one section, read
[the regression lock](#the-regression-lock). For how the crates fit together,
see [Architecture](Architecture).

<!-- OWNER: what kinds of contributions you want right now (bug reports, physics review, pull requests), and whether to open an issue before a large pull request. -->

## Building

You need a stable Rust toolchain, edition 2024. The app needs Rust 1.95 or
newer, because that is eframe 0.36's minimum. CI pins 1.97.

On Linux, eframe and winit need the X11 and Wayland development headers. CI
installs these on Ubuntu 24.04:

```bash
sudo apt-get install -y --no-install-recommends \
  libxkbcommon-dev libwayland-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libx11-dev libgl1-mesa-dev
```

Then, from the repository root:

```bash
cargo run --release                      # build and launch the desktop app
cargo build --release -p skipzone --lib  # the engine alone, with none of the app's dependencies
```

## Testing

```bash
cargo test --workspace --all-targets
```

The tests need no network access and no `corpus/` directory. Many of them run
real solves.

The engine also has Criterion benchmarks of one traced hop and a 64-ray fan:

```bash
cargo bench -p skipzone
```

## Before you push

These are the checks CI runs, in the form you run them locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build -p skipzone --lib
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
python3 scripts/check-boundaries.py
cargo deny --workspace --all-features check     # needs: cargo install cargo-deny
```

If you changed `docs/`, the README or a `docs/` citation in code, also run the docs link check:

```bash
python3 scripts/check-docs.py
```

## The regression lock

Every calibrated number in Skipzone was fitted against solver output. A
refactor that moves any ray invalidates that fit. So capture a digest before
the change and another after it. Each run takes more than ten minutes on a
four-core machine.

```bash
cargo run --release -p skipzone-lab --bin solve_digest > before.txt
# make the change
cargo run --release -p skipzone-lab --bin solve_digest > after.txt
diff before.txt after.txt
```

There are two cases:

- **A refactor or optimisation must leave the digest byte-identical.**
- A change meant to alter the physics produces a diff. Explain every changed
  line in the pull request.

`profile_solve` shows where the time in a solve goes before you optimise it.

## Commit style

- Imperative mood: "add", "fix", "move".
- A scope prefix naming what changed: `engine:`, `iono:`, `predict:`, `lab:`,
  `app:`, `ci:`, `docs:`, `wiki:` or `chore:`.
- A short body explaining why.

A real one from the history:

```text
ci: ban GUI crates in lab and the network crate in the app

lab is the only crate with network access and has no GUI; the app is
the only crate with a GUI. The boundary check now enforces both instead
of leaving them to cargo tree by hand.
```

If your change makes a wiki page wrong, fix the page in the same commit.

## Conventions

### Physics

- **Derive first.** Write the derivation before the code, and cite its file in
  a code comment. A pull request that adds an equation needs its derivation.
  The derivations are indexed in
  [`docs/README.md`](https://github.com/GabrielVicini/Skipzone/blob/master/docs/README.md).
- **Coordinates and signs.** Time dependence is `exp(-i omega t)`, so a lossy
  medium has `Im(n) > 0`. Budden and Davies use the opposite sign, so check this
  first when porting a textbook formula. Coordinates are geocentric spherical
  `(r, theta, phi)` with the local basis (up, south, east). The rest is in
  [`conventions.md`](https://github.com/GabrielVicini/Skipzone/blob/master/docs/derivations/conventions.md).
- **SI units and `f64` throughout.** Newtypes guard the public boundaries.
- **Analytic derivatives, finite differences as the oracle.** Every model that
  reports a gradient has a test named `*_matches_fd` or
  `*_matches_finite_differences`.
- **Find an independent check.** A new field model is tested against the dipole
  closed form and the divergence and curl invariants. An integrator change is
  tested against the convergence-order test. If your change has no independent
  check, say so in the pull request.

### Code

- **No `unsafe`.** The engine sets `unsafe_code = "forbid"`. Changing that needs
  a measured benchmark and an isolated, documented exception.
- **Errors.** Return an error only when the computation can no longer be
  trusted. A ray that lands or escapes is a normal result. A layer whose trace
  failed shows as failed in the UI, so a user can tell it from "nothing
  arrives".
- **Fallbacks.** When the code substitutes a guessed value, it records the
  substitution, so the run can report it.
- **Small samples.** A report cut with fewer than 30 samples prints no error
  figure. A median of a handful of points is mostly their own noise.
- **Document what a number is.** Each unverified anchor has a docstring
  giving its unit, its plausible range and how far it can be trusted.
- **Comments explain why.** Where a decision was measured, put the measurement
  in the comment.
- **Widgets.** In the app, widgets return a `ui::Action` instead of changing
  state directly.
- **Dependencies.** Each dependency in a `Cargo.toml` has a comment saying what
  it is for. Check the crate's allowed list in
  [Architecture](Architecture#the-dependency-rule) before adding one.

### Changing an anchor

The anchors are the unverified values in `crates/skipzone-predict/src/calib.rs`.
Move one only if the calibration can identify it. `wspr_calibrate` prints an
identification table at the end of its report saying which ones it can. If a
fit pushes an anchor to its bound, the error is probably somewhere else in the
model.
`iono_check --propose` suggests changes, and applying one is a separate commit
with its own test.

### Lints and rustdoc

The engine runs `clippy::pedantic` at warn level. `crates/skipzone/Cargo.toml`
allows five lints, each with a comment: physics notation such as `Y_L` follows
the derivations, and exact float comparisons back bit-identity invariants. The
other crates use the default lint set. CI denies all warnings.

Two rustdoc traps catch people:

- A unit in brackets, such as `r [m]`, parses as an intra-doc link to `m`. Wrap
  it in backticks or a ` ```text ` fence.
- A wrapped doc line that starts with `- ` reads as a list item, and clippy
  then wants the continuation indented.

Formatting is `rustfmt` defaults. There is no `rustfmt.toml`.

## Wiki links

The wiki lives in `wiki/`. Between pages, link with the bare page title and no
`.md`, for example `[Architecture](Architecture#the-dependency-rule)`. A link
ending in `.md` serves the raw markdown on the GitHub wiki.

## CI

[`.github/workflows/ci.yml`](https://github.com/GabrielVicini/Skipzone/blob/master/.github/workflows/ci.yml)
runs on every push to `master` and every pull request.

| Job | What it checks |
|---|---|
| `fmt` | `cargo fmt --all -- --check` |
| `clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | `cargo test --workspace --all-targets` on Ubuntu 24.04, Windows and macOS, then `cargo build -p skipzone --lib` to prove the engine builds alone |
| `msrv` | `cargo check` of `skipzone`, `skipzone-iono` and `skipzone-predict`, each on the `rust-version` its `Cargo.toml` declares |
| `docs` | `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings` |
| `boundaries` | `scripts/check-boundaries.py` (the dependency rule in [Architecture](Architecture#the-dependency-rule)) and `scripts/check-docs.py` (every derivation is cited and every citation resolves) |
| `deny` | `cargo deny` advisories, then bans, licences and sources, configured in `deny.toml` |

Two more workflows run on their own triggers. `wiki.yml` mirrors `wiki/` to
the GitHub wiki on every push to `master` that touches it. It needs a
`WIKI_TOKEN` secret: a fine-grained token with Contents read and write on the
repository. Without the secret it skips with a notice. `release.yml`
builds release archives from version tags, described [below](#building-a-release).

Dependabot opens update pull requests monthly for Cargo dependencies and for
GitHub Actions. The advisory ignores in `deny.toml` are GUI-stack crates with
no fixed release reachable from eframe 0.36. Revisit them when Dependabot
offers a newer eframe.

## Bumping the pinned toolchain

CI pins the toolchain so a new stable release cannot add lints without warning.
The pin is `RUST_TOOLCHAIN` in the `env` block of both `ci.yml` and
`release.yml`. To move it, for example to 1.98:

```bash
rustup toolchain install 1.98
cargo +1.98 clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo +1.98 doc --workspace --no-deps
```

Fix whatever the new lints find, change `RUST_TOOLCHAIN` in both files, and
commit everything as one `ci:` commit.

The `msrv` job is separate. It reads each crate's `rust-version` from its
`Cargo.toml`, so raising a crate's MSRV needs no workflow change. The README
badge shows the engine's MSRV, so update it too.

## Data and assets

Every data file the app needs is compiled in with `include_str!` or
`include_bytes!`.

| File | What it is | Licence |
|---|---|---|
| `crates/skipzone/data/igrf14coeffs.txt` | IGRF-14 geomagnetic field coefficients, valid 1900 to 2030 | Published by IAGA, freely available |
| `crates/skipzone-iono/data/nequick/` | Monthly CCIR foF2 and M(3000)F2 maps and the modified-dip grid, from the Galileo NeQuick G reference data | EUPL-1.2, (C) European Union 2019 |
| `crates/skipzone-iono/src/assets/fof2_grid.tsv` | A foF2 climatology grid generated by `gen_fof2_grid`. Its values are Skipzone's own construction and are not CCIR, URSI or IRI data | MIT |
| `crates/skipzone-predict/src/assets/ne_50m_land.shp` | Natural Earth 1:50m land polygons, for the land and sea mask | Public domain |
| `crates/skipzone-predict/src/assets/ne_50m_lakes.shp` | Natural Earth 1:50m lake polygons | Public domain |

The NeQuick directory has its own `README.md` and `_SOURCE_AND_LICENSE.txt`.
The EUPL requires that notice to travel with the data, so keep the directory
whole. If you ever remove the data, remove the entire directory in one commit.
The release archives ship the notice as `NEQUICK-DATA-LICENSE.txt`.

`fof2_grid.tsv` is the one generated file in the repository. If you change the
climatology, regenerate it and commit the file with the change:

```bash
cargo run -p skipzone-lab --bin gen_fof2_grid
```

The test `fof2::tests::bundled_grid_matches_its_generator` fails if the file
and the generator disagree.

Only the `.shp` geometry files from Natural Earth are shipped, because the
coastline code reads polygons and no attributes.

These are gitignored and stay out of commits: `target/`, `corpus/`,
`release/`, `graphify-out/` and `.idea/`.

## Building a release

Push a tag that starts with `v`:

```bash
git tag v0.1.0
git push origin v0.1.0
```

`release.yml` then builds `skipzone-app-bin` with `--locked` for four targets:
Windows x86_64, Linux x86_64, macOS arm64 and macOS x86_64. Each archive holds
the binary, `LICENSE`, `README.md` and `NEQUICK-DATA-LICENSE.txt`. The last job
publishes a GitHub release with the archives, a `SHA256SUMS.txt` and generated
release notes.

The workflow does not read the version in any `Cargo.toml`, so keep the tag
and the crate versions in step yourself.

<!-- OWNER: your release policy, if you have one (version numbering, when to tag, whether a release needs a CHANGELOG entry). -->
