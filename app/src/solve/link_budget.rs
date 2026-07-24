//! Link-budget loss terms: free-space spreading and Fresnel ground reflection.
//! Pure functions; no engine calls and no physics of the ionosphere here.

use crate::antenna::{Polarization, fresnel_coefficient};

/// Nepers -> dB for field amplitude: 20/ln(10).
pub const NEPERS_TO_DB: f64 = 8.685_889_638_065_035;

/// Basic free-space (spreading) loss [dB] over a path length `dist_km` at
/// `f_mhz`: the standard Friis form `32.44 + 20 log10(f_MHz) + 20 log10(d_km)`.
/// The distance used is the total ray arc length (the physical path the energy
/// travels), not the great-circle range.
#[must_use]
pub fn free_space_loss_db(dist_km: f64, f_mhz: f64) -> f64 {
    32.44 + 20.0 * f_mhz.log10() + 20.0 * dist_km.log10()
}

/// Loss [dB] at one ground reflection, from the Fresnel power reflection
/// coefficient of a lossy dielectric half-space.
///
/// The complex relative permittivity is `eps_r - j sigma/(omega eps0)`
/// (ITU-R P.527 form). Horizontal and vertical coefficients are
///   R_h = (sin g - w)/(sin g + w),  R_v = (eps_c sin g - w)/(eps_c sin g + w),
///   w = sqrt(eps_c - cos^2 g),
/// with `g` the grazing (elevation) angle. A sky wave is elliptically polarised
/// after its ionospheric reflection, so we use the average power coefficient
/// `(|R_h|^2 + |R_v|^2)/2`; the loss is `-10 log10` of it.
///
/// The coefficients themselves come from [`fresnel_coefficient`], shared with
/// the antenna models, which need the same two numbers for their image-theory
/// ground reflection. One implementation, so the mid-path bounce and the ground
/// under the antenna can never disagree.
#[must_use]
pub fn ground_reflection_loss_db(grazing_rad: f64, f_hz: f64, eps_r: f64, sigma: f64) -> f64 {
    let r_h = fresnel_coefficient(grazing_rad, f_hz, eps_r, sigma, Polarization::Horizontal);
    let r_v = fresnel_coefficient(grazing_rad, f_hz, eps_r, sigma, Polarization::Vertical);
    let power = 0.5 * (r_h.norm_sqr() + r_v.norm_sqr());
    -10.0 * power.clamp(1e-12, 1.0).log10()
}
