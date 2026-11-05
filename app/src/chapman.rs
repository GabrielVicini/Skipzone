//! Horizontally varying Chapman layers for the app.
//!
//! The only physics the app carries itself: one layer type,
//! [`SolarChapmanLayer`], that the D, E and F2 regions are built from, plus the
//! Chapman grazing-incidence function `Ch(X, chi)` the photochemical ones use.
//! Derivation, sign conventions and checks:
//! docs/derivations/chapman-grazing.md.
//!
//! Two things vary horizontally, as separate settings:
//!
//! * The ionising-flux slant factor ([`SlantFactor`]). For a layer in
//!   photochemical equilibrium this is `Ch(X, chi)` at each sampled point's
//!   local solar zenith angle. It replaces `ChapmanLayer::with_zenith_angle`
//!   so absorption and E-layer ionisation fade smoothly through the terminator
//!   instead of stopping at the engine's 85 deg plane-parallel limit
//!   (`MAX_CHAPMAN_ZENITH_ANGLE`), and only the sunlit part of a path is
//!   ionised.
//! * The overhead-sun peak density ([`PeakDensitySource`]), queried at each
//!   sampled point's horizontal position, so the F2 layer can follow a foF2
//!   map.
//!
//! Which of the two a layer uses follows from its physics:
//!
//! | layer | slant factor | peak-density source            |
//! |-------|--------------|--------------------------------|
//! | D     | `Solar`      | constant overhead anchor       |
//! | E     | `Solar`      | constant overhead anchor       |
//! | F2    | `Overhead`   | foF2 climatology (varies)      |
//!
//! D and E are close to alpha-Chapman photochemical equilibrium, so their
//! day/night behaviour is derived from `Ch(X, chi)`. F2 is transport-dominated
//! (it survives the night and shows the winter anomaly), so operational
//! predictions (CCIR, URSI, IRI) take foF2 from an empirical map. A map plus a
//! zenith law would count the diurnal variation twice, so F2 uses
//! [`SlantFactor::Overhead`]. See [`crate::fof2`].

use skipzone::density::{DensitySample, ElectronDensity};
use skipzone::geo::SphericalPoint;
use std::f64::consts::{FRAC_PI_2, PI};

/// 2/sqrt(pi), the constant in erf'(t) and erfcx'(t).
const TWO_OVER_SQRT_PI: f64 = std::f64::consts::FRAC_2_SQRT_PI;
/// sqrt(pi).
const SQRT_PI: f64 = 1.772_453_850_905_516;

/// Levels of the A&S 7.1.14 continued fraction evaluated in [`erfcx`].
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
        // whose ratio A_n/B_n is the n-th convergent. Unlike the backward
        // recurrence (one division per level in a dependency chain), this is
        // multiply-add with a single division at the end, which matters because
        // it runs twice per density sample. Every term is positive, so there is
        // no cancellation and the error stays near 1e-15;
        // `erfcx_forward_recurrence_matches_backward` checks it.
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
/// The partials are with respect to the engine's coordinates (colatitude
/// theta, longitude phi, both radians), so no sign convention is converted on
/// the way into [`SolarChapmanLayer`]. A constant source must report exactly
/// `[0.0, 0.0]`; see [`SolarChapmanLayer::sample`].
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
/// A layer queries the source at each sampled point's position.
/// Implementations must supply analytic partials: a density that varies
/// without a matching gradient drives Hamiltonian drift, which the engine's
/// `H = 0` diagnostic reports.
pub trait PeakDensitySource: Send + Sync {
    /// Sample at colatitude / longitude, radians. `lon` is not pre-wrapped:
    /// implementations that index a table must wrap it themselves.
    fn peak(&self, colat: f64, lon: f64) -> PeakSample;
}

