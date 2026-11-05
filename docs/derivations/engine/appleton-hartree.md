# Appleton–Hartree dispersion relation

- **Crate:** `crates/skipzone`
- **Implemented in:** `src/magnetoionic.rs::appleton_hartree`, `src/constants.rs::OMEGA_P_SQ_PER_DENSITY`, `src/constants.rs::OMEGA_H_PER_TESLA`
- **Tested by:** `src/magnetoionic.rs::tests::transverse_and_longitudinal_limits`, `src/magnetoionic.rs::tests::reflection_conditions`, `src/magnetoionic.rs::tests::agrees_with_classical_form`, `src/magnetoionic.rs::tests::ordinary_stable_across_reflection`, `src/magnetoionic.rs::tests::partials_match_finite_differences`, `src/magnetoionic.rs::tests::absorption_sign_and_collisionless_reality`, `src/magnetoionic.rs::tests::zero_field_modes_bit_identical_and_isotropic`
- **Section anchors used by code:** §1, §3, §4, §6, §7

Symbols and sign conventions: [conventions.md](../conventions.md#5-symbols).
The time convention is $e^{-i\omega t}$, so $U = 1 + iZ$ and
$\mathrm{Im}\, n > 0$ means loss.

This file derives the complex refractive index of a cold magnetised electron
plasma and the forms of it the code evaluates. Sections 1 to 3 go from the
electron equation of motion to the Appleton–Hartree formula. Section 4 fixes
which root is the ordinary mode. Section 5 rewrites the formula so it stays
accurate through O-mode reflection, section 6 lists where it degenerates, and
section 7 gives the analytic partial derivatives the ray equations need.

The model assumes a cold electron plasma with no thermal corrections and
immobile ions, which holds at HF where $\omega$ is far above the ion gyro and
plasma frequencies. Collisions enter as a velocity-independent drag $\nu$, so
the Sen–Wyller generalisation is not included. The medium is taken as locally
homogeneous over a wavelength, the condition for ray optics.

## 1 Dielectric tensor

The linearised electron momentum equation, with static field
$\mathbf{B}_0 = \lvert\mathbf{B}_0\rvert\hat{b}$ and collisional drag, is

$$
m_e\frac{d\mathbf{v}}{dt} = -e\left(\mathbf{E} + \mathbf{v}\times\mathbf{B}_0\right) - m_e\nu\mathbf{v},
\qquad e > 0 .
$$

With $e^{-i\omega t}$ and $\boldsymbol{\Omega} = (e/m_e)\mathbf{B}_0$, so that
$\lvert\boldsymbol{\Omega}\rvert = \omega_H$, this becomes

$$
(\nu - i\omega)\,\mathbf{v} + \boldsymbol{\Omega}\times\mathbf{v} = -\frac{e}{m_e}\mathbf{E}.
$$

In a frame with $\hat{z}$ along $\hat{b}$ the circular components
$v_\pm = v_x \pm i v_y$ decouple from each other and from $v_z$:

$$
(\nu - i\omega \pm i\omega_H)\,v_\pm = -\frac{e}{m_e}E_\pm ,
\qquad
(\nu - i\omega)\,v_z = -\frac{e}{m_e}E_z .
$$

The current is $\mathbf{J} = -eN_e\mathbf{v}$ and the permittivity follows as in
[conventions.md §2](../conventions.md#2-time-convention-and-the-sign-of-losses).
Since $\nu - i\omega \pm i\omega_H = -i\omega(U \mp Y)$, the principal values
of the dielectric tensor are

$$
R \equiv \varepsilon_+ = 1 - \frac{X}{U - Y},
\qquad
L \equiv \varepsilon_- = 1 - \frac{X}{U + Y},
\qquad
P \equiv \varepsilon_z = 1 - \frac{X}{U}.
$$

$R$ carries the electron gyroresonance: its denominator $U - Y$ vanishes at
$\omega = \omega_H$ when $Z = 0$. The Stix combinations
$S = (R + L)/2$ and $D = (R - L)/2$ are

$$
S = \frac{U(U - X) - Y^2}{U^2 - Y^2},
\qquad
D = -\frac{XY}{U^2 - Y^2},
\qquad
RL = \frac{(U - X)^2 - Y^2}{U^2 - Y^2}.
$$

## 2 Booker quartic and its discriminant

A plane wave with $\mathbf{n} = c\mathbf{k}/\omega$ at angle $\Theta$ to
$\hat{b}$ satisfies $\mathbf{n}\times(\mathbf{n}\times\mathbf{E}) + \varepsilon\mathbf{E} = 0$.
Expanding the $3\times 3$ determinant in the frame with $\hat{b} = \hat{z}$ and
$\mathbf{k}$ in the $x$–$z$ plane gives nontrivial solutions when

$$
\begin{aligned}
\mathcal{A}\,n^4 - \mathcal{B}\,n^2 + \mathcal{C} &= 0, \\
\mathcal{A} &= S\sin^2\Theta + P\cos^2\Theta, \\
\mathcal{B} &= RL\sin^2\Theta + PS\left(1 + \cos^2\Theta\right), \\
\mathcal{C} &= PRL .
\end{aligned}
$$

The stable root form of section 5 rests on a factorisation of the
discriminant. With $S^2 - RL = D^2$,

$$
\mathcal{B}^2 - 4\mathcal{A}\mathcal{C} = (RL - PS)^2\sin^4\Theta + 4P^2D^2\cos^2\Theta .
$$

<details><summary>Derivation of the discriminant</summary>

Write $s = \sin\Theta$, $c = \cos\Theta$ and use $(1 + c^2)^2 = (2 - s^2)^2$.
Expanding,

$$
\mathcal{B}^2 - 4\mathcal{A}\mathcal{C}
= R^2L^2s^4 + 2RLPS\,s^2(1 + c^2) + P^2S^2(1 + c^2)^2 - 4PRLS\,s^2 - 4P^2RL\,c^2 .
$$

The two middle $RLPS$ terms combine to $2PRLS\,s^2(1 + c^2 - 2) = -2PRLS\,s^4$.
The $P^2$ terms give
$P^2S^2(4 - 4s^2 + s^4) - 4P^2RL\,c^2 = P^2S^2s^4 + 4P^2c^2(S^2 - RL)$.
Collecting the $s^4$ terms into $(RL - PS)^2 s^4$ and using $S^2 - RL = D^2$
gives the result.

</details>

## 3 Reduction to Appleton–Hartree

Substitute the section 1 values into the quartic and clear the common
denominator $U(U^2 - Y^2)$. With $W = U - X$ the quartic becomes
$\mathcal{A}'n^4 - \mathcal{B}'n^2 + \mathcal{C}' = 0$ with

$$
\begin{aligned}
\mathcal{A}' &= U\left[UW - Y^2\right]\sin^2\Theta + W\left(U^2 - Y^2\right)\cos^2\Theta, \\
\mathcal{B}' &= U\left[W^2 - Y^2\right]\sin^2\Theta + W\left[UW - Y^2\right]\left(1 + \cos^2\Theta\right), \\
\mathcal{C}' &= W\left[W^2 - Y^2\right].
\end{aligned}
$$

The substitution $n^2 = 1 - \xi$ makes $\xi$ proportional to $X$ and turns the
quartic into $\mathcal{A}'\xi^2 - (2\mathcal{A}' - \mathcal{B}')\xi + (\mathcal{A}' - \mathcal{B}' + \mathcal{C}') = 0$.
The two coefficients simplify to

$$
\mathcal{A}' - \mathcal{B}' + \mathcal{C}' = WX^2,
\qquad
2\mathcal{A}' - \mathcal{B}' = X\left(2UW - Y^2\sin^2\Theta\right),
$$

so, after dividing by $X$, the quadratic for $\xi$ is

$$
\mathcal{A}'\xi^2 - X\left(2UW - Y^2\sin^2\Theta\right)\xi + X^2W = 0 .
$$

<details><summary>Derivation of the two coefficients</summary>

Write $s = \sin\Theta$, $c = \cos\Theta$. The $s^2$ and $c^2$ pieces of $UX$
combine, and $UX - UW = U(2X - U)$, so

$$
\mathcal{A}' - \mathcal{B}' = UXs^2W + W\left[UXc^2 - UW + Y^2\right] = W\left[2UX - U^2 + Y^2\right].
$$

Adding $\mathcal{C}' = W(W^2 - Y^2)$ gives
$W\left[2UX - U^2 + (U - X)^2\right] = WX^2$.

For the linear coefficient, $U + X - W = 2X$ and
$Us^2 + Wc^2 - W = (U - W)s^2 = Xs^2$, so

$$
2\mathcal{A}' - \mathcal{B}' = UW(U + X - W) - Y^2\left(Us^2 + Wc^2 - W\right) = 2UWX - XY^2s^2 .
$$

</details>

Its discriminant factorises in the same way as in section 2:

$$
X^2\left[\left(2UW - Y^2\sin^2\Theta\right)^2 - 4\mathcal{A}'W\right]
= X^2\left[Y^4\sin^4\Theta + 4W^2Y^2\cos^2\Theta\right].
$$

Take the root in product form,
$\xi = 2X^2W\big/\left(X\left[(2UW - Y^2\sin^2\Theta) \mp \sqrt{\cdots}\,\right]\right)$,
which avoids cancellation in the numerator. With $Y_T = Y\sin\Theta$ and
$Y_L = Y\cos\Theta$ this is the Appleton–Hartree formula:

$$
n^2 = 1 - \frac{X(U - X)}{U(U - X) - \tfrac12 Y_T^2 \pm \sqrt{\tfrac14 Y_T^4 + Y_L^2(U - X)^2}} .
$$

Dividing numerator and denominator by $W$, which is legitimate for real
$W > 0$, recovers the classical textbook form

$$
n^2 = 1 - \frac{X}{U - \dfrac{Y_T^2}{2(U - X)} \pm \sqrt{\dfrac{Y_T^4}{4(U - X)^2} + Y_L^2}} .
$$

The first form stays meaningful at $W = 0$ and is the one the code builds on.
`agrees_with_classical_form` checks the two against each other away from
$W = 0$.

## 4 Mode labels and branch conventions

The square root is the **principal branch**, with non-negative real part. For a
non-negative real argument this is the ordinary real root. When $Z = 0$ the
argument $\tfrac14 Y_T^4 + Y_L^2W^2$ is real and non-negative for **every**
$X$, because $W$ appears squared. The collisionless index is therefore real
everywhere, with no branch ambiguity.

The labels are anchored by limits, and each limit is a unit test:

- The `+` root is the **ordinary (O)** mode. At exactly transverse propagation
  ($Y_L = 0$) it gives $n^2 = 1 - X/U$: the wave with $\mathbf{E}$ along
  $\mathbf{B}_0$, which does not feel the field. Without collisions it
  vanishes at $X = 1$, reflection at the plasma frequency.
- The `-` root is the **extraordinary (X)** mode. Transverse and collisionless,
  $n^2 = 1 - X(1 - X)/(1 - X - Y^2)$, with zeros at $X = 1 - Y$ and
  $X = 1 + Y$. In the longitudinal limit it has the gyroresonance as
  $Y \to 1$.
- In the longitudinal limit ($Y_T = 0$, $W > 0$) the O mode is
  $1 - X/(U + \lvert Y_L\rvert)$ and the X mode is
  $1 - X/(U - \lvert Y_L\rvert)$: the L and R circular waves.

## 5 Numerically stable evaluation forms

Near O-mode reflection $X \to 1$, so $W \to 0$. The classical form then
suffers cancellation in $Y_T^2/(2W) \to \infty$, and the Appleton–Hartree form
becomes $0/0$. An exact rewrite removes both problems. Let

$$
S_m = \sqrt{\tfrac14 Y_T^4 + Y_L^2W^2},
\qquad
G = S_m + \tfrac12 Y_T^2 .
$$

Then $-\tfrac12 Y_T^2 + S_m = (S_m^2 - \tfrac14 Y_T^4)/G = Y_L^2W^2/G$, the O
denominator becomes $W\left[U + Y_L^2W/G\right]$, and the two modes are

$$
\begin{aligned}
\text{O:}\quad n^2 &= 1 - \frac{X}{U + Y_L^2W/G}, \\
\text{X:}\quad n^2 &= 1 - \frac{XW}{UW - \tfrac12 Y_T^2 - S_m}.
\end{aligned}
$$

The O form has no cancellation as $W \to 0$. The X denominator tends to
$-Y_T^2$ there. Both are algebraically identical to the Appleton–Hartree form.
The O form also makes the reflection behaviour plain: as $X \to 1$,
$n^2 \to 1 - X/U$, which vanishes at $X = 1$ when $Z = 0$. That is the O-mode
reflection condition for every $\Theta$ outside the degenerate window of
section 6. `ordinary_stable_across_reflection` checks it.

## 6 Conditioning and known degeneracies

- **$X = 1$, $\Theta = 0$ (the Ellis window, or critical coupling).** The
  limits $X \to 1$ and $\Theta \to 0$ do not commute. The two dispersion
  surfaces meet there, and no globally continuous mode labelling exists. This
  is physics (Budden's coupling points), so the code cannot remove it. At
  exactly $Y_L \ne 0$, $Y_T = 0$, $W = 0$ the stable O form hits $G = 0$. The
  code returns the quasi-transverse value $1 - X/U$ there. Within the sliver
  where $\lvert W\rvert$ is below the coupling scale the O and X labels can
  swap, and full mode conversion is out of scope. HF rays whose apex wave
  normal is not exactly field-aligned never enter the sliver.
- **Root coalescence ($S_m = 0$ with $Y_T > 0$).** This is a branch point of
  the dispersion surface, where $dS_m$ is singular. An exact floating-point hit
  has measure zero. The code returns the degenerate root and drops the
  singular $dS_m$ term, so the state never becomes NaN.
- **Resonances.** Where a denominator vanishes, $n^2 \to \infty$: the
  upper-hybrid family for the X mode. The gyroresonance $Y = 1$ lies below HF
  for F-region fields ($\omega_H/2\pi \approx 1.4$ MHz), but the formula is
  exact there too. The tracer's step controller fails with a typed step
  collapse rather than integrate through a resonance.
- **The collisional branch of $S_m$.** With $Z > 0$ the argument of $S_m$ is
  complex. The principal branch is discontinuous only where the argument
  crosses the negative real axis. That needs $\lvert 1 - X\rvert \lesssim Z$
  with significant $Y_T$, a sliver inside the coupling region above.
- **Exactness at $Z = 0$.** Every intermediate has zero imaginary part through
  $+$, $-$, $\times$, $\div$ and the square root of a non-negative real, so a
  collisionless evaluation returns $\mathrm{Im} = 0.0$ exactly. Zero
  collisions therefore give zero absorption bit for bit, which
  `absorption_sign_and_collisionless_reality` checks.

## 7 Analytic partial derivatives

The ray equations need $n^2$ differentiated with respect to $X$, $Y$,
$\cos\Theta$ and $Z$. All of them come from complex differentials of the
section 5 forms. Write $a_1 = Y_L^2$, $a_2 = Y_T^2$ and $W = U - X$. Then

$$
dS_m = \frac{\tfrac12 a_2\,da_2 + W^2\,da_1 + 2a_1W\,dW}{2S_m}\quad(S_m \ne 0),
\qquad
dG = dS_m + \tfrac12\,da_2 .
$$

For the O mode, with $F = U + a_1W/G$,

$$
\begin{aligned}
dF &= dU + \frac{W\,da_1 + a_1\,dW}{G} - \frac{a_1W}{G^2}\,dG, \\
d(n^2) &= -\frac{dX}{F} + \frac{X}{F^2}\,dF .
\end{aligned}
$$

For the X mode, with $F = UW - \tfrac12 a_2 - S_m$,

$$
\begin{aligned}
dF &= U\,dW + W\,dU - \tfrac12\,da_2 - dS_m, \\
d(n^2) &= -\frac{W\,dX + X\,dW}{F} + \frac{XW}{F^2}\,dF .
\end{aligned}
$$

The differentials of the inputs are $dW = dU - dX$ and $dU = i\,dZ$. In the
$(X, Y, \cos\Theta)$ parametrisation, with $c = \cos\Theta$,

$$
da_1 = 2Yc^2\,dY + 2Y^2c\,dc,
\qquad
da_2 = 2Y(1 - c^2)\,dY - 2Y^2c\,dc .
$$

At $Y = 0$ the code uses the isotropic result directly:

$$
n^2 = 1 - \frac{X}{U},
\qquad
\frac{\partial n^2}{\partial X} = -\frac{1}{U},
\qquad
\frac{\partial n^2}{\partial Z} = \frac{iX}{U^2},
\qquad
\frac{\partial n^2}{\partial Y} = \frac{\partial n^2}{\partial\cos\Theta} = 0 .
$$

The $Y$ derivatives vanish because $n^2$ depends on $Y$ quadratically near
$Y = 0$. Both modes route through this one expression, which is what makes O
and X bit-identical in zero field
(`zero_field_modes_bit_identical_and_isotropic`).

`partials_match_finite_differences` compares every partial with central
finite differences of the full complex evaluation, at random interior points
and near $W = 0$. Finite differences appear only in that test, never in
production code.
