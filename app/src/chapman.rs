//! Horizontally varying Chapman layers for the app.
//!
//! This is the one place the GUI carries physics of its own. It provides one
//! reusable layer type, [`SolarChapmanLayer`], that the D, E and F2 regions are
//! all built from, plus the Chapman grazing-incidence function `Ch(X, chi)` the
//! photochemical ones stand on. Full derivation, sign conventions, and checks:
//! docs/derivations/chapman-grazing.md. The engine crate is untouched.
//!
//! Two things vary horizontally, and they are deliberately separate knobs:
//!
//! * The **ionising-flux slant factor** ([`SlantFactor`]). For a layer in
//!   photochemical equilibrium this is `Ch(X, chi)` at the LOCAL solar zenith
//!   angle of each sampled point, which is why it belongs here and not in the
//!   engine: it replaces `ChapmanLayer::with_zenith_angle` so that absorption
//!   and E-layer ionisation fade smoothly through the terminator instead of
//!   switching off at the engine's 85 deg plane-parallel limit
//!   (`MAX_CHAPMAN_ZENITH_ANGLE`), and so a path crossing the terminator is
//!   ionised only on its sunlit part.
//! * The **overhead-sun peak density** ([`PeakDensitySource`]), queried at each
//!   sampled point's horizontal position. This is what lets the F2 layer follow
//!   a foF2 map instead of being one scalar for the whole domain.
//!
//! Which of the two a layer uses is a physics decision, not a style one:
//!
//! | layer | slant factor | peak-density source            |
//! |-------|--------------|--------------------------------|
//! | D     | `Solar`      | constant overhead anchor       |
//! | E     | `Solar`      | constant overhead anchor       |
//! | F2    | `Overhead`   | foF2 climatology (varies)      |
//!
//! D and E really are close to alpha-Chapman photochemical equilibrium, so
//! their day/night behaviour is *derived* from `Ch(X, chi)`. The F2 layer is
//! not: it is transport-dominated, which is exactly why it survives the night
//! and why the winter anomaly exists, and why every operational prediction
//! (CCIR, URSI, IRI) takes foF2 from an empirical map rather than a zenith-angle
//! law. Feeding F2 a map AND a zenith law would double-count the diurnal
//! variation, so F2 uses [`SlantFactor::Overhead`] and gets all of its
//! horizontal structure from the map. See [`crate::fof2`].

use skipzone::density::{DensitySample, ElectronDensity};
use skipzone::geo::SphericalPoint;
use std::f64::consts::{FRAC_PI_2, PI};

/// 2/sqrt(pi), the constant in erf'(t) and erfcx'(t).
const TWO_OVER_SQRT_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI;
/// sqrt(pi).
const SQRT_PI: f64 = 1.772_453_850_905_516;

/// Levels of the A&S 7.1.14 continued fraction evaluated in [`erfcx`]. Kept at
/// the 48 the function was written with: the recurrence changed, the truncation
/// depth did not.
const CF_DEPTH: u32 = 48;
/// Rescale the continuants once one exceeds this, so the recurrence cannot
/// overflow for a large argument.
const CF_RESCALE_ABOVE: f64 = 1e250;
/// The rescale factor, 2^-60 written exactly. A power of two, so applying it
/// only shifts exponents and leaves every mantissa untouched.
const CF_RESCALE: f64 = 1.0 / 1_152_921_504_606_846_976.0;

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
        //
        // Evaluated by the continuant (forward) recurrence
        //   A_k = t A_{k-1} + (k/2) A_{k-2},   B_k likewise,
        // whose ratio A_n/B_n is the n-th convergent - the SAME continued
        // fraction truncated at the SAME depth as the backward recurrence this
        // replaces. The backward form spends one division per level, 48 of them
        // in a single dependency chain at ~14 cycles each, and this runs twice
        // per density sample (once for D, once for E) on the ray equations' hot
        // path. The forward form is multiply-add only, with one division at the
        // end. Every term is positive, so there is no cancellation and the
        // accumulated error stays at the 1e-15 level; `erfcx_forward_recurrence_
        // matches_backward` pins it against the backward form.
        let (mut a_prev, mut a) = (1.0_f64, t);
        let (mut b_prev, mut b) = (0.0_f64, 1.0_f64);
        for k in 1..=CF_DEPTH {
            let ak = 0.5 * f64::from(k);
            let (a_next, b_next) = (t * a + ak * a_prev, t * b + ak * b_prev);
            a_prev = a;
            a = a_next;
            b_prev = b;
            b = b_next;
            // The continuants grow like t^k. Nothing this crate produces gets
            // near overflow (t <= ~24 here, so A <= 24^48 ~ 1e66), but `erfcx`
            // is public, so rescale defensively. The factor is an exact power
            // of two, making the rescale exact: it changes no digit of A/B.
            if a > CF_RESCALE_ABOVE {
                a *= CF_RESCALE;
                a_prev *= CF_RESCALE;
                b *= CF_RESCALE;
                b_prev *= CF_RESCALE;
            }
        }
        b / (SQRT_PI * a)
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

