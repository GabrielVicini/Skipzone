# Chapman grazing-incidence function and the twilight D region

- **Crate:** `crates/skipzone-iono`
- **Implemented in:** `src/chapman.rs::erfcx`, `src/chapman.rs::erfcx_deriv`, `src/chapman.rs::chapman_grazing`, `src/chapman.rs::SolarChapmanLayer::zenith_angle`, `src/chapman.rs::SolarChapmanLayer::sample`
- **Tested by:** `src/chapman.rs::tests::erfcx_reference_values`, `src/chapman.rs::tests::erfcx_deriv_matches_fd`, `src/chapman.rs::tests::chapman_grazing_limits`, `src/chapman.rs::tests::chapman_grazing_deriv_matches_fd`, `src/chapman.rs::tests::terminator_layer_is_finite_and_thinned`, `src/chapman.rs::tests::deep_night_is_vacuum`, `src/chapman.rs::tests::horizontal_partials_match_fd`
- **Section anchors used by code:** §1, §2, §3, §4; equations (3), (4), (5), (7), (8)

Symbols and sign conventions: [conventions.md](../conventions.md#5-symbols),
including the [symbols local to this file](../conventions.md#local-to-chapman-grazingmd).

This is the ionosphere crate's derivation. `skipzone-iono` builds a
day/night-aware Chapman layer and hands it to the unchanged engine tracer
through the `ElectronDensity` trait, and `skipzone-predict` builds its D, E and
F2 layers from it. Section 1 explains why the engine's own Chapman layer is not
enough for a path that crosses the terminator. Section 2 introduces the
Chapman grazing-incidence function that replaces it, section 3 the error
function it needs, and section 4 the horizontal gradients the ray equations
need. Sections 5 and 6 describe the night floor and the partial rise.

## 1 Why the plane-parallel layer is not enough

The engine's `ChapmanLayer::with_zenith_angle` places an $\alpha$-Chapman layer

$$
N_e(z) = N_m\exp\left(\tfrac12\left(1 - z - \sec\chi\,e^{-z}\right)\right),
\qquad z = \frac{r - r_m}{H},
$$

where $\sec\chi$ is the plane-parallel slant factor for the ionising flux. It
refuses $\chi \ge 85°$ (`MAX_CHAPMAN_ZENITH_ANGLE`). $\sec\chi$ is the optical
depth factor of a flat atmosphere and diverges as $\chi \to 90°$, which is
unphysical. Through a *curved* atmosphere the slant path stays finite at the
terminator, and the layer keeps producing ionisation a few degrees past it.

A point-to-point HF path can straddle the terminator, and that has two
consequences a single midpoint $\sec\chi$ cannot represent:

- Near $\chi = 90°$ absorption has to fade smoothly. A model that switches it
  off at an 85° cliff predicts zero absorption on paths that have some.
- Different points along one ray see different solar zenith angles, so the
  layer has to depend on horizontal position.

The fix replaces $\sec\chi$ by the Chapman grazing-incidence function
$\operatorname{Ch}(x, \chi)$ and evaluates $\chi$ locally at each sampled
point.

## 2 The Chapman function

$\operatorname{Ch}(x, \chi)$ is the ratio of slant to vertical optical depth for
an exponential atmosphere on a sphere. Its argument is

$$
x = \frac{R_0 + h}{H},
$$

the geocentric radius over the neutral scale height, about 1076 at 85 km with
$H = 6$ km. The code uses the standard closed form (Smith and Smith,
J. Geophys. Res. 77, 1972), written with the scaled complementary error
function $\operatorname{erfcx}(t) = e^{t^2}\operatorname{erfc}(t)$ so that
nothing overflows:

$$
\operatorname{Ch}(x, \chi) = \sqrt{\frac{\pi x}{2}}\;\operatorname{erfcx}\left(\sqrt{\frac{x}{2}}\cos\chi\right),
\qquad \chi \le 90°,
\tag{3a}
$$

$$
\operatorname{Ch}(x, \chi) = \sqrt{2\pi x}\,\sqrt{\sin\chi}\;e^{x(1 - \sin\chi)} - \sqrt{\frac{\pi x}{2}}\;\operatorname{erfcx}\left(\sqrt{\frac{x}{2}}\,\lvert\cos\chi\rvert\right),
\qquad \chi > 90°.
\tag{3b}
$$

Three limits show that this is the right function. `chapman_grazing_limits`
checks each of them.

- **Overhead sun.** For large $x$,
  $\operatorname{erfcx}(\sqrt{x/2}) \approx 1/(\sqrt{x/2}\sqrt{\pi})$, so
  $\operatorname{Ch} \to 1 = \sec 0$. More generally
  $\operatorname{Ch} \approx \sec\chi$ below about 75°, which reproduces the
  plane-parallel layer where that layer is valid.
- **Terminator.** At $\chi = 90°$, $\cos\chi = 0$ and $\operatorname{erfcx}(0) = 1$,
  so (3a) gives $\operatorname{Ch} = \sqrt{\pi x/2}$. This is finite, about 41
  at $x = 1076$. Just past 90°, (3b) gives
  $\sqrt{2\pi x} - \sqrt{\pi x/2} = \sqrt{\pi x/2}$, because
  $\sqrt{2\pi x} = 2\sqrt{\pi x/2}$. The two branches agree and
  $\operatorname{Ch}$ is continuous.
- **Deep night.** As $\chi \to 180°$, (3b) grows without bound through
  $e^{x(1 - \sin\chi)}$. That drives $N_e \to 0$ through the
  $-\tfrac12\operatorname{Ch}\,e^{-z}$ term, so the D region disappears
  smoothly at night with no threshold.

With $\sec\chi$ replaced by $\operatorname{Ch}$, the peak of the layer sits at
$z^* = \ln\operatorname{Ch}$, height $r_m + H\ln\operatorname{Ch}$, with peak
density $N_m\operatorname{Ch}^{-1/2}$. At the terminator that is
$N_m/\sqrt{41} \approx 0.16\,N_m$ at $r_m + H\ln 41$: a thinner, raised layer
that is still present (`terminator_layer_is_finite_and_thinned`).

The argument is held constant at $x = r_m/H$, the value at the reference
height. It varies by less than 0.5 % across the layer, so
$\partial\operatorname{Ch}/\partial r = 0$ and $\operatorname{Ch}$ depends on
position only through $\chi$.

## 3 Scaled complementary error function

$\operatorname{erfcx}(t)$ is needed only for $t \ge 0$, since the argument in
(3) is never negative. The code computes it without any external crate.

- For $t < 3$ it uses $\operatorname{erfcx}(t) = e^{t^2}\left(1 - \operatorname{erf}(t)\right)$
  with the Maclaurin series

  $$
  \operatorname{erf}(t) = \frac{2}{\sqrt{\pi}}\sum_{n \ge 0}\frac{(-1)^n\,t^{2n+1}}{n!\,(2n+1)} .
  $$

  Here $e^{t^2} \le e^9$, so nothing overflows, and the cancellation near
  $t = 3$ still leaves more than 12 correct digits.
- For $t \ge 3$ it uses the continued fraction of Abramowitz and Stegun 7.1.14,

  $$
  \sqrt{\pi}\,\operatorname{erfcx}(t) = \cfrac{1}{t + \cfrac{1/2}{t + \cfrac{2/2}{t + \cfrac{3/2}{t + \cdots}}}} ,
  $$

  evaluated to 48 levels by the forward continuant recurrence
  $A_k = tA_{k-1} + \tfrac{k}{2}A_{k-2}$ (and the same for $B_k$), whose ratio
  is the $k$-th convergent. Every term is positive, so there is no
  cancellation. `erfcx_forward_recurrence_matches_backward` compares it with
  backward evaluation.

`erfcx_reference_values` checks both branches against
$\operatorname{erfcx}(0) = 1$, $\operatorname{erfcx}(1) = 0.427583\ldots$,
$\operatorname{erfcx}(2) = 0.255396\ldots$ and
$\operatorname{erfcx}(5) = 0.110704\ldots$.

The horizontal gradient of section 4 needs the derivative. From
$\tfrac{d}{dt}\left(e^{t^2}\operatorname{erfc}t\right) = 2t\,e^{t^2}\operatorname{erfc}t - 2/\sqrt{\pi}$,

$$
\operatorname{erfcx}'(t) = 2t\,\operatorname{erfcx}(t) - \frac{2}{\sqrt{\pi}} .
\tag{4}
$$

## 4 Local zenith angle and density gradients

The engine's density trait returns $N_e$ with its coordinate partials in $r$,
$\theta$ and $\varphi$. The engine's own profiles are spherically symmetric,
but this layer varies horizontally, so the $\theta$ and $\varphi$ partials are
nonzero. The ray equations need them to stay on shell: a density that varies
without a matching gradient would make the Hamiltonian drift.

The solar geometry uses the latitude $\pi/2 - \theta$, the declination
$\delta$ and the hour angle $\eta = \varphi + (\pi/12)\,t_\text{UTC} - \pi$.
Since $\sin(\text{lat}) = \cos\theta$ and $\cos(\text{lat}) = \sin\theta$,

$$
\cos\chi = \cos\theta\sin\delta + \sin\theta\cos\delta\cos\eta .
\tag{5}
$$

Its partials are

$$
\frac{\partial\cos\chi}{\partial\theta} = -\sin\theta\sin\delta + \cos\theta\cos\delta\cos\eta,
\qquad
\frac{\partial\cos\chi}{\partial\varphi} = -\sin\theta\cos\delta\sin\eta,
\qquad
\frac{\partial\chi}{\partial q} = -\frac{1}{\sin\chi}\frac{\partial\cos\chi}{\partial q} .
$$

At the subsolar and antisolar points $\sin\chi \to 0$. The horizontal gradient
is zero there by symmetry, and the code returns zero once $\sin\chi < 10^{-9}$.

Write $f = \tfrac12\left(1 - z - \operatorname{Ch}e^{-z}\right)$, so that
$N_e = N_m e^f$. Then

$$
\frac{\partial N_e}{\partial r} = \frac{N_e}{2H}\left(\operatorname{Ch}e^{-z} - 1\right),
\qquad
\frac{\partial N_e}{\partial\chi} = -\frac12\,N_e\,e^{-z}\,\frac{\partial\operatorname{Ch}}{\partial\chi},
\tag{7}
$$

and $\partial N_e/\partial\theta$ and $\partial N_e/\partial\varphi$ follow by
the chain rule through $\partial\chi/\partial q$. When the peak density
$N_m(\theta, \varphi)$ comes from a map, its own gradient adds the product-rule
term $(\partial N_m/\partial q)\,e^f$.

Differentiating (3a) and (3b) with (4) gives
$\partial\operatorname{Ch}/\partial\chi$. Below 90°, with
$t = \sqrt{x/2}\cos\chi$,

$$
\frac{\partial\operatorname{Ch}}{\partial\chi} = \sqrt{\frac{\pi x}{2}}\;\operatorname{erfcx}'(t)\left(-\sqrt{\frac{x}{2}}\sin\chi\right),
\qquad \chi \le 90°.
\tag{8a}
$$

Above 90°, with $t = \sqrt{x/2}\,\lvert\cos\chi\rvert$,

$$
\frac{\partial\operatorname{Ch}}{\partial\chi} = \sqrt{2\pi x}\;e^{x(1 - \sin\chi)}\cos\chi\left(\frac{1}{2\sqrt{\sin\chi}} - x\sqrt{\sin\chi}\right) - \sqrt{\frac{\pi x}{2}}\;\operatorname{erfcx}'(t)\left(\sqrt{\frac{x}{2}}\sin\chi\right),
\qquad \chi > 90°.
\tag{8b}
$$

`chapman_grazing_deriv_matches_fd` and `horizontal_partials_match_fd` compare
these with central finite differences, on both branches of (3) and across
90°.

## 5 Night guard and night floor

Well past 90° the factor $e^{x(1 - \sin\chi)}$ in (3b) overflows `f64`. The
exponent passes about 709 near $\chi \approx 100°$ at $x = 1076$. By then
$\operatorname{Ch}e^{-z}$ dominates everything and the sunlit layer is zero to
machine precision. `chapman_grazing` returns $\operatorname{Ch} = +\infty$ once
the exponent passes 700, and the layer drops its photochemical part once
$f < -700$.

What remains is the night floor, set with
`SolarChapmanLayer::with_night_floor`. It is a residual $\alpha$-Chapman
profile with the same peak height and no zenith dependence. It stands in for
the ionisation by cosmic rays and scattered Lyman-$\alpha$ that a Chapman layer
does not describe. The layer returns the larger of the sunlit profile and the
floor, so a fully sunlit layer is unchanged
(`the_night_floor_replaces_an_exactly_zero_night`). With no floor set, the
deep-night layer is vacuum (`deep_night_is_vacuum`).

## 6 Partial rise

`SolarChapmanLayer::with_rise_fraction(f)` keeps only a fraction $f$ of the
upward translation $H\ln\operatorname{Ch}$. With $u = z - f\ln\operatorname{Ch}$
the layer is

$$
N_e = N_m\operatorname{Ch}^{-1/2}\exp\left(\tfrac12\left(1 - u - e^{-u}\right)\right).
$$

At $f = 1$ the prefactor and the shift inside $u$ cancel, and this is the
layer of section 2 (`partial_rise_reduces_to_the_textbook_layer_at_f_one`).
The physical reason for $f < 1$, the absorption zenith law it implies and the
derivative are in the rustdoc of `with_rise_fraction` and
`sample_partial_rise`.
