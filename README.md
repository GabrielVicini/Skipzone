<a name="top"></a>

[![Skipzone Banner](https://raw.githubusercontent.com/GabrielVicini/Skipzone/master/assets/banner.png)](https://github.com/GabrielVicini/Skipzone)

# Skipzone

**3D ionospheric HF ray tracing** with Haselgrove equations and full Appleton-Hartree magnetoionic theory.

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![edition](https://img.shields.io/badge/edition-2024-orange)](crates/skipzone/Cargo.toml)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-brightgreen)](crates/skipzone/Cargo.toml)
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
and so on is planned. Right now, everything is pre-alpha. Currently, we have a public build people can test with, check out the [Accuracy Report](#accuracy-report)!

## About

Skipzone traces HF radio rays through a 3D ionosphere, using the actual IGRF
geomagnetic field, the full Appleton-Hartree magnetoionic dispersion relation
(O and X modes), and a day/night-aware Chapman D-region model, solved via the
Haselgrove ray equations in Hamiltonian form.

- **Engine (`crates/skipzone/`):** no dependencies beyond `num-complex` and `thiserror`; no
  system libraries; single-threaded; `unsafe` forbidden. Built to stay portable
  and embeddable. CI builds it on its own, outside the workspace default
  members, so a dependency leak from the app cannot hide.
- **Ionosphere (`crates/skipzone-iono/`):** Chapman layers, the CCIR foF2 and
  M(3000)F2 maps, sporadic E and solar geometry, feeding the engine through its
  `ElectronDensity` trait.
- **Prediction (`crates/skipzone-predict/`):** the headless link-prediction
  library: scenarios, the solve, link budget, noise, antennas, ground
  reflection, frequency sweep and area coverage. Hosts the `hfpredict` CLI.
- **Lab (`crates/skipzone-lab/`):** validation and calibration against WSPR
  measurements, and every harness binary. The only crate with network access.
- **App (`app/`):** an `egui` desktop map application for
  point-to-point HF path prediction. Useful for amateur radio propagation
  planning, but general enough for any HF ray-tracing use case.
- **Docs (`docs/derivations/`):** every piece of physics derived from first
  principles and cross-checked against known analytic limits, before it was
  implemented.
- **Wiki ([github.com/GabrielVicini/Skipzone/wiki](https://github.com/GabrielVicini/Skipzone/wiki)):** how the whole thing fits
  together, module by module. The pages are versioned in `wiki/` in this
  repository and published to the GitHub wiki from there.

### Where the code actually is

The engine is about 4,000 lines and the app is about 30,000. That split is
deliberate but it is easy to misread, so to be explicit: `crates/skipzone/` is a ray tracer
and a magnetic field model. The ionosphere itself (Chapman layers, foF2
climatology, sporadic E) lives in `crates/skipzone-iono/`; the noise model, the
link budget and the solver live in `crates/skipzone-predict/`; the GUI lives in
`app/`. Embedding `skipzone` alone gets you a tracer that you
must supply an electron-density model to.

## Project Layout

```
skipzone/
├── Cargo.toml          # virtual workspace root
├── crates/skipzone/    # the engine: field model, magnetoionic theory, ray tracer
│   ├── data/           # IGRF-14 coefficients
│   ├── examples/       # nvis_probe, a high-angle diagnostic
│   ├── benches/        # criterion baseline for one traced hop
│   └── tests/          # analytic and invariant suites
├── crates/skipzone-iono/     # the ionosphere model, foF2 grid and NeQuick data
├── crates/skipzone-predict/  # headless prediction: scenario, solve, link budget
│   └── src/
│       ├── assets/     # bundled coastline shapefiles
│       └── bin/        # hfpredict, the headless engine-only CLI
├── crates/skipzone-lab/      # validation and calibration against measurements
│   └── src/
│       └── bin/        # validation and calibration harnesses
├── app/                # skipzone-app: egui desktop application
├── docs/derivations/   # the physics, derived before it was implemented
└── wiki/               # project wiki (GitHub wiki source)
```

The app is a separate workspace member so its `eframe`/`winit`/HTTP dependency
tree never enters the engine crate, which stays buildable with nothing beyond
`std` and two crates.

`corpus/` is not in the repository. It is gitignored working data that the WSPR
and ionosonde harnesses read and write. See
[Validation Harnesses](https://github.com/GabrielVicini/Skipzone/wiki/Validation-Harnesses) for how to build it.

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
in [Command Line Tools](https://github.com/GabrielVicini/Skipzone/wiki/Command-Line-Tools).

| Tool | Crate | What it does |
|---|---|---|
| `hfpredict` | predict | Headless first-order path predictor. Engine only, no app code. |
| `wspr_corpus` | lab | Fetches a reproducible WSPR corpus once and writes it to disk. |
| `wspr_calibrate` | lab | Fits the unverified anchors against that corpus, treating station effects as nuisance parameters. |
| `wspr_validate` | lab | Scores the model against a saved list of WSPR spots. |
| `wspr_live_check` | lab | Fetches live spots and the observed sunspot number, then scores against them. |
| `iono_check` | lab | Scores the ionosphere model against measured ionosonde foF2 and foE. |
| `mode_audit` | lab | Explains why the solver admitted a path and what it charged it. |
| `solve_digest` | lab | Stable fingerprint of everything `solve()` produces, over a scenario grid. |
| `profile_solve` | lab | Counts where the time in one solve actually goes. |
| `gen_fof2_grid` | lab | Regenerates the bundled foF2 climatology table. |

Run any of them with:

```bash
cargo run --release -p skipzone-lab --bin wspr_validate -- --help
```

## Accuracy Report

[docs/reports/2026-08-verification.md](docs/reports/2026-08-verification.md) is the latest, a dated snapshot.
It separates three tiers of evidence that are usually conflated: whether the
engine solves the equations correctly, whether the modelled ionosphere matches
measurement, and whether an end-to-end prediction matches observed propagation.

## Supported Platforms

Windows is the primary and most-tested target. Ubuntu 24.04 is also tested.
macOS and other Linux distributions should work but are unverified.

The Linux build requires glibc 2.39 or newer (Ubuntu 24.04+, Debian 13+,
Fedora 40+). Older distributions such as Ubuntu 22.04 or Debian 12 need a
rebuild from an older base.

There is no browser build. A WebAssembly proof of concept existed and was
removed: it carried its own code paths for having no threads, and keeping them
correct cost more than the target was worth. (Last commit to have WebASM was ebfbdea)

The engine crate declares `rust-version = "1.85"`, and `skipzone-iono` and
`skipzone-predict` declare 1.88. CI checks each one on the version it declares.
The app and lab track whatever `eframe` requires, which moves.

## Documentation

- [Wiki](https://github.com/GabrielVicini/Skipzone/wiki) - how the project is put together, module by module.
  (Sources in [wiki/](wiki/).)
- [docs/derivations/](docs/derivations/) - the physics, derived from first principles.
- `cargo doc --package skipzone --open` - engine API documentation.

## License

MIT, except `crates/skipzone-iono/data/nequick/` (EUPL-1.2). See [LICENSE](LICENSE).

The bundled NeQuick reference data under `crates/skipzone-iono/data/nequick/` is EUPL v1.2 and
carries its own notice; see [crates/skipzone-iono/data/nequick/README.md](crates/skipzone-iono/data/nequick/README.md).
The other bundled data keeps its upstream terms: the IGRF-14 coefficients in
`crates/skipzone/data/igrf14coeffs.txt` are published by IAGA and freely available, and the Natural
Earth shapefiles in `crates/skipzone-predict/src/assets/` are public domain.

For any licensing concerns or questions, email `hello@vicini.io`.