/// Overhead-sun peak density at one horizontal position, with the horizontal
/// partials the ray equations need.
///
/// The partials are with respect to the ENGINE's coordinates (colatitude
/// theta, longitude phi, both radians), not latitude, so that a source can be
/// dropped into [`SolarChapmanLayer`] without a sign convention being converted
/// anywhere in between. A source whose value happens to be constant reports
/// exactly `[0.0, 0.0]`, which is load-bearing: see
/// [`SolarChapmanLayer::sample`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeakSample {
    /// Overhead-sun peak electron density Nm, m^-3. Never negative.
    pub nm: f64,
    /// `(dNm/dtheta, dNm/dphi)`, m^-3 per radian.
    pub d_nm: [f64; 2],
}

impl PeakSample {
    /// A value with no horizontal structure.
    #[must_use]
    pub const fn uniform(nm: f64) -> Self {
        Self {
            nm,
            d_nm: [0.0, 0.0],
        }
    }
}

/// Where a [`SolarChapmanLayer`] gets its overhead-sun peak density.
///
/// This is the seam that replaces the old baked-in scalar: a layer asks the
/// source at each sampled point's own position, so the layer varies across the
/// domain instead of being one number for the whole Earth. Implementations must
/// supply ANALYTIC partials - a density that varies without a matching gradient
/// drives Hamiltonian drift, which the engine's `H = 0` diagnostic will report.
pub trait PeakDensitySource: Send + Sync {
    /// Sample at colatitude / longitude, radians. `lon` is NOT pre-wrapped:
    /// implementations that index a table must wrap it themselves.
    fn peak(&self, colat: f64, lon: f64) -> PeakSample;
}

/// One overhead-sun peak density for the whole domain: the behaviour every
/// layer had before [`PeakDensitySource`] existed. Kept as a first-class
/// backend rather than a special case, because it is the honest fallback when
/// no map is available.
pub struct ConstantPeak(pub f64);

impl PeakDensitySource for ConstantPeak {
    fn peak(&self, _colat: f64, _lon: f64) -> PeakSample {
        PeakSample::uniform(self.0)
    }
}

/// How a layer's ionising-flux slant factor behaves. See the module docs for
/// why this is a per-layer physics choice.
pub enum SlantFactor {
    /// Photochemical (alpha-Chapman) equilibrium: the grazing function
    /// `Ch(X, chi)` at the local solar zenith angle of each sampled point. The
    /// realised peak is `Nm Ch^{-1/2}` at `r_peak + H ln Ch` (derivation
    /// section 2), staying finite through the terminator where the
    /// plane-parallel `sec(chi)` does not.
    Solar {
        sin_decl: f64,
        cos_decl: f64,
        /// Hour-angle offset so that `H = phi + hour_offset` (radians).
        hour_offset: f64,
    },
    /// `Ch == 1` everywhere, i.e. the overhead-sun profile, with NO
    /// zenith-angle dependence of its own. For a layer whose peak density comes
    /// from an empirical map that already contains the diurnal variation, and
    /// which is not in photochemical equilibrium anyway (F2).
    ///
    /// With a constant peak source this reproduces the engine's
    /// `ChapmanLayer::new` exactly, which
    /// [`tests::overhead_slant_is_bit_identical_to_engine_chapman`] pins.
    Overhead,
}

impl SlantFactor {
    /// The photochemical branch, from the solar declination and the UTC hour
    /// the whole solve is fixed at.
    #[must_use]
    pub fn solar(declination_deg: f64, utc_hours: f64) -> Self {
        let (sin_decl, cos_decl) = declination_deg.to_radians().sin_cos();
        Self::Solar {
            sin_decl,
            cos_decl,
            // H = 15 deg/h (utc + lon/15 - 12) = phi + (pi/12) utc - pi.
            hour_offset: utc_hours * PI / 12.0 - PI,
        }
    }
}

