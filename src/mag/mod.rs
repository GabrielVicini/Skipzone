//! Magnetic field models behind one trait. Conventions: components on the
//! local (r_hat, theta_hat, phi_hat) basis in tesla; Jacobian entries are
//! plain coordinate partials of those component functions (NOT covariant
//! derivatives - the basis-rotation terms are handled once, exactly, in the
//! canonical Haselgrove derivation, docs/derivations/haselgrove.md).

mod dipole;
mod igrf;
pub mod legendre;

pub use dipole::Dipole;
pub use igrf::{Igrf, IgrfError, IgrfModel};

use crate::geo::SphericalPoint;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSample {
    /// (B_r, B_theta, B_phi), tesla.
    pub b: [f64; 3],
    /// `db[i][j]` = partial of `b[i]` with respect to coordinate `j`,
    /// coordinates ordered `(r [m], theta [rad], phi [rad])`.
    pub db: [[f64; 3]; 3],
}

impl FieldSample {
    pub const ZERO: Self = Self {
        b: [0.0; 3],
        db: [[0.0; 3]; 3],
    };
}

pub trait MagneticField {
    fn sample(&self, p: &SphericalPoint) -> FieldSample;
}

/// The unmagnetised limit: with |B| = 0 the Appleton-Hartree Y vanishes and
/// O/X are the same isotropic mode.
pub struct ZeroField;

impl MagneticField for ZeroField {
    fn sample(&self, _p: &SphericalPoint) -> FieldSample {
        FieldSample::ZERO
    }
}

/// Divergence and curl from a sample, spherical-coordinate formulas of
/// docs/derivations/magnetic-field.md section 5. Test-only: a potential field
/// must return zero for both, which exercises every Jacobian entry.
#[cfg(test)]
pub(crate) fn div_curl(p: &SphericalPoint, s: &FieldSample) -> (f64, [f64; 3]) {
    let r = p.r.get();
    let (st, ct) = p.colat.get().sin_cos();
    let [br, bt, bp] = s.b;
    let d = &s.db;
    let div = d[0][0] + 2.0 * br / r + (d[1][1] + bt * ct / st) / r + d[2][2] / (r * st);
    let curl = [
        ((ct * bp + st * d[2][1]) - d[1][2]) / (r * st),
        d[0][2] / (r * st) - (bp / r + d[2][0]),
        bt / r + d[1][0] - d[0][1] / r,
    ];
    (div, curl)
}
