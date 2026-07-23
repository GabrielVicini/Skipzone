//! Electron density models behind one trait. Sample returns Ne plus its
//! coordinate partials (per m, per rad, per rad), matching the field trait's
//! convention so the ray equations assemble gradients uniformly.
//!
//! The analytic profiles here are spherically symmetric (functions of r
//! only); horizontal structure arrives with NeQuick. Edge smoothness of each
//! profile is documented on the type: gradient discontinuities (kinks) are
//! legitimate test cases for the integrator's step control, not defects, but
//! they do reduce the local order of any step that straddles them.

use crate::constants::OMEGA_P_SQ_PER_DENSITY;
use crate::geo::SphericalPoint;
use crate::units::{Hertz, Meters, PerCubicMeter, Radians};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensitySample {
    /// Electron number density, m^-3. Never negative.
    pub ne: f64,
    /// Coordinate partials (d/dr [m^-4], d/dtheta, d/dphi [m^-3 rad^-1]).
    pub d_ne: [f64; 3],
}

impl DensitySample {
    pub const VACUUM: Self = Self {
        ne: 0.0,
        d_ne: [0.0; 3],
    };
}

pub trait ElectronDensity {
    fn sample(&self, p: &SphericalPoint) -> DensitySample;
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProfileError {
    #[error("invalid profile parameters: {0}")]
    Invalid(&'static str),
}

/// Peak density corresponding to a critical (plasma) frequency:
/// Nm = omega^2 / (e^2/(eps0 m)) from omega_p^2 = Ne e^2/(eps0 m).
#[must_use]
pub fn density_at_critical_frequency(f: Hertz) -> PerCubicMeter {
    let w = f.angular();
    PerCubicMeter::new(w * w / OMEGA_P_SQ_PER_DENSITY)
}



/// Plasma (critical) frequency of a density, inverse of the above.
#[must_use]
pub fn critical_frequency(ne: PerCubicMeter) -> Hertz {
    Hertz::new((ne.get() * OMEGA_P_SQ_PER_DENSITY).sqrt() / (2.0 * std::f64::consts::PI))
}

/// Hard upper limit on the solar zenith angle accepted by
/// `ChapmanLayer::with_zenith_angle`. At 85 deg the plane-parallel slant
/// factor sec(chi) is already ~11.5 and the flat-atmosphere assumption behind
/// it has broken down (the proper treatment is the Chapman grazing-incidence
/// function, not implemented here). Rejecting past this point keeps the code
/// from extrapolating a formula outside its domain; the consequence of raising
/// it would be silently wrong twilight densities.
pub const MAX_CHAPMAN_ZENITH_ANGLE: Radians = Radians::from_degrees(85.0);

/// Free space.
pub struct ZeroDensity;

impl ElectronDensity for ZeroDensity {
    fn sample(&self, _p: &SphericalPoint) -> DensitySample {
        DensitySample::VACUUM
    }
}

/// Ne = slope (r - r_base) above r_base, zero below. C0 at the base with a
/// gradient kink there. In flat geometry this profile gives exactly parabolic
/// rays (docs/derivations/analytic-solutions.md); in spherical geometry its
/// reference solution is the Bouguer quadrature.
pub struct LinearLayer {
    r_base: f64,
    /// dNe/dr inside the layer, m^-4.
    slope: f64,
}

impl LinearLayer {
    /// # Errors
    /// `slope` must be positive.
    pub fn new(r_base: Meters, slope: f64) -> Result<Self, ProfileError> {
        if !(slope > 0.0 && slope.is_finite()) {
            return Err(ProfileError::Invalid(
                "linear layer needs positive finite slope",
            ));
        }
        Ok(Self {
            r_base: r_base.get(),
            slope,
        })
    }
}

impl ElectronDensity for LinearLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let dr = p.r.get() - self.r_base;
        if dr <= 0.0 {
            DensitySample::VACUUM
        } else {
            DensitySample {
                ne: self.slope * dr,
                d_ne: [self.slope, 0.0, 0.0],
            }
        }
    }
}

