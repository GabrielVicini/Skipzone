# Physics Derivations

The rule is: **every equation is derived from first principles in
`docs/derivations/` before it is implemented, and the code cites the derivation
file and section.**

This is not documentation written after the fact. The derivations came first,
they are the specification, and the tests check the implementation against them.
Where a derivation and the code disagree, the derivation wins until someone
argues otherwise in writing.

This page indexes them. The derivations themselves are in
`docs/derivations/` and contain the actual mathematics.

## `conventions.md`

Fixed crate-wide. Every other derivation assumes these, and getting one wrong
produces plausible numbers that are quietly conjugated or mirrored.

| Section | Contents |
|---|---|
| Coordinates | Geocentric spherical `(r, theta, phi)`, the local `(r_hat, theta_hat, phi_hat)` = (up, south, east) basis |
| Time convention and the sign of losses | `exp(-i omega t)`, so `U = 1 + iZ` and a lossy medium has `Im(n) > 0`. Budden and Davies use the opposite convention and their formulas are the complex conjugates of these |
| Magnitudes and signs of X, Y, Z | The three magnetoionic parameters |
| Units | SI throughout, `f64`, newtypes at public boundaries only |

Implemented by: the whole engine. Enforced by: `src/units.rs`.

## `magnetic-field.md`

Implements `src/mag/`.

| Section | Contents |
|---|---|
| 1. Potential and components | The geomagnetic field as the gradient of a scalar potential, expanded in spherical harmonics |
| 2. Coordinate Jacobian | The plain coordinate partials of the component functions. **Not** covariant derivatives; the basis-rotation terms are handled once, exactly, in the ray equations |
| 3. Schmidt semi-normalised Legendre functions | The recurrences for `S_n^m(theta)` and its first and second theta-derivatives |
| 4. Centered tilted dipole | The `n = 1` term in closed form, used as a test oracle |
| 5. Test invariants | Divergence and curl of the field, both zero, checked numerically |

The pole is a coordinate singularity in this formulation and evaluation there is
restricted rather than fudged.

## `appleton-hartree.md`

Implements `src/magnetoionic.rs`. The longest and most delicate derivation.

| Section | Contents |
|---|---|
| 1. Dielectric tensor | The cold-plasma tensor and its `R`, `L`, `P`, `S`, `D` components |
| 2. Booker quartic and its discriminant | The quartic whose roots are the four characteristic modes |
| 3. Reduction to Appleton-Hartree | Getting from the quartic to the familiar form without approximating |
| 4. Mode labels and branch conventions | How O and X are anchored so they cannot swap identity across the domain |
| 5. Numerically stable evaluation forms | Algebraic rewrites that avoid catastrophic cancellation. Reverting one to its textbook form passes a spot check and fails near the reflection height |
| 6. Conditioning and known degeneracies | The Ellis window, and the exact bit-identity of `Z = 0` absorption |
| 7. Analytic partial derivatives | The derivatives of `n^2`, derived rather than differenced |

There is **no** quasi-longitudinal or quasi-transverse approximation anywhere in
this project. That is a deliberate position: the QL and QT forms are where most
HF codes lose the polarisation behaviour that makes O and X differ.

## `haselgrove.md`

Implements `src/hamiltonian.rs` and `src/trace.rs`.

| Section | Contents |
|---|---|
| 1. Canonical variables | The canonical form of the ray equations in spherical coordinates |
| 2. The Hamiltonian and the working variables | The real-ray approximation, and why `sigma` is parametrised in metres rather than arc length |
| 3. Group delay from the extended phase space | Group path as a state variable |
| 4. Phase path, absorption, arc length | The three integrated observables |
| 5. Doppler | Derived, then explicitly declared a non-goal for this project |
| 6. Initialization and termination | Launch conditions, ground landing, domain-top escape |
| 7. Conditioning near reflection | The Spitze, and why the high-angle branch is the hard case |

The field-free case is not a separate code path. It is `Y = 0`, which reuses the
full magnetoionic evaluation and is why `O == X` bit-identically there.

## `analytic-solutions.md`

Used by `tests/analytic_field_free.rs`. This is the derivation that makes the
engine testable at all: closed forms to compare a numerical tracer against.

| Section | Contents |
|---|---|
| 1. Bouguer invariant | Spherical Snell's law, derived from the ray equations rather than assumed |
| 2. Ray integrals in a stratified medium | The general quadrature |
| 3. Vacuum (`n = 1`) | Straight lines, the trivial check |
| 4. Quasi-parabolic layer: closed forms | Full closed-form range and apex, including the `acosh` versus `asinh` branch and its exact-apex-limit conditioning |
| 5. Linear-gradient rays are parabolas, not circles | A **specification correction**. The original spec said circles. The derivation says parabolas |
| 6. Bouguer quadrature reference | A numerical reference good for any stratified profile, not just the ones with closed forms |

Section 5 is worth noting as precedent: the derivation caught an error in the
specification, and the specification was changed. That is the intended direction
of authority.

## `chapman-grazing.md`

Implements `app/src/chapman.rs`. The only derivation for code outside the engine
crate, because the grazing-incidence layer is the one place the application
carries physics of its own.

| Section | Contents |
|---|---|
| 1. Why the plane-parallel layer is not enough | `sec(chi)` diverges and forces a hard 85 degree cutoff exactly at the terminator |
| 2. The Chapman function | `Ch(X, chi)`, finite through and past 90 degrees. Smith and Smith 1972 |
| 3. Scaled complementary error function | `erfcx` via the A&S 7.1.14 continued fraction, 48 levels, with continuant rescaling above 1e250 |
| 4. Local zenith angle and density gradients | The horizontal gradients, which are required rather than optional: omitting them lets the Hamiltonian drift along the ray |
| 5. Numerical night guard | Deep night is exactly vacuum |

## The discipline, restated

Three things recur across all six derivations and are worth naming, because they
are the project's actual method rather than incidental style:

1. **Analytic derivatives are the implementation; finite differences are the test
   oracle.** Never the reverse. Every model that reports a gradient has a test
   named `*_matches_finite_differences` or `*_matches_fd`.
2. **Conditioning is derived, not discovered.** The cancellation-avoiding rewrite
   in the Appleton-Hartree evaluation and the exact-apex-limit branch in the
   quasi-parabolic closed form are the same discipline in two different places:
   find where the naive form loses precision, derive the stable one, and comment
   the site.
3. **A closed form's job is to be a test oracle.** The dipole exists to check
   IGRF. The quasi-parabolic layer exists to check the tracer. The linear layer
   exists to check the quadrature. They are not there because anyone wants to
   predict with them.
