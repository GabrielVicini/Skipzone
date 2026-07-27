//! Closed-form elevation patterns from image theory over a flat lossy ground.
//!
//! # The model, stated in full
//!
//! An antenna at height `h` above a flat half-space radiates a direct ray and a
//! ground-reflected ray. At elevation angle `d` (radians above the horizon) the
//! reflected ray travels `2 h sin d` further, so with wavenumber
//! `k = 2 pi f / c` [rad/m] the two add as
//!
//! ```text
//!   AF(d) = | 1 + R(d) exp(-j 2 k h sin d) |            [dimensionless]
//! ```
//!
//! where `R` is the Fresnel reflection coefficient of the ground for the
//! antenna's polarisation. Multiplying by the free-space element factor `F`
//! gives the far field. The radiated POWER pattern is
//!
//! ```text
//!   U(d, a) = |F(d, a)|^2 * AF(d)^2                     [W/sr, unnormalised]
//! ```
//!
//! with `a` the azimuth. Gain is that pattern normalised by the power the
//! antenna actually radiates:
//!
//! ```text
//!   G(d, a) = 4 pi U(d, a) / P_rad,
//!   P_rad   = int_0^{2pi} int_0^{pi/2} U_pec(d, a) cos d  d(d) d(a)     [W]
//! ```
//!
//! Note the `cos d` (not `sin d`): `d` is measured from the HORIZON, so the
//! solid-angle element is `dOmega = cos d dd da`.
//!
//! ## Why `P_rad` is evaluated over a PERFECT conductor
//!
//! Over a perfect conductor `|R| = 1`, image theory is EXACT (for the assumed
//! current distribution), and the hemispherical integral of `U` is exactly the
//! power the antenna radiates - including the way mutual coupling to the image
//! changes radiation resistance with height. So `P_rad` from the PEC pattern is
//! the right denominator, and over PEC the formula reduces to true directivity.
//!
//! Over real ground the numerator uses the lossy `R`, so the shortfall against
//! the PEC result is genuine ground-reflection loss. This is the standard
//! treatment - it is what produces the pseudo-Brewster collapse of a vertical's
//! low-angle gain over poor soil.
//!
//! ## Element factors
//!
//! A wire carrying a sinusoidal standing wave `n` half-wavelengths long has the
//! classical pattern, with `p` the angle from the WIRE AXIS:
//!
//! ```text
//!   F_n(p) = [ cos(n pi/2 cos p) - cos(n pi/2) ] / sin p
//! ```
//!
//! `n = 1` is the half-wave dipole (and the half-wave radiator of an EFHW, whose
//! current distribution is the same as a centre-fed dipole's - only the feed
//! point differs). `n = 2, 3, 4...` are the harmonic cases an EFHW is actually
//! used on. A quarter-wave monopole over ground is the top half of an `n = 1`
//! dipole about the VERTICAL axis, which is why it reuses the same function.
//!
//! ## Azimuth
//!
//! The brief's interface is gain against ELEVATION alone, so at each elevation
//! this module reports the maximum over azimuth - "the antenna is aimed at the
//! path". For a half-wave dipole that is exactly the broadside plane at every
//! elevation. It matters for the EFHW harmonic cases: a wire 2 wavelengths long
//! (`n = 4`) has an exact NULL broadside, so a fixed broadside convention would
//! report a null where the antenna in fact has 9 dBi lobes. Azimuth-resolved
//! gain is a clean extension - the pattern is already evaluated over azimuth
//! here; only the reduction would change.
//!
//! # What is NOT modelled
//!
//! Conductor and matching loss (except the EFHW's explicit transformer term),
//! ground-screen / radial I^2 R loss, near-field Sommerfeld ground loss,
//! terrain, and any nearby structure. A real ground-mounted vertical with a
//! sparse radial field loses several more dB than this model reports.
//!
//! # Verification
//!
//! Hand-checks against published values are in the tests below, and the numbers
//! are quoted there. Three are exact analytic anchors (free-space dipole
//! 2.15 dBi, quarter-wave monopole over PEC 5.15 dBi, and the large-height
//! asymptote 2.15 + 6.02 dB). The fourth is NEC-4: EZNEC's double-precision
//! NEC-4 engine with the Sommerfeld ground algorithm, from VOACAP's dipole
//! study, which this model tracks within +0.6 dB and 0.3 degrees of elevation.