/// Alpha-Chapman layer with a horizontally varying overhead-sun peak density
/// and a per-layer choice of ionising-flux slant factor.
///
///   `Ne = Nm(theta, phi) exp( 1/2 (1 - z - Ch e^{-z}) )`,  `z = (r - r_m)/H`
///
/// where `Ch` is either `Ch(X, chi(theta, phi))` or `1`, per [`SlantFactor`].
/// Both sources of horizontal structure contribute to the gradient; the
/// derivation of the `Ch` terms is docs/derivations/chapman-grazing.md and the
/// `Nm` term is the plain product rule, `(dNm/dtheta) e^{F}`.
pub struct SolarChapmanLayer {
    source: Box<dyn PeakDensitySource>,
    slant: SlantFactor,
    r_peak: f64,
    scale_height: f64,
    /// X = r_peak / H, the (constant) Chapman-function argument.
    big_x: f64,
}

impl SolarChapmanLayer {
    /// `r_peak` and `scale_height` are the alpha-Chapman geometry (SI: m, m);
    /// `source` supplies the overhead-sun peak density (m^-3) at each position.
    #[must_use]
    pub fn new(
        source: Box<dyn PeakDensitySource>,
        slant: SlantFactor,
        r_peak: f64,
        scale_height: f64,
    ) -> Self {
        Self {
            source,
            slant,
            r_peak,
            scale_height,
            big_x: r_peak / scale_height,
        }
    }

    /// The D-region layer as it has always been built: a constant overhead
    /// anchor on the solar grazing branch. Kept as a named constructor so the
    /// call site reads as the physics it is, and so the generalisation cannot
    /// quietly change what the D region does.
    #[must_use]
    pub fn d_region(
        nm: f64,
        r_peak: f64,
        scale_height: f64,
        declination_deg: f64,
        utc_hours: f64,
    ) -> Self {
        Self::new(
            Box::new(ConstantPeak(nm)),
            SlantFactor::solar(declination_deg, utc_hours),
            r_peak,
            scale_height,
        )
    }

    /// Local solar zenith angle chi (radians) at a colatitude/longitude
    /// (derivation eq. 5), or 0 for an [`SlantFactor::Overhead`] layer, which
    /// has no sun of its own. `sample` computes this inline on the hot path.
    #[must_use]
    pub fn zenith_angle(&self, colat: f64, lon: f64) -> f64 {
        match self.slant {
            SlantFactor::Overhead => 0.0,
            SlantFactor::Solar {
                sin_decl,
                cos_decl,
                hour_offset,
            } => {
                let (sin_t, cos_t) = colat.sin_cos();
                let cos_h = (lon + hour_offset).cos();
                (cos_t * sin_decl + sin_t * cos_decl * cos_h)
                    .clamp(-1.0, 1.0)
                    .acos()
            }
        }
    }

    /// The grazing function and its derivative at a position, `(Ch, dCh/dchi)`.
    /// `(1, 0)` for an overhead layer.
    fn slant_at(&self, chi: f64) -> (f64, f64) {
        match self.slant {
            SlantFactor::Overhead => (1.0, 0.0),
            SlantFactor::Solar { .. } => chapman_grazing(self.big_x, chi),
        }
    }

    /// Realised peak density `Nm Ch^{-1/2}` at a position (display only).
    #[must_use]
    pub fn realised_peak_ne(&self, colat: f64, lon: f64) -> f64 {
        let (ch, _) = self.slant_at(self.zenith_angle(colat, lon));
        if ch.is_finite() {
            self.source.peak(colat, lon).nm / ch.sqrt()
        } else {
            0.0
        }
    }

    /// Realised peak altitude above `r_peak`, `H ln Ch` (m). Display only.
    #[must_use]
    pub fn realised_peak_rise(&self, colat: f64, lon: f64) -> f64 {
        let (ch, _) = self.slant_at(self.zenith_angle(colat, lon));
        if ch.is_finite() {
            self.scale_height * ch.ln()
        } else {
            f64::INFINITY
        }
    }
}

impl ElectronDensity for SolarChapmanLayer {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        let theta = p.colat.get();
        let phi = p.lon.get();

