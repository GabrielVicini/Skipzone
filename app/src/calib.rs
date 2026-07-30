//! The app's UNVERIFIED ANCHORS, gathered into one place so that a calibration
//! run can vary them and an ordinary run can ignore them.
//!
//! Every value here already existed as a module constant with a physical name, a
//! unit and a docstring saying what it is and how far it can be trusted - the
//! D-region and collision anchors in [`crate::scenario`], the foE anchor in
//! [`crate::fof2`], the sporadic-E climatology in [`crate::sporadic_e`], and the
//! atmospheric-noise surrogate in [`crate::noise`]. Nothing new is introduced.
//! What this module adds is the ability to *change* them from outside, plus the
//! plausible range each one is allowed to move in.
//!
//! # Why the bounds live in the code
//!
//! A calibration that is free to put the D region eight times denser than any
//! published value will do exactly that, if that is what minimises its residual,
//! and it will then report an excellent fit to a model that is physically wrong.
//! So each anchor carries the range it is defensible over, and a fit that wants
//! to leave that range is required to say so rather than to quietly widen it -
//! see [`Bounded::clamped`], which reports whether it had to clamp.
//!
//! A bound being reached is therefore a FINDING: it means the residual the fit
//! is chasing is not actually produced by the quantity it is pushing, and the
//! error lies somewhere else in the model.
//!
//! # What is deliberately absent
//!
//! Free-space loss, the antenna patterns, the WSPR reference bandwidth and decode
//! threshold, and everything in the engine crate. Those are either derived
//! results or load-bearing definitions; none of them is a calibration target.

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
    /// bit. A caller that discards the flag has thrown away the finding.
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
/// ionosphere, not a convenience window:
///
/// * D-region peak density 3e8 - 3e9 m^-3 spans quiet night through disturbed
///   day at the 80-90 km peak.
/// * The peak sits at 80-90 km and has a 4-10 km scale height in every
///   textbook treatment.
/// * `nu_e` at 70 km is quoted between about 1e6 and 2e7 s^-1 depending on
///   whose neutral-density profile is used; its scale height follows the neutral
///   atmosphere, 5-9 km.
/// * The E peak sits at 100-115 km with a scale height of 5-15 km.
/// * Overhead quiet-sun foE was taken as 3.0-3.8 MHz from the textbook range;
///   the lower end is now 2.6, because the anchor has since been MEASURED at
///   2.99 against ionosonde soundings and the old floor excluded the answer.
#[derive(Clone, Copy, Debug)]
pub struct IonosphereAnchors {
    /// Overhead-sun D-region peak electron density, m^-3.
    pub d_peak_ne_overhead: Bounded,
    /// Overhead-sun D-region peak height, km.
    pub d_peak_alt_km: Bounded,
    /// D-region Chapman scale height, km.
    pub d_scale_height_km: Bounded,
    /// Residual night-time D-region density, as a fraction of the overhead-sun
    /// peak. See [`crate::chapman::SolarChapmanLayer::with_night_floor`] for why
    /// an alpha-Chapman layer cannot derive this and must be given it.
    pub d_night_floor_fraction: Bounded,
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
    /// foEs at the occurrence maximum, MHz. See [`crate::sporadic_e`]: this is
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
            nu_ref_per_s: Bounded::new(s::NU_REF_PER_S, 1.0e6, 2.0e7),
            nu_ref_alt_km: Bounded::new(s::NU_REF_ALT_KM, 65.0, 80.0),
            nu_scale_height_km: Bounded::new(s::NU_SCALE_HEIGHT_KM, 5.0, 9.0),
            e_peak_alt_km: Bounded::new(s::E_REGION_PEAK_ALT_KM, 100.0, 115.0),
            e_scale_height_km: Bounded::new(s::E_REGION_SCALE_HEIGHT_KM, 5.0, 15.0),
            // Lower bound was 3.0, which the MEASURED value 2.99 sits just outside.
            // Widened deliberately and on evidence, not to unstick a fit: see
            // `fof2::FOE_OVERHEAD_QUIET_MHZ`, measured against 28 952 ionosonde
            // soundings over four seasons. This is the opposite case to the
            // atmospheric-noise bounds, which were widened on a theory and put back.
            foe_overhead_quiet_mhz: Bounded::new(crate::fof2::FOE_OVERHEAD_QUIET_MHZ, 2.6, 3.8),
            es_foes_max_mhz: Bounded::new(crate::sporadic_e::ES_FOES_MAX_MHZ, 5.0, 12.0),
            es_peak_probability: Bounded::new(crate::sporadic_e::ES_PEAK_PROBABILITY, 0.10, 0.70),
        }
    }
}

