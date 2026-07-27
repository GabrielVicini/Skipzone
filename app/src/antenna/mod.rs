//! Antenna elevation patterns: gain in dBi as a function of take-off angle.
//!
//! This module replaces the app's former isotropic assumption. Every antenna
//! type is a [`ElevationPattern`] implementor that, given a height above ground
//! and an operating frequency, produces a [`GainCurve`] - gain in dBi sampled
//! against elevation angle. The solver evaluates that curve at the launch
//! elevation of the first hop and the arrival elevation of the last, and both
//! numbers enter the link budget.
//!
//! # Why a curve, and not just a function
//!
//! The brief interface is `(elevation, height, frequency) -> dB`, and
//! [`ElevationPattern::gain_dbi`] is exactly that. But the REQUIRED method is
//! [`ElevationPattern::curve`], for two reasons:
//!
//!   * the closed-form types normalise their pattern by numerically integrating
//!     radiated power over the hemisphere (see [`image`]), which costs far more
//!     than one sample - doing it per angle would be wasteful;
//!   * a gain-vs-elevation curve is also precisely what a LOOKUP-TABLE antenna
//!     has. A future Yagi, log-periodic, trap dipole, random wire or loop
//!     arrives as measured or NEC-modelled samples, not as a formula.
//!
//! So the curve is the common currency: closed-form types compute it, table
//! types interpolate it, and nothing downstream of this module can tell which
//! it got. [`table::TabulatedPattern`] is the extension point - the type exists
//! and interpolates, but ships no data and is not offered in the UI yet.
//!
//! # What the closed-form types assume
//!
//! Provenance and hand-checks live in [`image`]. In summary: image theory over
//! a flat lossy half-space, with the antenna's own radiated power computed
//! exactly (for the assumed sinusoidal current) over a perfect conductor. The
//! model does NOT include conductor loss, ground-screen / radial I^2 R loss,
//! near-field Sommerfeld ground loss, terrain, or nearby structures. Checked
//! against NEC-4 it runs 0.3-0.6 dB optimistic; see [`image`] for the numbers.

use rayon::prelude::*;

mod image;
mod table;

pub use image::{Ground, HorizontalWire, Polarization, VerticalMonopole, fresnel_coefficient};
// The lookup-table extension point. Deliberately not referenced anywhere
// else yet: no table-backed antenna is offered until real data exists to
// back one. Re-exported so adding one is a change to `AntennaConfig::build`
// and nothing more.
#[allow(unused_imports)]
pub use table::{PatternBlock, TabulatedPattern};

/// Elevation step of a [`GainCurve`], degrees.
pub const CURVE_STEP_DEG: f64 = 0.5;
/// Number of samples in a [`GainCurve`]: 0 deg to 90 deg inclusive.
pub const CURVE_SAMPLES: usize = 181;

/// Gain floor, dBi. Real patterns have true nulls (a horizontal dipole over a
/// perfect conductor radiates nothing along the horizon); carrying `-inf` into
/// a link budget would poison every downstream number, so nulls are clamped
/// here. -60 dBi is far below any angle that could produce a usable path.
pub const GAIN_FLOOR_DBI: f64 = -60.0;

/// Gain in dBi against elevation angle, sampled on a fixed 0..90 deg grid.
///
/// Interpolation is linear in dB. That is the convention pattern data is
/// published and interpolated in, and on a 0.5 deg grid the difference from
/// interpolating in power is far below the model's own accuracy.
#[derive(Clone)]
pub struct GainCurve {
    label: String,
    /// dBi at `i * CURVE_STEP_DEG` degrees elevation, `CURVE_SAMPLES` long.
    samples: Vec<f64>,
}