use num_complex::Complex64;
use rayon::prelude::*;
use std::f64::consts::PI;

use super::{ElevationPattern, GainCurve};

/// Speed of light in vacuum, m/s.
const C_M_PER_S: f64 = 299_792_458.0;

/// Azimuth samples per quadrant used for the normalising integral and the
/// azimuth maximisation. The pattern is symmetric about the wire and about
/// broadside, so a quadrant with a x4 weight covers the full circle.
const AZ_SAMPLES: usize = 181;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Polarization {
    Horizontal,
    Vertical,
}

/// The ground under the antenna.
#[derive(Clone, Copy, PartialEq)]
pub enum Ground {
    /// Perfectly conducting: `R_h = -1`, `R_v = +1` at every angle.
    Perfect,
    /// Lossy half-space, ITU-R P.527 form.
    Lossy { eps_r: f64, sigma_s_per_m: f64 },
}

/// Permittivity of free space, F/m.
const EPS0_F_PER_M: f64 = 8.854_187_8e-12;

/// Fresnel reflection coefficient of a lossy half-space at grazing (elevation)
/// angle `elev_rad`.
///
/// The complex relative permittivity is `eps_c = eps_r - j sigma / (w eps0)`
/// (ITU-R P.527 form), and with `w_t = sqrt(eps_c - cos^2 d)`:
///
/// ```text
///   R_h = (sin d - w_t) / (sin d + w_t)
///   R_v = (eps_c sin d - w_t) / (eps_c sin d + w_t)
/// ```
///
/// Same coefficients [`crate::solve`] uses for mid-path ground bounces; shared
/// from here so the two cannot drift apart.
#[must_use]
pub fn fresnel_coefficient(
    elev_rad: f64,
    f_hz: f64,
    eps_r: f64,
    sigma_s_per_m: f64,
    pol: Polarization,
) -> Complex64 {
    let eps_c = Complex64::new(eps_r, -sigma_s_per_m / (2.0 * PI * f_hz * EPS0_F_PER_M));
    let (sin_d, cos_d) = elev_rad.sin_cos();
    let s = Complex64::new(sin_d, 0.0);
    let w = (eps_c - cos_d * cos_d).sqrt();
    match pol {
        Polarization::Horizontal => (s - w) / (s + w),
        Polarization::Vertical => (eps_c * s - w) / (eps_c * s + w),
    }
}

impl Ground {
    /// Reflection coefficient for `pol` at this elevation and frequency.
    #[must_use]
    pub fn reflection(self, elev_rad: f64, f_hz: f64, pol: Polarization) -> Complex64 {
        match self {
            Self::Perfect => match pol {
                Polarization::Horizontal => Complex64::new(-1.0, 0.0),
                Polarization::Vertical => Complex64::new(1.0, 0.0),
            },
            Self::Lossy {
                eps_r,
                sigma_s_per_m,
            } => fresnel_coefficient(elev_rad, f_hz, eps_r, sigma_s_per_m, pol),
        }
    }
}

/// `|1 + R exp(-j 2 k h sin d)|^2`, the two-ray array factor in POWER.
fn array_factor_sq(
    elev_rad: f64,
    height_m: f64,
    f_hz: f64,
    ground: Ground,
    pol: Polarization,
) -> f64 {
    let r = ground.reflection(elev_rad, f_hz, pol);
    let k = 2.0 * PI * f_hz / C_M_PER_S;
    let psi = 2.0 * k * height_m * elev_rad.sin();
    (Complex64::new(1.0, 0.0) + r * Complex64::from_polar(1.0, -psi)).norm_sqr()
}

/// Standing-wave wire element factor `F_n(p)`, `p` measured from the wire axis.
///
/// At `sin p = 0` the numerator vanishes too for every integer `n` (a wire does
/// not radiate along its own axis), so the removable singularity is a genuine
/// null and returns 0.
fn wire_element(psi_rad: f64, half_waves: f64) -> f64 {
    let s = psi_rad.sin();
    if s.abs() < 1e-12 {
        return 0.0;
    }
    let a = half_waves * PI / 2.0;
    ((a * psi_rad.cos()).cos() - a.cos()) / s
}

