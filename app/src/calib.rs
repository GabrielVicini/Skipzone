//! The app's unverified anchors, gathered so a calibration run can vary them
//! and an ordinary run can ignore them.
//!
//! Each value is a module constant documented where it is used: the D-region
//! and collision anchors in [`crate::scenario`], the foE anchor in
//! [`skipzone_iono::fof2`], the sporadic-E climatology in [`skipzone_iono::sporadic_e`], and
//! the atmospheric-noise surrogate in [`crate::noise`]. This module makes them
//! adjustable and gives each a plausible range.
//!
//! # Why the bounds live in the code
//!
//! An unbounded calibration will put the D region eight times denser than any
//! published value if that lowers its residual, and report a good fit to a
//! physically wrong model. Each anchor carries its defensible range, and
//! [`Bounded::clamped`] reports when a fit had to clamp.
//!
//! A bound being reached means the residual the fit is chasing does not come
//! from the quantity it is pushing, so the error lies elsewhere in the model.
//!
//! # Not calibrated
//!
//! Free-space loss, the antenna patterns, the WSPR reference bandwidth and
//! decode threshold, and everything in the engine crate. These are derived
//! results or definitions.

/// One anchor: a current value and the range it is physically defensible over.
#[derive(Clone, Copy, Debug)]
pub struct Bounded {
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

impl Bounded {
    #[must_use]
    pub const fn new(value: f64, min: f64, max: f64) -> Self {
        Self { value, min, max }
    }

    /// This anchor moved to `v`, clamped into its range, plus whether the clamp
    /// applied.
    #[must_use]
    pub fn clamped(self, v: f64) -> (Self, bool) {
        let clamped = v.clamp(self.min, self.max);
        (
            Self {
                value: clamped,
                ..self
            },
            (clamped - v).abs() > 1e-12 * self.max.abs().max(1.0),
        )
    }

    /// Is the current value sitting on either end of its range?
    #[must_use]
    pub fn at_bound(self) -> bool {
        let span = (self.max - self.min).abs().max(1e-30);
        (self.value - self.min).abs() < 1e-6 * span || (self.max - self.value).abs() < 1e-6 * span
    }

    /// Position in the range, 0 at the minimum and 1 at the maximum. Used to
    /// step every anchor on a common scale regardless of its units.
    #[must_use]
    pub fn unit_position(self) -> f64 {
        let span = self.max - self.min;
        if span.abs() < 1e-30 {
            0.5
        } else {
            ((self.value - self.min) / span).clamp(0.0, 1.0)
        }
    }

