# Chapman grazing-incidence function and the twilight D region

Derived here for `app/src/chapman.rs`. Unlike everything else in
docs/derivations/, this supports the *scenario-level* app, not the engine
crate: it is the day/night-aware D-region absorbing layer the app feeds to the
(unchanged) engine tracer. Notation and sign conventions follow conventions.md
and density.rs.

## 1. Why the plane-parallel layer is not enough

The engine's `ChapmanLayer::with_zenith_angle` places an alpha-Chapman layer

    Ne(z) = Nm exp( 1/2 ( 1 - z - sec(chi) e^{-z} ) ),   z = (r - r_m)/H     (1)

with `sec(chi)` the plane-parallel slant factor for the ionising flux. It
correctly refuses `chi >= 85 deg` because `sec(chi)` is the flat-atmosphere
optical-depth factor and diverges as `chi -> 90 deg`, which is unphysical: the
real slant path through a *curved* atmosphere stays finite at the terminator
and the layer keeps producing a few degrees past it.

For a point-to-point HF path that can straddle the terminator this has two
consequences the midpoint-`sec(chi)` model cannot represent:

  * near `chi = 90 deg` absorption must fade *smoothly*, not switch off at an
    85 deg cliff (the observed "absorption = 0" bug);
  * different points along the same ray see different solar zenith angles, so
    the layer must be a function of horizontal position, not one global value.

The fix replaces `sec(chi)` by the Chapman grazing-incidence function
`Ch(X, chi)` and evaluates `chi` locally at each sampled point.

## 2. The Chapman function

`Ch(X, chi)` is the ratio of the slant optical depth to the vertical optical
depth for an exponential atmosphere on a sphere, with

    X = (R0 + h) / H                                                          (2)

the ratio of geocentric radius to neutral scale height (X ~ 1076 at 85 km with
H = 6 km). We use the standard closed form (Smith & Smith, J. Geophys. Res.
77, 1972), written with the *scaled* complementary error function
`erfcx(t) = e^{t^2} erfc(t)` so nothing overflows:

  chi <= 90 deg:
    Ch(X, chi) = sqrt(pi X / 2) * erfcx( sqrt(X/2) cos chi )                  (3a)

  chi > 90 deg:
    Ch(X, chi) = sqrt(2 pi X) sqrt(sin chi) e^{ X(1 - sin chi) }
                 - sqrt(pi X / 2) * erfcx( sqrt(X/2) |cos chi| )              (3b)

Checks:

  * chi = 0: erfcx(sqrt(X/2)) ~ 1/(sqrt(X/2) sqrt(pi)) for large X, so
    Ch -> 1 = sec(0). More generally Ch ~ sec(chi) for chi below ~75 deg,
    reproducing (1) where the plane-parallel model is valid.
  * chi = 90 deg: cos chi = 0, erfcx(0) = 1, so (3a) gives Ch = sqrt(pi X / 2)
    (finite, ~41 at X = 1076) - not the divergent sec(90 deg). (3b) at
    90 deg+ gives sqrt(2 pi X) - sqrt(pi X / 2) = sqrt(pi X / 2) as well, since
    sqrt(2 pi X) = 2 sqrt(pi X / 2): the two branches agree, Ch is continuous.
  * chi -> 180 deg: (3b) grows without bound (the e^{X(1-sin chi)} term), which
    drives Ne -> 0 through the -1/2 Ch e^{-z} term in (1): deep night has no
    D region, reached smoothly rather than by a threshold.

The realised peak of (1) with sec -> Ch sits at z* = ln(Ch), height
`r_m + H ln(Ch)`, with peak density `Nm Ch^{-1/2}`. At the terminator that is
`Nm / sqrt(41) ~ 0.16 Nm` at `r_m + H ln(41)` - a real, thinned, raised layer,
not an absent one.

X is taken constant at `X = r_m / H` (the reference height); it varies by
< 0.5 % across the layer, so `dCh/dr = 0` and Ch depends on position only
through chi.

## 3. Scaled complementary error function

