//! Physical constants, SI units. Each value states its source. Values exact by
//! the 2019 SI redefinition are marked exact; the rest are CODATA 2022
//! recommended values.

/// Speed of light in vacuum, m/s. Exact by SI definition.
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0;

/// Elementary charge, C. Exact by SI definition.
pub const ELEMENTARY_CHARGE: f64 = 1.602_176_634e-19;

/// Electron mass, kg. CODATA 2022.
pub const ELECTRON_MASS: f64 = 9.109_383_713_9e-31;

/// Vacuum permittivity, F/m. CODATA 2022 (measured, no longer exact since the
/// 2019 SI redefinition).
pub const VACUUM_PERMITTIVITY: f64 = 8.854_187_818_8e-12;

/// Angular plasma frequency squared per unit electron density:
/// `omega_p^2 = Ne * e^2 / (eps0 * m_e)`, so this constant is
/// `e^2 / (eps0 * m_e)` in units of s^-2 m^3. Derived from the constants above;
/// the standard cold-plasma result (derivation: docs/derivations/appleton-hartree.md,
/// section 1).
pub const OMEGA_P_SQ_PER_DENSITY: f64 =
    ELEMENTARY_CHARGE * ELEMENTARY_CHARGE / (VACUUM_PERMITTIVITY * ELECTRON_MASS);

/// Electron angular gyrofrequency per unit magnetic flux density:
/// `omega_H = e |B| / m_e`, so this constant is `e / m_e` in units of
/// rad s^-1 T^-1. Taken positive; the sign conventions that absorb the
/// electron's negative charge are fixed in the Appleton-Hartree derivation.
pub const OMEGA_H_PER_TESLA: f64 = ELEMENTARY_CHARGE / ELECTRON_MASS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_frequency_of_known_density_is_in_hf_band() {
        // Sanity anchor, not a validation: Ne = 1e12 m^-3 must give a plasma
        // frequency near 9 MHz (the well-known f_p ~ 8.98 sqrt(Ne) rule is
        // itself derived from these constants, so this only guards against
        // transcription errors of orders of magnitude).
        let f_p = (OMEGA_P_SQ_PER_DENSITY * 1e12).sqrt() / (2.0 * std::f64::consts::PI);
        assert!((8.9e6..9.1e6).contains(&f_p), "f_p = {f_p}");
    }
}
