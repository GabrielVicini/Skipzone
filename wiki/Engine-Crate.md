# Engine Crate (`skipzone`, `src/`)

HF ionospheric ray tracing from first principles. Two dependencies
(`num-complex`, `thiserror`), no system libraries, `unsafe` forbidden at the
crate level, single-threaded.

## Crate-wide conventions

These are fixed, derived in `docs/derivations/conventions.md`, and every module
assumes them. Getting one wrong produces plausible numbers that are quietly
conjugated or mirrored, so they are worth stating in full.

- **Time dependence `exp(-i omega t)`.** A lossy medium therefore has
  `Im(n) > 0` and the field attenuates as `exp(-(omega/c) Im(n) s)`. Budden and
  Davies use `exp(+i omega t)`, so their formulas are the complex conjugates of
  the ones here. If you are porting an equation out of a textbook, this is the
  first thing to check.
- **Geocentric spherical coordinates `(r, theta, phi)`:** radius from Earth's
  centre, colatitude from the geographic north pole, east longitude. The local
  right-handed orthonormal basis `(r_hat, theta_hat, phi_hat)` is (up, south,
  east), so north is `-theta_hat`.
- **SI units and `f64` throughout.** Newtypes guard the public boundaries; the
  integrator state is raw `f64` in SI units, converted at the edges.
- **Every equation is derived before it is implemented.** Code comments cite the
  derivation file and section.

## Modules

### `units` and `constants`

`units` holds scalar newtypes (`Meters`, `Hertz`, `Radians`, `PerSecond`,
`PerCubicMeter`, `Nepers`) for public API boundaries. The policy is explicit: a
frequency cannot be passed where an altitude is expected, but inside the
integrator loop everything is raw `f64` in SI for speed. Conversion happens at
the edges, not per step.

`constants` holds physical constants in SI, each stating its source. Values made
exact by the 2019 SI redefinition are marked exact; the rest are CODATA 2022
recommended values.

### `geo`

Geocentric spherical points and directions on the sphere: `SphericalPoint`,
`central_angle`, `bearing`, `launch_direction`, `track_errors`. This is the
module that fixes the coordinate convention in code.

### `mag`

Magnetic field models behind one trait. A `FieldSample` carries the field
components `(B_r, B_theta, B_phi)` in tesla on the local basis, plus `db[i][j]`,
the plain coordinate partial of component `i` with respect to coordinate `j`.

Note the "plain coordinate partial" carefully. These are **not** covariant
derivatives. The basis-rotation terms are handled once, exactly, in the ray
equations, so a field model must not pre-apply them.

- `igrf` implements IGRF-14 from its defining Schmidt semi-normalised
  spherical-harmonic expansion to degree 13, valid 1900.0 to 2030.0, with a
  secular-variation column for 2025 to 2030. The coefficient table is compiled
  in with `include_str!`; see [Data and Assets](Data-and-Assets.md).
- `legendre` computes the Schmidt semi-normalised associated Legendre functions
  and their first and second theta-derivatives by recurrence.
- `dipole` is the `n = 1` term in closed form. It exists as a test oracle: the
  full IGRF evaluation is checked against it, and both are checked against
  divergence and curl invariants.
- `ZeroField` is the field-free case, used by the analytic test suite.

### `density`

Electron density models behind one trait. A sample returns `Ne` plus its
coordinate partials (per m, per rad, per rad), matching the field trait's
convention so the ray equations assemble gradients uniformly.

The engine ships `ChapmanLayer`, `QuasiParabolicLayer` and a linear layer,
mainly because they have closed-form analytic solutions the test suite can check
the tracer against. The ionosphere the application actually uses is built in
`app/`; see [Ionosphere Model](Ionosphere-Model.md).

`density_at_critical_frequency` and `critical_frequency` convert between peak
density and plasma frequency, and round-trip exactly.

### `collision`

Electron collision frequency models, same sample convention as density. The
collision frequency enters Appleton-Hartree as `Z = nu/omega` and is the **sole
source of absorption** in the model. `ExponentialCollisions` is the production
model; `ZeroCollisions` gives a lossless medium for the analytic tests.

### `magnetoionic`