/// One overhead-sun peak density for the whole domain, the fallback when no
/// map is available.
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
    /// `Ch == 1` everywhere, i.e. the overhead-sun profile, with no
    /// zenith-angle dependence of its own. For a layer whose peak density comes
    /// from an empirical map that already contains the diurnal variation, and
    /// which is not in photochemical equilibrium anyway (F2).
    ///
    /// With a constant peak source this reproduces the engine's
    /// `ChapmanLayer::new` exactly, as
    /// `tests::overhead_slant_is_bit_identical_to_engine_chapman` checks.
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
    /// Residual peak density that survives the night, m^-3. See [`Self::floor_at`].
    night_floor_nm: f64,
    /// How much of the photochemical peak rise the profile takes.
    /// See [`Self::with_rise_fraction`]. 1.0 is the textbook layer.
    rise_fraction: f64,
    /// `rise_fraction == 1.0`, precomputed for [`ElectronDensity::sample`]. The
    /// full-rise arithmetic has its own branch so its results stay
    /// bit-identical; the flag is constant per solve.
    full_rise: bool,
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
            night_floor_nm: 0.0,
            rise_fraction: 1.0,
            full_rise: true,
        }
    }

    /// This layer with only `f` of the photochemical peak rise.
    ///
    /// # Why
    ///
    /// A textbook alpha-Chapman layer thins to `Nm Ch^{-1/2}` as the sun sets,
    /// and the whole profile translates upward by `H ln Ch`. Both follow from
    /// photochemical equilibrium and are right for the ionisation peak.
    ///
    /// For absorption the translation dominates and is wrong. The collision
    /// frequency falls on its own scale height `H_nu`, so a profile
    /// that rises by `H ln Ch` loses a further `Ch^{-H/H_nu}` of the `Ne nu`
    /// product on top of the `Ch^{-1/2}` thinning. That makes the absorption
    /// zenith law
    ///
    /// ```text
    /// A(chi) ~ cos^n chi,   n = 1/2 + f H / H_nu
    /// ```
    ///
    /// which at `f = 1` and the shipping anchors is `n = 1.39`, against the
    /// `n ~ 0.75` that riometer and A1 absorption measurements report. No
    /// setting of the two scale heights inside their calibration bounds reaches
    /// 0.75, and with `H_nu -> infinity` the law still floors at `n = 1/2`.
    /// `bin/zenith_probe` measures this.
    ///
    /// # Why a fraction
    ///
    /// The translation assumes the whole layer is in photochemical equilibrium
    /// with a recombination time short against the sun's motion. That holds near
    /// the peak but not in the lower D region (60 to 75 km), where `nu` is
    /// largest and most absorption happens: there `Ne` tracks production at
    /// roughly fixed height, the `n = 1/2` law with no rise.
    ///
    /// The real layer sits between the two limits, and `f` says where:
    ///
    /// * `f = 1`: the textbook layer, the default from [`Self::new`];
    /// * `f = 0`: amplitude law only, `Nm Ch^{-1/2}` at fixed height, `n = 1/2`.
    ///
    /// # The algebra
    ///
    /// Writing `u = z - f ln Ch`, the layer is
    ///
    /// ```text
    /// Ne = Nm Ch^{-1/2} exp( 1/2 (1 - u - e^{-u}) )
    /// ```
    ///
    /// At `f = 1` the `Ch^{-1/2}` prefactor and the `+ln Ch` inside `u` cancel
    /// exactly, giving the textbook form.
    ///
    /// # Panics
    /// If `f` is not finite and in `0..=1`. Values outside have no derived
    /// meaning, and clamping would hide a caller's error in the parameter the
    /// model is most sensitive to.
    #[must_use]
    pub fn with_rise_fraction(mut self, f: f64) -> Self {
        assert!(
            f.is_finite() && (0.0..=1.0).contains(&f),
            "rise fraction must be in 0..=1, got {f}"
        );
        self.rise_fraction = f;
        self.full_rise = f == 1.0;
        self
    }

    /// This layer with a residual peak density [m^-3] that survives the night.
    ///
    /// # Why the floor is an input
    ///
    /// `Ch(X, chi)` diverges past the terminator, so a pure photochemical layer
    /// is exactly zero in darkness. The real night D region settles onto a
    /// residual maintained by galactic cosmic rays and scattered Lyman-alpha on
    /// NO, sources a Chapman layer does not describe.
    ///
    /// Absorption is proportional to `Ne nu`, so without a floor there is no
    /// night-time absorption on any band (0.00 dB at 7 and 14 MHz on a midnight
    /// path), and a calibration pushes the atmospheric noise anchors to their
    /// bounds to compensate.
    #[must_use]
    pub fn with_night_floor(mut self, night_floor_nm: f64) -> Self {
        self.night_floor_nm = night_floor_nm.max(0.0);
        self
    }

    /// The residual night layer at radius `r`: `(Ne, dNe/dr)`.
    ///
    /// Same alpha-Chapman shape and peak height as the sunlit layer, since the
    /// profile either side of the peak is set by the neutral scale height. Only
    /// the amplitude differs, with no zenith dependence because its source is
    /// not the sun.
    fn floor_at(&self, r: f64) -> (f64, f64) {
        if self.night_floor_nm <= 0.0 {
            return (0.0, 0.0);
        }
        let z = (r - self.r_peak) / self.scale_height;
        let emz = (-z).exp();
        let f = 0.5 * (1.0 - z - emz);
        if f < -700.0 {
            return (0.0, 0.0);
        }
        let ne = self.night_floor_nm * f.exp();
        (ne, ne * 0.5 * (emz - 1.0) / self.scale_height)
    }

    /// The D-region layer: a constant overhead anchor on the solar grazing
    /// branch.
    #[must_use]
    pub fn d_region(
        nm: f64,
        r_peak: f64,
        scale_height: f64,
        declination_deg: f64,
        utc_hours: f64,
        night_floor_fraction: f64,
    ) -> Self {
        Self::new(
            Box::new(ConstantPeak(nm)),
            SlantFactor::solar(declination_deg, utc_hours),
            r_peak,
            scale_height,
        )
        .with_night_floor(nm * night_floor_fraction)
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

    /// Realised peak altitude above `r_peak`, `f H ln Ch` (m). Display only.
    ///
    /// Includes the rise fraction, so a layer built with `f < 1` reports its
    /// own peak height.
    #[must_use]
    pub fn realised_peak_rise(&self, colat: f64, lon: f64) -> f64 {
        let (ch, _) = self.slant_at(self.zenith_angle(colat, lon));
        if ch.is_finite() {
            self.rise_fraction * self.scale_height * ch.ln()
        } else {
            f64::INFINITY
        }
    }
}