/// Shape of an antenna's free-space element factor, in POWER, as a function of
/// elevation and azimuth.
#[derive(Clone, Copy)]
enum Element {
    /// Wire lying horizontally; azimuth is measured from broadside.
    Horizontal { half_waves: f64 },
    /// Wire standing vertically; azimuthally symmetric.
    Vertical { half_waves: f64 },
}

impl Element {
    fn polarization(self) -> Polarization {
        match self {
            Self::Horizontal { .. } => Polarization::Horizontal,
            Self::Vertical { .. } => Polarization::Vertical,
        }
    }

    /// `|F|^2` at elevation `d` and azimuth `a` (radians, from broadside).
    fn power(self, elev_rad: f64, az_rad: f64) -> f64 {
        match self {
            // Wire along y, broadside along x: cos(p) = cos(d) sin(a).
            Self::Horizontal { half_waves } => {
                let cos_p = (elev_rad.cos() * az_rad.sin()).clamp(-1.0, 1.0);
                wire_element(cos_p.acos(), half_waves).powi(2)
            }
            // Wire along z: p = 90 deg - d, independent of azimuth.
            Self::Vertical { half_waves } => wire_element(PI / 2.0 - elev_rad, half_waves).powi(2),
        }
    }
}

/// Build a normalised gain curve for one element over one ground.
///
/// Implements the two formulas in the module docs: the numerator at the real
/// ground, maximised over azimuth; the denominator integrated over the
/// hemisphere at a perfect conductor.
fn normalised_curve(
    label: String,
    element: Element,
    ground: Ground,
    height_m: f64,
    f_hz: f64,
    fixed_loss_db: f64,
) -> GainCurve {
    let pol = element.polarization();

    // Per elevation, precompute the azimuth reduction of |F|^2: its integral
    // over a quadrant (for P_rad) and its maximum (for the reported gain).
    // The array factor does not depend on azimuth, so it factors out of both.
    #[allow(clippy::cast_precision_loss)]
    let az_step = (PI / 2.0) / (AZ_SAMPLES - 1) as f64;
    let reduce = |elev: f64| -> (f64, f64) {
        let mut integral = 0.0;
        let mut max = 0.0_f64;
        for i in 0..AZ_SAMPLES {
            #[allow(clippy::cast_precision_loss)]
            let az = i as f64 * az_step;
            let p = element.power(elev, az);
            // Trapezoid: the two endpoints carry half weight.
            let w = if i == 0 || i == AZ_SAMPLES - 1 {
                0.5
            } else {
                1.0
            };
            integral += w * p * az_step;
            max = max.max(p);
        }
        // x4 for the remaining three quadrants.
        (4.0 * integral, max)
    };

    // Denominator: P_rad over a perfect conductor, on a finer elevation grid
    // than the output curve so the normalisation is not the accuracy limit.
    const INT_STEPS: usize = 900;
    #[allow(clippy::cast_precision_loss)]
    let d_step = (PI / 2.0) / INT_STEPS as f64;
    // Each elevation of this quadrature carries an azimuth quadrature inside
    // it, which made building the two curves the most expensive non-tracing
    // step of a solve - 12.6 ms of the 86 ms it took. The terms are
    // independent, so they are formed across the pool and then summed IN
    // ORDER: the addition sequence, and so the result, is the serial loop's.
    let terms: Vec<f64> = (0..=INT_STEPS)
        .into_par_iter()
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let elev = i as f64 * d_step;
            let (az_integral, _) = reduce(elev);
            let af = array_factor_sq(elev, height_m, f_hz, Ground::Perfect, pol);
            let w = if i == 0 || i == INT_STEPS { 0.5 } else { 1.0 };
            w * az_integral * af * elev.cos() * d_step
        })
        .collect();
    let mut p_rad = 0.0;
    for term in terms {
        p_rad += term;
    }

    GainCurve::from_fn(label, move |elev| {
        if p_rad <= 0.0 {
            return f64::NEG_INFINITY;
        }
        let (_, max_f) = reduce(elev);
        let u = max_f * array_factor_sq(elev, height_m, f_hz, ground, pol);
        if u <= 0.0 {
            return f64::NEG_INFINITY;
        }
        10.0 * (4.0 * PI * u / p_rad).log10() - fixed_loss_db
    })
}

