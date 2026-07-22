# Geomagnetic field from a scalar potential

Implements: `src/mag/`. Conventions: see `conventions.md`.

## 1. Potential and components

In the current-free region above the Earth's surface, `∇×B = 0` and
`∇·B = 0`, so `B = -∇V` with `∇²V = 0`. The internal-source solution of
Laplace's equation in spherical coordinates, regular at infinity, is the IGRF
defining expansion:

    V(r,θ,φ) = a Σ_{n=1}^{N} (a/r)^{n+1} Σ_{m=0}^{n}
               [g_n^m cos(mφ) + h_n^m sin(mφ)] P_n^m(θ)

with `a` the reference radius (IGRF: a = 6371.2 km, part of the model
definition) and `P_n^m` the **Schmidt semi-normalised** associated Legendre
functions (section 3). Gauss coefficients `g, h` are in nT.

Components on `(r̂, θ̂, φ̂)` from `B = -∇V`,
`∇ = (∂_r, r⁻¹∂_θ, (r sinθ)⁻¹∂_φ)`:

Since `d/dr (a/r)^{n+1} = -((n+1)/r)(a/r)^{n+1}`:

    B_r = -∂V/∂r      = Σ_n (n+1)(a/r)^{n+2} Σ_m [g cos mφ + h sin mφ] P_n^m
    B_θ = -(1/r)∂V/∂θ = -Σ_n (a/r)^{n+2} Σ_m [g cos mφ + h sin mφ] dP_n^m/dθ
    B_φ = -(1/(r sinθ))∂V/∂φ
        = (1/sinθ) Σ_n (a/r)^{n+2} Σ_m m [g sin mφ - h cos mφ] P_n^m

Sanity anchor: Earth's `g_1^0 < 0`, so at the north geographic pole
`B_r = 2(a/r)³g_1^0 < 0`: the field points downward there, as observed.

## 2. Coordinate Jacobian ∂B_i/∂(r,θ,φ)

Needed analytically by the ray equations (chain rule through Y and the field
direction). Each component is a finite sum of terms
`(a/r)^{n+2} × (trig in φ) × (P or dP/dθ or P/sinθ)`, so:

- `∂/∂r` multiplies each degree-n term by `-(n+2)/r`.
- `∂/∂φ` maps `cos mφ ↔ sin mφ` with factors `∓m`.
- `∂/∂θ` needs `dP/dθ` and `d²P/dθ²` (section 3), and for `B_φ`
  `d/dθ (P/sinθ) = (dP/dθ)/sinθ - P cosθ/sin²θ`.

The `1/sinθ` factors are finite for `m ≥ 1` because `P_n^m ∝ sin^m θ`, but the
implementation evaluates them literally and is therefore **not valid at the
coordinate poles**; the tracer's pole guard keeps rays away, and the field
module documents the restriction rather than special-casing a region the
tracer cannot enter.

## 3. Schmidt semi-normalised Legendre functions

Definition: with `P̂_n^m` the unnormalised associated Legendre functions
*without* the Condon–Shortley phase (geomagnetic convention),

    S_n^m = c_m sqrt((n-m)!/(n+m)!) P̂_n^m,   c_0 = 1, c_m = sqrt(2) (m ≥ 1)

Normalisation check used in tests: `∫∫ [S_n^m cos(mφ)]² dΩ = 4π/(2n+1)`.

Recurrences, derived from the unnormalised ones
(`P̂_m^m = (2m-1)!! sin^m θ`, `(n-m)P̂_n^m = (2n-1)cosθ P̂_{n-1}^m - (n+m-1)P̂_{n-2}^m`)
by inserting the normalisation factors:

**Diagonal.** `S_m^m = c_m sqrt(1/(2m)!) (2m-1)!! sin^m θ`. The ratio of
successive diagonal terms (m ≥ 2) is

    S_m^m / S_{m-1}^{m-1} = sinθ (2m-1) sqrt((2m-2)!/(2m)!)
                          = sinθ sqrt((2m-1)/(2m))