`erfcx(t)`, t >= 0 (the argument in (3) is always non-negative), is computed
without any external crate:

  * t < 2: erfcx(t) = e^{t^2} (1 - erf(t)) with erf from its Maclaurin series
    erf(t) = (2/sqrt(pi)) sum_{n>=0} (-1)^n t^{2n+1} / (n! (2n+1)); e^{t^2} <=
    e^4 here so no overflow.
  * t >= 2: the classical continued fraction (Abramowitz & Stegun 7.1.14)
        sqrt(pi) erfcx(t) = 1 / ( t + (1/2)/( t + (2/2)/( t + (3/2)/( t + ... ))))
    evaluated backward; converges to double precision in ~40 terms for t >= 2.

Both are exercised in `chapman.rs` tests against reference values
erfcx(0)=1, erfcx(1)=0.427583..., erfcx(2)=0.255396..., erfcx(5)=0.110704....

Derivative (needed for the horizontal density gradient, section 4):

    erfcx'(t) = 2 t erfcx(t) - 2/sqrt(pi)                                     (4)

from d/dt ( e^{t^2} erfc(t) ) = 2 t e^{t^2} erfc(t) - (2/sqrt(pi)).

## 4. Local zenith angle and density gradients

The engine's density trait returns Ne plus its coordinate partials
(d/dr, d/dtheta, d/dphi). Unlike the spherically-symmetric engine profiles this
layer varies horizontally, so the theta/phi partials are non-zero and must be
supplied for the ray equations to stay on-shell (a value that varies without a
matching gradient would drive Hamiltonian drift).

Solar geometry (solar.rs): with geographic latitude `lat = pi/2 - theta`
(theta the colatitude), declination `delta`, and hour angle
`H = phi + (pi/12) t_utc - pi`,

    cos chi = cos theta sin delta + sin theta cos delta cos H                 (5)

(sin lat = cos theta, cos lat = sin theta). Then

    d(cos chi)/dtheta = -sin theta sin delta + cos theta cos delta cos H
    d(cos chi)/dphi   = -sin theta cos delta sin H
    dchi/dq = -(1/sin chi) d(cos chi)/dq                                      (6)

with the sin chi -> 0 subsolar/antisolar points guarded (the horizontal
gradient is zero there by symmetry, matching the code's pole guards).

Writing `f = 1/2 (1 - z - Ch e^{-z})` so `Ne = Nm e^f`,

    dNe/dr     = Ne * 1/2 (Ch e^{-z} - 1) / H          (as in (1), sec -> Ch)
    dNe/dchi   = Ne * ( -1/2 e^{-z} ) * dCh/dchi                              (7)
    dNe/dtheta = dNe/dchi * dchi/dtheta
    dNe/dphi   = dNe/dchi * dchi/dphi

with `dCh/dchi` obtained by differentiating (3a)/(3b) and using (4):

  chi <= 90 deg, t = sqrt(X/2) cos chi:
    dCh/dchi = sqrt(pi X/2) * erfcx'(t) * ( -sqrt(X/2) sin chi )              (8a)

  chi > 90 deg, t = sqrt(X/2) |cos chi|:
    dCh/dchi = sqrt(2 pi X) e^{X(1-sin chi)} cos chi
                 * ( 1/(2 sqrt(sin chi)) - X sqrt(sin chi) )
               - sqrt(pi X/2) * erfcx'(t) * ( sqrt(X/2) sin chi )            (8b)

All partials are checked against central finite differences in the module
tests, both modes of (3) and across the 90 deg branch.

## 5. Numerical night guard

For chi well past 90 deg the `e^{X(1-sin chi)}` term in (3b) overflows f64
(argument exceeds ~709 near chi ~ 100 deg at X = 1076). By then `Ch e^{-z}`
already dwarfs everything and Ne is 0 to machine precision, so the layer
returns vacuum once the exponent `f` underflows (`f < -700`) or the grazing
term overflows. This is the correct physics - no ionising flux reaches the
deep-night D region - reached without a hand-set zenith threshold.