/// Parabolic layer: Ne = Nm (1 - ((r - r_m)/y_m)^2) on |r - r_m| <= y_m,
/// zero outside. C0 at both edges with gradient kinks (dNe/dr jumps by
/// -+ 2 Nm/y_m). No elementary spherical-geometry closed form exists for the
/// ray integrals (they are elliptic); the quasi-parabolic layer below is the
/// closed-form validation target, and this layer is checked against the
/// Bouguer quadrature.
pub struct ParabolicLayer {
    nm: f64,
    r_peak: f64,
    y_m: f64,
}

impl ParabolicLayer {
    /// # Errors
    /// Peak density and semi-thickness must be positive.
    pub fn new(
        nm: PerCubicMeter,
        r_peak: Meters,
        semi_thickness: Meters,
    ) -> Result<Self, ProfileError> {
        if !(nm.get() > 0.0 && semi_thickness.get() > 0.0) {
            return Err(ProfileError::Invalid(
                "parabolic layer needs positive Nm and semi-thickness",
            ));
        }
        Ok(Self {
            nm: nm.get(),
            r_peak: r_peak.get(),
            y_m: semi_thickness.get(),
        })
    }
}

impl ElectronDensity for ParabolicLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let u = (p.r.get() - self.r_peak) / self.y_m;
        if u.abs() >= 1.0 {
            DensitySample::VACUUM
        } else {
            DensitySample {
                ne: self.nm * (1.0 - u * u),
                d_ne: [-2.0 * self.nm * u / self.y_m, 0.0, 0.0],
            }
        }
    }
}

/// Quasi-parabolic (QP) layer:
///
///   Ne(r) = Nm [ 1 - ((r - r_m) r_b / (y_m r))^2 ]   for r_b <= r <= r_top
///
/// with r_b = r_m - y_m the layer base and r_top = r_m r_b/(r_b - y_m) the
/// upper zero (requires r_b > y_m, amply true for realistic layers). Chosen
/// because n^2 r^2 is quadratic in r for 1/f^2-scaled X, which makes every
/// spherical ray integral elementary - the closed-form validation target
/// (docs/derivations/analytic-solutions.md). C0 with kinks at both zeros.
pub struct QuasiParabolicLayer {
    nm: f64,
    r_peak: f64,
    y_m: f64,
    r_base: f64,
    r_top: f64,
}

impl QuasiParabolicLayer {
    /// # Errors
    /// Requires positive Nm, y_m, and r_m > 2 y_m (so the base radius
    /// exceeds the semi-thickness and the upper zero exists).
    pub fn new(
        nm: PerCubicMeter,
        r_peak: Meters,
        semi_thickness: Meters,
    ) -> Result<Self, ProfileError> {
        let (nm, r_m, y_m) = (nm.get(), r_peak.get(), semi_thickness.get());
        if !(nm > 0.0 && y_m > 0.0) {
            return Err(ProfileError::Invalid(
                "QP layer needs positive Nm and semi-thickness",
            ));
        }
        let r_base = r_m - y_m;
        if r_base <= y_m {
            return Err(ProfileError::Invalid(
                "QP layer needs r_peak > 2 * semi_thickness",
            ));
        }
        Ok(Self {
            nm,
            r_peak: r_m,
            y_m,
            r_base,
            r_top: r_m * r_base / (r_base - y_m),
        })
    }

    #[must_use]
    pub fn base_radius(&self) -> Meters {
        Meters::new(self.r_base)
    }

    #[must_use]
    pub fn top_radius(&self) -> Meters {
        Meters::new(self.r_top)
    }
}

impl ElectronDensity for QuasiParabolicLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let r = p.r.get();
        if r <= self.r_base || r >= self.r_top {
            return DensitySample::VACUUM;
        }
        // f = (r - r_m)/r; term = (f r_b / y_m)^2; df/dr = r_m / r^2.
        let f = (r - self.r_peak) / r;
        let b = self.r_base / self.y_m;
        let ne = self.nm * (1.0 - (f * b) * (f * b));
        let dne = -2.0 * self.nm * b * b * f * self.r_peak / (r * r);
        DensitySample {
            ne,
            d_ne: [dne, 0.0, 0.0],
        }
    }
}