impl GainCurve {
    /// Build from a closure taking elevation in RADIANS and returning dBi.
    /// Samples are independent, and each costs an azimuth quadrature, so they
    /// are evaluated across the pool. `map` keeps input order, so the curve is
    /// bit-for-bit the one the serial loop produced.
    pub fn from_fn(label: impl Into<String>, f: impl Fn(f64) -> f64 + Sync) -> Self {
        let samples = (0..CURVE_SAMPLES)
            .into_par_iter()
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let deg = i as f64 * CURVE_STEP_DEG;
                f(deg.to_radians()).max(GAIN_FLOOR_DBI)
            })
            .collect();
        Self {
            label: label.into(),
            samples,
        }
    }

    /// A flat pattern at `gain_dbi` - the isotropic radiator, and the baseline
    /// the app used before this module existed.
    #[must_use]
    pub fn flat(label: impl Into<String>, gain_dbi: f64) -> Self {
        Self {
            label: label.into(),
            samples: vec![gain_dbi; CURVE_SAMPLES],
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Gain [dBi] at `elevation_rad`, linearly interpolated. Angles outside
    /// 0..90 deg clamp to the ends: a ray cannot leave below the horizon, and
    /// 90 deg is straight up.
    #[must_use]
    pub fn gain_dbi(&self, elevation_rad: f64) -> f64 {
        let deg = elevation_rad.to_degrees().clamp(0.0, 90.0);
        let x = deg / CURVE_STEP_DEG;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let i = (x.floor() as usize).min(CURVE_SAMPLES - 2);
        #[allow(clippy::cast_precision_loss)]
        let t = x - i as f64;
        self.samples[i] * (1.0 - t) + self.samples[i + 1] * t
    }

    /// `(peak gain [dBi], elevation of the peak [deg])`.
    #[must_use]
    pub fn peak(&self) -> (f64, f64) {
        let mut best = (f64::NEG_INFINITY, 0.0);
        for (i, &g) in self.samples.iter().enumerate() {
            if g > best.0 {
                #[allow(clippy::cast_precision_loss)]
                let deg = i as f64 * CURVE_STEP_DEG;
                best = (g, deg);
            }
        }
        best
    }

    /// `(elevation [deg], gain [dBi])` for every sample - for plotting.
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.samples.iter().enumerate().map(|(i, &g)| {
            #[allow(clippy::cast_precision_loss)]
            let deg = i as f64 * CURVE_STEP_DEG;
            (deg, g)
        })
    }
}

/// An antenna: something that can state its gain against elevation angle.
///
/// Implementors are either closed-form (see [`image`]) or table-backed (see
/// [`table`]). Nothing outside this module depends on which.
pub trait ElevationPattern: Send + Sync {
    /// Human-readable name, including the parameters that shaped the pattern.
    fn label(&self, height_m: f64, freq_hz: f64) -> String;

    /// Where the numbers come from, stated plainly for the UI. This project
    /// surfaces the provenance of every physical model rather than presenting
    /// all of them with equal confidence.
    fn provenance(&self) -> &'static str;

    /// REQUIRED: the gain-vs-elevation curve at one height and frequency.
    fn curve(&self, height_m: f64, freq_hz: f64) -> GainCurve;

    /// The interface in the brief: `(elevation, height, frequency) -> dBi`.
    ///
    /// Correct but not cheap for closed-form types, which rebuild the whole
    /// normalised curve per call. The solver calls [`Self::curve`] once per
    /// scenario and samples that instead - which is why this method carries no
    /// non-test caller and is marked accordingly.
    #[allow(dead_code)]
    fn gain_dbi(&self, elevation_rad: f64, height_m: f64, freq_hz: f64) -> f64 {
        self.curve(height_m, freq_hz).gain_dbi(elevation_rad)
    }
}

/// A perfectly isotropic radiator: 0 dBi at every angle.
///
/// Physically unrealisable, and retained deliberately - it is the baseline the
/// link budget used before patterns existed, so selecting it at both ends
/// reproduces the app's previous numbers exactly.
pub struct Isotropic;

impl ElevationPattern for Isotropic {
    fn label(&self, _height_m: f64, _freq_hz: f64) -> String {
        "isotropic, 0 dBi".to_string()
    }

    fn provenance(&self) -> &'static str {
        "Definition, not a model: 0 dBi at every angle. Physically unrealisable; \
         kept as the reference baseline."
    }

    fn curve(&self, _height_m: f64, _freq_hz: f64) -> GainCurve {
        GainCurve::flat("isotropic", 0.0)
    }
}

/// Insertion loss of a typical 49:1 EFHW transformer, dB.
///
/// ENGINEERING FIGURE, NOT A CITED STANDARD - the same status as the operating
/// mode presets in [`crate::noise`]. Published measurements of 49:1 ununs on
/// FT240-43 cores into a matched load scatter over roughly 0.3-0.8 dB across
/// HF, rising at the top of the band. A single flat 0.5 dB is used because
/// this model has no basis for a frequency shape.
pub const EFHW_UNUN_LOSS_DB: f64 = 0.5;

/// The antenna types the UI offers today.
///
/// Adding a table-backed type (Yagi, log-periodic, trap dipole, random wire,
/// loop) means adding a variant here and returning a [`TabulatedPattern`] from
/// [`AntennaConfig::build`]. Nothing else in the app needs to change.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AntennaKind {
    Isotropic,
    HorizontalDipole,
    VerticalMonopole,
    Efhw,
}