    /// The value at a fractional position in the range.
    #[must_use]
    pub fn at_unit_position(self, u: f64) -> Self {
        Self {
            value: self.min + (self.max - self.min) * u.clamp(0.0, 1.0),
            ..self
        }
    }
}

/// The absorbing and reflecting layer anchors: D-region density and geometry,
/// electron-neutral collision frequency, E-region geometry and foE.
///
/// Ranges are the published spread of each quantity for the mid-latitude
/// ionosphere:
///
/// * D-region peak density 3e8 - 3e9 m^-3 spans quiet night through disturbed
///   day at the 80-90 km peak.
/// * The peak sits at 80-90 km and has a 4-10 km scale height in every
///   textbook treatment.
/// * `nu_e` at 70 km is quoted between about 1e6 and 2e7 s^-1 depending on
///   whose neutral-density profile is used; its scale height follows the neutral
///   atmosphere, 5-9 km.
/// * The E peak sits at 100-115 km with a scale height of 5-15 km.
/// * Overhead quiet-sun foE spans 2.6-3.8 MHz. The textbook range starts at
///   3.0, but the anchor was measured at 2.99 against ionosonde soundings.
#[derive(Clone, Copy, Debug)]
pub struct IonosphereAnchors {
    /// Overhead-sun D-region peak electron density, m^-3.
    pub d_peak_ne_overhead: Bounded,
    /// Overhead-sun D-region peak height, km.
    pub d_peak_alt_km: Bounded,
    /// D-region Chapman scale height, km.
    pub d_scale_height_km: Bounded,
    /// Residual night-time D-region density, as a fraction of the overhead-sun
    /// peak. See [`skipzone_iono::chapman::SolarChapmanLayer::with_night_floor`] for why
    /// an alpha-Chapman layer needs it as an input.
    pub d_night_floor_fraction: Bounded,
    /// How much of the photochemical peak rise the D-region profile takes,
    /// 0..1. See [`skipzone_iono::chapman::SolarChapmanLayer::with_rise_fraction`].
    ///
    /// The only anchor that changes the model's shape rather than a level. The
    /// layer imposes `A ~ cos^n chi` with `n = 1/2 + f H_D/H_nu`; at the
    /// shipping `f = 1` that is 1.39 against a measured `n ~ 0.75`, which no
    /// setting of the two scale heights within bounds reaches
    /// (`bin/zenith_probe`).
    ///
    /// Both endpoints are derived (1 is the textbook photochemical layer, 0 the
    /// fixed-height limit), so reaching either end is not itself a finding.
    pub d_rise_fraction: Bounded,
    /// Electron-neutral collision frequency at `nu_ref_alt_km`, s^-1.
    pub nu_ref_per_s: Bounded,
    /// Reference altitude for `nu_ref_per_s`, km.
    pub nu_ref_alt_km: Bounded,
    /// Neutral scale height controlling the fall-off of nu, km.
    pub nu_scale_height_km: Bounded,
    /// E-layer peak height, km.
    pub e_peak_alt_km: Bounded,
    /// E-layer Chapman scale height, km.
    pub e_scale_height_km: Bounded,
    /// Overhead quiet-sun foE, MHz.
    pub foe_overhead_quiet_mhz: Bounded,
    /// foEs at the occurrence maximum, MHz. See [`skipzone_iono::sporadic_e`]: this is
    /// the top of the modelled foEs range, and the model currently treats it as
    /// the value Es takes whenever Es is present.
    pub es_foes_max_mhz: Bounded,
    /// Peak Es occurrence probability, 0..1.
    pub es_peak_probability: Bounded,
}

impl Default for IonosphereAnchors {
    fn default() -> Self {
        use crate::scenario as s;
        Self {
            d_peak_ne_overhead: Bounded::new(s::D_REGION_PEAK_NE_OVERHEAD, 3.0e8, 3.0e9),
            d_peak_alt_km: Bounded::new(s::D_REGION_PEAK_ALT_KM, 80.0, 90.0),
            d_scale_height_km: Bounded::new(s::D_REGION_SCALE_HEIGHT_KM, 4.0, 10.0),
            d_night_floor_fraction: Bounded::new(s::D_REGION_NIGHT_FLOOR_FRACTION, 0.0, 0.25),
            d_rise_fraction: Bounded::new(s::D_REGION_RISE_FRACTION, 0.0, 1.0),
            nu_ref_per_s: Bounded::new(s::NU_REF_PER_S, 1.0e6, 2.0e7),
            nu_ref_alt_km: Bounded::new(s::NU_REF_ALT_KM, 65.0, 80.0),
            nu_scale_height_km: Bounded::new(s::NU_SCALE_HEIGHT_KM, 5.0, 9.0),
            e_peak_alt_km: Bounded::new(s::E_REGION_PEAK_ALT_KM, 100.0, 115.0),
            e_scale_height_km: Bounded::new(s::E_REGION_SCALE_HEIGHT_KM, 5.0, 15.0),
            // Lower bound below the textbook 3.0 because the measured value is
            // 2.99 (`fof2::FOE_OVERHEAD_QUIET_MHZ`, from 28 952 ionosonde
            // soundings over four seasons).
            foe_overhead_quiet_mhz: Bounded::new(
                skipzone_iono::fof2::FOE_OVERHEAD_QUIET_MHZ,
                2.6,
                3.8,
            ),
            es_foes_max_mhz: Bounded::new(skipzone_iono::sporadic_e::ES_FOES_MAX_MHZ, 5.0, 12.0),
            es_peak_probability: Bounded::new(
                skipzone_iono::sporadic_e::ES_PEAK_PROBABILITY,
                0.10,
                0.70,
            ),
        }
    }
}

/// The anchors of the atmospheric-noise surrogate in [`crate::noise`].
///
/// The shape (log-linear in frequency, with additive day/night, season and
/// latitude terms) follows the published P.372 curves and is not calibrated
/// here. These are its unverified absolute anchors.
///
/// Ranges are wide because the quantity is that uncertain: the
/// P.372 atmospheric maps span roughly 40 dB of `Fa` at 1 MHz between a quiet
/// polar winter night and an equatorial summer night.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphericAnchors {
    /// `Fa` at 1 MHz, day, mid-latitude, equinox, dB above kT0b.
    pub f1_day_db: Bounded,
    /// Day fall-off with frequency, dB per decade.
    pub slope_day_db: Bounded,
    /// Day -> night step at 1 MHz, dB. See
    /// [`crate::noise::atmospheric_noise_figure_db`] for why the night curve is a
    /// step on the day curve rather than a second independent line.
    pub step_1mhz_db: Bounded,
    /// Linear frequency dependence of that step, dB per decade. Negative narrows
    /// the step with rising frequency; positive inverts it.
    pub step_slope_db: Bounded,
    /// Curvature of the step in `log10(f)`, dB per decade squared.
    pub step_curve_db: Bounded,
    /// Summer / winter offset about the equinox value, dB.
    pub season_swing_db: Bounded,
    /// Equatorial excess over the polar value, dB.
    pub tropical_boost_db: Bounded,
    /// Polar offset subtracted everywhere, dB.
    pub polar_offset_db: Bounded,
}

