# Haselgrove ray equations in spherical coordinates, derived canonically

Implements: `src/hamiltonian.rs`, `src/trace.rs`. Conventions: `conventions.md`.

## 1. Canonical variables

Ray phase psi = S(r) - omega t, wave vector k = grad S. In spherical
coordinates dl = dr r_hat + r dtheta theta_hat + r sin(theta) dphi phi_hat, so

    k . dl = k_r dr + (r k_theta) dtheta + (r sin(theta) k_phi) dphi

and the momenta conjugate to q = (r, theta, phi) are

    p = (p_r, p_th, p_ph) = (k_r, r k_theta, r sin(theta) k_phi)

with (k_r, k_theta, k_phi) the physical components on (r_hat, theta_hat,
phi_hat). For any dispersion function D(q, k) = 0, Hamilton's equations in
(q, p) with parameter tau are dq/dtau = dD/dp, dp/dtau = -dD/dq. Expressing D
in physical components and transforming p -> k gives, by the chain rule alone
(no covariant-derivative machinery is needed; the p <-> k map carries all
basis rotation exactly):

    dr/dtau     = D_kr
    dtheta/dtau = D_kth / r
    dphi/dtau   = D_kph / (r sin th)
    dk_r/dtau   = -D_r  + (k_th D_kth + k_ph D_kph)/r
    dk_th/dtau  = [-D_th + k_ph D_kph cot th - k_th D_kr]/r
    dk_ph/dtau  = [-D_ph / sin th - k_ph D_kr - k_ph D_kth cot th]/r

where D_q are coordinate partials at fixed *physical* k components and D_k
partials at fixed position. (Derivation: substitute k_th = p_th/r,
k_ph = p_ph/(r sin th) into D, differentiate, e.g. dp_th/dtau = -dD/dtheta|_p
= -D_th + D_kph k_ph cot th, and dp_th/dtau = (dr/dtau) k_th + r dk_th/dtau.)

## 2. The Hamiltonian and the working variables

    H(q, m) = 1/2 [ m.m - Re n^2(q, m_hat) ] ,   m := (c/omega) k

n^2 is the complex Appleton-Hartree index (appleton-hartree.md); the ray path
uses its real part (real-ray approximation: collisions perturb the path at
O(Z^2) but attenuate at O(Z); Jones-Stephenson practice). On shell |m| = n
because n^2 depends only on the direction m_hat.

Scaling: with sigma := (c/omega) tau, the equations keep the section-1 form
with k -> m and all right-hand sides O(1); sigma has units of metres and
d(position)/d(sigma) = v with |v| = n in the isotropic case (so sigma is arc
length divided by n there, and phase path accumulates as m.v, see section 4).

    v_i := dH/dm_i = m_i - 1/2 (dn^2/dcos) (b_i - cos(Th) m_hat_i)/|m|

using d cos(Th)/d m_i = (b_i - cos(Th) m_hat_i)/|m| for cos(Th) =
(m.B)/(|m||B|); b = B/|B|. Position and momentum equations:

    dr/ds     = v_r
    dth/ds    = v_th / r
    dph/ds    = v_ph / (r sin th)
    dm_r/ds   = 1/2 G_r + (m_th v_th + m_ph v_ph)/r
    dm_th/ds  = [1/2 G_th + m_ph v_ph cot th - m_th v_r]/r
    dm_ph/ds  = [1/2 G_ph / sin th - m_ph v_r - m_ph v_th cot th]/r

with G_q := d(Re n^2)/dq at fixed physical m:

    G_q = Re[ d_x dX/dq + d_y dY/dq + d_z dZ/dq + d_cos dcos(Th)/dq ]
    dX/dq   = (e^2/(eps0 m_e omega^2)) dNe/dq
    dY/dq   = (e/(m_e omega)) b . dB/dq
    dZ/dq   = (1/omega) dnu/dq
    dcosTh/dq = (m_hat . dB/dq - cos(Th) b . dB/dq)/|B|

dB/dq is column q of the field trait's component Jacobian; dNe/dq, dnu/dq the
density/collision coordinate partials. Zero field: v = m exactly and G has
only the d_x term - the field-free (2D) tracer is this system with Y = 0, not
separate code.

## 3. Group delay from the extended phase space

