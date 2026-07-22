# Appleton–Hartree dispersion relation, derived

Implements: `src/magnetoionic.rs`. Conventions: `conventions.md`
(`exp(-iωt)`, `U = 1 + iZ`, `Im(n) > 0` = loss).

Assumptions, stated up front: cold electron plasma (no thermal corrections),
ions immobile (valid at HF, ω ≫ ion gyro/plasma frequencies), collision
frequency enters as a velocity-independent drag ν (documented limitation: no
Sen–Wyller generalisation), medium locally homogeneous over a wavelength
(WKB / ray optics).

## 1. Dielectric tensor

Linearised electron momentum equation with static B₀ = |B₀| b̂ and drag:

    m dv/dt = -e(E + v × B₀) - m ν v ,  e > 0

With `exp(-iωt)`: `(ν - iω) v + Ω × v = -(e/m) E`, where `Ω = (e/m) B₀`,
`Ω = |Ω| = ω_H`. In a frame with ẑ ∥ b̂, circular components
`v± = v_x ± i v_y` decouple:

    (ν - iω ± iΩ) v± = -(e/m) E± ,  (ν - iω) v_z = -(e/m) E_z

Current `J = -e Nₑ v`, permittivity `ε = 1 + iσ/(ε₀ω)` (see conventions.md),
and `ν - iω ± iΩ = -iω(U ∓ Y)` give the principal values

    R := ε₊ = 1 - X/(U - Y) ,  L := ε₋ = 1 - X/(U + Y) ,  P := ε_z = 1 - X/U

Check: R has the electron-gyroresonance denominator (`U - Y → 0` at ω = ω_H,
Z = 0), as it must for electrons. Define S = (R+L)/2, D = (R-L)/2:

    S = (U(U-X) - Y²)/(U² - Y²) ,  D = -XY/(U² - Y²) ,  RL = ((U-X)² - Y²)/(U² - Y²)

## 2. Booker quartic and its discriminant

`n×(n×E) + εE = 0` with n = ck/ω at angle θ to b̂ (c := cos θ, s := sin θ)
has nontrivial solutions when (standard determinant expansion of the 3×3
system in the frame with b̂ = ẑ, k in the x-z plane)

    A n⁴ - B n² + C = 0
    A = S s² + P c² ,  B = RL s² + PS(1 + c²) ,  C = P R L

Discriminant identity, verified by expansion (needed for the stable root
form): using `(1+c²)² = (2-s²)²` and `S² - RL = D²`,

    B² - 4AC = (RL - PS)² s⁴ + 4 P² D² c²   ... (*)

[Expansion check of (*): B² - 4AC = R²L²s⁴ + 2RLPSs²(1+c²) + P²S²(1+c²)²
- 4PRLSs² - 4P²RLc²; the middle terms combine as 2PRLSs²(1+c²-2) = -2PRLSs⁴;
P²S²(4-4s²+s⁴) - 4P²RLc² = P²S²s⁴ + 4P²c²(S²-RL); collecting gives
s⁴(RL-PS)² + 4P²D²c².]

## 3. Reduction to Appleton–Hartree

Substituting the section-1 values and clearing the common denominator
`U(U² - Y²)` turns the quartic into `A' n⁴ - B' n² + C' = 0` with

    A' = U[U(U-X) - Y²] s² + (U-X)(U² - Y²) c²
    B' = U[(U-X)² - Y²] s² + (U-X)[U(U-X) - Y²](1 + c²)
    C' = (U-X)[(U-X)² - Y²]

Substituting `n² = 1 - ξ` (so ξ = X-proportional by construction) gives
`A'ξ² - (2A' - B')ξ + (A' - B' + C') = 0`. Direct expansion, W := U - X:

    A' - B' = UXs²W + W[UXc² - UW + Y²] = W[UX - UW + Y²] = W[2UX - U² + Y²]
              (the s² and c² pieces of UX combine; UX - UW = U(2X - U))
    A' - B' + C' = W[2UX - U² + Y²] + W[W² - Y²]
                 = W[2UX - U² + (U-X)²] = W X²
    2A' - B' = UW(U + X - W) - Y²(U s² + W c² - W)
             = 2UWX - X Y² s²
               (U + X - W = 2X;  U s² + W c² - W = (U - W)s² = X s²)
             = X (2UW - Y² s²)

so, dividing by X, the quadratic for ξ is

    A' ξ² - X (2UW - Y²s²) ξ + X² W = 0