        // The chi terms, and the pieces the chi partials need. An Overhead
        // layer skips the whole solar branch: Ch is 1, dCh/dchi is 0, and
        // there is no zenith angle to differentiate.
        let (ch, dch, chi_partials) = match self.slant {
            SlantFactor::Overhead => (1.0, 0.0, None),
            SlantFactor::Solar {
                sin_decl,
                cos_decl,
                hour_offset,
            } => {
                let (sin_t, cos_t) = theta.sin_cos();
                let (sin_h, cos_h) = (phi + hour_offset).sin_cos();
                let cos_chi = (cos_t * sin_decl + sin_t * cos_decl * cos_h).clamp(-1.0, 1.0);
                let (ch, dch) = chapman_grazing(self.big_x, cos_chi.acos());
                // Zero at the subsolar and antisolar points where sin chi -> 0
                // (the gradient vanishes there by symmetry).
                let sin_chi = (1.0 - cos_chi * cos_chi).max(0.0).sqrt();
                let partials = if sin_chi < 1e-9 {
                    None
                } else {
                    let dcos_dtheta = -sin_t * sin_decl + cos_t * cos_decl * cos_h;
                    let dcos_dphi = -sin_t * cos_decl * sin_h;
                    Some((-dcos_dtheta / sin_chi, -dcos_dphi / sin_chi))
                };
                (ch, dch, partials)
            }
        };
        if !ch.is_finite() {
            return DensitySample::VACUUM; // deep night: no layer
        }

        let peak = self.source.peak(theta, phi);
        let z = (p.r.get() - self.r_peak) / self.scale_height;
        let emz = (-z).exp();
        let f = 0.5 * (1.0 - z - ch * emz);
        if f < -700.0 {
            return DensitySample::VACUUM; // underflow: negligible density
        }
        let ef = f.exp();
        let ne = peak.nm * ef;

        // dNe/dr = Ne * 1/2 (Ch e^{-z} - 1)/H (derivation eq. 7).
        let dne_dr = ne * 0.5 * (ch * emz - 1.0) / self.scale_height;

        // Horizontal partials, in two independent contributions:
        //   (a) the slant factor through chi(theta, phi),
        //   (b) the peak density's own horizontal gradient.
        let (mut dne_dtheta, mut dne_dphi) = match chi_partials {
            None => (0.0, 0.0),
            Some((dchi_dtheta, dchi_dphi)) => {
                let dne_dchi = ne * (-0.5 * emz) * dch;
                (dne_dchi * dchi_dtheta, dne_dchi * dchi_dphi)
            }
        };
        // Guarded rather than added unconditionally so that a constant source
        // is BIT-IDENTICAL to the pre-generalisation layer: `x + 0.0` is not
        // the identity on `-0.0`, and these gradients legitimately reach zero
        // with either sign at the subsolar point and at the layer peak.
        if peak.d_nm[0] != 0.0 || peak.d_nm[1] != 0.0 {
            dne_dtheta += peak.d_nm[0] * ef;
            dne_dphi += peak.d_nm[1] * ef;
        }

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

    /// The forward (continuant) recurrence evaluates the SAME continued
    /// fraction, to the same depth, as the backward one it replaced. It is not
    /// bit-identical - the operations differ - so this pins the agreement
    /// instead, densely across the branch and out past the largest `t` any
    /// layer in this crate can produce (`sqrt(X/2)` with `X = r_peak/H`, about
    /// 23 for the D region). Every term of both recurrences is positive, so
    /// there is no cancellation and the two stay within a few 1e-16.
    #[test]
    fn erfcx_forward_recurrence_matches_backward() {
        fn backward(t: f64) -> f64 {
            let mut frac = 0.0_f64;
            for k in (1..=CF_DEPTH).rev() {
                frac = 0.5 * f64::from(k) / (t + frac);
            }
            1.0 / (SQRT_PI * (t + frac))
        }
        let (lo, hi, n) = (3.0_f64, 60.0_f64, 500_000);
        let mut worst = 0.0_f64;
        for i in 0..=n {
            let t = lo + (hi - lo) * f64::from(i) / f64::from(n);
            let (got, want) = (erfcx(t), backward(t));
            worst = worst.max(((got - want) / want).abs());
        }
        assert!(worst < 1e-14, "worst relative disagreement {worst:e}");
    }