and `S_1^1 = sinθ S_0^0 = sinθ` (the `c_1 = √2` cancels `sqrt(1/2!)`), so

    S_0^0 = 1
    S_1^1 = sinθ
    S_m^m = sinθ sqrt((2m-1)/(2m)) S_{m-1}^{m-1}    (m ≥ 2)

**Vertical.** Multiplying the unnormalised three-term recurrence by
`N_n^m = c_m sqrt((n-m)!/(n+m)!)` and using
`N_n^m/N_{n-1}^m = sqrt((n-m)/(n+m))`,
`N_n^m/N_{n-2}^m = sqrt((n-m)(n-m-1)/((n+m)(n+m-1)))`:

    S_n^m = [ (2n-1) cosθ S_{n-1}^m - sqrt((n-1)²-m²) S_{n-2}^m ]
            / sqrt(n²-m²)                                   (n > m)

with `S_{m-1}^m := 0` (covers the `n = m+1` seed row exactly, since the
square-root factor multiplies zero).

Checks (hand-expanded): `S_1^0 = cosθ`, `S_2^0 = (3cos²θ-1)/2`,
`S_2^1 = √3 sinθcosθ`, `S_2^2 = (√3/2)sin²θ`, `S_3^1 = (√6/4)(5cos²θ-1)sinθ`.
These exact forms are asserted in tests.

**θ-derivatives.** The recurrences are linear with coefficients `sinθ`,
`cosθ`, constants; differentiating them once and twice w.r.t. θ gives exact
recurrences for `S' = dS/dθ` and `S'' = d²S/dθ²` carried alongside `S`
(`s = sinθ`, `x = cosθ`, `α_m = sqrt((2m-1)/(2m))`):

    diagonal:  S'_m = α_m (x S_{m-1} + s S'_{m-1})
               S''_m = α_m (-s S_{m-1} + 2x S'_{m-1} + s S''_{m-1})
               (α_1 = 1, α_m = sqrt((2m-1)/(2m)) for m ≥ 2, exactly as for
               S itself - the √2 cancellation at m = 1 applies to all three)
    vertical:  S'_n = [ (2n-1)(-s S_{n-1} + x S'_{n-1}) - β S'_{n-2} ] / γ
               S''_n = [ (2n-1)(-x S_{n-1} - 2s S'_{n-1} + x S''_{n-1})
                         - β S''_{n-2} ] / γ
    with β = sqrt((n-1)²-m²), γ = sqrt(n²-m²)

Derivatives are validated against central finite differences in tests.

## 4. Centered tilted dipole (n = 1 closed form)

Keeping only n = 1 with `G(φ) = g_1^1 cos φ + h_1^1 sin φ` and
`S_1^0 = cosθ`, `S_1^1 = sinθ`:

    B_r = 2 (a/r)³ [ g_1^0 cosθ + G sinθ ]
    B_θ =   (a/r)³ [ g_1^0 sinθ - G cosθ ]
    B_φ =   (a/r)³ [ g_1^1 sinφ - h_1^1 cosφ ]

All nine coordinate partials follow by inspection (`∂/∂r → -3/r ×`, θ and φ
derivatives of the brackets). Axial case (`g_1^1 = h_1^1 = 0`):
`|B| = (a/r)³ |g_1^0| sqrt(1+3cos²θ)`, the textbook dipole magnitude, asserted
in tests.

The IGRF truncated to n = 1 must agree with this closed form to rounding
error; asserted in tests (internal consistency, not external differencing).

## 5. Test invariants

From the Jacobian, in spherical coordinates:

    ∇·B = ∂B_r/∂r + 2B_r/r + (1/r)(∂B_θ/∂θ + B_θ cotθ) + (1/(r sinθ))∂B_φ/∂φ
    (∇×B)_r = (1/(r sinθ)) [ ∂(sinθ B_φ)/∂θ - ∂B_θ/∂φ ]
    (∇×B)_θ = (1/(r sinθ)) ∂B_r/∂φ - (1/r) ∂(r B_φ)/∂r
    (∇×B)_φ = (1/r) [ ∂(r B_θ)/∂r - ∂B_r/∂θ ]

Both must vanish identically for a potential field; the tests assert this at
random points to near machine precision, which exercises every Jacobian entry.
