# Conventions

- **Crate:** `crates/skipzone`, and every crate built on it
- **Implemented in:** `src/lib.rs`, `src/magnetoionic.rs::appleton_hartree`, `src/geo.rs::launch_direction`, `src/units.rs`
- **Tested by:** `src/magnetoionic.rs::tests::absorption_sign_and_collisionless_reality`, `tests/absorption.rs::absorption_monotone_and_positive`, `src/geo.rs::tests::launch_direction_is_unit_and_oriented`
- **Section anchors used by code:** none; the file is cited as a whole

These conventions are fixed across the workspace. Every other derivation
assumes them, and the [symbol table](#5-symbols) at the end is the one the
other files link to.

## 1 Coordinates

Positions are geocentric spherical coordinates $(r, \theta, \varphi)$: radius
from the Earth's centre in metres, colatitude from the geographic north pole,
and east longitude, both in radians. The local basis
$(\hat{r}, \hat{\theta}, \hat{\varphi})$ is right-handed and orthonormal and
points up, south and east. North is $-\hat{\theta}$. The line element is

$$
d\mathbf{l} = dr\,\hat{r} + r\,d\theta\,\hat{\theta} + r\sin\theta\,d\varphi\,\hat{\varphi}.
$$

The geomagnetic elements in this basis are $X_\text{north} = -B_\theta$,
$Y_\text{east} = B_\varphi$ and $Z_\text{down} = -B_r$.

The Earth is a sphere in all of the physics. Geodetic shape could only enter
in input and output conversion, outside the tracer, and at present it enters
nowhere.

## 2 Time convention and the sign of losses

Fields vary as $e^{-i\omega t}$, so a plane wave is
$e^{i(\mathbf{k}\cdot\mathbf{x} - \omega t)}$. This choice fixes the sign of
every imaginary part in the code, so the derivation is given in full.

Take a cold electron gas with no static field and a collisional drag
$-m_e \nu \mathbf{v}$. The electron charge is $-e$ with $e > 0$. The momentum
equation and its time-harmonic form are

$$
m_e \frac{d\mathbf{v}}{dt} = -e\mathbf{E} - m_e \nu \mathbf{v}
\quad\Longrightarrow\quad
\mathbf{v} = -\frac{e\,\mathbf{E}}{m_e(\nu - i\omega)}.
$$

The current density is $\mathbf{J} = -e N_e \mathbf{v}$, so the AC
conductivity is $\sigma_c = e^2 N_e / (m_e(\nu - i\omega))$. With
$e^{-i\omega t}$, Ampère's law turns the conductivity into a relative
permittivity $\varepsilon = 1 + i\sigma_c/(\varepsilon_0\omega)$:

$$
\varepsilon = 1 + \frac{i\,\omega_p^2}{\omega(\nu - i\omega)},
\qquad
\omega_p^2 = \frac{N_e e^2}{\varepsilon_0 m_e}.
$$

Writing $\nu - i\omega = -i\omega(1 + i\nu/\omega)$ gives the form used
everywhere else:

$$
\varepsilon = 1 - \frac{X}{1 + iZ} = 1 - \frac{X}{U},
\qquad
X = \frac{\omega_p^2}{\omega^2},\quad Z = \frac{\nu}{\omega},\quad U = 1 + iZ.
$$

For $Z > 0$ the imaginary part is $\mathrm{Im}\,\varepsilon = XZ/(1+Z^2) > 0$.
Then $\mathrm{Im}\, n > 0$ and $e^{i\mathbf{k}\cdot\mathbf{x}}$ decays along
the direction of propagation. **Losses correspond to positive imaginary
parts.**

Budden and Davies write $e^{+i\omega t}$ and $U = 1 - iZ$. Their formulas are
the complex conjugates of ours. A formula imported from that literature is
conjugated first, and the use site says so.

The amplitude attenuation along a path, in nepers, is

$$
A = \frac{\omega}{c}\int \mathrm{Im}\, n \, ds,
$$

and the power loss in decibels is $(20/\ln 10)\,A \approx 8.6859\,A$.

## 3 Magnitudes and signs of X, Y, Z

All three ratios are non-negative. $X = \omega_p^2/\omega^2 \ge 0$ and
$Z = \nu/\omega \ge 0$. The gyrofrequency ratio $Y = \omega_H/\omega$ uses the
magnitude $\omega_H = e\lvert\mathbf{B}\rvert/m_e > 0$. The electron's negative
charge is absorbed into the sign conventions of the Appleton–Hartree
derivation, where the O and X labels are anchored to physical limits
([appleton-hartree.md §4](engine/appleton-hartree.md#4-mode-labels-and-branch-conventions)).

## 4 Units

The code uses SI units and `f64` everywhere. Public API boundaries take
newtypes (`Meters`, `Hertz`, `Radians` and the others in `src/units.rs`). The
integrator state is raw `f64` in SI units, converted at the edges.

## 5 Symbols

One table for every derivation. A symbol that only one file uses is listed
under that file's heading below the main table.

| Symbol | Meaning | Unit | Code |
|---|---|---|---|
| $r, \theta, \varphi$ | radius, colatitude, east longitude | m, rad, rad | `r`, `colat`, `lon` |
| $\hat{r}, \hat{\theta}, \hat{\varphi}$ | local basis: up, south, east | | |
| $f$, $\omega = 2\pi f$ | wave frequency, angular frequency | Hz, rad/s | `f`, `omega` |
| $c$ | speed of light | m/s | `SPEED_OF_LIGHT` |
| $e$, $m_e$, $\varepsilon_0$ | elementary charge, electron mass, vacuum permittivity | SI | `constants.rs` |
| $N_e$ | electron density | m⁻³ | `ne` |
| $\omega_p$ | plasma angular frequency, $\omega_p^2 = N_e e^2/(\varepsilon_0 m_e)$ | rad/s | `OMEGA_P_SQ_PER_DENSITY` |
| $\omega_H$ | electron gyrofrequency, $e\lvert\mathbf{B}\rvert/m_e$ | rad/s | `OMEGA_H_PER_TESLA` |
| $\nu$ | electron collision frequency | s⁻¹ | `CollisionFrequency` |
| $X$ | $\omega_p^2/\omega^2$ | | `x` |
| $Y$ | $\omega_H/\omega$ | | `y` |
| $Z$ | $\nu/\omega$ | | `z` |
| $U$ | $1 + iZ$ | | `u` |
| $W$ | $U - X$ | | `w` |
| $\mathbf{B}$, $\hat{b}$ | geomagnetic field, its unit vector | T | `FieldSample` |
| $\Theta$ | angle between the wave normal and $\hat{b}$ | rad | `cos_theta` |
| $Y_L, Y_T$ | $Y\cos\Theta$, $Y\sin\Theta$ | | `a1` $= Y_L^2$, `a2` $= Y_T^2$ |
| $n$, $n^2$ | complex refractive index and its square | | `n_sq` |
| $\mathbf{k}$ | wave vector | m⁻¹ | |
| $\mathbf{m}$ | scaled wave vector $(c/\omega)\mathbf{k}$, on shell $\lvert\mathbf{m}\rvert = n$ | | `y[3..6]` |
| $\sigma$ | ray parameter, $(c/\omega)\tau$ | m | integrator variable |
| $\mathcal{H}$ | ray Hamiltonian | | `RayEquations::hamiltonian` |
| $\mathbf{v}$ | $\partial\mathcal{H}/\partial\mathbf{m}$, the ray direction in $\sigma$ | | `v` |
| $P'$, $P$ | group path, phase path | m | `y[6]`, `y[7]` |
| $A$ | absorption | Np | `y[8]` |
| $s$ | arc length | m | `y[9]` |

### Local to appleton-hartree.md

$R, L, P, S, D$ are the Stix dielectric elements (§1 and §2 only).
$S_m = \sqrt{\tfrac14 Y_T^4 + Y_L^2 W^2}$ and $G = S_m + \tfrac12 Y_T^2$ are
the stable-form intermediates (§5 onwards; code `s_m`, `g`).

### Local to magnetic-field.md

$a$ is the IGRF reference radius, $g_n^m, h_n^m$ the Gauss coefficients,
$V$ the scalar potential and $S_n^m(\theta)$ the Schmidt semi-normalised
associated Legendre functions (code `schmidt`).

### Local to analytic-solutions.md

$r_0$ is the ground radius, $\beta$ the launch elevation, $\chi$ the angle
between the ray and the local vertical, $C = r_0\cos\beta$ the Bouguer
constant, $\Delta$ the central angle and $r_t$ the turning-point radius.

### Local to chapman-grazing.md

$\chi$ is the solar zenith angle, $H$ the neutral scale height,
$z = (r - r_m)/H$ the reduced height, $x = r_m/H$ the Chapman-function argument
(code `big_x`), $\mathrm{Ch}(x, \chi)$ the grazing-incidence function,
$\delta$ the solar declination and $\eta$ the hour angle. The literature writes
the Chapman argument as $X$. This repository keeps $X$ for
$\omega_p^2/\omega^2$.