impl SolarChapmanLayer {
    /// The `rise_fraction < 1` branch of [`ElectronDensity::sample`], kept
    /// separate so the full-rise path keeps its own arithmetic.
    ///
    /// With `L = ln Ch` and `u = z - f L`:
    ///
    /// ```text
    /// Ne        = Nm Ch^{-1/2} exp(F),      F = 1/2 (1 - u - e^{-u})
    /// dNe/dr    = Ne (1/2)(e^{-u} - 1) / H
    /// dNe/dchi  = Ne (Ch'/Ch) (-1/2) [ 1 + f (e^{-u} - 1) ]
    /// ```
    ///
    /// The `dNe/dchi` form is the product rule over both places `Ch` appears -
    /// the `Ch^{-1/2}` prefactor and the shift inside `u` - and at `f = 1` it
    /// collapses to the full-rise expression `Ne (-1/2 e^{-z}) Ch'`, which
    /// `partial_rise_reduces_to_the_textbook_layer_at_f_one` checks.
    fn sample_partial_rise(
        &self,
        p: &SphericalPoint,
        peak: &PeakSample,
        z: f64,
        ch: f64,
        dch: f64,
        chi_partials: Option<(f64, f64)>,
    ) -> DensitySample {
        let ln_ch = ch.ln();
        let u = z - self.rise_fraction * ln_ch;
        let emu = (-u).exp();
        let big_f = 0.5 * (1.0 - u - emu);
        // Ch^{-1/2} as a sqrt rather than exp(-L/2).
        let thinning = 1.0 / ch.sqrt();
        if big_f < -700.0 || !thinning.is_finite() {
            let (ne, dne_dr) = self.floor_at(p.r.get());
            return DensitySample {
                ne,
                d_ne: [dne_dr, 0.0, 0.0],
            };
        }
        // Ne = Nm * shape, so the peak's own gradient enters as d_nm * shape,
        // exactly as `ef` does on the full-rise path.
        let shape = thinning * big_f.exp();
        let ne = peak.nm * shape;

        let (floor_ne, floor_dr) = self.floor_at(p.r.get());
        if floor_ne > ne {
            return DensitySample {
                ne: floor_ne,
                d_ne: [floor_dr, 0.0, 0.0],
            };
        }

        let dne_dr = ne * 0.5 * (emu - 1.0) / self.scale_height;

        let (mut dne_dtheta, mut dne_dphi) = match chi_partials {
            None => (0.0, 0.0),
            Some((dchi_dtheta, dchi_dphi)) => {
                let dne_dchi = ne * (dch / ch) * (-0.5) * (1.0 + self.rise_fraction * (emu - 1.0));
                (dne_dchi * dchi_dtheta, dne_dchi * dchi_dphi)
            }
        };
        if peak.d_nm[0] != 0.0 || peak.d_nm[1] != 0.0 {
            dne_dtheta += peak.d_nm[0] * shape;
            dne_dphi += peak.d_nm[1] * shape;
        }

        DensitySample {
            ne,
            d_ne: [dne_dr, dne_dtheta, dne_dphi],
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
            // Deep night: the photochemical layer is gone, and what is left is
            // the residual the sun did not make.
            let (ne, dne_dr) = self.floor_at(p.r.get());
            return DensitySample {
                ne,
                d_ne: [dne_dr, 0.0, 0.0],
            };
        }

        let peak = self.source.peak(theta, phi);
        let z = (p.r.get() - self.r_peak) / self.scale_height;
        // `emz` and `f` below are the full-rise quantities; the partial-rise
        // branch substitutes u = z - rise_fraction ln Ch for z and carries a
        // Ch^{-1/2} prefactor. The two agree algebraically at f = 1, but the
        // full-rise arithmetic is kept separate so the default layer stays
        // bit-identical.
        if !self.full_rise {
            return self.sample_partial_rise(p, &peak, z, ch, dch, chi_partials);
        }
        let emz = (-z).exp();
        let f = 0.5 * (1.0 - z - ch * emz);
        if f < -700.0 {
            // Underflow: the sunlit layer is negligible here, so whatever the
            // residual holds is all there is.
            let (ne, dne_dr) = self.floor_at(p.r.get());
            return DensitySample {
                ne,
                d_ne: [dne_dr, 0.0, 0.0],
            };
        }
        let ef = f.exp();
        let ne = peak.nm * ef;
        // The floor is a lower bound on the ionisation and takes over wherever
        // the sunlit layer falls below it, on both sides of the terminator.
        // Taking the larger of the two rather than summing leaves a fully
        // sunlit layer unchanged.
        let (floor_ne, floor_dr) = self.floor_at(p.r.get());
        if floor_ne > ne {
            return DensitySample {
                ne: floor_ne,
                d_ne: [floor_dr, 0.0, 0.0],
            };
        }

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
        // Guarded rather than added unconditionally: `x + 0.0` is not the
        // identity on `-0.0`, and these gradients reach zero with either sign at
        // the subsolar point and the layer peak, so a constant source must skip
        // the addition to stay bit-identical.
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
    /// A pure alpha-Chapman layer is exactly zero past the terminator. The
    /// night floor must leave a density behind at night and leave the sunlit
    /// layer untouched.
    #[test]
    fn the_night_floor_replaces_an_exactly_zero_night() {
        let r0 = 6_371_000.0;
        let r_peak = r0 + 85e3;
        let nm = 1.0e9;
        // Midnight at longitude 0 with the sun over the far side.
        let at = |layer: &SolarChapmanLayer, alt_km: f64, lon_deg: f64| {
            layer
                .sample(&SphericalPoint::new(
                    skipzone::units::Meters::new(r0 + alt_km * 1e3),
                    skipzone::units::Radians::new(45.0_f64.to_radians()),
                    skipzone::units::Radians::new(lon_deg.to_radians()),
                ))
                .ne
        };
        let bare = SolarChapmanLayer::d_region(nm, r_peak, 6e3, 0.0, 0.0, 0.0);
        let floored = SolarChapmanLayer::d_region(nm, r_peak, 6e3, 0.0, 0.0, 0.10);

        // Night side: the bare layer is exactly zero; the floored one is not.
        let night_lon = 0.0;
        assert_eq!(
            at(&bare, 85.0, night_lon),
            0.0,
            "a bare Chapman night is exact vacuum"
        );
        let night = at(&floored, 85.0, night_lon);
        assert!(
            night > 0.05 * nm && night < 0.15 * nm,
            "the night floor should sit near its fraction of the peak, got {night:e}"
        );

        // Day side: the floor must not touch a layer that is well above it.
        let day_lon = 180.0;
        let (bare_day, floored_day) = (at(&bare, 85.0, day_lon), at(&floored, 85.0, day_lon));
        assert!(bare_day > 0.5 * nm, "sanity: the day layer is producing");
        assert!(
            (bare_day - floored_day).abs() < 1e-9 * bare_day,
            "a sunlit layer must be bit-identical: {bare_day:e} vs {floored_day:e}"
        );

        // The floor has the layer's profile shape: it falls away from the peak
        // in both directions.
        let peak = at(&floored, 85.0, night_lon);
        assert!(at(&floored, 70.0, night_lon) < peak);
        assert!(at(&floored, 100.0, night_lon) < peak);

        // A zero fraction gives the bare layer exactly.
        let off = SolarChapmanLayer::d_region(nm, r_peak, 6e3, 0.0, 0.0, 0.0);
        assert_eq!(at(&off, 85.0, night_lon), 0.0);
    }

    use super::*;
    use skipzone::units::{Meters, Radians};

    fn point(r: f64, colat: f64, lon: f64) -> SphericalPoint {
        SphericalPoint::new(Meters::new(r), Radians::new(colat), Radians::new(lon))
    }

    /// The partial-rise branch must reduce to the textbook one at `f = 1`.
    ///
    /// `with_rise_fraction(1.0)` sets `full_rise`, so the flag is forced off
    /// here to run the general arithmetic. The operation order differs, so
    /// agreement is to a few ulps rather than bit for bit.
    #[test]
    fn partial_rise_reduces_to_the_textbook_layer_at_f_one() {
        const R0: f64 = 6_371_000.0;
        let textbook = SolarChapmanLayer::d_region(1.0e9, R0 + 85e3, 6e3, 15.0, 12.0, 0.10);
        let mut general = SolarChapmanLayer::d_region(1.0e9, R0 + 85e3, 6e3, 15.0, 12.0, 0.10)
            .with_rise_fraction(1.0);
        general.full_rise = false; // force the general path at f = 1

        let mut worst = 0.0_f64;
        for ti in 0..=16 {
            let theta = PI * f64::from(ti) / 16.0;
            for pi_ in 0..=16 {
                let phi = -PI + 2.0 * PI * f64::from(pi_) / 16.0;
                for ri in 0..=16 {
                    let r = R0 + 55e3 + (140e3 - 55e3) * f64::from(ri) / 16.0;
                    let want = textbook.sample(&point(r, theta, phi));
                    let got = general.sample(&point(r, theta, phi));
                    let rel = |a: f64, b: f64, scale: f64| {
                        if a == b {
                            0.0
                        } else {
                            (a - b).abs() / scale.abs().max(1e-300)
                        }
                    };
                    worst = worst.max(rel(got.ne, want.ne, want.ne));
                    for k in 0..3 {
                        worst = worst.max(rel(
                            got.d_ne[k],
                            want.d_ne[k],
                            want.d_ne[k].abs().max(1e-6 * want.ne),
                        ));
                    }
                }
            }
        }
        assert!(
            worst < 1e-12,
            "partial-rise at f=1 disagrees with the textbook layer by {worst:e}"
        );
    }

    /// The partial-rise branch's analytic gradients must match central
    /// differences on both slant branches and with a varying peak source. A
    /// mismatched gradient shows up as Hamiltonian drift along the ray.
    #[test]
    fn partial_rise_partials_match_fd() {
        const R0: f64 = 6_371_000.0;
        for f in [0.0_f64, 0.15, 0.28, 0.5, 0.9] {
            for source_varies in [false, true] {
                let layer = if source_varies {
                    SolarChapmanLayer::new(
                        Box::new(WavySource { n0: 1.0e9, a: 0.45 }),
                        SlantFactor::solar(15.0, 12.0),
                        R0 + 85e3,
                        6e3,
                    )
                    .with_rise_fraction(f)
                } else {
                    SolarChapmanLayer::d_region(1.0e9, R0 + 85e3, 6e3, 15.0, 12.0, 0.0)
                        .with_rise_fraction(f)
                };
                for (r_off, colat, lon) in [
                    (70e3, 0.9, 0.3),
                    (85e3, 1.2, 0.0),
                    (95e3, 1.9, -1.1),
                    (110e3, 0.6, -2.8),
                ] {
                    let r = R0 + r_off;
                    let s = layer.sample(&point(r, colat, lon));
                    if s.ne <= 0.0 {
                        continue;
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
                    let ok = |an: f64, fd: f64| (an - fd).abs() < 1e-5 * fd.abs().max(1e-3 * s.ne);
                    assert!(ok(s.d_ne[0], fd_r), "f={f}: dr {} vs {fd_r}", s.d_ne[0]);
                    assert!(
                        ok(s.d_ne[1], fd_theta),
                        "f={f}: dtheta {} vs {fd_theta}",
                        s.d_ne[1]
                    );
                    assert!(
                        ok(s.d_ne[2], fd_phi),
                        "f={f}: dphi {} vs {fd_phi}",
                        s.d_ne[2]
                    );
                }
            }
        }
    }

    /// The solar-zenith law this layer imposes on absorption is derived:
    /// `A ~ cos^n chi` with `n = 1/2 + H_layer/H_nu`, from two contributions:
    ///
    ///   * the realised peak is `Nm Ch^{-1/2}`, worth exactly 1/2;
    ///   * the profile translates upward by `H ln Ch` while the collision
    ///     frequency falls on `H_nu`, losing a further `Ch^{-H/H_nu}` of the
    ///     `Ne nu` product.
    ///
    /// This bounds what a D-region calibration can reach. `bin/mode_audit`
    /// measures 1.3-1.5 through a traced ray at the shipping anchors, and
    /// `bin/zenith_probe` measures 1.386 on a vertical column where this formula
    /// predicts 1.396. If this test fails, conclusions from both tools need
    /// re-deriving.
    #[test]
    fn the_zenith_exponent_is_one_half_plus_the_scale_height_ratio() {
        use skipzone::collision::{CollisionFrequency, ExponentialCollisions};
        use skipzone::units::PerSecond;

        const R0: f64 = 6_371_000.0;
        let omega = 2.0 * PI * 7.04e6;

        // Vertical integral of the non-deviative absorption kernel, Ne nu/omega^2,
        // in arbitrary units - constants independent of chi cancel in the ratio.
        let column = |h_layer_km: f64, h_nu_km: f64, rise: f64, chi_deg: f64| -> f64 {
            let layer = SolarChapmanLayer::d_region(
                1.0e9,
                R0 + 85e3,
                h_layer_km * 1e3,
                0.0,  // declination
                12.0, // UTC: subsolar at colat 90 deg, lon 0
                0.0,  // no night floor: a chi-independent term would flatten
                      // the law being measured
            )
            .with_rise_fraction(rise);
            let coll = ExponentialCollisions::new(
                PerSecond::new(5.0e6),
                Meters::new(R0 + 70e3),
                Meters::new(h_nu_km * 1e3),
            )
            .expect("valid collision profile");
            // Along lon = 0 with decl = 0, cos chi = sin(colat), so colat = 90 - chi.
            let colat = (90.0 - chi_deg).to_radians();
            let (lo, hi, step) = (40e3, 140e3, 25.0);
            let n = ((hi - lo) / step) as usize;
            (0..=n)
                .map(|i| {
                    let p = point(R0 + lo + step * i as f64, colat, 0.0);
                    let nu = coll.sample(&p).nu;
                    let w = if i == 0 || i == n { 0.5 } else { 1.0 };
                    w * layer.sample(&p).ne * nu / (omega * omega + nu * nu) * step
                })
                .sum()
        };

        // Least-squares slope of log A on log cos chi, fitted below 70 deg where
        // Ch still tracks sec chi (past that it is the grazing correction being
        // measured, not the law).
        let exponent = |h_layer_km: f64, h_nu_km: f64, rise: f64| -> f64 {
            let pts: Vec<(f64, f64)> = [0.0_f64, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0]
                .iter()
                .map(|&c| {
                    (
                        c.to_radians().cos().ln(),
                        column(h_layer_km, h_nu_km, rise, c).ln(),
                    )
                })
                .collect();
            let n = pts.len() as f64;
            let mx = pts.iter().map(|p| p.0).sum::<f64>() / n;
            let my = pts.iter().map(|p| p.1).sum::<f64>() / n;
            let num: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            let den: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
            num / den
        };

        // Across the whole box `calib.rs` allows both scale heights to move in.
        for h_layer in [4.0_f64, 6.0, 8.0] {
            for h_nu in [5.0_f64, 6.7, 9.0] {
                let got = exponent(h_layer, h_nu, 1.0);
                let want = 0.5 + h_layer / h_nu;
                assert!(
                    (got - want).abs() < 0.03,
                    "H={h_layer} H_nu={h_nu}: measured n={got:.4}, derived {want:.4}"
                );
            }
        }

        // Across the rise fraction, of which the above is the f = 1 slice. It is
        // the one anchor that reaches the exponent.
        for h_layer in [4.0_f64, 6.0] {
            for h_nu in [5.0_f64, 6.7] {
                for rise in [0.0_f64, 0.25, 0.5, 0.75, 1.0] {
                    let got = exponent(h_layer, h_nu, rise);
                    let want = 0.5 + rise * h_layer / h_nu;
                    assert!(
                        (got - want).abs() < 0.03,
                        "H={h_layer} H_nu={h_nu} f={rise}: measured n={got:.4}, \
                         derived {want:.4}"
                    );
                }
            }
        }

        // The f = 0 endpoint is the floor of the functional form: Ch^{-1/2}
        // thinning alone, with no rise term.
        let floor = exponent(6.0, 6.7, 0.0);
        assert!(
            (floor - 0.5).abs() < 0.02,
            "the f = 0 limit must be exactly the Ch^-1/2 law, n = 1/2, got {floor:.4}"
        );

        // With the shipping anchors the law is far steeper than the ~0.75 that
        // riometer and A1 absorption measurements report, and no allowed
        // setting of the two scale heights reaches it (the box floors at
        // 1/2 + 4/9).
        let shipping = exponent(
            crate::scenario::D_REGION_SCALE_HEIGHT_KM,
            crate::scenario::NU_SCALE_HEIGHT_KM,
            crate::scenario::D_REGION_RISE_FRACTION,
        );
        assert!(
            shipping > 1.3 && shipping < 1.45,
            "shipping anchors should give n ~ 1.39, got {shipping:.4}"
        );
        // Read from `calib.rs` rather than literals so that widening a bound
        // trips this. The smallest reachable exponent is
        // 1/2 + H_D(min)/H_nu(max); if it drops to ~0.75, the conclusion in the
        // wiki and `bin/zenith_probe` no longer holds.
        let ion = crate::calib::Anchors::default().ionosphere;
        let n_min = 0.5 + ion.d_scale_height_km.min / ion.nu_scale_height_km.max;
        assert!(
            n_min > 0.75,
            "the scale-height bounds now admit n = {n_min:.3} <= 0.75, so the \
             calibration CAN reach the measured zenith law by scale heights \
             alone - Ionosphere-Model.md and Calibration.md need rewriting"
        );
    }

    /// The forward (continuant) recurrence evaluates the same continued
    /// fraction to the same depth as the backward one. The operations differ,
    /// so this checks agreement within a few 1e-16 across the branch and past
    /// the largest `t` any layer here produces (`sqrt(X/2)` with
    /// `X = r_peak/H`, about 23 for the D region).
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
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 0.0, 12.0, 0.0);
        // Subsolar point (decl 0, utc 12 => hour_offset 0) is colat 90, lon 0.
        // A point 86 deg away from it sees chi ~ 86 deg: colat 4 deg, lon 0
        // gives cos chi = sin(4 deg) => chi = 86 deg, just past the engine's
        // 85 deg plane-parallel limit.
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
        // Finite, positive, and thinned below the overhead Nm.
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
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 0.0, 0.0, 0.0);
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
        let layer = SolarChapmanLayer::d_region(1.0e9, r0 + 85e3, 6e3, 15.0, 12.0, 0.0);
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

    /// With a constant peak source, the D region must match the plain
    /// constant-peak arithmetic bit for bit.
    ///
    /// This is what the `d_nm != 0` guard in `sample` is for: adding a literal
    /// `0.0` would turn a negative-zero gradient into `+0.0`.
    #[test]
    fn constant_source_is_bit_identical_to_the_old_d_region() {
        let r0 = 6_371_000.0;
        // Reference implementation: the constant-peak arithmetic written out
        // inline, independent of the layer type.
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

        let layer = SolarChapmanLayer::d_region(nm, r_peak, h, 15.0, 12.0, 0.0);
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

    /// An `Overhead` layer with a constant source must reproduce the engine's
    /// `ChapmanLayer::new` exactly, so the F2 profile shape matches the engine's
    /// and only its peak density gains structure.
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
    /// finite differences on both slant branches. The theta/phi gradient is a
    /// sum of two terms, and a sign error in either shows up as Hamiltonian
    /// drift rather than a wrong density.
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
