<a name="top"></a>

[![Skipzone Banner](https://github.com/GabrielVicini/Skipzone/blob/226be75c7dda1d9d37d8a78dd785157fa5c85173/assets/banner.png)](https://github.com/GabrielVicini/Skipzone)

# Skipzone

**3D ionospheric HF ray tracing** — Haselgrove equations with full Appleton-Hartree magnetoionic theory.

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)](https://www.rust-lang.org)
[![edition](https://img.shields.io/badge/edition-2024-orange)](Cargo.toml)
[![license](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-brightgreen)](Cargo.toml)
[![build](https://img.shields.io/github/actions/workflow/status/USER/skipzone/ci.yml?branch=main)](#)
[![docs](https://img.shields.io/badge/docs-derivations-1D76DB)](docs/derivations)

## Important Message!
Currently, the engine is in a non production state as I fine-tune it and verify it's accuracy and work out minor inputs bugs. Do not expect the results as of 7/28/26 to be accurate!

⭐ Star us on GitHub if this is useful to you!

## Table of Contents
- [About](#about)
- [Project Layout](#project-layout)
- [Accuracy Report](#accuracy-report)
- [Building](#building)
- [License](#license)

## About

Skipzone traces HF radio rays through a 3D ionosphere, using the actual IGRF
geomagnetic field, the full Appleton-Hartree magnetoionic dispersion relation
(O and X modes), and a day/night-aware Chapman D-region model — solved via
the Haselgrove ray equations in Hamiltonian form.

- **Engine (`src/`):** no dependencies beyond `num-complex`, `rayon`, and
  `thiserror`; no system libraries; `unsafe` forbidden. Built to stay
  portable and embeddable.
- **App (`app/`):** an `egui` + `Walkers` desktop map application for
  point-to-point HF path prediction — useful for amateur radio propagation
  planning, but general enough for any HF ray-tracing use case.
- **Docs (`docs/derivations/`):** every piece of physics — the magnetic
  field, the dispersion relation, the ray equations, the D-region model —
  derived from first principles and cross-checked against known analytic
  limits.

## Project Layout

```
skipzone/
├── Cargo.toml          # engine crate (workspace root)
├── src/                # the engine: field model, magnetoionic theory, ray tracer
├── app/                # skipzone-app: egui + Walkers desktop application
│   └── Cargo.toml
├── docs/
│   └── derivations/    # analytic-solutions, appleton-hartree, chapman-grazing,
│                       # conventions, haselgrove, magnetic-field
├── examples/
├── benches/
├── tests/
├── corpus/
└── data/
```

The app is a separate workspace member so its `eframe`/`winit`/HTTP
dependency tree never enters the engine crate, which stays buildable with
nothing beyond `std`.

## Accuracy Report

Coming soon — a validation report against real-world propagation data will
be published once the project reaches its first tagged release.

## Building (To run the GUI)
Make sure you have the repo downloaded!
```
cargo build --release
cargo test
run -p skipzone-app --bin skipzone-app
```
Keep an eye out on the other tools the repo can offer like ``wspr_calibrate``! See Docs

## Supported Platforms
Currently, it should theoretically support most modern operating systems such as Windows (+ Arm), MacOS, Linux — however, only Windows 11 has been tested. It also does have support for WebASM for browser support although it's not the main focus and is a proof of concept.

## License

GPL-3.0 — see [LICENSE](LICENSE).