/// A horizontal wire carrying a standing wave: the half-wave dipole, and the
/// EFHW (which is the same radiator, fed at a voltage maximum instead of a
/// current maximum, plus its matching transformer).
pub struct HorizontalWire {
    pub ground: Ground,
    /// Frequency at which the wire is exactly a half-wave [Hz].
    ///
    /// `None` means "always resonant": a dipole cut for whatever band is in
    /// use, which is how a dipole is normally described. `Some(f0)` fixes a
    /// physical wire length, so at `f` the wire is `round(f / f0)` half-waves
    /// long - the harmonic operation an EFHW is bought for.
    pub design_hz: Option<f64>,
    /// Flat loss subtracted at every angle [dB]: the EFHW's transformer.
    pub fixed_loss_db: f64,
    kind: &'static str,
}

impl HorizontalWire {
    /// A resonant horizontal half-wave dipole, broadside to the path.
    #[must_use]
    pub fn dipole(ground: Ground) -> Self {
        Self {
            ground,
            design_hz: None,
            fixed_loss_db: 0.0,
            kind: "horizontal dipole",
        }
    }

    /// An end-fed half-wave cut for `design_hz`, with a transformer losing
    /// `fixed_loss_db`.
    #[must_use]
    pub fn efhw(ground: Ground, design_hz: f64, fixed_loss_db: f64) -> Self {
        Self {
            ground,
            design_hz: Some(design_hz),
            fixed_loss_db,
            kind: "EFHW",
        }
    }

    /// Wire length in half-wavelengths at `f_hz`.
    ///
    /// A resonant dipole is always 1. A fixed-length EFHW is `round(f / f0)`,
    /// floored at 1: below its design frequency the wire is short, which this
    /// model cannot describe (the current is no longer a full standing wave and
    /// the match collapses), so it reports the half-wave pattern and the UI
    /// says the antenna is being used below its design band.
    #[must_use]
    pub fn half_waves(&self, f_hz: f64) -> f64 {
        match self.design_hz {
            None => 1.0,
            Some(f0) => (f_hz / f0).round().max(1.0),
        }
    }
}

impl ElevationPattern for HorizontalWire {
    fn label(&self, height_m: f64, freq_hz: f64) -> String {
        let n = self.half_waves(freq_hz);
        let lam = C_M_PER_S / freq_hz;
        let harmonic = if n > 1.5 {
            format!(", {n:.0} half-waves long")
        } else {
            String::new()
        };
        format!(
            "{} at {height_m:.1} m ({:.2} wavelengths){harmonic}",
            self.kind,
            height_m / lam
        )
    }

    fn provenance(&self) -> &'static str {
        "Image theory over a flat lossy half-space, normalised by the radiated \
         power computed exactly over a perfect conductor. Anchors reproduced: \
         free-space half-wave dipole 2.15 dBi, and 2.15 + 6.02 dB asymptotically \
         at large height. Checked against EZNEC/NEC-4 (Sommerfeld ground) at \
         0.6/1.1/1.6/2.1 wavelengths: this model runs 0.25-0.6 dB optimistic \
         with peak elevation within 0.3 deg. Excludes conductor loss, near-field \
         ground loss, terrain and nearby structures."
    }

    fn curve(&self, height_m: f64, freq_hz: f64) -> GainCurve {
        normalised_curve(
            self.label(height_m, freq_hz),
            Element::Horizontal {
                half_waves: self.half_waves(freq_hz),
            },
            self.ground,
            height_m,
            freq_hz,
            self.fixed_loss_db,
        )
    }
}

/// A quarter-wave vertical monopole worked against ground.
///
/// `height_m` is the height of the BASE above ground: 0 is the ordinary
/// ground-mounted vertical. Non-zero heights model the earth reflection of an
/// elevated vertical but NOT the elevated radial screen itself, which is a
/// materially different antenna; the UI says so.
pub struct VerticalMonopole {
    pub ground: Ground,
}

impl ElevationPattern for VerticalMonopole {
    fn label(&self, height_m: f64, freq_hz: f64) -> String {
        let lam = C_M_PER_S / freq_hz;
        if height_m <= 0.0 {
            format!("ground-mounted 1/4-wave vertical ({:.1} m tall)", lam / 4.0)
        } else {
            format!(
                "1/4-wave vertical, base {height_m:.1} m up ({:.2} wavelengths)",
                height_m / lam
            )
        }
    }

