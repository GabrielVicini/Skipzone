# Analytic reference solutions for the field-free tracer

All derived here from scratch; used by `tests/analytic_field_free.rs`.
Setting: isotropic (Y = 0), collisionless (Z = 0), spherically stratified
n = n(r); ground radius r0; launch elevation beta.

## 1. Bouguer invariant (derived from the ray equations)

For n = n(r) the Haselgrove system (haselgrove.md section 2, v = m) has
G_th = G_ph = 0, and

    d(r m_th)/ds = v_r m_th + [m_ph v_ph cot th - m_th v_r] = m_ph^2 cot th
    d(r m_ph)/ds = v_r m_ph + [-m_ph v_r - m_ph v_th cot th] = -m_th m_ph cot th

so with L^2 := (r m_th)^2 + (r m_ph)^2:

    d(L^2)/ds = 2 r m_th (m_ph^2 cot th) + 2 r m_ph (-m_th m_ph cot th) = 0

L = r sqrt(m_th^2 + m_ph^2) = n r sin(chi) is exactly conserved (chi = angle
between ray and local vertical; |m| = n on shell). This is Bouguer's rule /
spherical Snell. At the ground C := L = r0 cos(beta).

A second exact invariant for any phi-independent medium: p_ph = r sin(th) m_ph
(shown by direct substitution the same way). Both are asserted along traced
rays.

## 2. Ray integrals in a stratified medium

From C = n r sin(chi): along the up-leg,
dr/ds(arc) = cos(chi), r dDelta/ds(arc) = sin(chi) with Delta the central
angle, so

    dDelta/dr = tan(chi)/r = C / (r sqrt(n^2 r^2 - C^2))
    ds(arc)/dr = n r / sqrt(n^2 r^2 - C^2)
    dP'/dr = (1/n) ds/dr = r / sqrt(n^2 r^2 - C^2)          (group)
    dP /dr =  n   ds/dr = n^2 r / sqrt(n^2 r^2 - C^2)       (phase)

The turning point r_t solves n^2 r_t^2 = C^2. A ray that reflects has
Delta_total = 2 * integral(r0 -> r_t), etc., by up/down symmetry.

## 3. Vacuum (n = 1): straight lines

    Delta(r) = arccos(C/r) - arccos(C/r0),  s(r) = sqrt(r^2 - C^2) - sqrt(r0^2 - C^2)

(d/dr arccos(C/r) = C/(r sqrt(r^2 - C^2)) confirms the integrand.) Group =
phase = arc = chord length; the traced direction re-expressed in a fixed
Cartesian frame must be constant, and its wander is the reported integrator
error ("zero density means exactly straight rays").

## 4. Quasi-parabolic layer: closed forms

Layer (density.rs): Ne = Nm [1 - ((r - rm) rb/(ym r))^2] on [rb, r_top],
rb = rm - ym. With F2 := (fc/f)^2 (fc = critical frequency of Nm):
X(r) = F2 [1 - ((r-rm) rb/(ym r))^2] and

    n^2 r^2 - C^2 = A r^2 + B r + C0 =: F(r)
    A  = 1 - F2 + F2 rb^2/ym^2
    B  = -2 F2 rm rb^2/ym^2
    C0 = F2 rm^2 rb^2 / ym^2 - C^2

a plain quadratic - the reason this layer has elementary closed forms in
*spherical* geometry (flat-earth parabolic results do not transfer). A > 0
always (rb > ym); we additionally require C0 > 0, i.e.
sqrt(F2) rm rb/ym > C, satisfied by any HF scenario that actually reflects
(documented check in the test harness).

Turning point: smaller root r_t = (-B - sqrt(disc))/(2A), disc = B^2 - 4 A C0;
the ray reflects iff disc > 0 and rb < r_t (< r_top automatic since F(rb) =
rb^2 - C^2 > 0 and A > 0 makes F negative only between the roots).

Antiderivatives. A reflecting ray has disc > 0 (real roots), which selects
the **acosh** branch of int dr/sqrt(quadratic) - the asinh form belongs to
disc < 0 and using it here is a sign error that the quadrature cross-check
test catches. On the propagation side r <= r_t (below the smaller root),
2Ar + B <= -sqrt(disc) < 0:

    I1(r) = int dr / sqrt(F)      = -(1/sqrt(A)) acosh( -(2Ar + B)/sqrt(disc) )
    I2(r) = int r dr / sqrt(F)    = sqrt(F)/A - (B/(2A)) I1(r)
    I3(r) = int dr / (r sqrt(F))  = -(1/sqrt(C0)) acosh( (2 C0/r + B)/sqrt(disc) )
    I4(r) = int sqrt(F) dr / r    = sqrt(F) + (B/2) I1(r) + C0 I3(r)

