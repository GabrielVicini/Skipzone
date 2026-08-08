<a name="top"></a>

[![Skipzone Banner](https://raw.githubusercontent.com/GabrielVicini/Skipzone/master/assets/banner.png)](https://github.com/GabrielVicini/Skipzone)

# Skipzone

**3D ionospheric HF ray tracing** with Haselgrove equations and full Appleton-Hartree magnetoionic theory.

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![edition](https://img.shields.io/badge/edition-2024-orange)](Cargo.toml)
[![license](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-brightgreen)](Cargo.toml)
[![build](https://github.com/GabrielVicini/Skipzone/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/GabrielVicini/Skipzone/actions/workflows/ci.yml)
[![docs](https://img.shields.io/badge/docs-derivations-1D76DB)](docs/derivations)

Star us on GitHub if this is useful to you.

## Table of Contents
- [Important Message](#important-message)
- [About](#about)
- [Project Layout](#project-layout)
- [Building](#building)
- [Command Line Tools](#command-line-tools)
- [Accuracy Report](#accuracy-report)
- [Supported Platforms](#supported-platforms)
- [Documentation](#documentation)
- [License](#license)

## Important Message

The engine is in a non-production state while I fine-tune it, verify its
accuracy and work out minor input bugs. A full wiki of app functions, releases
and so on is planned. Right now, everything is pre-alpha.

## About

Skipzone traces HF radio rays through a 3D ionosphere, using the actual IGRF
geomagnetic field, the full Appleton-Hartree magnetoionic dispersion relation
(O and X modes), and a day/night-aware Chapman D-region model, solved via the
Haselgrove ray equations in Hamiltonian form.

- **Engine (`src/`):** no dependencies beyond `num-complex` and `thiserror`; no
  system libraries; single-threaded; `unsafe` forbidden. Built to stay portable
  and embeddable. CI builds it on its own, outside the workspace default
  members, so a dependency leak from the app cannot hide.
- **App (`app/`):** an `egui` + `Walkers` desktop map application for
  point-to-point HF path prediction. Useful for amateur radio propagation
  planning, but general enough for any HF ray-tracing use case. It also holds
  the ionosphere model layer, the solver, the link budget and every validation
  harness.
- **Docs (`docs/derivations/`):** every piece of physics derived from first
  principles and cross-checked against known analytic limits, before it was
  implemented.
- **Wiki (`wiki/`):** how the whole thing fits together, module by module.

### Where the code actually is

The engine is about 4,000 lines and the app is about 30,000. That split is
deliberate but it is easy to misread, so to be explicit: `src/` is a ray tracer
and a magnetic field model. The ionosphere itself (Chapman layers, foF2
climatology, sporadic E), the noise model, the link budget, the solver and the
GUI all live in `app/`. Embedding `skipzone` alone gets you a tracer that you
must supply an electron-density model to.

## Project Layout

```
skipzone/
├── Cargo.toml          # engine crate (workspace root)
├── src/                # the engine: field model, magnetoionic theory, ray tracer
│   └── bin/            # hfpredict, the headless engine CLI
├── app/                # skipzone-app: egui + Walkers desktop application
│   ├── Cargo.toml
│   └── src/
│       ├── assets/     # bundled foF2 grid and coastline shapefiles
│       └── bin/        # validation and calibration harnesses
├── docs/derivations/   # the physics, derived before it was implemented
├── wiki/               # project wiki (GitHub wiki source)
├── data/               # IGRF-14 coefficients; staged NeQuick reference data
├── examples/           # nvis_probe, a high-angle diagnostic
├── benches/            # criterion baseline for one traced hop
└── tests/              # analytic and invariant suites
```

The app is a separate workspace member so its `eframe`/`winit`/HTTP dependency
tree never enters the engine crate, which stays buildable with nothing beyond
`std` and two crates.

`corpus/` is not in the repository. It is gitignored working data that the WSPR
and ionosonde harnesses read and write. See
[wiki/Validation-Harnesses.md](wiki/Validation-Harnesses.md) for how to build it.

## Building

```bash
cargo run --release
```

That builds and launches the desktop app, which is the workspace default member.

To build only the engine, with none of the app's dependency tree:

```bash
cargo build --release --package skipzone --lib
```

## Command Line Tools

Ten binaries besides the GUI. None of them is required to use the app; they
exist so that every claim the model makes can be checked. Full descriptions are
in [wiki/Command-Line-Tools.md](wiki/Command-Line-Tools.md).

| Tool | Crate | What it does |
|---|---|---|
| `hfpredict` | engine | Headless first-order path predictor. Engine only, no app code. |
| `wspr_corpus` | app | Fetches a reproducible WSPR corpus once and writes it to disk. |
| `wspr_calibrate` | app | Fits the unverified anchors against that corpus, treating station effects as nuisance parameters. |
| `wspr_validate` | app | Scores the model against a saved list of WSPR spots. |
| `wspr_live_check` | app | Fetches live spots and the observed sunspot number, then scores against them. |
| `iono_check` | app | Scores the ionosphere model against measured ionosonde foF2 and foE. |
| `mode_audit` | app | Explains why the solver admitted a path and what it charged it. |
| `solve_digest` | app | Stable fingerprint of everything `solve()` produces, over a scenario grid. |
| `profile_solve` | app | Counts where the time in one solve actually goes. |
| `gen_fof2_grid` | app | Regenerates the bundled foF2 climatology table. |

Run any of them with:

```bash
cargo run --release -p skipzone-app --bin wspr_validate -- --help
```

## Accuracy Report

Coming soon. A validation report against real-world propagation data will be
published once the project reaches its first tagged release. So far the only
accuracy issues have been with the inputs coming from the app rather than the
physics engine, though that may change.

## Supported Platforms

Windows is the primary and most-tested target. Ubuntu 24.04 is also tested.
macOS and other Linux distributions should work but are unverified.

The Linux build requires glibc 2.39 or newer (Ubuntu 24.04+, Debian 13+,
Fedora 40+). Older distributions such as Ubuntu 22.04 or Debian 12 need a
rebuild from an older base.

There is no browser build. A WebAssembly proof of concept existed and was
removed: it carried its own code paths for having no threads, and keeping them
correct cost more than the target was worth.

`rust-version = "1.85"` is a promise about the engine crate, and CI enforces it
there. The app tracks whatever `eframe` and `walkers` require, which moves.

## Documentation

- [wiki/](wiki/) - how the project is put together, module by module.
- [docs/derivations/](docs/derivations/) - the physics, derived from first principles.
- `cargo doc --package skipzone --open` - engine API documentation.

## License

All files are governed under GPL-3.0, see [LICENSE](LICENSE), except where noted
otherwise. The bundled NeQuick reference data under `data/nequick/` is EUPL v1.2
and carries its own notice; see [data/nequick/README.md](data/nequick/README.md).

For any licensing concerns or questions, email `hello@vicini.io`.