impl Default for AtmosphericAnchors {
    fn default() -> Self {
        use crate::noise as n;
        Self {
            f1_day_db: Bounded::new(n::ATM_1MHZ_DAY_DB, 55.0, 90.0),
            slope_day_db: Bounded::new(n::ATM_SLOPE_DAY_DB, 30.0, 60.0),
            // The three step bounds are drawn around a contrast rather than an
            // absolute level. Level-shaped bounds (80..110 dB at 1 MHz,
            // 35..65 dB/decade for the night line) made the fit rail every one,
            // and widening the night slope ceiling to 90 only moved RMS from 7.81
            // to 7.83 dB.
            //
            // A 0 dB step must be reachable for bands whose floor is set by the
            // man-made and galactic curves, which above ~10 MHz is every band.
            step_1mhz_db: Bounded::new(n::ATM_STEP_1MHZ_DB, 0.0, 40.0),
            // Spans both signs. Negative is the published behaviour (day and
            // night curves converge as frequency rises); positive, an inverted
            // step, is needed for the measured step over 7-14 MHz.
            step_slope_db: Bounded::new(n::ATM_STEP_SLOPE_DB, -45.0, 25.0),
            // Curvature, so the step may bend. Six corpus bands have a cell on
            // both sides of the terminator, enough for three step parameters. The
            // lowest three have a daytime side made almost entirely of E-layer
            // paths with their own defect (see `THE STEP AT THE TERMINATOR`), so a
            // fourth step parameter would only chase that.
            step_curve_db: Bounded::new(n::ATM_STEP_CURVE_DB, -25.0, 25.0),
            // Measured: further tuning of this surrogate does not help. Three
            // noise parameterisations fitted to the 9126-spot corpus (two
            // independent lines; the same with the night slope ceiling at 90; and
            // this step form, which the fit drove to a flat inverted step of about
            // -8 dB) have day/night steps more than 20 dB apart and score:
            //
            //   RMS, effects removed   7.81   7.83   7.88 dB
            //   AUC decode/non-decode  0.771  0.776  0.775
            //
            // The step form cut the residual's day/night spread across bands from
            // 21 dB to 9 dB without improving prediction. With IQR ~10.4 dB in the
            // residual, the model is limited by per-path spread rather than any
            // smooth systematic term.
            season_swing_db: Bounded::new(n::ATM_SEASON_SWING_DB, 0.0, 15.0),
            tropical_boost_db: Bounded::new(n::ATM_TROPICAL_BOOST_DB, 0.0, 30.0),
            polar_offset_db: Bounded::new(n::ATM_POLAR_OFFSET_DB, 0.0, 15.0),
        }
    }
}

