//! Electron collision frequency models. Same sample convention as density:
//! value plus coordinate partials. The collision frequency enters the
//! Appleton-Hartree Z = nu/omega and is the sole source of absorption.

use crate::geo::SphericalPoint;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionSample {
    /// Effective electron collision frequency, s^-1.
    pub nu: f64,
    /// Coordinate partials (d/dr [s^-1 m^-1], d/dtheta, d/dphi [s^-1 rad^-1]).
    pub d_nu: [f64; 3],
}

pub trait CollisionFrequency {
    fn sample(&self, p: &SphericalPoint) -> CollisionSample;
}

/// The collisionless limit: Z = 0 exactly, absorption exactly zero (a
/// validation invariant, see appleton-hartree.md section 6).
pub struct ZeroCollisions;

impl CollisionFrequency for ZeroCollisions {
    fn sample(&self, _p: &SphericalPoint) -> CollisionSample {
        CollisionSample {
            nu: 0.0,
            d_nu: [0.0; 3],
        }
    }
}

/// Single-scale-height exponential: nu(r) = nu0 exp(-(r - r_ref)/H).
///
/// This matches the leading behaviour of the electron-neutral collision
/// frequency in an isothermal neutral atmosphere (nu_en tracks the neutral
/// density). Documented limits: valid where electron-neutral collisions
/// dominate (D/E region, below roughly 150 km); electron-ion collisions,
/// which take over above, are not modelled; a single scale height cannot
/// represent the real temperature structure. No default magnitude is
/// provided: nu0 must come from a neutral-atmosphere source chosen by the
/// caller - this crate does not invent one.
pub struct ExponentialCollisions {
    nu0: f64,
    r_ref: f64,
    scale_height: f64,
}

impl ExponentialCollisions {
    /// `nu0`: collision frequency at `r_ref`; `scale_height`: e-folding
    /// distance of the neutral atmosphere.
    ///
    /// # Errors
    /// `nu0` must be non-negative and finite, `scale_height` positive.
    pub fn new(
        nu0: crate::units::PerSecond,
        r_ref: crate::units::Meters,
        scale_height: crate::units::Meters,
    ) -> Result<Self, crate::density::ProfileError> {
        if !(nu0.get() >= 0.0 && nu0.get().is_finite() && scale_height.get() > 0.0) {
            return Err(crate::density::ProfileError::Invalid(
                "exponential collisions need nu0 >= 0 and positive scale height",
            ));
        }
        Ok(Self {
            nu0: nu0.get(),
            r_ref: r_ref.get(),
            scale_height: scale_height.get(),
        })
    }
}

impl CollisionFrequency for ExponentialCollisions {
    fn sample(&self, p: &SphericalPoint) -> CollisionSample {
        let nu = self.nu0 * (-(p.r.get() - self.r_ref) / self.scale_height).exp();
        CollisionSample {
            nu,
            d_nu: [-nu / self.scale_height, 0.0, 0.0],
        }
    }
}