The full complex Appleton-Hartree refractive index for the ordinary and
extraordinary modes, with analytic partial derivatives. No quasi-longitudinal or
quasi-transverse approximation anywhere.

Points worth knowing before touching this file:

- Mode labels are anchored so that O and X do not swap identity across the
  domain.
- The expressions are written in numerically stable form. Several are algebraic
  rewrites specifically to avoid catastrophic cancellation, and reverting one to
  its textbook form will pass a spot check and fail near the reflection height.
- `Z = 0` absorption is exactly zero by short circuit, not approximately zero.
  The zero-field case satisfies `O == X` bit-identically. Both are invariants
  the validation suite enforces, and both are why `float_cmp` is allowed at the
  crate level.

### `hamiltonian`

Assembly of the Haselgrove ray equations from the medium models. The state
vector is fixed size, ten elements, no allocation in the loop:

```text
y[0] r [m]              y[1] theta [rad]      y[2] phi [rad]
y[3..6] m = (c/omega) k, physical components (r, theta, phi)
y[6] group path P' [m]  y[7] phase path P [m]
y[8] absorption A [Np]  y[9] arc length s [m]
```

The independent variable `sigma` has units of metres and is **not** arc length.
Arc length is carried separately as `y[9]`. This is deliberate and is derived in
`docs/derivations/haselgrove.md`.

### `integrate`

Dormand-Prince 5(4) embedded Runge-Kutta pair (RK5(4)7M, Dormand and Prince
1980, standard DOPRI5 tableau as given by Hairer, Norsett and Wanner). Seven
stages, FSAL, fixed-size state, zero allocation per step. The convergence order
is verified by test rather than asserted.

### `trace`

The ray trace driver: adaptive stepping, event location, observables. Events
(ground landing, domain-top escape, apex passage) are located by re-integrating
a partial step from the last accepted state and root-finding on the event
function, with a secant iteration guarded so that equal bracket endpoints fall
back to bisection.

The tracer is single-threaded and traces one ray per call. There is no parallel
fan-out here; batching is the app's job.

### `homing`

Find the launch elevation and azimuth that land a ray on a target. Two
independent methods, cross-checked by the validation suite:

1. `home_scan`: an elevation scan at the great-circle bearing, bracketing the
   sign change in the range error and refining it.
2. A Newton method on the two-dimensional miss vector, with finite-difference
   Jacobian.

`HomingConfig` carries the scan bounds, step, iteration cap, miss tolerance and
finite-difference step. High elevations near the vertical are the hard case (the
Spitze), and the configuration documents that.

### `error`

Typed errors for numerical failure modes only. A ray that lands or escapes the
domain is a physical **outcome**, not an error. Errors are reserved for cases
where the computation itself can no longer be trusted, such as the adaptive
controller driving the step below the configured minimum.

## Testing

The engine's tests are integration tests in `tests/`, plus unit tests in the
modules.

| Suite | What it pins |
|---|---|
| `tests/analytic_field_free.rs` | Closed-form quasi-parabolic solutions, the Bouguer invariant along the ray, apex conditions, fifth-order convergence, straight rays in vacuum |
| `tests/magnetoionic_3d.rs` | Full 3D magnetoionic behaviour against an Earth-like dipole, and the high-frequency geometric limit |
| `tests/absorption.rs` | Absorption monotone and positive, and reciprocal |
| `tests/homing.rs` | The two homing methods agree |
| `tests/common/mod.rs` | Shared fixtures, including the Bouguer reference quadrature |

The pattern throughout is that analytic derivatives are the implementation and
finite differences are the test oracle, never the other way round.

## Embedding it

```rust
use skipzone::collision::ExponentialCollisions;
use skipzone::density::{ChapmanLayer, density_at_critical_frequency};
use skipzone::geo::SphericalPoint;
use skipzone::mag::Igrf;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, PerSecond, Radians};
```

Supply your own `ElectronDensity`, `MagneticField` and `CollisionFrequency`
implementations if the bundled ones do not fit. `examples/nvis_probe.rs` is the
smallest complete example: a field-free Chapman layer probed across the
high-angle part of the elevation fan.