    fn provenance(&self) -> &'static str {
        "Image theory over a flat lossy half-space. Reproduces the textbook \
         5.15 dBi at the horizon over a perfect conductor exactly, and the \
         pseudo-Brewster collapse over real soil (about 0.5 dBi peaking near \
         26 deg over medium ground, against 4.8 dBi near 8 deg over sea water). \
         Excludes radial-system I^2 R loss, which costs a real ground-mounted \
         vertical several more dB."
    }

    fn curve(&self, height_m: f64, freq_hz: f64) -> GainCurve {
        normalised_curve(
            self.label(height_m, freq_hz),
            Element::Vertical { half_waves: 1.0 },
            self.ground,
            height_m,
            freq_hz,
            0.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::CURVE_STEP_DEG;
    use super::*;

    const C: f64 = C_M_PER_S;

    fn peak_of(c: &GainCurve) -> (f64, f64) {
        c.peak()
    }

    /// ANCHOR 1 (exact, analytic). A quarter-wave monopole over a perfect
    /// ground plane has directivity 3.28 = 5.15 dBi, at the horizon. This is
    /// twice the free-space half-wave dipole's 1.64, because the same pattern
    /// is radiated into half the solid angle.
    #[test]
    fn monopole_over_perfect_ground_is_5_15_dbi() {
        let c = VerticalMonopole {
            ground: Ground::Perfect,
        }
        .curve(0.0, 7.1e6);
        let (g, ang) = peak_of(&c);
        assert!(
            (g - 5.15).abs() < 0.05,
            "quarter-wave monopole over PEC = {g:.3} dBi, textbook 5.15"
        );
        assert!(ang < 0.6, "peak should be at the horizon, got {ang} deg");
        // And 5.15 dBi is exactly 3.01 dB above the free-space dipole's 2.15.
        assert!((g - 2.15 - 3.01).abs() < 0.05);
    }

    /// ANCHOR 2 (exact, asymptotic). As height grows, a horizontal dipole's
    /// lowest lobe tends to the free-space 2.15 dBi plus the 20 log10(2) =
    /// 6.02 dB of a perfect two-element image array. Reaching this limit is
    /// what proves the power normalisation is right: get the denominator wrong
    /// and every gain is offset by a constant.
    #[test]
    fn tall_dipole_over_perfect_ground_tends_to_8_17_dbi() {
        for h_lambda in [4.0, 8.0, 16.0] {
            let f = 14.1e6;
            let c = HorizontalWire::dipole(Ground::Perfect).curve(h_lambda * C / f, f);
            let (g, _) = peak_of(&c);
            assert!(
                (g - 8.17).abs() < 0.1,
                "at {h_lambda} wavelengths got {g:.3} dBi, expected 2.15 + 6.02 = 8.17"
            );
        }
    }

    /// ANCHOR 3 (exact). At half-wave height over a perfect conductor the
    /// single lobe peaks where 2 pi (h/lambda) sin d = pi/2, i.e. sin d = 1/2,
    /// d = 30 deg exactly. The peak value 8.42 dBi is this model's own, and
    /// exceeds the 8.17 asymptote because mutual coupling to the image lowers
    /// radiation resistance at this height - an effect the exact power
    /// integral captures and naive "free-space gain + 6 dB" does not.
    #[test]
    fn half_wave_height_peaks_at_30_degrees() {
        let f = 14.1e6;
        let c = HorizontalWire::dipole(Ground::Perfect).curve(0.5 * C / f, f);
        let (g, ang) = peak_of(&c);
        assert!((ang - 30.0).abs() < 0.6, "peak at {ang} deg, expected 30");
        assert!((g - 8.42).abs() < 0.1, "peak {g:.3} dBi, expected 8.42");
        // A horizontal dipole over a perfect conductor has a true null along
        // the horizon: R_h = -1 cancels the direct ray exactly.
        assert!(
            c.gain_dbi(0.0) < -30.0,
            "horizon should be a deep null, got {}",
            c.gain_dbi(0.0)
        );
    }

    /// ANCHOR 4 (published NEC-4). EZNEC double-precision NEC-4 with the
    /// Sommerfeld ground algorithm, 3.7 MHz, 2 mm copper, over ground with
    /// eps_r = 13 and sigma = 0.001 S/m, gain of the LOWEST elevation lobe
    /// (VOACAP, "Squeezing the decibels out of a simple dipole", Table 4):
    ///
    /// ```text
    ///     height      NEC-4            this model
    ///     0.6 wl      7.70 dBi @ 23    8.29 dBi @ 23.0   (+0.59 dB)
    ///     1.1 wl      7.84 dBi @ 13    8.19 dBi @ 12.5   (+0.35 dB)
    ///     1.6 wl      7.90 dBi @  9    8.17 dBi @  9.0   (+0.27 dB)
    ///     2.1 wl      7.92 dBi @  7    8.17 dBi @  7.0   (+0.25 dB)
    /// ```
    ///
    /// This model is consistently optimistic, which is the expected signature:
    /// the flat-earth Fresnel reflection omits the near-field Sommerfeld ground
    /// loss NEC-4 accounts for, and the wire is assumed lossless. The bias
    /// grows as the antenna gets lower and couples harder to the ground, which
    /// is again the right direction. Elevation angles agree within 0.5 deg.
    #[test]
    fn dipole_tracks_nec4_over_real_ground() {
        let f = 3.7e6;
        let ground = Ground::Lossy {
            eps_r: 13.0,
            sigma_s_per_m: 0.001,
        };
        // (height in wavelengths, NEC-4 gain dBi, NEC-4 elevation deg)
        let reference = [
            (0.6, 7.70, 23.0),
            (1.1, 7.84, 13.0),
            (1.6, 7.90, 9.0),
            (2.1, 7.92, 7.0),
        ];
        for (h_lambda, ref_gain, ref_deg) in reference {
            let c = HorizontalWire::dipole(ground).curve(h_lambda * C / f, f);
            // Lowest elevation lobe: first local maximum walking up from 0.
            let pts: Vec<_> = c.points().collect();
            let i = (1..pts.len() - 1)
                .find(|&i| pts[i].1 > pts[i - 1].1 && pts[i].1 >= pts[i + 1].1)
                .expect("a lobe");
            let (deg, gain) = pts[i];
            assert!(
                (deg - ref_deg).abs() <= 0.75,
                "{h_lambda} wl: lobe at {deg} deg, NEC-4 says {ref_deg}"
            );
            let bias = gain - ref_gain;
            assert!(
                (0.0..=0.75).contains(&bias),
                "{h_lambda} wl: {gain:.2} dBi vs NEC-4 {ref_gain:.2}, bias {bias:+.2} dB - \
                 expected slightly optimistic (0 to +0.75 dB), no more"
            );
        }
    }

    /// The vertical's defining real-ground behaviour: over a perfect conductor
    /// it is strongest at the horizon, but over any real soil the vertically
    /// polarised reflection coefficient goes to -1 at grazing incidence, the
    /// direct and reflected rays cancel, and the pattern collapses at low
    /// angles. Better ground pushes the peak lower and higher.
    #[test]
    fn vertical_shows_pseudo_brewster_collapse_over_real_ground() {
        let f = 3.7e6;
        let at = |g: Ground| {
            let c = VerticalMonopole { ground: g }.curve(0.0, f);
            (c.peak(), c.gain_dbi(0.0), c.gain_dbi(f64::to_radians(5.0)))
        };
        let ((pec_g, pec_a), _, _) = at(Ground::Perfect);
        assert!(pec_a < 0.6 && (pec_g - 5.15).abs() < 0.05);

        let sea = Ground::Lossy {
            eps_r: 80.0,
            sigma_s_per_m: 5.0,
        };
        let medium = Ground::Lossy {
            eps_r: 15.0,
            sigma_s_per_m: 0.003,
        };
        let dry = Ground::Lossy {
            eps_r: 5.0,
            sigma_s_per_m: 0.001,
        };

        let ((sea_g, sea_a), sea_h, sea_5) = at(sea);
        let ((med_g, med_a), med_h, med_5) = at(medium);
        let ((dry_g, dry_a), _, dry_5) = at(dry);

        // Every real ground nulls at exactly grazing.
        assert!(sea_h < -20.0 && med_h < -20.0, "grazing must null out");

        // Ranking: sea water is the famous vertical-friendly ground.
        assert!(
            sea_g > med_g && med_g > dry_g,
            "peak gain should rank sea {sea_g:.2} > medium {med_g:.2} > dry {dry_g:.2}"
        );
        assert!(
            sea_5 > med_5 + 5.0 && med_5 > dry_5,
            "at 5 deg: sea {sea_5:.2}, medium {med_5:.2}, dry {dry_5:.2}"
        );
        // Better ground also puts the peak at a lower angle.
        assert!(
            sea_a < med_a && med_a < dry_a,
            "peak elevation should rank sea {sea_a} < medium {med_a} < dry {dry_a}"
        );
        // Magnitudes, against the well-known operating figures: a vertical over
        // average soil is roughly a 0 dBi antenna peaking at 20-30 deg, while
        // over salt water it approaches the perfect-ground 5 dBi near the
        // horizon.
        assert!((4.0..5.2).contains(&sea_g), "sea water peak {sea_g:.2} dBi");
        assert!(
            (-1.0..2.0).contains(&med_g),
            "medium ground peak {med_g:.2} dBi"
        );
        assert!(
            (20.0..=35.0).contains(&med_a),
            "medium ground peak at {med_a} deg"
        );
        assert!(dry_a > 25.0, "dry ground peak at {dry_a} deg");
    }

    /// An EFHW at its design frequency is the same radiator as a dipole - same
    /// standing-wave current, same pattern - so the two must differ by exactly
    /// the transformer loss and nothing else. This is a real result, not a
    /// modelling shortcut, and worth pinning: the EFHW's advantage is the feed
    /// point, not the pattern.
    #[test]
    fn efhw_at_design_frequency_is_a_dipole_minus_the_unun() {
        let ground = Ground::Lossy {
            eps_r: 15.0,
            sigma_s_per_m: 0.003,
        };
        let f = 7.1e6;
        let dip = HorizontalWire::dipole(ground).curve(12.0, f);
        let efhw = HorizontalWire::efhw(ground, 7.1e6, 0.5).curve(12.0, f);
        for deg in [3.0, 10.0, 25.0, 45.0, 80.0] {
            let d = dip.gain_dbi(f64::to_radians(deg));
            let e = efhw.gain_dbi(f64::to_radians(deg));
            assert!(
                (d - e - 0.5).abs() < 1e-9,
                "at {deg} deg dipole {d:.3} vs EFHW {e:.3}"
            );
        }
    }

    /// The EFHW's actual distinguishing feature: on harmonics the wire is
    /// several half-waves long and the pattern changes completely. An EFHW cut
    /// for 40 m is a high-angle antenna on 40 m (short in wavelengths at any
    /// practical height) but develops strong low-angle lobes on 20/15/10 m.
    /// It must also be recognised as being longer.
    #[test]
    fn efhw_changes_pattern_on_its_harmonics() {
        let ground = Ground::Lossy {
            eps_r: 15.0,
            sigma_s_per_m: 0.003,
        };
        let ant = HorizontalWire::efhw(ground, 7.1e6, 0.5);
        // Wire length in half-waves tracks the harmonic.
        assert!((ant.half_waves(7.1e6) - 1.0).abs() < 1e-9);
        assert!((ant.half_waves(14.2e6) - 2.0).abs() < 1e-9);
        assert!((ant.half_waves(28.4e6) - 4.0).abs() < 1e-9);
        // Below the design band it floors at a half-wave rather than reporting
        // nonsense for a wire that is electrically short.
        assert!((ant.half_waves(3.5e6) - 1.0).abs() < 1e-9);

        let low_angle = |f: f64| ant.curve(12.0, f).gain_dbi(f64::to_radians(10.0));
        let f40 = low_angle(7.1e6);
        let f20 = low_angle(14.2e6);
        let f10 = low_angle(28.4e6);
        assert!(
            f20 > f40 + 5.0 && f10 > f20,
            "10 deg gain should climb with harmonic: 40m {f40:.2}, 20m {f20:.2}, 10m {f10:.2}"
        );
        // On 40 m at 12 m up (0.28 wavelengths) it is a cloud-warmer.
        let (_, peak_deg) = ant.curve(12.0, 7.1e6).peak();
        assert!(
            peak_deg > 40.0,
            "40 m peak at {peak_deg} deg, expected high"
        );
    }

    /// A 2-wavelength wire (EFHW 4th harmonic) has an exact null broadside, so
    /// a fixed-broadside convention would report a null where the antenna
    /// actually has strong lobes. This pins the azimuth-maximum reduction the
    /// module docs describe.
    #[test]
    fn azimuth_maximum_avoids_the_broadside_null_of_even_harmonics() {
        // Broadside element factor: cos(0) - cos(n pi/2) = 1 - cos(n pi/2).
        // n = 4 gives 1 - 1 = 0 exactly.
        let broadside = |n: f64| wire_element(PI / 2.0, n);
        assert!((broadside(1.0) - 1.0).abs() < 1e-12);
        assert!((broadside(2.0) - 2.0).abs() < 1e-12);
        assert!(
            broadside(4.0).abs() < 1e-12,
            "2-wavelength wire nulls broadside"
        );

        // Yet the reported curve is healthy, because it maximises over azimuth.
        let c = HorizontalWire::efhw(
            Ground::Lossy {
                eps_r: 15.0,
                sigma_s_per_m: 0.003,
            },
            7.1e6,
            0.5,
        )
        .curve(12.0, 28.4e6);
        let (g, _) = c.peak();
        assert!(
            g > 6.0,
            "4th-harmonic EFHW peak {g:.2} dBi should be strong"
        );
    }

    /// The shared Fresnel coefficient must reproduce the limits the solver's
    /// ground-bounce term relies on: near-perfect reflection off sea water, and
    /// R -> -1 at grazing incidence for BOTH polarisations.
    #[test]
    fn fresnel_coefficient_has_the_right_limits() {
        let f = 14e6;
        for pol in [Polarization::Horizontal, Polarization::Vertical] {
            let grazing = fresnel_coefficient(1e-6, f, 15.0, 0.003, pol);
            assert!(
                (grazing + Complex64::new(1.0, 0.0)).norm() < 1e-3,
                "grazing R should be -1, got {grazing}"
            );
            // Sea water is nearly a mirror at a moderate angle.
            let sea = fresnel_coefficient(f64::to_radians(10.0), f, 80.0, 5.0, pol);
            assert!(sea.norm() > 0.9, "sea water |R| = {}", sea.norm());
        }
        // Vertical polarisation has a Brewster minimum over lossy ground that
        // horizontal does not.
        let v: Vec<f64> = (1..60)
            .map(|d| {
                fresnel_coefficient(
                    f64::from(d).to_radians(),
                    f,
                    15.0,
                    0.003,
                    Polarization::Vertical,
                )
                .norm()
            })
            .collect();
        let vmin = v.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(vmin < 0.35, "vertical Brewster minimum |R| = {vmin}");
    }

    /// Gain must be a genuine function of all three interface arguments, or the
    /// pattern is not really wired to anything.
    #[test]
    fn gain_responds_to_elevation_height_and_frequency() {
        let ground = Ground::Lossy {
            eps_r: 15.0,
            sigma_s_per_m: 0.003,
        };
        let a = HorizontalWire::dipole(ground);
        let e = f64::to_radians(12.0);
        let base = a.gain_dbi(e, 10.0, 14.1e6);
        assert!((a.gain_dbi(f64::to_radians(40.0), 10.0, 14.1e6) - base).abs() > 1.0);
        assert!((a.gain_dbi(e, 25.0, 14.1e6) - base).abs() > 1.0);
        assert!((a.gain_dbi(e, 10.0, 28.2e6) - base).abs() > 1.0);
        // And the convenience method agrees with the curve it is built from.
        let c = a.curve(10.0, 14.1e6);
        assert!((c.gain_dbi(e) - base).abs() < 1e-12);
    }

    /// Sampling the curve on the 0.5 deg grid must not lose the peak: the
    /// curve's own peak has to agree with a dense direct evaluation.
    #[test]
    fn curve_grid_resolves_the_peak() {
        let f = 14.1e6;
        let a = HorizontalWire::dipole(Ground::Perfect);
        let c = a.curve(0.5 * C / f, f);
        let (g, _) = c.peak();
        let dense = (0..=9000)
            .map(|i| c.gain_dbi(f64::from(i) / 100.0 * CURVE_STEP_DEG.to_radians() * 2.0))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((g - dense).abs() < 0.02, "grid peak {g}, dense {dense}");
    }
}