/// The anchors of the atmospheric-noise surrogate in [`crate::noise`].
///
/// The SHAPE - log-linear in frequency, with additive day/night, season and
/// latitude terms - follows the published P.372 curves and is NOT a calibration
/// target here. These are its absolute anchors, which the module documentation
/// already flags as unverified.
///
/// Ranges are wide because the quantity really is uncertain by that much: the
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
    /// the step with rising frequency; POSITIVE inverts it.
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
            // The three step bounds are drawn around a CONTRAST, not around an
            // absolute level, which is the whole point of the reparameterisation.
            //
            // Do not restore level-shaped bounds here. The old night anchors were
            // boxed by what an absolute atmospheric Fa may plausibly be
            // (80..110 dB at 1 MHz, 35..65 dB/decade), and the fit railed all of
            // them because it was steering a DIFFERENCE through boxes drawn around
            // LEVELS. Raising the night slope ceiling from 65 to 90 was tried on
            // the whole corpus: it railed again, Fa night stayed pinned, the
            // absorption scale stayed pinned, and RMS with station effects removed
            // moved 7.81 -> 7.83 dB. The extra range was not the missing thing.
            //
            // 0 dB of step is allowed deliberately: "no day/night contrast at all"
            // has to be reachable, or the surrogate cannot represent a band whose
            // floor is set by the man-made and galactic curves underneath it,
            // which above ~10 MHz is every band.
            step_1mhz_db: Bounded::new(n::ATM_STEP_1MHZ_DB, 0.0, 40.0),
            // Spans both signs. Negative is the published behaviour - the day and
            // night atmospheric curves converge as frequency rises. Positive is an
            // INVERTED step, unreachable under the old bounds, and the measured
            // step over 7-14 MHz is the reason it must be reachable.
            step_slope_db: Bounded::new(n::ATM_STEP_SLOPE_DB, -45.0, 25.0),
            // Curvature, so the step may bend rather than being forced straight.
            // The corpus carries six bands with a legal cell on both sides of the
            // terminator, so three step parameters are comfortably identified -
            // but see `THE STEP AT THE TERMINATOR`: the three lowest of those six
            // have a daytime side made almost entirely of E-layer paths, which
            // carry a defect of their own. Do not add a fourth step parameter to
            // chase them.
            step_curve_db: Bounded::new(n::ATM_STEP_CURVE_DB, -25.0, 25.0),
            // MEASURED, and the reason not to keep tuning this surrogate.
            //
            // Three noise parameterisations were fitted to the full 9126-spot
            // corpus: the original two lines; the same with the night slope
            // ceiling raised 65 -> 90; and this step form, which the fit drove to
            // a FLAT INVERTED step of about -8 dB (night quieter than day on every
            // band) by railing all three step parameters at once. Their day/night
            // steps differ by more than 20 dB. They score:
            //
            //   RMS, effects removed   7.81   7.83   7.88 dB
            //   AUC decode/non-decode  0.771  0.776  0.775
            //
            // The objective is FLAT along this whole direction. The step form did
            // what it was built to do - it more than halved the spread of the
            // residual's day/night structure across bands, from a 21 dB range to
            // 9 dB - and the model got no better at predicting anything.
            //
            // That is the AUC section's warning arriving: with IQR ~10.4 dB about
            // the residual, this model is limited by per-path SPREAD, not by any
            // smooth systematic term. Fitting more smooth systematic parameters
            // cannot move it. Anything further spent here is spent on the wrong
            // thing.
            season_swing_db: Bounded::new(n::ATM_SEASON_SWING_DB, 0.0, 15.0),
            tropical_boost_db: Bounded::new(n::ATM_TROPICAL_BOOST_DB, 0.0, 30.0),
            polar_offset_db: Bounded::new(n::ATM_POLAR_OFFSET_DB, 0.0, 15.0),
        }
    }
}

/// Everything a calibration may move, in one value carried on
/// [`Inputs`](crate::scenario::Inputs).
///
/// `Default` reproduces the module constants exactly, so a default `Inputs` is
/// bit-identical to the pre-calibration model. `tests::default_anchors_match_the_module_constants`
/// pins that, which is what makes it safe for the GUI to carry this without
/// knowing it exists.
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

    /// The default anchors must BE the module constants. If this drifts, every
    /// run of the GUI silently stops matching the values its own assumptions
    /// panel prints.
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
            (i.foe_overhead_quiet_mhz.value - crate::fof2::FOE_OVERHEAD_QUIET_MHZ).abs() < 1e-12
        );
        assert!((i.es_foes_max_mhz.value - crate::sporadic_e::ES_FOES_MAX_MHZ).abs() < 1e-12);
        assert!(
            (i.es_peak_probability.value - crate::sporadic_e::ES_PEAK_PROBABILITY).abs() < 1e-12
        );

        let n = a.atmospheric;
        assert!((n.f1_day_db.value - crate::noise::ATM_1MHZ_DAY_DB).abs() < 1e-12);
        assert!((n.slope_day_db.value - crate::noise::ATM_SLOPE_DAY_DB).abs() < 1e-12);
        assert!((n.step_1mhz_db.value - crate::noise::ATM_STEP_1MHZ_DB).abs() < 1e-12);
        assert!((n.step_slope_db.value - crate::noise::ATM_STEP_SLOPE_DB).abs() < 1e-12);
        assert!((n.step_curve_db.value - crate::noise::ATM_STEP_CURVE_DB).abs() < 1e-12);
        // The step constants are the OLD night anchors, re-expressed. If either
        // side of that identity is edited alone the surrogate silently stops
        // reproducing the two-line form it replaced.
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

    /// Every default value must sit INSIDE its own stated range. A prior that
    /// starts on a bound cannot be reported as "hit the bound" meaningfully.
    #[test]
    fn every_prior_sits_inside_its_range() {
        let a = Anchors::default();
        for (name, get, _) in Anchors::ionosphere_fields() {
            let b = get(&a.ionosphere);
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

    /// Clamping must REPORT that it clamped, or a fit can leave its physical
    /// range without anyone finding out.
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
}