    /// The rescale guard keeps a large argument finite and accurate rather
    /// than letting the continuants overflow to inf/inf.
    #[test]
    fn erfcx_is_stable_for_large_arguments() {
        for t in [50.0_f64, 500.0, 5e3, 1e5, 1e8] {
            let got = erfcx(t);
            // erfcx(t) -> 1/(t sqrt(pi)) as t -> infinity.
            let asymptotic = 1.0 / (t * std::f64::consts::PI.sqrt());
            assert!(got.is_finite(), "erfcx({t}) = {got}");
            assert!(
                ((got - asymptotic) / asymptotic).abs() < 1e-3,
                "erfcx({t}) = {got}, asymptote {asymptotic}"
            );
        }
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
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 0.0, 12.0);
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
        let want_peak = layer.realised_peak_ne(colat, lon);
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
        // Midnight at lon 0.
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 0.0, 0.0);
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
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 15.0, 12.0);
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

    /// A peak-density source that varies, with hand-differentiable partials:
    /// `Nm = n0 (1 + a sin theta cos phi)`.
    struct WavySource {
        n0: f64,
        a: f64,
    }

    impl PeakDensitySource for WavySource {
        fn peak(&self, colat: f64, lon: f64) -> PeakSample {
            let (sin_t, cos_t) = colat.sin_cos();
            let (sin_p, cos_p) = lon.sin_cos();
            PeakSample {
                nm: self.n0 * (1.0 + self.a * sin_t * cos_p),
                d_nm: [
                    self.n0 * self.a * cos_t * cos_p,
                    -self.n0 * self.a * sin_t * sin_p,
                ],
            }
        }
    }

    /// THE bit-identity guard for item 1. Generalising the peak density to a
    /// trait object must not perturb the D region by one ulp when the source
    /// returns a constant: the whole validated absorption behaviour rides on it.
    ///
    /// The `d_nm != 0` guard in `sample` exists for exactly this: adding a
    /// literal `0.0` would turn a legitimately negative-zero gradient into
    /// `+0.0` and break bit-identity without changing any physics.
    #[test]
    fn constant_source_is_bit_identical_to_the_old_d_region() {
        let r0 = 6_371_000.0;
        // Reference implementation: the pre-generalisation arithmetic, inlined
        // verbatim, so this test compares against the code that was replaced
        // rather than against the replacement's own output.
        let (nm, r_peak, h) = (1.0e9, r0 + 85e3, 6e3);
        let (sin_decl, cos_decl) = 15.0_f64.to_radians().sin_cos();
        let hour_offset = 12.0 * PI / 12.0 - PI;
        let big_x = r_peak / h;
        let old = |r: f64, theta: f64, phi: f64| -> DensitySample {
            let (sin_t, cos_t) = theta.sin_cos();
            let (sin_h, cos_h) = (phi + hour_offset).sin_cos();
            let cos_chi = (cos_t * sin_decl + sin_t * cos_decl * cos_h).clamp(-1.0, 1.0);
            let chi = cos_chi.acos();
            let (ch, dch) = chapman_grazing(big_x, chi);
            if !ch.is_finite() {
                return DensitySample::VACUUM;
            }
            let z = (r - r_peak) / h;
            let emz = (-z).exp();
            let f = 0.5 * (1.0 - z - ch * emz);
            if f < -700.0 {
                return DensitySample::VACUUM;
            }
            let ne = nm * f.exp();
            let dne_dr = ne * 0.5 * (ch * emz - 1.0) / h;
            let sin_chi = (1.0 - cos_chi * cos_chi).max(0.0).sqrt();
            let (dne_dtheta, dne_dphi) = if sin_chi < 1e-9 {
                (0.0, 0.0)
            } else {
                let dcos_dtheta = -sin_t * sin_decl + cos_t * cos_decl * cos_h;
                let dcos_dphi = -sin_t * cos_decl * sin_h;
                let dne_dchi = ne * (-0.5 * emz) * dch;
                (
                    dne_dchi * (-dcos_dtheta / sin_chi),
                    dne_dchi * (-dcos_dphi / sin_chi),
                )
            };
            DensitySample {
                ne,
                d_ne: [dne_dr, dne_dtheta, dne_dphi],
            }
        };

        let layer = SolarChapmanLayer::d_region(nm, r_peak, h, 15.0, 12.0);
        // Sweep the whole domain the D region is ever sampled over, day and
        // night, including the subsolar point and the deep-night vacuum.
        for ti in 0..=24 {
            let theta = PI * f64::from(ti) / 24.0;
            for pi_ in 0..=24 {
                let phi = -PI + 2.0 * PI * f64::from(pi_) / 24.0;
                for ri in 0..=20 {
                    let r = r0 + 55e3 + (140e3 - 55e3) * f64::from(ri) / 20.0;
                    let want = old(r, theta, phi);
                    let got = layer.sample(&point(r, theta, phi));
                    assert_eq!(
                        got.ne.to_bits(),
                        want.ne.to_bits(),
                        "Ne differs at r={r} theta={theta} phi={phi}"
                    );
                    for k in 0..3 {
                        assert_eq!(
                            got.d_ne[k].to_bits(),
                            want.d_ne[k].to_bits(),
                            "d_ne[{k}] differs at r={r} theta={theta} phi={phi}"
                        );
                    }
                }
            }
        }
    }

    /// The second bit-identity guard: an `Overhead` layer with a constant
    /// source must reproduce the engine's own `ChapmanLayer::new` exactly. That
    /// is what makes it safe to move the F2 layer onto this type - the F2
    /// profile is unchanged, and only its peak density gains structure.
    #[test]
    fn overhead_slant_is_bit_identical_to_engine_chapman() {
        use skipzone::density::ChapmanLayer;
        use skipzone::units::PerCubicMeter;

        let r0 = 6_371_000.0;
        let (nm, r_peak, h) = (1.2e12, r0 + 300e3, 50e3);
        let engine =
            ChapmanLayer::new(PerCubicMeter::new(nm), Meters::new(r_peak), Meters::new(h)).unwrap();
        let ours =
            SolarChapmanLayer::new(Box::new(ConstantPeak(nm)), SlantFactor::Overhead, r_peak, h);
        for ri in 0..=200 {
            let r = r0 + 60e3 + (700e3 - 60e3) * f64::from(ri) / 200.0;
            // Position must not matter for either layer.
            for (theta, phi) in [(0.4_f64, -2.1_f64), (1.5, 0.0), (2.9, 3.0)] {
                let want = engine.sample(&point(r, theta, phi));
                let got = ours.sample(&point(r, theta, phi));
                assert_eq!(got.ne.to_bits(), want.ne.to_bits(), "Ne at r={r}");
                assert_eq!(got.d_ne[0].to_bits(), want.d_ne[0].to_bits(), "dr at r={r}");
                assert_eq!(got.d_ne[1], 0.0);
                assert_eq!(got.d_ne[2], 0.0);
            }
        }
    }

    /// With a varying source, all three partials must still match central
    /// finite differences - on BOTH slant branches, because the theta/phi
    /// gradient is then a sum of two independent terms and a sign error in
    /// either would show up as Hamiltonian drift rather than as a wrong density.
    #[test]
    fn varying_source_partials_match_fd() {
        let r0 = 6_371_000.0;
        let cases: [(&str, SolarChapmanLayer); 2] = [
            (
                "overhead (F2-style)",
                SolarChapmanLayer::new(
                    Box::new(WavySource {
                        n0: 1.0e12,
                        a: 0.45,
                    }),
                    SlantFactor::Overhead,
                    r0 + 300e3,
                    50e3,
                ),
            ),
            (
                "solar (E-style)",
                SolarChapmanLayer::new(
                    Box::new(WavySource {
                        n0: 1.5e11,
                        a: 0.45,
                    }),
                    SlantFactor::solar(15.0, 12.0),
                    r0 + 105e3,
                    10e3,
                ),
            ),
        ];
        for (name, layer) in cases {
            for (r_off, colat, lon) in [
                (280e3, 0.9, 0.3),
                (330e3, 1.9, -1.1),
                (105e3, 1.2, 2.4),
                (140e3, 0.6, -2.8),
            ] {
                let r = r0 + r_off;
                let s = layer.sample(&point(r, colat, lon));
                if s.ne <= 0.0 {
                    continue; // vacuum branch: nothing to differentiate
                }
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
                let tol = |an: f64, fd: f64| (an - fd).abs() < 1e-5 * fd.abs().max(1e-3 * s.ne);
                assert!(tol(s.d_ne[0], fd_r), "{name} dr {} vs {fd_r}", s.d_ne[0]);
                assert!(
                    tol(s.d_ne[1], fd_theta),
                    "{name} dtheta {} vs {fd_theta}",
                    s.d_ne[1]
                );
                assert!(
                    tol(s.d_ne[2], fd_phi),
                    "{name} dphi {} vs {fd_phi}",
                    s.d_ne[2]
                );
            }
        }
    }
}
