# Geomagnetic field from a scalar potential

- **Crate:** `crates/skipzone`
- **Implemented in:** `src/mag/igrf.rs::IgrfModel::sample`, `src/mag/legendre.rs::schmidt`, `src/mag/dipole.rs::Dipole::sample`, `src/mag/mod.rs::div_curl`
- **Tested by:** `src/mag/legendre.rs::tests::low_degree_closed_forms`, `src/mag/legendre.rs::tests::schmidt_normalisation`, `src/mag/legendre.rs::tests::derivatives_match_finite_differences`, `src/mag/igrf.rs::tests::parses_and_field_is_potential`, `src/mag/igrf.rs::tests::jacobian_matches_finite_differences`, `src/mag/igrf.rs::tests::degree_one_truncation_matches_analytic_dipole`, `src/mag/dipole.rs::tests::axial_magnitude_matches_closed_form`, `src/mag/dipole.rs::tests::tilted_dipole_is_divergence_and_curl_free`
- **Section anchors used by code:** §1, §2, §3, §4, §5

Symbols and sign conventions: [conventions.md](../conventions.md#5-symbols).

The ray equations need the geomagnetic field and its full coordinate Jacobian
at every step. This file derives both from the IGRF spherical-harmonic
expansion (sections 1 and 2), the recurrences that evaluate the Legendre
functions and their derivatives (section 3), the closed-form dipole used as a
reference (section 4), and the identities the tests use to check the Jacobian
(section 5).

## 1 Potential and components

Above the Earth's surface the region is free of currents, so
$\nabla\times\mathbf{B} = 0$ and $\nabla\cdot\mathbf{B} = 0$. The field is then
$\mathbf{B} = -\nabla V$ with $\nabla^2 V = 0$. The solution of Laplace's
equation for internal sources that is regular at infinity is the expansion
that defines the IGRF:

$$
V(r, \theta, \varphi) = a\sum_{n=1}^{N}\left(\frac{a}{r}\right)^{n+1}\sum_{m=0}^{n}
\left[g_n^m\cos m\varphi + h_n^m\sin m\varphi\right] S_n^m(\theta).
$$

Here $a = 6371.2$ km is the reference radius, part of the model definition,
and $S_n^m$ are the **Schmidt semi-normalised** associated Legendre functions
of section 3. The Gauss coefficients $g_n^m, h_n^m$ are in nT.

The components on $(\hat{r}, \hat{\theta}, \hat{\varphi})$ follow from
$\mathbf{B} = -\nabla V$ with
$\nabla = \left(\partial_r,\; r^{-1}\partial_\theta,\; (r\sin\theta)^{-1}\partial_\varphi\right)$
and $\tfrac{d}{dr}(a/r)^{n+1} = -\tfrac{n+1}{r}(a/r)^{n+1}$:

$$
\begin{aligned}
B_r &= -\frac{\partial V}{\partial r} = \sum_n (n+1)\left(\frac{a}{r}\right)^{n+2}\sum_m\left[g_n^m\cos m\varphi + h_n^m\sin m\varphi\right]S_n^m, \\
B_\theta &= -\frac{1}{r}\frac{\partial V}{\partial\theta} = -\sum_n\left(\frac{a}{r}\right)^{n+2}\sum_m\left[g_n^m\cos m\varphi + h_n^m\sin m\varphi\right]\frac{dS_n^m}{d\theta}, \\
B_\varphi &= -\frac{1}{r\sin\theta}\frac{\partial V}{\partial\varphi} = \frac{1}{\sin\theta}\sum_n\left(\frac{a}{r}\right)^{n+2}\sum_m m\left[g_n^m\sin m\varphi - h_n^m\cos m\varphi\right]S_n^m .
\end{aligned}
$$

The signs can be checked against the real Earth. Its $g_1^0$ is negative, so at
the north geographic pole $B_r = 2(a/r)^3 g_1^0 < 0$ and the field points
down, as observed.

## 2 Coordinate Jacobian

The ray equations need $\partial B_i/\partial(r, \theta, \varphi)$ analytically,
for the chain rule through $Y$ and the field direction
([haselgrove.md §2](haselgrove.md#2-the-hamiltonian-and-the-working-variables)).
Each component is a finite sum of terms of the form
$(a/r)^{n+2}\times(\text{trigonometric in }\varphi)\times(S,\ dS/d\theta\ \text{or}\ S/\sin\theta)$,
so the partials follow term by term:

- $\partial/\partial r$ multiplies each degree-$n$ term by $-(n+2)/r$.
- $\partial/\partial\varphi$ maps $\cos m\varphi \leftrightarrow \sin m\varphi$ with factors $\mp m$.
- $\partial/\partial\theta$ needs $dS/d\theta$ and $d^2S/d\theta^2$ from
  section 3, and for $B_\varphi$
  $\dfrac{d}{d\theta}\dfrac{S}{\sin\theta} = \dfrac{dS/d\theta}{\sin\theta} - \dfrac{S\cos\theta}{\sin^2\theta}$.

The $1/\sin\theta$ factors are finite for $m \ge 1$ because
$S_n^m \propto \sin^m\theta$. The code evaluates them literally, so it is
**not valid at the coordinate poles**. The tracer's pole guard
(`SIN_COLAT_MIN` in `src/hamiltonian.rs`) keeps rays away from them.

## 3 Schmidt semi-normalised Legendre functions

Let $\hat{P}_n^m$ be the unnormalised associated Legendre functions *without*
the Condon–Shortley phase, the geomagnetic convention. The Schmidt functions
are

$$
S_n^m = \kappa_m\sqrt{\frac{(n-m)!}{(n+m)!}}\,\hat{P}_n^m,
\qquad
\kappa_0 = 1,\quad \kappa_m = \sqrt{2}\ \ (m \ge 1),
$$

normalised so that
$\iint\left[S_n^m\cos m\varphi\right]^2 d\Omega = 4\pi/(2n+1)$, which
`schmidt_normalisation` checks.

The recurrences come from the unnormalised ones,
$\hat{P}_m^m = (2m-1)!!\,\sin^m\theta$ and
$(n-m)\hat{P}_n^m = (2n-1)\cos\theta\,\hat{P}_{n-1}^m - (n+m-1)\hat{P}_{n-2}^m$,
by inserting the normalisation factors.

**Diagonal.** $S_m^m = \kappa_m\sqrt{1/(2m)!}\,(2m-1)!!\,\sin^m\theta$. For
$m \ge 2$ the ratio of successive diagonal terms is

$$
\frac{S_m^m}{S_{m-1}^{m-1}} = \sin\theta\,(2m-1)\sqrt{\frac{(2m-2)!}{(2m)!}} = \sin\theta\sqrt{\frac{2m-1}{2m}} .
$$

At $m = 1$ the factor $\kappa_1 = \sqrt{2}$ cancels $\sqrt{1/2!}$, so
$S_1^1 = \sin\theta$. The diagonal is therefore

$$
S_0^0 = 1,
\qquad
S_1^1 = \sin\theta,
\qquad
S_m^m = \sin\theta\sqrt{\frac{2m-1}{2m}}\,S_{m-1}^{m-1}\quad(m \ge 2).
$$

**Vertical.** Multiply the three-term recurrence by
$N_n^m = \kappa_m\sqrt{(n-m)!/(n+m)!}$ and use
$N_n^m/N_{n-1}^m = \sqrt{(n-m)/(n+m)}$ and
$N_n^m/N_{n-2}^m = \sqrt{(n-m)(n-m-1)/((n+m)(n+m-1))}$:

$$
S_n^m = \frac{(2n-1)\cos\theta\,S_{n-1}^m - \sqrt{(n-1)^2 - m^2}\;S_{n-2}^m}{\sqrt{n^2 - m^2}}
\qquad(n > m),
$$

with $S_{m-1}^m = 0$. That seed covers the row $n = m + 1$ exactly, because the
square-root factor multiplies zero.

The low-degree functions expand by hand to

$$
S_1^0 = \cos\theta,\quad
S_2^0 = \tfrac12(3\cos^2\theta - 1),\quad
S_2^1 = \sqrt{3}\sin\theta\cos\theta,\quad
S_2^2 = \tfrac{\sqrt{3}}{2}\sin^2\theta,\quad
S_3^1 = \tfrac{\sqrt{6}}{4}(5\cos^2\theta - 1)\sin\theta,
$$

and `low_degree_closed_forms` asserts each of them.

**$\theta$-derivatives.** The recurrences are linear, with coefficients built
from $\sin\theta$, $\cos\theta$ and constants. Differentiating them once and
twice gives exact recurrences for $S' = dS/d\theta$ and $S'' = d^2S/d\theta^2$,
carried alongside $S$. With $s = \sin\theta$, $x = \cos\theta$ and
$\alpha_m = \sqrt{(2m-1)/(2m)}$ for $m \ge 2$, $\alpha_1 = 1$:

$$
\begin{aligned}
\text{diagonal:}\quad S'_m &= \alpha_m\left(x\,S_{m-1} + s\,S'_{m-1}\right), \\
S''_m &= \alpha_m\left(-s\,S_{m-1} + 2x\,S'_{m-1} + s\,S''_{m-1}\right), \\
\text{vertical:}\quad S'_n &= \frac{(2n-1)\left(-s\,S_{n-1} + x\,S'_{n-1}\right) - \beta\,S'_{n-2}}{\gamma}, \\
S''_n &= \frac{(2n-1)\left(-x\,S_{n-1} - 2s\,S'_{n-1} + x\,S''_{n-1}\right) - \beta\,S''_{n-2}}{\gamma},
\end{aligned}
$$

with $\beta = \sqrt{(n-1)^2 - m^2}$ and $\gamma = \sqrt{n^2 - m^2}$. The
$\sqrt{2}$ cancellation at $m = 1$ applies to all three diagonal recurrences,
which is why $\alpha_1 = 1$. `derivatives_match_finite_differences` checks both
derivatives against central finite differences.

## 4 Centered tilted dipole

The degree-one terms alone give a closed form that serves as an independent
reference. With $\Gamma(\varphi) = g_1^1\cos\varphi + h_1^1\sin\varphi$,
$S_1^0 = \cos\theta$ and $S_1^1 = \sin\theta$:

$$
\begin{aligned}
B_r &= 2\left(\frac{a}{r}\right)^3\left[g_1^0\cos\theta + \Gamma\sin\theta\right], \\
B_\theta &= \left(\frac{a}{r}\right)^3\left[g_1^0\sin\theta - \Gamma\cos\theta\right], \\
B_\varphi &= \left(\frac{a}{r}\right)^3\left[g_1^1\sin\varphi - h_1^1\cos\varphi\right].
\end{aligned}
$$

All nine coordinate partials follow by inspection: $\partial/\partial r$
multiplies by $-3/r$, and the $\theta$ and $\varphi$ derivatives act on the
brackets. In the axial case $g_1^1 = h_1^1 = 0$ the magnitude is the textbook
dipole value

$$
\lvert\mathbf{B}\rvert = \left(\frac{a}{r}\right)^3\lvert g_1^0\rvert\sqrt{1 + 3\cos^2\theta},
$$

which `axial_magnitude_matches_closed_form` asserts. The IGRF truncated to
$n = 1$ agrees with this closed form to rounding error
(`degree_one_truncation_matches_analytic_dipole`). That is a check of internal
consistency between two code paths, with no finite differencing involved.

## 5 Test invariants

A potential field has zero divergence and zero curl. In spherical coordinates,
written with the Jacobian entries,

$$
\begin{aligned}
\nabla\cdot\mathbf{B} &= \frac{\partial B_r}{\partial r} + \frac{2B_r}{r} + \frac{1}{r}\left(\frac{\partial B_\theta}{\partial\theta} + B_\theta\cot\theta\right) + \frac{1}{r\sin\theta}\frac{\partial B_\varphi}{\partial\varphi}, \\
(\nabla\times\mathbf{B})_r &= \frac{1}{r\sin\theta}\left[\frac{\partial(\sin\theta\,B_\varphi)}{\partial\theta} - \frac{\partial B_\theta}{\partial\varphi}\right], \\
(\nabla\times\mathbf{B})_\theta &= \frac{1}{r\sin\theta}\frac{\partial B_r}{\partial\varphi} - \frac{1}{r}\frac{\partial(rB_\varphi)}{\partial r}, \\
(\nabla\times\mathbf{B})_\varphi &= \frac{1}{r}\left[\frac{\partial(rB_\theta)}{\partial r} - \frac{\partial B_r}{\partial\theta}\right].
\end{aligned}
$$

Both vanish identically. The tests evaluate them at random points from the
model's own Jacobian (`div_curl`) and require zero to near machine precision.
Every Jacobian entry appears in one of the four expressions, so this exercises
all of them (`parses_and_field_is_potential`,
`tilted_dipole_is_divergence_and_curl_free`).