Treat (t, -omega) as a conjugate pair: dt/dtau = -dH/domega at fixed k.
With H above (k fixed, so m = ck/omega varies as omega^-1):

    dH/domega|_k = -(1/omega) [ m.m + (omega/2) d(Re n^2)/domega ]

n^2 depends on omega through X ~ omega^-2, Y ~ omega^-1, Z ~ omega^-1, so
(omega/2) dn^2/domega = -Re[ d_x X + 1/2 d_y Y + 1/2 d_z Z ] and, in sigma,
group path P' = c t accumulates as

    dP'/ds = m.m - Re[ d_x X + (1/2) d_y Y + (1/2) d_z Z ]

Checks: vacuum gives dP'/ds = 1; isotropic collisionless gives
dP'/ds = n^2 + X = 1 (with ds arc length = n dsigma this is the classical
group index mu' = 1/mu).

## 4. Phase path, absorption, arc length

    dP/ds = m . v          (phase path; = (c/omega) k . dr/dsigma)
    dA/ds = (omega/c) Im(n) |v|   , n = principal sqrt of complex n^2;
                                    exactly 0 when Z = 0 (see appleton-hartree.md)
    d(arc)/ds = |v|

Absorption in nepers; the field amplitude decays as exp(-A).

## 5. Doppler

For a time-varying medium the received frequency shifts by
df = -(f/c) dP/dt|_medium. All media in this crate are static, so the shift
is identically zero; the observable P is what a caller differentiates across
model epochs to get Doppler. No integrand is carried for it (documented
scope decision, not an omission).

## 6. Initialization and termination

Launch at point q0 with unit wave normal k_hat (from elevation/azimuth):
cos(Th) from k_hat and b; n0^2 = Re AH; if n0^2 <= 0 the mode is evanescent
at the source (typed error); else m = sqrt(n0^2) k_hat, which puts the state
exactly on shell (n depends only on direction, so no iteration is needed).
H = 0 is then a conserved diagnostic; its drift measures integrator error.

## 7. Conditioning near reflection (spec-required statement)

At an O-mode apex n^2 -> 0: v -> m -> 0, so d(position)/dsigma -> 0 while
dm/dsigma -> (1/2) grad n^2, finite. The trajectory is smooth in sigma; this
is precisely why the tracer must not reparametrise by arc length (dk/darc
diverges at the apex like 1/n). Consequences handled:

- Absorption integrand: Im(n) ~ Im(n^2)/(2 Re n) diverges at the apex, but
  dA/dsigma multiplies by |v| ~ Re n, so the product stays finite (deviative
  absorption at reflection is finite - physics agrees).
- Group integrand dP'/dsigma -> X = 1 at an isotropic apex: finite.
- Step control near the apex tightens through the error norm on m, not
  through any special-casing.
- Evanescent overshoot: trial RK stages may evaluate slightly beyond the
  turning point where Re n^2 < 0. All formulas remain smooth there
  (collisionless n^2 is real and analytic through zero), so rejected/interior
  stage evaluations are well-defined; the Hamiltonian pulls the ray back.
- **The Spitze** (observed, and the sharpest conditioning limit): a
  near-vertical O-mode ray propagating in the magnetic meridian plane has
  its wave normal refracted toward the field direction as X -> 1, arriving
  at exactly the (X = 1, theta = 0) Ellis-window degeneracy of
  appleton-hartree.md section 6. The physical ray path there is a cusp
  (direction reverses discontinuously); the dispersion surface is conical,
  dn^2/dcos(theta) diverges, and smooth Hamiltonian integration is
  impossible in principle, not just in practice. Diagnostic signature
  (reproduced in a probe): position freezes while v_r runs away and the
  step collapses. The tracer reports StepSizeCollapse (typed) rather than
  stepping over the cusp; analytic Spitze continuation (mode-preserving ray
  reversal) is a documented non-goal. The configuration requires the ray
  plane to contain B at X ~ 1: off-meridian azimuths and the X mode (cutoff
  X = 1 - Y, reached with W bounded away from 0) are unaffected.
- Exactly-critical vertical incidence with a field is additionally
  degenerate through |m| -> 0 with direction-dependent n^2 (m_hat
  discontinuous at the apex); same typed-failure handling. Field-free
  vertical reflection is unaffected (isotropic n^2 does not see m_hat).