impl AntennaKind {
    pub const ALL: [Self; 4] = [
        Self::HorizontalDipole,
        Self::VerticalMonopole,
        Self::Efhw,
        Self::Isotropic,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Isotropic => "isotropic (0 dBi)",
            Self::HorizontalDipole => "horizontal half-wave dipole",
            Self::VerticalMonopole => "ground-mounted vertical (1/4 wave)",
            Self::Efhw => "end-fed half-wave (EFHW)",
        }
    }

    /// Does this type use the height-above-ground input? All three real types
    /// do; only the isotropic reference ignores it.
    #[must_use]
    pub fn uses_height(self) -> bool {
        !matches!(self, Self::Isotropic)
    }

    /// Does this type use the EFHW design-frequency input?
    #[must_use]
    pub fn uses_design_freq(self) -> bool {
        matches!(self, Self::Efhw)
    }
}

/// One end of the link: an antenna type plus the parameters it needs.
///
/// Type-specific parameters live HERE, not in the `gain_dbi` call, which is
/// what keeps the `(elevation, height, frequency)` interface uniform across
/// closed-form and table-backed types.
#[derive(Clone, Copy, PartialEq)]
pub struct AntennaConfig {
    pub kind: AntennaKind,
    /// Height above ground [m]. For the vertical this is the height of the
    /// BASE: 0 is a ground-mounted vertical, the usual case.
    pub height_m: f64,
    /// EFHW design frequency [MHz]: the wire is a half-wave here, and `n`
    /// half-waves at `n` times this. Ignored by the other types.
    pub efhw_design_mhz: f64,
}

impl Default for AntennaConfig {
    fn default() -> Self {
        Self {
            kind: AntennaKind::HorizontalDipole,
            height_m: 10.0,
            efhw_design_mhz: 7.1,
        }
    }
}

impl AntennaConfig {
    /// Instantiate the pattern model for this configuration over `ground`.
    #[must_use]
    pub fn build(&self, ground: Ground) -> Box<dyn ElevationPattern> {
        match self.kind {
            AntennaKind::Isotropic => Box::new(Isotropic),
            AntennaKind::HorizontalDipole => Box::new(HorizontalWire::dipole(ground)),
            AntennaKind::VerticalMonopole => Box::new(VerticalMonopole { ground }),
            AntennaKind::Efhw => Box::new(HorizontalWire::efhw(
                ground,
                self.efhw_design_mhz * 1e6,
                EFHW_UNUN_LOSS_DB,
            )),
        }
    }

    /// The gain curve for this end at `freq_hz`, over `ground`.
    #[must_use]
    pub fn curve(&self, ground: Ground, freq_hz: f64) -> GainCurve {
        self.build(ground).curve(self.height_m, freq_hz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_curve_reproduces_the_isotropic_baseline() {
        let c = Isotropic.curve(10.0, 14.1e6);
        for d in [0.0, 3.0, 17.5, 45.0, 90.0] {
            assert!((c.gain_dbi(f64::to_radians(d)) - 0.0).abs() < 1e-12);
        }
        assert!((Isotropic.gain_dbi(0.3, 10.0, 14.1e6)).abs() < 1e-12);
    }

    /// Interpolation lands on the samples exactly and is linear between them.
    #[test]
    fn curve_interpolates_linearly_and_clamps() {
        let c = GainCurve::from_fn("ramp", |e| e.to_degrees());
        assert!((c.gain_dbi(f64::to_radians(10.0)) - 10.0).abs() < 1e-9);
        // Midway between the 10.0 and 10.5 deg samples.
        assert!((c.gain_dbi(f64::to_radians(10.25)) - 10.25).abs() < 1e-9);
        // Outside the grid clamps rather than extrapolating.
        assert!((c.gain_dbi(f64::to_radians(-5.0)) - 0.0).abs() < 1e-9);
        assert!((c.gain_dbi(f64::to_radians(120.0)) - 90.0).abs() < 1e-9);
    }

    /// True nulls are floored, not carried into the budget as -inf.
    #[test]
    fn nulls_are_floored() {
        let c = GainCurve::from_fn("null", |_| f64::NEG_INFINITY);
        assert!((c.gain_dbi(0.5) - GAIN_FLOOR_DBI).abs() < 1e-9);
        assert!(c.gain_dbi(0.5).is_finite());
    }

    /// The default configuration is the reference antenna named in the brief.
    #[test]
    fn default_is_a_horizontal_dipole() {
        let d = AntennaConfig::default();
        assert!(d.kind == AntennaKind::HorizontalDipole);
        assert!(d.height_m > 0.0);
    }
}
