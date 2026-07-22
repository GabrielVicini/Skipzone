//! HF ionospheric ray tracing from first principles.
//!
//! Physics core: full complex Appleton–Hartree refractive index (collisional,
//! magnetized, O and X modes, no quasi-longitudinal/quasi-transverse
//! approximation) and the Haselgrove ray equations in geocentric spherical
//! coordinates, integrated with an adaptive embedded Runge–Kutta pair.
//!
//! Conventions, fixed crate-wide (derived in `docs/derivations/conventions.md`):
//!
//! - Time dependence `exp(-i omega t)`: a lossy medium has `Im(n) > 0` and the
//!   field attenuates as `exp(-(omega/c) Im(n) s)`. Formulas in Budden and
//!   Davies use `exp(+i omega t)` and are the complex conjugates of ours.
//! - Geocentric spherical coordinates `(r, theta, phi)`: radius from Earth's
//!   centre, colatitude from the geographic north pole, east longitude. Local
//!   right-handed orthonormal basis `(r_hat, theta_hat, phi_hat)` = (up, south,
//!   east); north is `-theta_hat`.
//! - SI units and `f64` throughout. Newtypes guard public boundaries; the
//!   integrator state is raw `f64` in SI units, converted at the edges.
//!
//! Every equation is derived in `docs/derivations/` before implementation;
//! code comments cite the derivation file.

pub mod collision;
pub mod constants;
pub mod density;
pub mod error;
pub mod geo;
pub mod hamiltonian;
pub mod homing;
pub mod integrate;
pub mod mag;
pub mod magnetoionic;
pub mod trace;
pub mod units;
