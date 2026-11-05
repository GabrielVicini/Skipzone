# Analytic reference solutions for the field-free tracer

- **Crate:** `crates/skipzone`
- **Implemented in:** `tests/common/mod.rs::bouguer_reference`, `tests/common/mod.rs::qp_closed_form`, `src/density.rs::QuasiParabolicLayer`, `src/density.rs::LinearLayer`
- **Tested by:** `tests/analytic_field_free.rs::vacuum_rays_are_straight`, `tests/analytic_field_free.rs::quasi_parabolic_closed_form_agreement`, `tests/analytic_field_free.rs::qp_closed_form_cross_checked_by_quadrature`, `tests/analytic_field_free.rs::chapman_linear_parabolic_match_bouguer_quadrature`, `tests/analytic_field_free.rs::bouguer_invariant_and_apex_condition_along_ray`, `tests/homing.rs::qp_field_free_agrees_with_closed_form_and_between_methods`
- **Section anchors used by code:** §1, §4, §5, §6

Symbols and sign conventions: [conventions.md](../conventions.md#5-symbols).

The validation suites compare the tracer against answers computed without it.
This file derives those answers. The setting throughout is an isotropic
($Y = 0$), collisionless ($Z = 0$), spherically stratified medium $n = n(r)$
with ground radius $r_0$ and launch elevation $\beta$. Section 1 derives the
invariant every stratified ray conserves. Section 2 turns it into ray
integrals, which have closed forms in vacuum (section 3) and in a
quasi-parabolic layer (section 4). Section 5 records what a linear layer gives
in flat geometry, and section 6 evaluates the section 2 integrals numerically
for any profile.

## 1 Bouguer invariant

For $n = n(r)$ the Haselgrove system of
[haselgrove.md §2](haselgrove.md#2-the-hamiltonian-and-the-working-variables)
has $\mathbf{v} = \mathbf{m}$ and $G_\theta = G_\varphi = 0$. Then

$$
\begin{aligned}
\frac{d(r m_\theta)}{d\sigma} &= v_r m_\theta + \left[m_\varphi v_\varphi\cot\theta - m_\theta v_r\right] = m_\varphi^2\cot\theta, \\
\frac{d(r m_\varphi)}{d\sigma} &= v_r m_\varphi + \left[-m_\varphi v_r - m_\varphi v_\theta\cot\theta\right] = -m_\theta m_\varphi\cot\theta ,
\end{aligned}
$$

so with $L^2 = (r m_\theta)^2 + (r m_\varphi)^2$,

$$
\frac{d(L^2)}{d\sigma} = 2rm_\theta\left(m_\varphi^2\cot\theta\right) + 2rm_\varphi\left(-m_\theta m_\varphi\cot\theta\right) = 0 .
$$

$L = r\sqrt{m_\theta^2 + m_\varphi^2} = nr\sin\chi$ is exactly conserved, where
$\chi$ is the angle between the ray and the local vertical and
$\lvert\mathbf{m}\rvert = n$ on shell. This is Bouguer's rule, Snell's law in
spherical geometry. At the ground

$$
C \equiv L = r_0\cos\beta .
$$

Any medium independent of $\varphi$ has a second exact invariant,
$p_\varphi = r\sin\theta\,m_\varphi$, shown by the same substitution.
`bouguer_invariant_and_apex_condition_along_ray` asserts both along traced
rays.

## 2 Ray integrals in a stratified medium

From $C = nr\sin\chi$, along the upward leg $dr/ds = \cos\chi$ and
$r\,d\Delta/ds = \sin\chi$, where $\Delta$ is the central angle and $s$ the arc
length. Hence

$$
\begin{aligned}
\frac{d\Delta}{dr} &= \frac{\tan\chi}{r} = \frac{C}{r\sqrt{n^2r^2 - C^2}}, &
\frac{ds}{dr} &= \frac{nr}{\sqrt{n^2r^2 - C^2}}, \\
\frac{dP'}{dr} &= \frac{1}{n}\frac{ds}{dr} = \frac{r}{\sqrt{n^2r^2 - C^2}}, &
\frac{dP}{dr} &= n\frac{ds}{dr} = \frac{n^2r}{\sqrt{n^2r^2 - C^2}} ,
\end{aligned}
$$

for the group path $P'$ and the phase path $P$. The turning point $r_t$
solves $n^2r_t^2 = C^2$. By up–down symmetry a reflecting ray has
$\Delta_\text{total} = 2\int_{r_0}^{r_t} d\Delta$, and likewise for the paths.

## 3 Vacuum: straight lines

With $n = 1$,

$$
\Delta(r) = \arccos\frac{C}{r} - \arccos\frac{C}{r_0},
\qquad
s(r) = \sqrt{r^2 - C^2} - \sqrt{r_0^2 - C^2},
$$

since $\tfrac{d}{dr}\arccos(C/r) = C/(r\sqrt{r^2 - C^2})$. Group path, phase
path and arc length all equal the chord length. Re-expressed in a fixed
Cartesian frame the traced direction must be constant, and its wander is the
reported integrator error (`vacuum_rays_are_straight`).

## 4 Quasi-parabolic layer: closed forms

The quasi-parabolic layer (`QuasiParabolicLayer` in `src/density.rs`) is

$$
N_e = N_m\left[1 - \left(\frac{(r - r_m)\,r_b}{y_m\,r}\right)^2\right]
\quad\text{on } [r_b, r_\text{top}],
\qquad r_b = r_m - y_m .
$$

With $F_2 = (f_c/f)^2$, where $f_c$ is the critical frequency of $N_m$,
$X(r) = F_2\left[1 - \left((r - r_m)r_b/(y_m r)\right)^2\right]$ and

$$
\begin{aligned}
n^2r^2 - C^2 &= \mathsf{A}r^2 + \mathsf{B}r + \mathsf{C}_0 \equiv F(r), \\
\mathsf{A} &= 1 - F_2 + F_2\frac{r_b^2}{y_m^2}, \qquad
\mathsf{B} = -2F_2\frac{r_m r_b^2}{y_m^2}, \qquad
\mathsf{C}_0 = F_2\frac{r_m^2 r_b^2}{y_m^2} - C^2 .
\end{aligned}
$$

$F$ is a plain quadratic in $r$. That is why this layer has elementary closed
forms in *spherical* geometry, where flat-earth parabolic-layer results do not
carry over. $\mathsf{A} > 0$ always, because $r_b > y_m$. The derivation also
requires $\mathsf{C}_0 > 0$, that is $\sqrt{F_2}\,r_m r_b/y_m > C$. Any HF
scenario that actually reflects satisfies it, and the test harness checks it.

**Turning point.** It is the smaller root,
$r_t = \left(-\mathsf{B} - \sqrt{\mathrm{disc}}\right)/(2\mathsf{A})$ with
$\mathrm{disc} = \mathsf{B}^2 - 4\mathsf{A}\mathsf{C}_0$. The ray reflects if
and only if $\mathrm{disc} > 0$ and $r_b < r_t$. The bound $r_t < r_\text{top}$
holds automatically, because $F(r_b) = r_b^2 - C^2 > 0$ and $\mathsf{A} > 0$
make $F$ negative only between its roots.

**Antiderivatives.** A reflecting ray has $\mathrm{disc} > 0$, real roots,
which selects the $\mathrm{arccosh}$ branch of
$\int dr/\sqrt{\text{quadratic}}$. The $\mathrm{arcsinh}$ form belongs to
$\mathrm{disc} < 0$, and using it here is a sign error that the quadrature
cross-check (`qp_closed_form_cross_checked_by_quadrature`) catches. On the
propagating side $r \le r_t$, below the smaller root,
$2\mathsf{A}r + \mathsf{B} \le -\sqrt{\mathrm{disc}} < 0$, and

$$
\begin{aligned}
I_1(r) &= \int\frac{dr}{\sqrt{F}} = -\frac{1}{\sqrt{\mathsf{A}}}\mathrm{arccosh}\left(-\frac{2\mathsf{A}r + \mathsf{B}}{\sqrt{\mathrm{disc}}}\right), \\
I_2(r) &= \int\frac{r\,dr}{\sqrt{F}} = \frac{\sqrt{F}}{\mathsf{A}} - \frac{\mathsf{B}}{2\mathsf{A}}I_1(r), \\
I_3(r) &= \int\frac{dr}{r\sqrt{F}} = -\frac{1}{\sqrt{\mathsf{C}_0}}\mathrm{arccosh}\left(\frac{2\mathsf{C}_0/r + \mathsf{B}}{\sqrt{\mathrm{disc}}}\right), \\
I_4(r) &= \int\frac{\sqrt{F}}{r}\,dr = \sqrt{F} + \frac{\mathsf{B}}{2}I_1(r) + \mathsf{C}_0 I_3(r).
\end{aligned}
$$

<details><summary>Derivation of the antiderivatives</summary>

**$I_1$.** Let $u = -(2\mathsf{A}r + \mathsf{B})/\sqrt{\mathrm{disc}}$, so
$u \ge 1$ for $r \le r_t$. Then $u' = -2\mathsf{A}/\sqrt{\mathrm{disc}}$ and
$u^2 - 1 = \left((2\mathsf{A}r + \mathsf{B})^2 - \mathrm{disc}\right)/\mathrm{disc} = 4\mathsf{A}F/\mathrm{disc}$, so

$$
\frac{d}{dr}\left[-\frac{\mathrm{arccosh}\,u}{\sqrt{\mathsf{A}}}\right]
= -\frac{u'}{\sqrt{\mathsf{A}}\sqrt{u^2 - 1}}
= \frac{2\sqrt{\mathsf{A}}}{\sqrt{\mathrm{disc}}}\cdot\frac{\sqrt{\mathrm{disc}}}{2\sqrt{\mathsf{A}F}} = \frac{1}{\sqrt{F}} .
$$

**$I_3$.** The map $w = 1/r$ sends $[r_b, r_t]$ to $w \ge 1/r_t$, the larger
root of $G(w) = \mathsf{C}_0w^2 + \mathsf{B}w + \mathsf{A}$, which has the same
discriminant. There $2\mathsf{C}_0w + \mathsf{B} \ge +\sqrt{\mathrm{disc}}$, so
$v = (2\mathsf{C}_0w + \mathsf{B})/\sqrt{\mathrm{disc}} \ge 1$, and
$v^2 - 1 = 4\mathsf{C}_0G/\mathrm{disc}$ gives
$\tfrac{d}{dw}\left[\mathrm{arccosh}(v)/\sqrt{\mathsf{C}_0}\right] = 1/\sqrt{G}$.
Since $\int dr/(r\sqrt{F}) = -\int dw/\sqrt{G}$, the minus sign follows.

**$I_2$ and $I_4$.** $\tfrac{d}{dr}\sqrt{F} = (2\mathsf{A}r + \mathsf{B})/(2\sqrt{F})$.
For $I_4$, adding $(\mathsf{B}/2)/\sqrt{F}$ and $\mathsf{C}_0/(r\sqrt{F})$
gives $F/(r\sqrt{F}) = \sqrt{F}/r$. $I_2$ follows in the same way.

</details>

**Conditioning at the apex.** All four antiderivatives vanish exactly at
$r_t$, where $\mathrm{arccosh}(1) = 0$ and $\sqrt{F(r_t)} = 0$. The code
takes the apex terms as these exact zeros, and it has to. Evaluated at the
floating-point $r_t$, the arccosh argument sits at $1 + O(10^{-14})$ from
cancellation in $2\mathsf{A}r + \mathsf{B}$. The square root turns that into
about $10^{-7}$, and the multipliers $\mathsf{B}/(2\mathsf{A}) \sim 10^7$ and
$\mathsf{C}_0 \sim 10^{17}$ lift it to 0.01–100 m of noise in the path
integrals. With exact apex limits the closed forms and the independent Bouguer
quadrature agree to about $10^{-4}$ m at every tested elevation.

**Totals.** Add the vacuum segment below the layer (section 3, from $r_0$ to
$r_b$) and write $J_k = I_k(r_t) - I_k(r_b)$ and
$s_0 = \sqrt{r_b^2 - C^2} - \sqrt{r_0^2 - C^2}$. Then

$$
\begin{aligned}
\Delta_\text{total} &= 2\left[\arccos\frac{C}{r_b} - \arccos\frac{C}{r_0} + C J_3\right],
\qquad D = r_0\,\Delta_\text{total}, \\
P'_\text{total} &= 2\left[s_0 + J_2\right],
\qquad
P_\text{total} = 2\left[s_0 + J_4 + C^2 J_3\right],
\end{aligned}
$$

where $D$ is the ground range. The apex is $r_t$, the true height of
reflection, and $n^2r^2 = C^2$ there. The phase total uses
$n^2r/\sqrt{F} = \sqrt{F}/r + C^2/(r\sqrt{F})$.
`quasi_parabolic_closed_form_agreement` compares the tracer with these totals,
and `qp_field_free_agrees_with_closed_form_and_between_methods` uses them as
the homing target.

## 5 Linear-gradient rays are parabolas

In flat stratified geometry Snell's law is $n\sin\vartheta = C_f$, with
$\vartheta$ the angle from the vertical. The ray curvature is

$$
\frac{d\vartheta}{ds} = -\frac{1}{n}\frac{dn}{dz}\sin\vartheta = -\frac{C_f}{n^2}\frac{dn}{dz}.
$$

A circular arc needs constant curvature, so $dn/dz \propto n^2$, which means
$1/n$ linear in $z$. Linear $n$ or linear $n^2$ does not give circles. A
linear electron density, $n^2 = 1 - kz$ (the profile of `LinearLayer`), gives
the ray $x(z)$ from $dx/dz = C_f/\sqrt{n^2 - C_f^2}$:

$$
x - x_\text{turn} = -\frac{2C_f}{k}\sqrt{1 - C_f^2 - kz}
\quad\Longrightarrow\quad
z(x) = \frac{1 - C_f^2}{k} - \frac{k}{4C_f^2}\left(x - x_\text{turn}\right)^2 ,
$$

an exact **parabola**. This is the flat-geometry property the `LinearLayer`
documentation cites. No test checks it. In spherical geometry neither profile
is elementary, because $n^2r^2 - C^2$ becomes cubic, so the spherical tracer is
checked against the quadrature of section 6 instead.

## 6 Bouguer quadrature reference

Evaluating the section 2 integrals numerically gives a reference independent of
the tracer for *any* $n(r)$. The tests use it for linear, parabolic and Chapman
layers (`chapman_linear_parabolic_match_bouguer_quadrature`) and to cross-check
the closed forms of section 4.

The integrand has an inverse-square-root singularity at the turning point. The
substitution $t^2 = r_t - r$ removes it exactly. Write
$F(r) = (r_t - r)\,Q(r)$, with $Q$ smooth and $Q(r_t) = -F'(r_t) > 0$. Then

$$
\int_r^{r_t}\frac{g(r')\,dr'}{\sqrt{F(r')}} = \int_0^{\sqrt{r_t - r}}\frac{2\,g(r_t - t^2)}{\sqrt{Q(r_t - t^2)}}\,dt ,
$$

and the right-hand integrand is smooth. In `bouguer_reference`, $Q$ is
evaluated as $F/(r_t - r)$, frozen at its value 1 mm below the apex inside the
last millimetre where that quotient becomes $0/0$. The integral uses composite
Simpson's rule, doubling the number of panels until successive results agree
to $10^{-12}$ relative. $r_t$ is located by an outward scan in 1 km steps and
then bisection on $n^2r^2 - C^2$ to $10^{-9}$ m.