[Check I1: u := -(2Ar+B)/sqrt(disc) >= 1 on r <= r_t;
d/dr[-acosh(u)/sqrt(A)] = -u'/(sqrt(A) sqrt(u^2-1)); u' = -2A/sqrt(disc);
u^2 - 1 = ((2Ar+B)^2 - disc)/disc = 4AF/disc, so the derivative is
(2 sqrt(A)/sqrt(disc)) (sqrt(disc)/(2 sqrt(A F))) = 1/sqrt(F).
Check I3: w = 1/r maps [rb, r_t] to w >= 1/r_t = the larger root of
G(w) = C0 w^2 + B w + A (same discriminant: B^2 - 4 C0 A = disc), where
2 C0 w + B >= +sqrt(disc); v := (2 C0 w + B)/sqrt(disc) >= 1 and
v^2 - 1 = 4 C0 G/disc gives d/dw[acosh(v)/sqrt(C0)] = 1/sqrt(G);
int dr/(r sqrt(F)) = -int dw/sqrt(G) supplies the minus sign.
Check I4: d/dr[sqrt(F)] = (2Ar+B)/(2 sqrt(F)); adding (B/2)/sqrt(F)
+ C0/(r sqrt(F)) totals F/(r sqrt(F)) = sqrt(F)/r.
All four vanish identically at r_t (acosh(1) = 0, sqrt(F(r_t)) = 0). This is
not merely convenient: the apex-side terms MUST be taken as these exact
zeros. Evaluating them at the floating-point r_t is ill-conditioned - the
acosh argument sits at 1 + O(ulp-cancellation of 2Ar+B, ~1e-14), the square
root turns that into ~1e-7, and the B/(2A) ~ 1e7 and C0 ~ 1e17 multipliers
lift it to 0.01-100 m of noise in the path integrals (measured). With exact
apex limits the closed forms and the independent Bouguer quadrature agree to
~1e-4 m at every tested elevation.]

With the vacuum under-layer segment (section 3, r0 -> rb) and
J_k := I_k(r_t) - I_k(rb):

    Delta_total = 2 [ arccos(C/rb) - arccos(C/r0) + C J3 ]
    ground range D = r0 Delta_total
    P'_total    = 2 [ s0 + J2 ]                 s0 = sqrt(rb^2-C^2) - sqrt(r0^2-C^2)
    P_total     = 2 [ s0 + J4 + C^2 J3 ]
    apex        = r_t  (true height of reflection; and X, n^2 r^2 = C^2 there)

At r_t, F = 0: I2 and I4's sqrt(F) terms vanish; asinh arguments are finite
(u(r_t) = -1), so every expression is evaluable without limits.

## 5. Linear-gradient rays: parabolas, not circles (spec correction)

Flat stratified geometry, Snell n sin(theta) = C_f: the ray curvature is
d(theta)/d(arc) = -(1/n) dn/dz sin(theta) = -(C_f/n^2) dn/dz. A circular arc
needs constant curvature, i.e. dn/dz proportional to n^2, i.e. **1/n linear
in z** - not a linear n or n^2. For a linear *electron density* (n^2 = 1 - kz,
the "linear layer" profile) the exact flat-geometry ray is x(z) from
dx/dz = C_f/sqrt(n^2 - C_f^2):

    x - x_turn = -(2 C_f/k) sqrt(1 - C_f^2 - k z)
    =>  z(x) = (1 - C_f^2)/k - (k/(4 C_f^2)) (x - x_turn)^2

an exact **parabola** (the statement "linear gradient gives exact circular
arcs" in the task brief is therefore incorrect as written; the nearest true
statements are the two above). In spherical geometry neither profile is
elementary (n^2 r^2 - C^2 becomes cubic), so the spherical tracer is checked
against:

## 6. Bouguer quadrature reference (any stratified profile)

The section-2 integrals evaluated by high-order numerical quadrature give a
tracer-independent reference for *any* n(r) (used for linear, parabolic and
Chapman layers; also cross-checks the QP closed forms). The 1/sqrt
turning-point singularity is removed exactly by t^2 = r_t - r:
F(r) = (r_t - r) Q(r) with Q(r_t) = -F'(r_t) > 0, so

    int_{r}^{r_t} g(r) dr / sqrt(F) = int_0^{sqrt(r_t - r)} 2 g(r_t - t^2) / sqrt(Q(r_t - t^2)) dt

with a smooth integrand (Q evaluated as F/(r_t - r), or -F' at the endpoint).
Composite Simpson with Richardson refinement to 1e-12 relative; r_t located
by bisection + Newton on n^2 r^2 - C^2 to machine precision.
