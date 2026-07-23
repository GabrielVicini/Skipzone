//! Day/night-aware D-region absorbing layer for the app.
//!
//! This is the one place the GUI carries physics of its own: a solar
//! zenith-angle dependent alpha-Chapman layer built on the Chapman
//! grazing-incidence function `Ch(X, chi)` instead of the plane-parallel
//! `sec(chi)`. It replaces the engine's `ChapmanLayer::with_zenith_angle` for
//! the D region so that (a) absorption fades smoothly through the terminator
//! rather than switching off at the engine's 85 deg plane-parallel limit, and
//! (b) the layer varies with horizontal position, so a path that crosses the
//! terminator sees real D-region loss on its sunlit portion and none on its
//! night portion. Full derivation, sign conventions, and checks:
//! docs/derivations/chapman-grazing.md. The engine crate is untouched.

use skipzone::density::{DensitySample, ElectronDensity};
use skipzone::geo::SphericalPoint;
use std::f64::consts::{FRAC_PI_2, PI};

/// 2/sqrt(pi), the constant in erf'(t) and erfcx'(t).
const TWO_OVER_SQRT_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI;
/// sqrt(pi).
const SQRT_PI: f64 = 1.772_453_850_905_516;

/// Scaled complementary error function `erfcx(t) = e^{t^2} erfc(t)` for
/// `t >= 0`. Stable everywhere (no `e^{t^2}` overflow): series below 2, the
/// A&S 7.1.14 continued fraction above. See derivation section 3.
#[must_use]
pub fn erfcx(t: f64) -> f64 {
    debug_assert!(t >= 0.0, "erfcx here is only needed for t >= 0");
    if t < 3.0 {
        // erf(t) via its Maclaurin series; e^{t^2} <= e^9 is harmless and the
        // ~3-digit cancellation at t = 3 still leaves >12 accurate digits. The
        // continued fraction converges slowly right at its lower edge, so the
        // series carries the crossover.
        let mut term = t; // n = 0 term of sum t^{2n+1}/(n!(2n+1)) is t
        let mut sum = t;
        let mut n = 0.0_f64;
        loop {
            n += 1.0;
            // term_{n} = term_{n-1} * (-t^2 / n) * (2n-1)/(2n+1)
            term *= -t * t / n;
            let add = term / (2.0 * n + 1.0);
            sum += add;
            if add.abs() <= 1e-18 * sum.abs().max(1e-300) {
                break;
            }
        }
        let erf = TWO_OVER_SQRT_PI * sum;
        (t * t).exp() * (1.0 - erf)
    } else {
        // sqrt(pi) erfcx(t) = 1/(t + (1/2)/(t + (2/2)/(t + (3/2)/(t + ...)))).
        let mut frac = 0.0_f64;
        for k in (1..=48).rev() {
            frac = 0.5 * f64::from(k) / (t + frac);
        }
        1.0 / (SQRT_PI * (t + frac))
    }
}

/// `erfcx'(t) = 2 t erfcx(t) - 2/sqrt(pi)` (derivation eq. 4).
#[must_use]
pub fn erfcx_deriv(t: f64, erfcx_t: f64) -> f64 {
    2.0 * t * erfcx_t - TWO_OVER_SQRT_PI
}

/// Chapman grazing-incidence function `Ch(X, chi)` and its derivative
/// `dCh/dchi` (derivation eqs. 3, 8). `x` is `X = r_m/H`, `chi` in radians.
/// Returns `(Ch, dCh/dchi)`; `Ch = +inf` in deep night where the closed form
/// overflows (the caller then yields vacuum).
#[must_use]
pub fn chapman_grazing(x: f64, chi: f64) -> (f64, f64) {
    let (sin_c, cos_c) = chi.sin_cos();
    let a = (0.5 * PI * x).sqrt(); // sqrt(pi X / 2)
    let root = (0.5 * x).sqrt(); // sqrt(X / 2)
    let t = root * cos_c.abs();
    let ex = erfcx(t);
    let dex = erfcx_deriv(t, ex);

    if chi <= FRAC_PI_2 {
        // t = root cos_c (cos_c >= 0), dt/dchi = -root sin_c.
        let ch = a * ex;
        let dch = a * dex * (-root * sin_c);
        (ch, dch)
    } else {
        let arg = x * (1.0 - sin_c);
        if arg > 700.0 {
            // e^{X(1-sin chi)} would overflow; Ch is astronomically large and
            // the layer is vacuum to machine precision. Derivative is moot.
            return (f64::INFINITY, 0.0);
        }
        let e1 = arg.exp();
        let sqrt2pix = (2.0 * PI * x).sqrt();
        let sqrt_sin = sin_c.sqrt();
        let term1 = sqrt2pix * sqrt_sin * e1;
        let day = a * ex; // t = root |cos_c| = -root cos_c
        let ch = term1 - day;
        // dTerm1/dchi = sqrt2pix e1 cos_c ( 1/(2 sqrt sin) - X sqrt sin );
        // t = -root cos_c so dt/dchi = root sin_c, dday/dchi = a dex root sin_c.
        let dterm1 = sqrt2pix * e1 * cos_c * (0.5 / sqrt_sin - x * sqrt_sin);
        let dday = a * dex * (root * sin_c);
        (ch, dterm1 - dday)
    }
}