/// Alpha-Chapman layer with solar zenith angle:
///
///   Ne = Nm exp(1/2 (1 - z - sec(chi) e^{-z})),   z = (r - r_m)/H
///
/// The classic profile of a monochromatically-absorbed ionising flux in an
/// exponential atmosphere at photochemical (alpha-recombination) equilibrium
/// (Chapman 1931). Smooth everywhere (C-infinity), never exactly zero -
/// callers wanting vacuum below must rely on its exponential decay, which step
/// control handles.
///
/// Derivation of the zenith-angle behaviour, used by callers that place the
/// layer from a computed chi. Setting d/dz = 0 gives
/// `-1 + sec(chi) e^{-z} = 0`, so the true peak sits at
///
///   z_max = ln(sec chi)          i.e. height r_m + H ln(sec chi)
///   Ne(z_max) = Nm (sec chi)^{-1/2} = Nm sqrt(cos chi)
///
/// so `nm` is the *overhead-sun* peak density and the layer both thins as
/// sqrt(cos chi) and rises by H ln(sec chi) as the sun sets. Both are the
/// standard Chapman results.
///
/// `new` fixes sec(chi) = 1 (overhead sun) and is bit-identical to the
/// original two-parameter layer.
///
/// Known limitation: `sec(chi)` is the *plane-parallel* slant-path factor. It
/// assumes the ionising ray traverses a flat stratified atmosphere, which
/// degrades as the path lengthens near the terminator; the standard fix is the
/// Chapman grazing-incidence function `Ch(x, chi)`, which is NOT implemented
/// here. Treat results beyond chi ~ 75 deg as indicative only, and refuse them
/// entirely past `MAX_CHAPMAN_ZENITH_ANGLE` rather than extrapolating a
/// formula outside its domain.
pub struct ChapmanLayer {
    nm: f64,
    r_peak: f64,
    scale_height: f64,
    /// sec(chi); exactly 1.0 for the overhead-sun constructor.
    sec_chi: f64,
}

impl ChapmanLayer {
    /// Overhead-sun layer (sec chi = 1).
    ///
    /// # Errors
    /// Peak density and scale height must be positive.
    pub fn new(
        nm: PerCubicMeter,
        r_peak: Meters,
        scale_height: Meters,
    ) -> Result<Self, ProfileError> {
        Self::with_sec_chi(nm, r_peak, scale_height, 1.0)
    }

    /// Layer at solar zenith angle `chi`. `nm` remains the overhead-sun peak
    /// density; the realised peak is `nm sqrt(cos chi)` at
    /// `r_peak + H ln(sec chi)` (see the type docs).
    ///
    /// # Errors
    /// As `new`, plus `chi` must be below `MAX_CHAPMAN_ZENITH_ANGLE`. Note
    /// that `cos(90 deg)` is 6.1e-17 rather than 0 in floating point, so a
    /// bare `cos > 0` test would happily accept sec(chi) ~ 1e16; the limit is
    /// enforced on the angle itself.
    pub fn with_zenith_angle(
        nm: PerCubicMeter,
        r_peak: Meters,
        scale_height: Meters,
        chi: Radians,
    ) -> Result<Self, ProfileError> {
        // NaN must be rejected, so test it explicitly rather than relying on a
        // negated comparison (which clippy flags on partially ordered types).
        let chi_abs = chi.get().abs();
        if !chi_abs.is_finite() || chi_abs >= MAX_CHAPMAN_ZENITH_ANGLE.get() {
            return Err(ProfileError::Invalid(
                "zenith angle outside the plane-parallel Chapman domain; \
                 use a night/twilight model past the limit",
            ));
        }
        Self::with_sec_chi(nm, r_peak, scale_height, 1.0 / chi.get().cos())
    }

    fn with_sec_chi(
        nm: PerCubicMeter,
        r_peak: Meters,
        scale_height: Meters,
        sec_chi: f64,
    ) -> Result<Self, ProfileError> {
        if !(nm.get() > 0.0 && scale_height.get() > 0.0) {
            return Err(ProfileError::Invalid(
                "Chapman layer needs positive Nm and scale height",
            ));
        }
        if !(sec_chi.is_finite() && sec_chi >= 1.0) {
            return Err(ProfileError::Invalid(
                "Chapman layer needs a finite sec(chi) >= 1",
            ));
        }
        Ok(Self {
            nm: nm.get(),
            r_peak: r_peak.get(),
            scale_height: scale_height.get(),
            sec_chi,
        })
    }