/// Everything a calibration may move, in one value carried on
/// [`Inputs`](crate::scenario::Inputs).
///
/// `Default` reproduces the module constants exactly
/// (`tests::default_anchors_match_the_module_constants`), so the GUI can carry
/// this without using it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Anchors {
    pub ionosphere: IonosphereAnchors,
    pub atmospheric: AtmosphericAnchors,
}

impl Anchors {
    /// Every ionospheric anchor, as `(name, accessor)` pairs, so a fit can walk
    /// them without a hand-maintained parallel list going stale.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn ionosphere_fields() -> Vec<(
        &'static str,
        fn(&IonosphereAnchors) -> Bounded,
        fn(&mut IonosphereAnchors, Bounded),
    )> {
        vec![
            (
                "D peak Ne [m^-3]",
                |a| a.d_peak_ne_overhead,
                |a, v| a.d_peak_ne_overhead = v,
            ),
            (
                "D peak altitude [km]",
                |a| a.d_peak_alt_km,
                |a, v| a.d_peak_alt_km = v,
            ),
            (
                "D scale height [km]",
                |a| a.d_scale_height_km,
                |a, v| a.d_scale_height_km = v,
            ),
            (
                "D night floor [fraction]",
                |a| a.d_night_floor_fraction,
                |a, v| a.d_night_floor_fraction = v,
            ),
            (
                "D rise fraction [0..1]",
                |a| a.d_rise_fraction,
                |a, v| a.d_rise_fraction = v,
            ),
            (
                "nu at ref alt [1/s]",
                |a| a.nu_ref_per_s,
                |a, v| {
                    a.nu_ref_per_s = v;
                },
            ),
            (
                "nu ref altitude [km]",
                |a| a.nu_ref_alt_km,
                |a, v| {
                    a.nu_ref_alt_km = v;
                },
            ),
            (
                "nu scale height [km]",
                |a| a.nu_scale_height_km,
                |a, v| a.nu_scale_height_km = v,
            ),
            (
                "E peak altitude [km]",
                |a| a.e_peak_alt_km,
                |a, v| {
                    a.e_peak_alt_km = v;
                },
            ),
            (
                "E scale height [km]",
                |a| a.e_scale_height_km,
                |a, v| a.e_scale_height_km = v,
            ),
            (
                "foE overhead quiet [MHz]",
                |a| a.foe_overhead_quiet_mhz,
                |a, v| a.foe_overhead_quiet_mhz = v,
            ),
            (
                "foEs at max [MHz]",
                |a| a.es_foes_max_mhz,
                |a, v| {
                    a.es_foes_max_mhz = v;
                },
            ),
            (
                "Es peak probability",
                |a| a.es_peak_probability,
                |a, v| a.es_peak_probability = v,
            ),
        ]
    }

    /// Every atmospheric-noise anchor, same shape.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn atmospheric_fields() -> Vec<(
        &'static str,
        fn(&AtmosphericAnchors) -> Bounded,
        fn(&mut AtmosphericAnchors, Bounded),
    )> {
        vec![
            (
                "atm Fa 1 MHz day [dB]",
                |a| a.f1_day_db,
                |a, v| {
                    a.f1_day_db = v;
                },
            ),
            (
                "atm slope day [dB/dec]",
                |a| a.slope_day_db,
                |a, v| {
                    a.slope_day_db = v;
                },
            ),
            (
                "atm day->night step [dB]",
                |a| a.step_1mhz_db,
                |a, v| {
                    a.step_1mhz_db = v;
                },
            ),
            (
                "atm step slope [dB/dec]",
                |a| a.step_slope_db,
                |a, v| {
                    a.step_slope_db = v;
                },
            ),
            (
                "atm step curve [dB/dec2]",
                |a| a.step_curve_db,
                |a, v| {
                    a.step_curve_db = v;
                },
            ),
            (
                "atm season swing [dB]",
                |a| a.season_swing_db,
                |a, v| {
                    a.season_swing_db = v;
                },
            ),
            (
                "atm tropical boost [dB]",
                |a| a.tropical_boost_db,
                |a, v| a.tropical_boost_db = v,
            ),
            (
                "atm polar offset [dB]",
                |a| a.polar_offset_db,
                |a, v| {
                    a.polar_offset_db = v;
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skipzone::density::ElectronDensity;
    use skipzone::geo::SphericalPoint;
    use skipzone::units::{Meters, Radians};
    use skipzone_iono::chapman::SolarChapmanLayer;
    use std::f64::consts::PI;

    fn point(r: f64, colat: f64, lon: f64) -> SphericalPoint {
        SphericalPoint::new(Meters::new(r), Radians::new(colat), Radians::new(lon))
    }

    /// The default anchors must equal the module constants, or the GUI stops
    /// matching the values its assumptions panel prints.
    #[test]
    fn default_anchors_match_the_module_constants() {
        let a = Anchors::default();
        let i = a.ionosphere;
        assert!(
            (i.d_peak_ne_overhead.value - crate::scenario::D_REGION_PEAK_NE_OVERHEAD).abs() < 1.0
        );
        assert!((i.d_peak_alt_km.value - crate::scenario::D_REGION_PEAK_ALT_KM).abs() < 1e-12);
        assert!(
            (i.d_scale_height_km.value - crate::scenario::D_REGION_SCALE_HEIGHT_KM).abs() < 1e-12
        );
        assert!(
            (i.d_night_floor_fraction.value - crate::scenario::D_REGION_NIGHT_FLOOR_FRACTION).abs()
                < 1e-12
        );
        assert!((i.nu_ref_per_s.value - crate::scenario::NU_REF_PER_S).abs() < 1.0);
        assert!((i.nu_ref_alt_km.value - crate::scenario::NU_REF_ALT_KM).abs() < 1e-12);
        assert!((i.nu_scale_height_km.value - crate::scenario::NU_SCALE_HEIGHT_KM).abs() < 1e-12);
        assert!((i.e_peak_alt_km.value - crate::scenario::E_REGION_PEAK_ALT_KM).abs() < 1e-12);
        assert!(
            (i.e_scale_height_km.value - crate::scenario::E_REGION_SCALE_HEIGHT_KM).abs() < 1e-12
        );
        assert!(
            (i.foe_overhead_quiet_mhz.value - skipzone_iono::fof2::FOE_OVERHEAD_QUIET_MHZ).abs()
                < 1e-12
        );
        assert!(
            (i.es_foes_max_mhz.value - skipzone_iono::sporadic_e::ES_FOES_MAX_MHZ).abs() < 1e-12
        );
        assert!(
            (i.es_peak_probability.value - skipzone_iono::sporadic_e::ES_PEAK_PROBABILITY).abs()
                < 1e-12
        );

        let n = a.atmospheric;
        assert!((n.f1_day_db.value - crate::noise::ATM_1MHZ_DAY_DB).abs() < 1e-12);
        assert!((n.slope_day_db.value - crate::noise::ATM_SLOPE_DAY_DB).abs() < 1e-12);
        assert!((n.step_1mhz_db.value - crate::noise::ATM_STEP_1MHZ_DB).abs() < 1e-12);
        assert!((n.step_slope_db.value - crate::noise::ATM_STEP_SLOPE_DB).abs() < 1e-12);
        assert!((n.step_curve_db.value - crate::noise::ATM_STEP_CURVE_DB).abs() < 1e-12);
        // The step constants are derived from the night anchors; editing one
        // side alone would break the identity.
        assert!(
            (n.f1_day_db.value + n.step_1mhz_db.value - crate::noise::ATM_1MHZ_NIGHT_DB).abs()
                < 1e-12
        );
        assert!(
            (n.slope_day_db.value - n.step_slope_db.value - crate::noise::ATM_SLOPE_NIGHT_DB).abs()
                < 1e-12
        );
        assert!((n.season_swing_db.value - crate::noise::ATM_SEASON_SWING_DB).abs() < 1e-12);
        assert!((n.tropical_boost_db.value - crate::noise::ATM_TROPICAL_BOOST_DB).abs() < 1e-12);
        assert!((n.polar_offset_db.value - crate::noise::ATM_POLAR_OFFSET_DB).abs() < 1e-12);
    }

    /// Every default value must sit strictly inside its stated range, so that
    /// "hit the bound" is meaningful.
    ///
    /// The exception is `d_rise_fraction`, whose prior 1.0 is also its physical
    /// maximum (a profile cannot take more than all of its photochemical rise).
    /// Staying there means the corpus found no reason to leave the textbook
    /// layer. The exemption is by name so any other anchor on a bound still
    /// fails.
    #[test]
    fn every_prior_sits_inside_its_range() {
        /// The prior legitimately equals a bound here; see the doc comment.
        const AT_BOUND_BY_CONSTRUCTION: [&str; 1] = ["D rise fraction [0..1]"];

        let a = Anchors::default();
        for (name, get, _) in Anchors::ionosphere_fields() {
            let b = get(&a.ionosphere);
            if AT_BOUND_BY_CONSTRUCTION.contains(&name) {
                // Still within the range and with a real interval to move in;
                // only the strictness is relaxed.
                assert!(
                    b.value >= b.min && b.value <= b.max && b.min < b.max,
                    "{name}: exempt prior {} is still outside [{}, {}]",
                    b.value,
                    b.min,
                    b.max
                );
                continue;
            }
            assert!(
                b.value > b.min && b.value < b.max,
                "{name}: prior {} is not strictly inside [{}, {}]",
                b.value,
                b.min,
                b.max
            );
        }
        for (name, get, _) in Anchors::atmospheric_fields() {
            let b = get(&a.atmospheric);
            assert!(
                b.value > b.min && b.value < b.max,
                "{name}: prior {} is not strictly inside [{}, {}]",
                b.value,
                b.min,
                b.max
            );
        }
    }

    /// Clamping must report that it clamped.
    #[test]
    fn clamping_reports_itself() {
        let b = Bounded::new(1.0e9, 3.0e8, 3.0e9);
        let (inside, hit) = b.clamped(2.0e9);
        assert!(!hit);
        assert!((inside.value - 2.0e9).abs() < 1.0);
        let (outside, hit) = b.clamped(8.0e9);
        assert!(hit, "leaving the range must be reported");
        assert!((outside.value - 3.0e9).abs() < 1.0);
        assert!(outside.at_bound());
        assert!(!inside.at_bound());
    }

    /// The unit-position round trip is what lets a search step parameters with
    /// different units on one scale.
    #[test]
    fn unit_position_round_trips() {
        let b = Bounded::new(6.0, 4.0, 10.0);
        assert!((b.unit_position() - 1.0 / 3.0).abs() < 1e-12);
        assert!((b.at_unit_position(b.unit_position()).value - 6.0).abs() < 1e-12);
        assert!((b.at_unit_position(0.0).value - 4.0).abs() < 1e-12);
        assert!((b.at_unit_position(1.0).value - 10.0).abs() < 1e-12);
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
}