/// Alpha-Chapman D layer whose ionising-flux slant factor is the grazing
/// function `Ch(X, chi(theta, phi))` evaluated at each sampled point. `nm` is
/// the overhead-sun peak density; the realised peak is `nm Ch^{-1/2}` at
/// `r_peak + H ln Ch` (derivation section 2).
pub struct SolarChapmanD {
    nm: f64,
    r_peak: f64,
    scale_height: f64,
    sin_decl: f64,
    cos_decl: f64,
    /// Hour-angle offset so that H = phi + hour_offset (radians).
    hour_offset: f64,
    /// X = r_peak / H, the (constant) Chapman-function argument.
    big_x: f64,
}

impl SolarChapmanD {
    /// `declination_deg` and `utc_hours` fix the sun position for the solve;
    /// `nm`, `r_peak`, `scale_height` are the overhead-sun alpha-Chapman
    /// parameters (SI: m^-3, m, m).
    #[must_use]
    pub fn new(
        nm: f64,
        r_peak: f64,
        scale_height: f64,
        declination_deg: f64,
        utc_hours: f64,
    ) -> Self {
        let (sin_decl, cos_decl) = declination_deg.to_radians().sin_cos();
        Self {
            nm,
            r_peak,
            scale_height,
            sin_decl,
            cos_decl,
            // H = 15 deg/h (utc + lon/15 - 12) = phi + (pi/12) utc - pi.
            hour_offset: utc_hours * PI / 12.0 - PI,
            big_x: r_peak / scale_height,
        }
    }

    /// Local solar zenith angle chi (radians) at a colatitude/longitude
    /// (derivation eq. 5). Used by the module tests to recover the chi a
    /// sampled point actually sees; `sample` computes it inline on the hot path.
    #[cfg(test)]
    #[must_use]
    pub fn zenith_angle(&self, colat: f64, lon: f64) -> f64 {
        let (sin_t, cos_t) = colat.sin_cos();
        let cos_h = (lon + self.hour_offset).cos();
        (cos_t * self.sin_decl + sin_t * self.cos_decl * cos_h)
            .clamp(-1.0, 1.0)
            .acos()
    }

    /// Realised peak density `nm Ch^{-1/2}` at a given zenith angle (display).
    #[must_use]
    pub fn realised_peak_ne(&self, chi: f64) -> f64 {
        let (ch, _) = chapman_grazing(self.big_x, chi);
        if ch.is_finite() {
            self.nm / ch.sqrt()
        } else {
            0.0
        }
    }

    /// Realised peak altitude above `r_peak`'s reference, `H ln Ch` (m).
    #[must_use]
    pub fn realised_peak_rise(&self, chi: f64) -> f64 {
        let (ch, _) = chapman_grazing(self.big_x, chi);
        if ch.is_finite() {
            self.scale_height * ch.ln()
        } else {
            f64::INFINITY
        }
    }
}

impl ElectronDensity for SolarChapmanD {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let theta = p.colat.get();
        let phi = p.lon.get();
        let (sin_t, cos_t) = theta.sin_cos();
        let (sin_h, cos_h) = (phi + self.hour_offset).sin_cos();
        let cos_chi = (cos_t * self.sin_decl + sin_t * self.cos_decl * cos_h).clamp(-1.0, 1.0);
        let chi = cos_chi.acos();

        let (ch, dch) = chapman_grazing(self.big_x, chi);
        if !ch.is_finite() {
            return DensitySample::VACUUM; // deep night: no D region
        }
        let z = (p.r.get() - self.r_peak) / self.scale_height;
        let emz = (-z).exp();
        let f = 0.5 * (1.0 - z - ch * emz);
        if f < -700.0 {
            return DensitySample::VACUUM; // underflow: negligible density
        }
        let ne = self.nm * f.exp();

        // dNe/dr = Ne * 1/2 (Ch e^{-z} - 1)/H (derivation eq. 7).
        let dne_dr = ne * 0.5 * (ch * emz - 1.0) / self.scale_height;