Its discriminant simplifies (expansion in the same way as (*)):

    X²[(2UW - Y²s²)² - 4A'W] = X²[ Y⁴s⁴ + 4W²Y²c² ]

Writing the root with the product-of-roots form ξ = 2X²W / (X[(2UW - Y²s²) ∓ √...])
to avoid cancellation, and with `Y_T = Y s`, `Y_L = Y c`:

    n² = 1 - X (U - X) / [ U(U-X) - ½Y_T² ± √( ¼Y_T⁴ + Y_L²(U-X)² ) ]   (AH)

Dividing numerator and denominator by W (legitimate for real W > 0)
recovers the classical textbook form
`n² = 1 - X/[U - Y_T²/(2(U-X)) ± √(Y_T⁴/4(U-X)² + Y_L²)]`; (AH) is the
form that remains meaningful at W = 0.

## 4. Mode labels and branch conventions

Square root: **principal branch** (Re ≥ 0; for a non-negative real argument
this is the ordinary real root). For Z = 0 the argument `¼Y_T⁴ + Y_L²W²` is
real and non-negative for **all** X (W² appears squared), so the collisionless
index is real everywhere and no branch ambiguity exists.

Labels, anchored by limits (each is a unit test):

- `+` root = **ordinary (O)**: at exact transverse propagation (Y_L = 0) it
  gives n² = 1 - X/U — the wave with E ∥ B₀ that does not feel the field;
  collisionless it vanishes at X = 1 (reflection at the plasma frequency).
- `-` root = **extraordinary (X)**: transverse collisionless
  n² = 1 - X(1-X)/(1-X-Y²); zeros at X = 1-Y and X = 1+Y; gyroresonance
  as Y → 1 in the longitudinal limit.
- Longitudinal (Y_T = 0, W > 0): O → 1 - X/(U+|Y_L|), X → 1 - X/(U-|Y_L|),
  the L/R circular waves.

## 5. Numerically stable evaluation forms

Near O-mode reflection X → 1 (W → 0) the classical form suffers
`Y_T²/(2W) → ∞` cancellation and (AH) becomes 0/0. Exact rewrite: with
`S_m = √(¼Y_T⁴ + Y_L²W²)` and `G = S_m + ½Y_T²`,

    -½Y_T² + S_m = (S_m² - ¼Y_T⁴)/G = Y_L²W²/G

so the O denominator is `W[U + Y_L²W/G]` and

    O:  n² = 1 - X / ( U + Y_L² W / G )        — no cancellation at W → 0
    X:  n² = 1 - X W / ( UW - ½Y_T² - S_m )    — denominator → -Y_T² at W → 0

Both are algebraically identical to (AH); the O form makes the X → 1
behaviour manifest: n² → 1 - X/U, which vanishes at X = 1, Z = 0. This is
the O reflection condition for every θ except the degenerate window below.

## 6. Conditioning and known degeneracies (spec: state them)

- **X = 1, θ = 0 (Ellis window / critical coupling).** The limits X→1 and
  θ→0 do not commute; the dispersion surfaces are degenerate and no global
  continuous mode labelling exists (this is physics, not implementation:
  Budden's coupling points). At exactly Y_L ≠ 0, Y_T = 0, W = 0 the stable
  O form hits G = 0. The code returns the QT-limit value 1 - X/U there and
  documents that within the sliver |W| ≲ (coupling scale) the O/X labels
  can swap; full mode-conversion physics is out of scope. HF rays with
  apex wave-normal not exactly field-aligned never enter the sliver.
- **Resonances** (denominator → 0, n² → ∞): upper-hybrid family for the X
  mode; the gyroresonance Y = 1 sits below HF for F-region fields
  (ω_H/2π ≈ 1.4 MHz) but the formula itself is exact there; the tracer's
  step controller will fail loudly (step collapse) rather than integrate
  through a resonance.
- **Collisional branch of S_m**: with Z > 0 the argument of S_m is complex;
  the principal branch is discontinuous only where the argument crosses the
  negative real axis, which requires |1-X| ≲ Z with significant Y_T — a
  sliver of the coupling region already documented above.
- **Z = 0 exactness**: all quantities are real complex-parts-zero through
  +,-,*,/ and √ of a non-negative real, so collisionless evaluation returns
  Im = 0.0 exactly, making "zero collisions ⇒ zero absorption" bit-exact.

## 7. Analytic partial derivatives

All ray-equation gradients come from the complex differentials of the forms
in section 5, with a1 := Y_L², a2 := Y_T², W = U - X:

    dS_m = [ ½ a2 da2 + W² da1 + 2 a1 W dW ] / (2 S_m)     (S_m ≠ 0)
    dG   = dS_m + ½ da2

    O:  F = U + a1 W / G
        dF = dU + (W da1 + a1 dW)/G - (a1 W / G²) dG
        d(n²) = -dX/F + (X/F²) dF
    X:  F = U W - ½ a2 - S_m
        dF = U dW + W dU - ½ da2 - dS_m
        d(n²) = -(W dX + X dW)/F + (X W / F²) dF

with `dW = dU - dX`, `dU = i dZ`, and the (X, Y, cosθ =: c) parametrisation

    da1 = 2Yc² dY + 2Y²c dc ,   da2 = 2Y(1-c²) dY - 2Y²c dc

Special cases: Y = 0 → n² = 1 - X/U, ∂n²/∂X = -1/U, ∂n²/∂Z = iX/U²,
∂n²/∂Y = ∂n²/∂c = 0 (the Y-dependence is quadratic near Y = 0). This branch
is also what makes O and X bit-identical in zero field: both modes route
through the same expression.

Validation: every partial is compared against central finite differences of
the full complex evaluation at random interior points and near W ≈ 0 (tests);
finite differences appear only as a test oracle, never in production code.