    /// Realised peak density, `Nm sqrt(cos chi)`.
    #[must_use]
    pub fn peak_density(&self) -> PerCubicMeter {
        PerCubicMeter::new(self.nm / self.sec_chi.sqrt())
    }

    /// Realised peak radius, `r_peak + H ln(sec chi)`.
    #[must_use]
    pub fn peak_radius(&self) -> Meters {
        Meters::new(self.r_peak + self.scale_height * self.sec_chi.ln())
    }
}

impl ElectronDensity for ChapmanLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let z = (p.r.get() - self.r_peak) / self.scale_height;
        let ne = self.nm * (0.5 * (1.0 - z - self.sec_chi * (-z).exp())).exp();
        // dNe/dr = Ne * (1/2)(-1 + sec(chi) e^{-z}) / H.
        let dne = ne * 0.5 * (self.sec_chi * (-z).exp() - 1.0) / self.scale_height;
        DensitySample {
            ne,
            d_ne: [dne, 0.0, 0.0],
        }
    }
}

/// Sum of layers. Ne and gradients add; smoothness is the worst of the parts.
pub struct MultiLayer {
    layers: Vec<Box<dyn ElectronDensity + Send + Sync>>,
}

impl MultiLayer {
    #[must_use]
    pub fn new(layers: Vec<Box<dyn ElectronDensity + Send + Sync>>) -> Self {
        Self { layers }
    }
}