        // Horizontal partials via chi(theta, phi); zero at the subsolar and
        // antisolar points where sin chi -> 0 (gradient vanishes by symmetry).
        let sin_chi = (1.0 - cos_chi * cos_chi).max(0.0).sqrt();
        let (dne_dtheta, dne_dphi) = if sin_chi < 1e-9 {
            (0.0, 0.0)
        } else {
            let dcos_dtheta = -sin_t * self.sin_decl + cos_t * self.cos_decl * cos_h;
            let dcos_dphi = -sin_t * self.cos_decl * sin_h;
            let dchi_dtheta = -dcos_dtheta / sin_chi;
            let dchi_dphi = -dcos_dphi / sin_chi;
            let dne_dchi = ne * (-0.5 * emz) * dch;
            (dne_dchi * dchi_dtheta, dne_dchi * dchi_dphi)
        };

        DensitySample {
            ne,
            d_ne: [dne_dr, dne_dtheta, dne_dphi],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skipzone::units::{Meters, Radians};

    fn point(r: f64, colat: f64, lon: f64) -> SphericalPoint {
        SphericalPoint::new(Meters::new(r), Radians::new(colat), Radians::new(lon))
    }

    /// erfcx against reference values (Abramowitz & Stegun / direct e^{t^2}
    /// erfc(t)), spanning both the series and continued-fraction branches.
    #[test]
    fn erfcx_reference_values() {
        // High-precision anchors: 0 (exact), the series branch (<2), and the
        // continued-fraction branch (>=2).
        let cases = [
            (0.0, 1.0),
            (0.5, 0.615_690_344_192_510),
            (1.0, 0.427_583_576_155_807),
            (2.0, 0.255_395_676_310_87),
            (5.0, 0.110_704_637_733),
            (10.0, 0.056_140_983),
        ];
        for (t, want) in cases {
            let got = erfcx(t);
            assert!(
                (got - want).abs() < 1e-6 * want.max(1.0),
                "erfcx({t}) = {got}, want {want}"
            );
        }
        // The two branches must agree across the t = 3 crossover (independent
        // check that the series and continued fraction compute the same thing).
        // Branches agree to ~2.5e-10 at the crossover (the series' cancellation
        // error at t = 3; the continued fraction is exact there), far tighter
        // than the ~1e-6 the physics needs.
        assert!(
            (erfcx(3.0 - 1e-9) - erfcx(3.0 + 1e-9)).abs() < 1e-9,
            "erfcx branch discontinuity at t = 3"
        );
    }

    /// erfcx' via eq. 4 matches central differences.
    #[test]
    fn erfcx_deriv_matches_fd() {
        for &t in &[0.3_f64, 1.0, 1.9, 2.5, 6.0] {
            let h = 1e-6;
            let fd = (erfcx(t + h) - erfcx(t - h)) / (2.0 * h);
            let an = erfcx_deriv(t, erfcx(t));
            assert!(
                (an - fd).abs() < 1e-6 * fd.abs().max(1.0),
                "t={t}: {an} vs {fd}"
            );
        }
    }

    /// Ch limits (derivation section 2): overhead = sec = 1; below ~75 deg it
    /// tracks sec(chi); at 90 deg it is finite = sqrt(pi X/2); continuous
    /// across the branch at 90 deg.
    #[test]
    fn chapman_grazing_limits() {
        let x = 1076.0;
        let (ch0, _) = chapman_grazing(x, 0.0);
        assert!((ch0 - 1.0).abs() < 1e-3, "Ch(0) = {ch0}");
        for chi_deg in [30.0_f64, 50.0, 70.0] {
            let (ch, _) = chapman_grazing(x, chi_deg.to_radians());
            let sec = 1.0 / chi_deg.to_radians().cos();
            assert!(
                (ch - sec).abs() < 1e-2 * sec,
                "Ch({chi_deg}) {ch} vs sec {sec}"
            );
        }
        let (ch90, _) = chapman_grazing(x, FRAC_PI_2);
        assert!(
            (ch90 - (0.5 * PI * x).sqrt()).abs() < 1e-6,
            "Ch(90) = {ch90}"
        );
        // Continuity across 90 deg.
        let (below, _) = chapman_grazing(x, FRAC_PI_2 - 1e-7);
        let (above, _) = chapman_grazing(x, FRAC_PI_2 + 1e-7);
        assert!(
            (below - above).abs() < 1e-3,
            "Ch jump at 90 deg: {below} vs {above}"
        );
    }

    /// dCh/dchi (eqs. 8a/8b) matches central differences on both branches.
    #[test]
    fn chapman_grazing_deriv_matches_fd() {
        let x = 1076.0;
        for chi_deg in [10.0_f64, 45.0, 80.0, 89.0, 92.0, 96.0] {
            let chi = chi_deg.to_radians();
            let h = 1e-7;
            let (_, an) = chapman_grazing(x, chi);
            let (cp, _) = chapman_grazing(x, chi + h);
            let (cm, _) = chapman_grazing(x, chi - h);
            let fd = (cp - cm) / (2.0 * h);
            assert!(
                (an - fd).abs() < 1e-4 * fd.abs().max(1.0),
                "chi={chi_deg}: dCh {an} vs fd {fd}"
            );
        }
    }

    /// The layer produces a finite, thinned, raised peak at the terminator
    /// where the plane-parallel engine layer refuses to exist at all.
    #[test]
    fn terminator_layer_is_finite_and_thinned() {
        // Overhead anchors matching scenario.rs.
        let r0 = 6_371_000.0;
        let layer = SolarChapmanD::new(1.0e9, r0 + 85e3, 6e3, 0.0, 12.0);
        // Subsolar point (decl 0, utc 12 => hour_offset 0) is colat 90, lon 0.
        // A point 86 deg away from it sees chi ~ 86 deg: colat 4 deg, lon 0
        // gives cos chi = sin(4 deg) => chi = 86 deg, just past the engine's
        // 85 deg plane-parallel limit where the old model refused a layer.
        let colat = 4.0_f64.to_radians();
        let lon = 0.0;
        let chi = layer.zenith_angle(colat, lon);
        assert!(
            chi.to_degrees() > 85.0,
            "test point chi = {} deg",
            chi.to_degrees()
        );

        let mut best = 0.0_f64;
        let mut best_r = 0.0;
        for i in 0..=800 {
            let r = r0 + 60e3 + (130e3 - 60e3) * f64::from(i) / 800.0;
            let ne = layer.sample(&point(r, colat, lon)).ne;
            if ne > best {
                best = ne;
                best_r = r;
            }
        }
        // Finite, positive (the whole point), and thinned below the overhead Nm.
        assert!(best > 0.0 && best < 1.0e9, "terminator peak Ne = {best}");
        // Scanned peak matches Nm/sqrt(Ch) and sits above 85 km (raised).
        let want_peak = layer.realised_peak_ne(chi);
        assert!(
            (best - want_peak).abs() < 5e-3 * want_peak,
            "{best} vs {want_peak}"
        );
        assert!(
            best_r > r0 + 85e3,
            "peak should rise above 85 km, got {best_r}"
        );
    }

    /// Deep night returns exact vacuum without NaN/overflow.
    #[test]
    fn deep_night_is_vacuum() {
        let r0 = 6_371_000.0;
        let layer = SolarChapmanD::new(1.0e9, r0 + 85e3, 6e3, 0.0, 0.0); // midnight at lon 0
        for alt in [60e3, 85e3, 110e3] {
            let s = layer.sample(&point(r0 + alt, FRAC_PI_2, 0.0));
            assert_eq!(s.ne, 0.0, "night Ne at {alt} m");
            assert!(s.d_ne.iter().all(|v| v.is_finite()));
        }
    }

    /// Horizontal partials match central finite differences at a sunlit point
    /// (the consistency the ray equations rely on).
    #[test]
    fn horizontal_partials_match_fd() {
        let r0 = 6_371_000.0;
        let layer = SolarChapmanD::new(1.0e9, r0 + 85e3, 6e3, 15.0, 12.0);
        let (r, colat, lon) = (r0 + 88e3, 0.9, 0.3);
        let s = layer.sample(&point(r, colat, lon));
        let h = 1e-6;
        let fd_theta = (layer.sample(&point(r, colat + h, lon)).ne
            - layer.sample(&point(r, colat - h, lon)).ne)
            / (2.0 * h);
        let fd_phi = (layer.sample(&point(r, colat, lon + h)).ne
            - layer.sample(&point(r, colat, lon - h)).ne)
            / (2.0 * h);
        let fd_r = (layer.sample(&point(r + 0.5, colat, lon)).ne
            - layer.sample(&point(r - 0.5, colat, lon)).ne)
            / 1.0;
        assert!(
            (s.d_ne[0] - fd_r).abs() < 1e-6 * fd_r.abs().max(1e3),
            "dr {} vs {fd_r}",
            s.d_ne[0]
        );
        assert!(
            (s.d_ne[1] - fd_theta).abs() < 1e-5 * fd_theta.abs().max(1e3),
            "dtheta {} vs {fd_theta}",
            s.d_ne[1]
        );
        assert!(
            (s.d_ne[2] - fd_phi).abs() < 1e-5 * fd_phi.abs().max(1e3),
            "dphi {} vs {fd_phi}",
            s.d_ne[2]
        );
    }
}
