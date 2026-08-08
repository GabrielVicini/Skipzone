# Building and CI

## Requirements

- Rust 1.85 or newer, edition 2024.
- On Linux, the X11 and Wayland development headers that `eframe`/`winit` need.
  The engine crate needs none of them, which is the point of the workspace
  split.

```bash
sudo apt-get install -y \
  libxkbcommon-dev libwayland-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libx11-dev libgl1-mesa-dev
```

## Building

```bash
cargo run --release
```

That builds and launches the desktop app, which is the workspace default member.

```bash
cargo build --release --package skipzone --lib
```

That builds the engine alone, with none of the app's dependency tree.

```bash
cargo run --release -p skipzone-app --bin wspr_validate -- --help
```

That runs one of the harnesses. See [Command Line Tools](Command-Line-Tools.md).

## Testing

```bash
cargo test --workspace --all-targets
```

241 tests. The app's suite dominates the runtime at about 105 seconds, because
many of its tests run real solves rather than mocking them. That is deliberate:
a test that mocks the solver tests the mock.

No test touches the network, and no test needs `corpus/`. Everything fetched is
in the harnesses, which are not on the test path.

## Linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```

Both are clean and CI runs them with `RUSTFLAGS=-D warnings`.

### Lint policy

The **engine** is `clippy::pedantic` at warn level, with four documented
exceptions in the root `Cargo.toml`, each of which exists for a stated reason
rather than to silence noise:

| Allowed | Why |
|---|---|
| `similar_names` | Physics notation (`Y_L`, `Y_T`, `k_r`) follows the derivations. Renaming to satisfy the linter would break the correspondence between code and derivation |
| `many_single_char_names` | Same reason |
| `module_name_repetitions` | `mag::Dipole` reads better than an artificially shortened name |
| `float_cmp` | Exact float comparisons are load bearing here: bit-identity invariants (zero field implies `O == X`) and exact-zero short circuits (`y == 0.0`, `z == 0.0`) that the validation suite requires. Every site is commented |
| `doc_markdown` | The prose is formula-heavy (`O` mode, Appleton-Hartree, `X = wp^2/w^2`). Policing backticks on physics notation produces noise, not clarity |

`unsafe_code` is **forbidden**, not denied, at the engine crate level. The stated
bar for changing that is a measured benchmark justifying an isolated, documented
exception.

The **app** is deliberately not pedantic. It is an egui application crate, and
pedantic lint churn on UI layout code buys nothing. It is still held to the
default lint set with `-D warnings`.

## Documentation

```bash
cargo doc --package skipzone --no-deps --open
```

CI builds this with `RUSTDOCFLAGS=-D warnings`, so a broken intra-doc link fails
the build. That matters here because the derivation cross-references are how the
physics stays connected to the code, and they rot silently otherwise.

Watch for one specific trap: unit brackets in doc comments. `r [m]` is parsed as
an intra-doc link to an item named `m`. Either wrap the whole expression in
backticks or put the block in a ` ```text ` fence.

## Benchmarks

```bash
cargo bench
```

Criterion, two benchmarks in `benches/trace.rs`, both in the production shape: a
full magnetoionic ray through IGRF-14 plus a Chapman layer plus exponential
collisions, landing after one hop.

- `single_hop_igrf_chapman` - one traced ray.
- `fan_64_rays_serial` - 64 launches traced one after another. The engine is
  single-threaded, so this measures the serial cost that the app's compute layer
  divides up, not a parallel speedup.

## CI

`.github/workflows/ci.yml`. Three jobs.

### `check`

Matrix over `windows-latest` and `ubuntu-24.04`, on stable:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets`
3. `cargo test --workspace --all-targets`
4. `cargo build --package skipzone --lib`

Step 4 is the one worth explaining. The engine must stay buildable with nothing
but `std` and its two crates.io dependencies, and **building it alone, outside
the workspace default members, is the only thing that proves that**. A workspace
build would pull the app's tree in and hide a leak.

### `msrv`

`cargo check --package skipzone --lib` on Rust 1.85.

`rust-version = "1.85"` in the root `Cargo.toml` is a promise about the **engine**
crate, which is the portable, embeddable one. The app is not held to it: eframe,
walkers and their trees move their own minimums, and chasing those would mean
either pinning the GUI or weakening the claim that actually matters.

### `docs`

`cargo doc --package skipzone --no-deps` with `RUSTDOCFLAGS=-D warnings`.

## Environment variables

Read at runtime by the compute layer, so the parallelism can be capped or
switched off without a rebuild. Useful for A/B timing or for debugging a
suspected parallelism bug.

| Variable | Effect |
|---|---|
| `SKIPZONE_COMPUTE=sequential` | Switch the parallel layer off entirely |
| `SKIPZONE_COMPUTE_THREADS=N` | Cap worker threads at N |

Because sequential and parallel results are bit-identical, setting the first one
is a valid way to check whether a suspicious result is a parallelism artefact.
If the answer changes, that is a bug in the compute layer, not in the physics.

## Platforms

Windows is the primary and most-tested target. Ubuntu 24.04 is also tested and
built in CI. macOS and other Linux distributions should work but are unverified.

The Linux build requires glibc 2.39 or newer (Ubuntu 24.04+, Debian 13+,
Fedora 40+). Older distributions such as Ubuntu 22.04 or Debian 12 need a
rebuild from an older base.

There is no browser build. See
[Architecture](Architecture.md#the-removed-web-target).