impl ElectronDensity for MultiLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let mut out = DensitySample::VACUUM;
        for l in &self.layers {
            let s = l.sample(p);
            out.ne += s.ne;
            for i in 0..3 {
                out.d_ne[i] += s.d_ne[i];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Radians;

    const R0: f64 = 6_371_000.0;

    fn at(r: f64) -> SphericalPoint {
        SphericalPoint::new(Meters::new(r), Radians::new(1.2), Radians::new(0.3))
    }

    fn fd_check(model: &dyn ElectronDensity, r_lo: f64, r_hi: f64, tol_rel: f64) {
        let n = 400;
        for i in 0..=n {
            let r = r_lo + (r_hi - r_lo) * f64::from(i) / f64::from(n);
            let h = 0.5; // m; profiles vary on km scales
            let s = model.sample(&at(r));
            let (sp, sm) = (model.sample(&at(r + h)), model.sample(&at(r - h)));
            // Skip straddles of a documented kink: FD is invalid across them.
            if (sp.ne == 0.0) != (sm.ne == 0.0) {
                continue;
            }
            let fd = (sp.ne - sm.ne) / (2.0 * h);
            let scale = s.d_ne[0].abs().max(fd.abs()).max(1e3);
            // Central-difference floor: cancellation rounds at ~eps*Ne/h.
            let roundoff = 4.0 * f64::EPSILON * s.ne.abs() / h;
            assert!(
                (s.d_ne[0] - fd).abs() < tol_rel * scale + roundoff,
                "at r={r}: {} vs fd {fd}",
                s.d_ne[0]
            );
            assert_eq!(s.d_ne[1], 0.0);
            assert_eq!(s.d_ne[2], 0.0);
        }
    }

    #[test]
    fn critical_frequency_round_trips() {
        let ne = PerCubicMeter::new(1.0e12);
        let f = critical_frequency(ne);
        assert!((density_at_critical_frequency(f).get() - 1.0e12).abs() < 1e-3);
        assert!((8.9e6..9.1e6).contains(&f.get()));
    }

    #[test]
    fn linear_layer_gradient_and_base() {
        let m = LinearLayer::new(Meters::new(R0 + 100e3), 1e9).unwrap();
        assert_eq!(m.sample(&at(R0 + 99e3)), DensitySample::VACUUM);
        let s = m.sample(&at(R0 + 150e3));
        assert!((s.ne - 1e9 * 50e3).abs() < 1.0);
        fd_check(&m, R0 + 90e3, R0 + 200e3, 1e-9);
    }

    #[test]
    fn parabolic_layer_shape() {
        let m = ParabolicLayer::new(
            PerCubicMeter::new(1e12),
            Meters::new(R0 + 300e3),
            Meters::new(100e3),
        )
        .unwrap();
        assert_eq!(m.sample(&at(R0 + 300e3)).ne, 1e12);
        assert_eq!(m.sample(&at(R0 + 300e3)).d_ne[0], 0.0);
        assert_eq!(m.sample(&at(R0 + 401e3)), DensitySample::VACUUM);
        assert_eq!(m.sample(&at(R0 + 199e3)), DensitySample::VACUUM);
        fd_check(&m, R0 + 150e3, R0 + 450e3, 1e-7);
    }

    #[test]
    fn quasi_parabolic_layer_zeros_peak_and_gradient() {
        let m = QuasiParabolicLayer::new(
            PerCubicMeter::new(2e12),
            Meters::new(R0 + 300e3),
            Meters::new(100e3),
        )
        .unwrap();
        let rb = m.base_radius().get();
        let rt = m.top_radius().get();
        assert!((rb - (R0 + 200e3)).abs() < 1e-6);
        // r_top = r_m r_b/(r_b - y_m), above the peak by more than y_m
        // (the QP layer is top-side stretched).
        assert!(rt > R0 + 400e3);
        assert_eq!(m.sample(&at(R0 + 300e3)).ne, 2e12);
        assert!(m.sample(&at(rb + 1.0)).ne < 2e12 * 3e-5);
        assert!(m.sample(&at(rt - 1.0)).ne < 2e12 * 3e-5);
        assert_eq!(m.sample(&at(rb - 1.0)), DensitySample::VACUUM);
        assert_eq!(m.sample(&at(rt + 1.0)), DensitySample::VACUUM);
        fd_check(&m, rb - 20e3, rt + 20e3, 1e-7);
        assert!(
            QuasiParabolicLayer::new(
                PerCubicMeter::new(1e12),
                Meters::new(150e3), // r_peak <= 2 y_m: no upper zero
                Meters::new(100e3),
            )
            .is_err()
        );
    }

    #[test]
    fn chapman_layer_peak_and_gradient() {
        let m = ChapmanLayer::new(
            PerCubicMeter::new(1e12),
            Meters::new(R0 + 300e3),
            Meters::new(50e3),
        )
        .unwrap();
        // At z=0: exp(1/2 (1-0-1)) = 1, and dNe/dr = 0 (the peak).
        assert_eq!(m.sample(&at(R0 + 300e3)).ne, 1e12);
        assert!(m.sample(&at(R0 + 300e3)).d_ne[0].abs() < 1e-3);
        fd_check(&m, R0 + 100e3, R0 + 600e3, 1e-7);
    }

    /// The zenith-angle constructor at chi = 0 must reproduce the original
    /// overhead-sun layer exactly, bit for bit: the added parameter must not
    /// perturb any previously validated result.
    #[test]
    fn chapman_zenith_zero_is_bit_identical_to_overhead() {
        let nm = PerCubicMeter::new(1e12);
        let (rp, h) = (Meters::new(R0 + 300e3), Meters::new(50e3));
        let overhead = ChapmanLayer::new(nm, rp, h).unwrap();
        let chi0 = ChapmanLayer::with_zenith_angle(nm, rp, h, Radians::new(0.0)).unwrap();
        for i in 0..=200 {
            let r = R0 + 60e3 + (600e3 - 60e3) * f64::from(i) / 200.0;
            let a = overhead.sample(&at(r));
            let b = chi0.sample(&at(r));
            assert_eq!(a.ne.to_bits(), b.ne.to_bits(), "Ne differs at r={r}");
            assert_eq!(
                a.d_ne[0].to_bits(),
                b.d_ne[0].to_bits(),
                "dNe differs at r={r}"
            );
        }
    }

    /// The derived Chapman relations: peak density Nm sqrt(cos chi) at height
    /// r_peak + H ln(sec chi). Verified against a scan of the actual profile,
    /// not just against the accessors.
    #[test]
    fn chapman_zenith_peak_relations() {
        let nm_val = 1e12;
        let nm = PerCubicMeter::new(nm_val);
        let (rp, h) = (R0 + 300e3, 50e3);
        for chi_deg in [0.0, 30.0, 60.0, 75.0, 84.0] {
            let chi = Radians::from_degrees(chi_deg);
            let layer =
                ChapmanLayer::with_zenith_angle(nm, Meters::new(rp), Meters::new(h), chi).unwrap();
            let cos_chi = chi.get().cos();
            let want_peak_ne = nm_val * cos_chi.sqrt();
            let want_peak_r = rp + h * (1.0 / cos_chi).ln();
            assert!(
                (layer.peak_density().get() - want_peak_ne).abs() < 1e-6 * want_peak_ne,
                "chi={chi_deg}: peak Ne {} vs {want_peak_ne}",
                layer.peak_density().get()
            );
            assert!((layer.peak_radius().get() - want_peak_r).abs() < 1e-6);

            // Scan: the true maximum of the sampled profile must sit at the
            // predicted radius with the predicted value.
            let (mut best_r, mut best_ne) = (0.0, f64::NEG_INFINITY);
            for i in 0..=20_000 {
                let r = rp - 200e3 + 400e3 * f64::from(i) / 20_000.0;
                let ne = layer.sample(&at(r)).ne;
                if ne > best_ne {
                    best_ne = ne;
                    best_r = r;
                }
            }
            assert!(
                (best_r - want_peak_r).abs() < 40.0,
                "chi={chi_deg}: scanned peak at {best_r} vs predicted {want_peak_r}"
            );
            assert!(
                (best_ne - want_peak_ne).abs() < 1e-4 * want_peak_ne,
                "chi={chi_deg}: scanned peak Ne {best_ne} vs {want_peak_ne}"
            );
        }
    }

    #[test]
    fn chapman_zenith_gradient_and_domain() {
        let nm = PerCubicMeter::new(5e11);
        let layer = ChapmanLayer::with_zenith_angle(
            nm,
            Meters::new(R0 + 90e3),
            Meters::new(7e3),
            Radians::from_degrees(70.0),
        )
        .unwrap();
        // Looser than the F2 gradient tests (1e-7) for an oracle reason, not a
        // physics one: a 7 km scale height at sec(chi) ~ 2.9 varies on a ~435 m
        // length, so fd_check's fixed h = 0.5 m carries ~2e-7 relative
        // central-difference truncation - larger than its 1e3 absolute floor
        // allows. The analytic gradient was confirmed correct separately by
        // halving h and observing the discrepancy fall at order 2.000. A real
        // term or sign error would show up at O(1) relative, far above 1e-5.
        fd_check(&layer, R0 + 60e3, R0 + 200e3, 1e-5);

        // Just inside the domain must still be accepted.
        assert!(
            ChapmanLayer::with_zenith_angle(
                nm,
                Meters::new(R0 + 90e3),
                Meters::new(7e3),
                Radians::from_degrees(84.0),
            )
            .is_ok()
        );
        // Past the plane-parallel domain the constructor must refuse rather
        // than extrapolate. 90 deg specifically guards the floating-point trap
        // that cos(90 deg) = 6.1e-17 > 0.
        for chi_deg in [85.0, 90.0, 100.0] {
            assert!(
                ChapmanLayer::with_zenith_angle(
                    nm,
                    Meters::new(R0 + 90e3),
                    Meters::new(7e3),
                    Radians::from_degrees(chi_deg),
                )
                .is_err(),
                "chi={chi_deg} should be rejected"
            );
        }
    }

    #[test]
    fn multi_layer_sums() {
        let e = ChapmanLayer::new(
            PerCubicMeter::new(1.5e11),
            Meters::new(R0 + 110e3),
            Meters::new(10e3),
        )
        .unwrap();
        let f2 = ChapmanLayer::new(
            PerCubicMeter::new(1e12),
            Meters::new(R0 + 300e3),
            Meters::new(50e3),
        )
        .unwrap();
        let both = MultiLayer::new(vec![Box::new(e), Box::new(f2)]);
        let p = at(R0 + 200e3);
        let e2 = ChapmanLayer::new(
            PerCubicMeter::new(1.5e11),
            Meters::new(R0 + 110e3),
            Meters::new(10e3),
        )
        .unwrap();
        let f22 = ChapmanLayer::new(
            PerCubicMeter::new(1e12),
            Meters::new(R0 + 300e3),
            Meters::new(50e3),
        )
        .unwrap();
        let want_ne = e2.sample(&p).ne + f22.sample(&p).ne;
        let want_g = e2.sample(&p).d_ne[0] + f22.sample(&p).d_ne[0];
        let s = both.sample(&p);
        assert_eq!(s.ne, want_ne);
        assert_eq!(s.d_ne[0], want_g);
    }
}
