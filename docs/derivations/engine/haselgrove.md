# Haselgrove ray equations in spherical coordinates

- **Crate:** `crates/skipzone`
- **Implemented in:** `src/hamiltonian.rs::RayEquations::rhs`, `src/hamiltonian.rs::RayEquations::hamiltonian`, `src/hamiltonian.rs::RayEquations::initial_state`, `src/trace.rs::Tracer::trace`
- **Tested by:** `tests/analytic_field_free.rs::bouguer_invariant_and_apex_condition_along_ray`, `tests/magnetoionic_3d.rs::reciprocity_forward_backward`, `tests/magnetoionic_3d.rs::spitze_meridian_ray_fails_typed`, `tests/absorption.rs::field_free_absorption_matches_quadrature`
- **Section anchors used by code:** §1, §2, §3, §4, §5, §6

Symbols and sign conventions: [conventions.md](../conventions.md#5-symbols).

This file derives the ray equations the tracer integrates. Section 1 obtains
Hamilton's equations in spherical coordinates for any dispersion function.
Section 2 picks the Hamiltonian and rescales the variables so every right-hand
side is of order one. Sections 3 and 4 add the path integrals the tracer
reports, section 5 puts the launch state on the dispersion shell, and
section 6 explains why the equations stay well conditioned at reflection,
except at one configuration where they cannot.

## 1 Canonical variables

Write the ray phase as $\psi = S(\mathbf{r}) - \omega t$ with wave vector
$\mathbf{k} = \nabla S$. With the line element of
[conventions.md §1](../conventions.md#1-coordinates),

$$
\mathbf{k}\cdot d\mathbf{l} = k_r\,dr + (r k_\theta)\,d\theta + (r\sin\theta\, k_\varphi)\,d\varphi ,
$$

so the momenta conjugate to $q = (r, \theta, \varphi)$ are

$$
p = (p_r, p_\theta, p_\varphi) = (k_r,\; r k_\theta,\; r\sin\theta\, k_\varphi),
$$

where $(k_r, k_\theta, k_\varphi)$ are the physical components on
$(\hat{r}, \hat{\theta}, \hat{\varphi})$.

For any dispersion function $D(q, \mathbf{k}) = 0$, Hamilton's equations in
$(q, p)$ with parameter $\tau$ are $dq/d\tau = \partial D/\partial p$ and
$dp/d\tau = -\partial D/\partial q$. Expressing $D$ in physical components and
transforming $p \to \mathbf{k}$ needs only the chain rule. The map between $p$
and $\mathbf{k}$ carries all of the basis rotation, so no covariant
derivatives appear. The result is

$$
\begin{aligned}
\frac{dr}{d\tau} &= D_{k_r}, &
\frac{dk_r}{d\tau} &= -D_r + \frac{k_\theta D_{k_\theta} + k_\varphi D_{k_\varphi}}{r}, \\
\frac{d\theta}{d\tau} &= \frac{D_{k_\theta}}{r}, &
\frac{dk_\theta}{d\tau} &= \frac{1}{r}\left[-D_\theta + k_\varphi D_{k_\varphi}\cot\theta - k_\theta D_{k_r}\right], \\
\frac{d\varphi}{d\tau} &= \frac{D_{k_\varphi}}{r\sin\theta}, &
\frac{dk_\varphi}{d\tau} &= \frac{1}{r}\left[-\frac{D_\varphi}{\sin\theta} - k_\varphi D_{k_r} - k_\varphi D_{k_\theta}\cot\theta\right].
\end{aligned}
$$

Here $D_q$ are coordinate partials at fixed *physical* components of
$\mathbf{k}$, and $D_{k}$ are partials at fixed position.

<details><summary>Derivation of the momentum equations</summary>

Substitute $k_\theta = p_\theta/r$ and $k_\varphi = p_\varphi/(r\sin\theta)$
into $D$ and differentiate. For the $\theta$ equation,

$$
\frac{dp_\theta}{d\tau} = -\left.\frac{\partial D}{\partial\theta}\right|_p = -D_\theta + D_{k_\varphi} k_\varphi\cot\theta ,
$$

while differentiating $p_\theta = r k_\theta$ along the ray gives
$dp_\theta/d\tau = (dr/d\tau)\,k_\theta + r\,dk_\theta/d\tau$. Equating the two
and using $dr/d\tau = D_{k_r}$ gives the $k_\theta$ equation. The $k_r$ and
$k_\varphi$ equations follow in the same way.

</details>

## 2 The Hamiltonian and the working variables

The tracer uses

$$
\mathcal{H}(q, \mathbf{m}) = \frac12\left[\mathbf{m}\cdot\mathbf{m} - \mathrm{Re}\, n^2(q, \hat{\mathbf{m}})\right],
\qquad
\mathbf{m} = \frac{c}{\omega}\mathbf{k}.
$$

$n^2$ is the complex Appleton–Hartree index of
[appleton-hartree.md](appleton-hartree.md). The ray path uses its real part.
This is the real-ray approximation of Jones and Stephenson: collisions perturb
the path at $O(Z^2)$ but attenuate the wave at $O(Z)$. On shell
$\lvert\mathbf{m}\rvert = n$, because $n^2$ depends on $\mathbf{m}$ only
through its direction.

With $\sigma = (c/\omega)\tau$ the equations of section 1 keep their form with
$\mathbf{k} \to \mathbf{m}$, and every right-hand side is of order one.
$\sigma$ has units of metres. The rate $d(\text{position})/d\sigma$ is
$\mathbf{v}$, with $\lvert\mathbf{v}\rvert = n$ in an isotropic medium, so
there $\sigma$ is arc length divided by $n$.

The ray direction is

$$
v_i = \frac{\partial\mathcal{H}}{\partial m_i}
= m_i - \frac12\,\frac{\partial n^2}{\partial\cos\Theta}\,\frac{b_i - \cos\Theta\,\hat{m}_i}{\lvert\mathbf{m}\rvert},
$$

using $\partial\cos\Theta/\partial m_i = (b_i - \cos\Theta\,\hat{m}_i)/\lvert\mathbf{m}\rvert$
for $\cos\Theta = \mathbf{m}\cdot\mathbf{B}/(\lvert\mathbf{m}\rvert\lvert\mathbf{B}\rvert)$
and $\hat{b} = \mathbf{B}/\lvert\mathbf{B}\rvert$. The position and momentum
equations are

$$
\begin{aligned}
\frac{dr}{d\sigma} &= v_r, &
\frac{dm_r}{d\sigma} &= \tfrac12 G_r + \frac{m_\theta v_\theta + m_\varphi v_\varphi}{r}, \\
\frac{d\theta}{d\sigma} &= \frac{v_\theta}{r}, &
\frac{dm_\theta}{d\sigma} &= \frac{1}{r}\left[\tfrac12 G_\theta + m_\varphi v_\varphi\cot\theta - m_\theta v_r\right], \\
\frac{d\varphi}{d\sigma} &= \frac{v_\varphi}{r\sin\theta}, &
\frac{dm_\varphi}{d\sigma} &= \frac{1}{r}\left[\frac{G_\varphi}{2\sin\theta} - m_\varphi v_r - m_\varphi v_\theta\cot\theta\right],
\end{aligned}
$$

with $G_q = \partial(\mathrm{Re}\, n^2)/\partial q$ at fixed physical
$\mathbf{m}$. The chain rule through the four magnetoionic parameters gives

$$
\begin{aligned}
G_q &= \mathrm{Re}\left[
\frac{\partial n^2}{\partial X}\frac{\partial X}{\partial q}
+ \frac{\partial n^2}{\partial Y}\frac{\partial Y}{\partial q}
+ \frac{\partial n^2}{\partial Z}\frac{\partial Z}{\partial q}
+ \frac{\partial n^2}{\partial\cos\Theta}\frac{\partial\cos\Theta}{\partial q}\right], \\
\frac{\partial X}{\partial q} &= \frac{e^2}{\varepsilon_0 m_e\omega^2}\frac{\partial N_e}{\partial q}, \qquad
\frac{\partial Y}{\partial q} = \frac{e}{m_e\omega}\,\hat{b}\cdot\frac{\partial\mathbf{B}}{\partial q}, \qquad
\frac{\partial Z}{\partial q} = \frac{1}{\omega}\frac{\partial\nu}{\partial q}, \\
\frac{\partial\cos\Theta}{\partial q} &= \frac{1}{\lvert\mathbf{B}\rvert}\left[\hat{\mathbf{m}}\cdot\frac{\partial\mathbf{B}}{\partial q} - \cos\Theta\;\hat{b}\cdot\frac{\partial\mathbf{B}}{\partial q}\right].
\end{aligned}
$$

$\partial\mathbf{B}/\partial q$ is column $q$ of the field model's component
Jacobian ([magnetic-field.md §2](magnetic-field.md#2-coordinate-jacobian)).
$\partial N_e/\partial q$ and $\partial\nu/\partial q$ are the coordinate
partials the density and collision models return.

In zero field $\mathbf{v} = \mathbf{m}$ exactly and $G_q$ keeps only its $X$
term. The field-free tracer is this same system with $Y = 0$, so it shares
every line of code with the magnetised one.

## 3 Group delay from the extended phase space

Group delay comes from treating $(t, -\omega)$ as one more conjugate pair:
$dt/d\tau = -\partial\mathcal{H}/\partial\omega$ at fixed $\mathbf{k}$. At
fixed $\mathbf{k}$ the scaled vector $\mathbf{m} = c\mathbf{k}/\omega$ varies
as $\omega^{-1}$, so

$$
\left.\frac{\partial\mathcal{H}}{\partial\omega}\right|_{\mathbf{k}}
= -\frac{1}{\omega}\left[\mathbf{m}\cdot\mathbf{m} + \frac{\omega}{2}\frac{\partial(\mathrm{Re}\, n^2)}{\partial\omega}\right].
$$

$n^2$ depends on $\omega$ through $X \propto \omega^{-2}$,
$Y \propto \omega^{-1}$ and $Z \propto \omega^{-1}$. Hence

$$
\frac{\omega}{2}\frac{\partial n^2}{\partial\omega}
= -\left[\frac{\partial n^2}{\partial X}X + \frac12\frac{\partial n^2}{\partial Y}Y + \frac12\frac{\partial n^2}{\partial Z}Z\right],
$$

and the group path $P' = ct$ accumulates in $\sigma$ as

$$
\frac{dP'}{d\sigma} = \mathbf{m}\cdot\mathbf{m} - \mathrm{Re}\left[\frac{\partial n^2}{\partial X}X + \frac12\frac{\partial n^2}{\partial Y}Y + \frac12\frac{\partial n^2}{\partial Z}Z\right].
$$

Two limits confirm the sign. In vacuum $dP'/d\sigma = 1$. In an isotropic
collisionless plasma $dP'/d\sigma = n^2 + X = 1$. With
$ds = n\,d\sigma$ this is the classical group index $\mu' = 1/\mu$.

## 4 Phase path, absorption, arc length

The remaining integrands are

$$
\frac{dP}{d\sigma} = \mathbf{m}\cdot\mathbf{v},
\qquad
\frac{dA}{d\sigma} = \frac{\omega}{c}\,\mathrm{Im}(n)\,\lvert\mathbf{v}\rvert,
\qquad
\frac{ds}{d\sigma} = \lvert\mathbf{v}\rvert .
$$

The phase path is $(c/\omega)\,\mathbf{k}\cdot d\mathbf{r}/d\sigma$. In the
absorption integrand $n$ is the principal square root of the complex $n^2$.
With $Z = 0$ the code sets the absorption integrand to exactly zero, which
[appleton-hartree.md §6](appleton-hartree.md#6-conditioning-and-known-degeneracies)
shows is also what the arithmetic gives. Absorption is in nepers, and the field
amplitude decays as $e^{-A}$.

The tracer computes no Doppler shift. Every medium in the workspace is static,
so the shift is zero. A caller that wants one differentiates the phase path
$P$ across model epochs.

## 5 Initialization and termination

The launch state is placed exactly on the dispersion shell, so that
$\mathcal{H}$ starts at zero and its drift measures integrator error.

At the launch point $q_0$ the elevation and azimuth give a unit wave normal
$\hat{\mathbf{k}}$. With $\cos\Theta$ from $\hat{\mathbf{k}}$ and $\hat{b}$,
evaluate $n_0^2 = \mathrm{Re}\, n^2$. If $n_0^2 \le 0$ the mode is
evanescent at the source and the tracer returns the typed error
`EvanescentLaunch`. Otherwise $\mathbf{m} = \sqrt{n_0^2}\,\hat{\mathbf{k}}$.
This is on shell with no iteration, because $n$ depends only on the direction
of $\mathbf{m}$.

Within `SIN_COLAT_MIN` of a coordinate pole the $\cot\theta$ and
$1/\sin\theta$ terms amplify rounding beyond any useful tolerance, and the
tracer returns `PoleProximity`.

## 6 Conditioning near reflection

At an O-mode apex $n^2 \to 0$, so $\mathbf{v} \to \mathbf{m} \to 0$. The
position rate $d(\text{position})/d\sigma$ goes to zero while
$d\mathbf{m}/d\sigma \to \tfrac12\nabla n^2$ stays finite. The trajectory is
smooth in $\sigma$. This is why the tracer must not reparametrise by arc
length: $d\mathbf{k}/ds$ diverges at the apex like $1/n$. Five consequences
follow.

- **Absorption integrand.** $\mathrm{Im}\, n \approx \mathrm{Im}\, n^2/(2\mathrm{Re}\, n)$
  diverges at the apex, but $dA/d\sigma$ multiplies it by
  $\lvert\mathbf{v}\rvert \approx \mathrm{Re}\, n$. The product stays
  finite, which matches the physics: deviative absorption at reflection is
  finite.
- **Group integrand.** $dP'/d\sigma \to X = 1$ at an isotropic apex, which is
  finite.
- **Step control.** The step tightens near the apex through the error norm on
  $\mathbf{m}$. Nothing in the code special-cases the apex.
- **Evanescent overshoot.** Trial Runge–Kutta stages may land slightly beyond
  the turning point, where $\mathrm{Re}\, n^2 < 0$. The collisionless
  $n^2$ is real and analytic through zero, so those stage evaluations are well
  defined and the Hamiltonian flow pulls the ray back.
- **The Spitze.** A near-vertical O-mode ray in the magnetic meridian plane has
  its wave normal refracted toward the field as $X \to 1$. It arrives at the
  $(X = 1, \Theta = 0)$ degeneracy of
  [appleton-hartree.md §6](appleton-hartree.md#6-conditioning-and-known-degeneracies).
  There the physical ray has a cusp, the dispersion surface is conical and
  $\partial n^2/\partial\cos\Theta$ diverges, so no smooth integration can
  pass. In a trace the position freezes while $v_r$ runs away and the step
  collapses. The tracer reports `StepSizeCollapse`, and
  `spitze_meridian_ray_fails_typed` checks that it does. Continuing the ray
  analytically through the cusp is out of scope. Only rays whose plane
  contains $\mathbf{B}$ near $X = 1$ reach this point. Off-meridian azimuths
  and the X mode, whose cutoff $X = 1 - Y$ is reached with $W$ bounded away
  from zero, are unaffected.

Exactly critical vertical incidence with a field present is degenerate in a
second way. $\lvert\mathbf{m}\rvert \to 0$ while $n^2$ depends on direction,
so $\hat{\mathbf{m}}$ is discontinuous at the apex. The tracer handles it with
the same typed failure. Field-free vertical reflection is unaffected, because
an isotropic $n^2$ does not depend on $\hat{\mathbf{m}}$.
