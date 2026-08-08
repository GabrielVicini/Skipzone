# Skipzone Wiki

3D ionospheric HF ray tracing: Haselgrove ray equations solved through a real
IGRF geomagnetic field with the full complex Appleton-Hartree refractive index,
plus a desktop application that turns that into point-to-point and area
propagation predictions.

This wiki is the source of record for how the project is put together. It lives
in `wiki/` in the repository, so it is versioned with the code it describes and
a change that invalidates a page can be caught in the same commit.

**Status: pre-alpha.** The physics is derived and tested. The inputs feeding it
are still being calibrated. Nothing here is a production propagation forecast.

## Start here

| If you want to | Read |
|---|---|
| Understand the shape of the project | [Architecture](Architecture.md) |
| Embed or extend the ray tracer | [Engine Crate](Engine-Crate.md) |
| Work on the application | [App Crate](App-Crate.md) |
| Know where the ionosphere comes from | [Ionosphere Model](Ionosphere-Model.md) |
| Know how a path is decided | [Solver and Link Budget](Solver-and-Link-Budget.md) |
| Work on the interface | [User Interface](User-Interface.md) |
| Run something from a terminal | [Command Line Tools](Command-Line-Tools.md) |
| Check the model against reality | [Validation Harnesses](Validation-Harnesses.md) |
| Understand the fitted numbers | [Calibration](Calibration.md) |
| Know what data ships and under what licence | [Data and Assets](Data-and-Assets.md) |
| Read the physics | [Physics Derivations](Physics-Derivations.md) |
| Build, test or change CI | [Building and CI](Building-and-CI.md) |
| Contribute | [Conventions](Conventions.md) and [Contributing](Contributing.md) |

## The one-paragraph version

`src/` is the engine: a magnetic field model, a cold-plasma dispersion relation,
the Haselgrove ray equations, an adaptive integrator, and a homing search. It
has two dependencies, forbids `unsafe`, is single-threaded, and knows nothing
about the ionosphere beyond a trait you hand it. `app/` is everything else: the
ionosphere model itself, the noise floor, the antenna patterns, the link budget,
the multi-hop solver, the parallel compute layer, the map interface, and ten
command line harnesses that exist to check the model against measurements.

## What this project refuses to do

These are not omissions, they are positions, and each one is defended in the
page that owns it.

- **No approximations in the dispersion relation.** No quasi-longitudinal, no
  quasi-transverse. The full complex Appleton-Hartree, with analytic
  derivatives. See [Physics Derivations](Physics-Derivations.md).
- **No physics without a derivation first.** Every equation in the engine is
  derived from scratch in `docs/derivations/` and the code cites the section.
- **No `unsafe`.** Forbidden at the crate level in the engine.
- **No fitting to a number the data cannot identify.** The calibration explicitly
  enumerates what WSPR can and cannot see, and refuses to move the rest. See
  [Calibration](Calibration.md).
- **No silent fallbacks.** A guessed value that gets used says so on stderr.
- **No browser build.** A WebAssembly proof of concept existed and was removed.
  See [Architecture](Architecture.md#the-removed-web-target).
