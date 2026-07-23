# Conventions

Fixed crate-wide. Every other derivation assumes these.

## Coordinates

Geocentric spherical `(r, θ, φ)`: radius from Earth's centre [m], colatitude
from the geographic north pole [rad], east longitude [rad]. Local right-handed
orthonormal basis `(r̂, θ̂, φ̂)` = (up, south, east); north = `-θ̂`.
Line element: `dl = dr r̂ + r dθ θ̂ + r sinθ dφ φ̂`.

Geomagnetic elements in this basis: X (north) = `-B_θ`, Y (east) = `B_φ`,
Z (down) = `-B_r`.

The Earth is a sphere in the core physics. Geodetic shape enters, if ever, only
in input/output conversion outside the tracer.

## Time convention and the sign of losses

Fields vary as `exp(-iωt)`; a plane wave is `exp(i(k·x - ωt))`.

Electron momentum equation with a collisional drag `-m ν v` (cold plasma,
no field for this argument; charge `-e`, `e > 0`):

    m dv/dt = -e E - m ν v   →   (-iω + ν) m v = -e E
    v = -e E / (m (ν - iω))

Current density `J = -e Nₑ v = e² Nₑ E / (m (ν - iω))`, so the AC conductivity
is `σ = e² Nₑ / (m (ν - iω))`. With `exp(-iωt)`, Ampère's law gives an
effective relative permittivity `ε = 1 + iσ/(ε₀ω)`:

    ε = 1 + i ωₚ² / (ω (ν - iω)),   ωₚ² = Nₑe²/(ε₀m)

Using `ν - iω = -iω(1 + iν/ω)`:

    ε = 1 - X / (1 + iZ),   X = ωₚ²/ω²,  Z = ν/ω

Define `U = 1 + iZ`. Then `Im(ε) = XZ/(1+Z²) > 0` for `Z > 0`, so `Im(n) > 0`
and `exp(ik·x)` decays along propagation: **losses correspond to positive
imaginary parts**. Budden's and Davies' formulas use `exp(+iωt)` and
`U = 1 - iZ`; ours are their complex conjugates. Any formula imported from
that literature must be conjugated, and this is noted at each use site.

Amplitude attenuation in nepers over a path: `A = (ω/c) ∫ Im(n) ds`;
power in dB is `20/ln 10 × A ≈ 8.6859 A`.

## Magnitudes and signs of X, Y, Z

`X = ωₚ²/ω² ≥ 0`. `Y = ω_H/ω` with `ω_H = e|B|/mₑ > 0` (magnitude only; the
electron's negative charge is absorbed into the sign conventions fixed in the
Appleton–Hartree derivation, where the ± mode labels are anchored to physical
limits). `Z = ν/ω ≥ 0`.

## Units

SI everywhere, `f64` everywhere. Public API boundaries use newtypes
(`Meters`, `Hertz`, ...); integrator internals are raw `f64` in SI units.
