//! Sporadic E: a thin, intense, intermittent layer near 100 km.
//!
//! Es is the reason a 17 m signal is heard at 400 km when the F2 layer has no
//! solution at that geometry at all. It is also the layer that least resembles
//! the others, in two ways that both shape this module:
//!
//! * **It is not photochemical.** Es is metallic ions concentrated into a sheet
//!   by wind shear, not a production/recombination balance with the solar
//!   flux. So it gets no Chapman profile and no `Ch(X, chi)`: the shape is a
//!   [`QuasiParabolicLayer`], thin, which is also the engine's closed-form
//!   validation target (docs/derivations/analytic-solutions.md).
//! * **It is probabilistic.** Es either is or is not there. Reporting a path it
//!   supports as simply "available", next to an F2 path that is available every
//!   day, would be a false equivalence. So this module produces an OCCURRENCE
//!   PROBABILITY alongside foEs, and the solver keeps Es-supported paths in a
//!   separate verdict carrying that probability. They are never folded into the
//!   deterministic yes/no.
//!
//! # Provenance
//!
//! Everything below is an ORDER-OF-MAGNITUDE ANCHOR, not a fitted model and not
//! traceable to a citable table, in the same sense as the D-region anchors in
//! [`crate::scenario`]. The SHAPE follows the well-documented midlatitude Es
//! climatology - a strong summer maximum, a diurnal cycle with a mid-morning
//! and an early-evening peak, a temperate-latitude maximum - and the magnitudes
//! are chosen to sit in the published ranges (midlatitude summer daytime
//! occurrence of a few tens of percent, winter night a few percent; foEs
//! typically 3-10 MHz when present). The TRENDS are the defensible part.
//!
//! Deliberately NOT modelled: equatorial Es (a different mechanism entirely -
//! the equatorial electrojet - with its own much higher occurrence), auroral
//! Es, and any spatial patchiness. A real Es cloud is tens to a few hundred km
//! across; this models a uniform blanket, which is why the occurrence
//! probability is reported rather than a yes/no.

use skipzone::constants::SPEED_OF_LIGHT;
use skipzone::density::{ProfileError, QuasiParabolicLayer, density_at_critical_frequency};
use skipzone::units::{Hertz, Meters, PerCubicMeter};

use crate::solar::Season;

/// Height of the Es layer, km. Es is observed between about 90 and 120 km and
/// is most often near 100. ANCHOR.
pub const ES_HEIGHT_KM: f64 = 100.0;
/// Semi-thickness of the Es layer, km. Es sheets are 0.5-3 km thick; 1.5 km is
/// mid-range. ANCHOR. This is what makes it a "thin" layer, and it is the
/// property that lets Es reflect frequencies far above foE.
pub const ES_SEMI_THICKNESS_KM: f64 = 1.5;

/// Peak occurrence probability, local summer, at the diurnal maximum, at the
/// temperate-latitude maximum. ANCHOR.
pub const ES_PEAK_PROBABILITY: f64 = 0.45;
/// Occurrence relative to summer, at equinox. ANCHOR.
pub const ES_EQUINOX_FRACTION: f64 = 0.33;
/// Occurrence relative to summer, in local winter. ANCHOR.
pub const ES_WINTER_FRACTION: f64 = 0.13;

/// Local solar time of the mid-morning occurrence peak, hours. ANCHOR.
pub const ES_MORNING_PEAK_LST_H: f64 = 10.0;
/// Local solar time of the early-evening occurrence peak, hours. ANCHOR.
pub const ES_EVENING_PEAK_LST_H: f64 = 19.0;
/// Width of each diurnal peak, hours. ANCHOR.
pub const ES_PEAK_WIDTH_H: f64 = 3.2;
/// Floor of the diurnal cycle as a fraction of its peak: Es does occur at
/// night, just less often. ANCHOR.
pub const ES_DIURNAL_FLOOR: f64 = 0.30;

/// Latitude of the temperate-zone occurrence maximum, degrees. ANCHOR.
pub const ES_PEAK_LAT_DEG: f64 = 40.0;
/// Width of the latitude dependence, degrees. ANCHOR.
pub const ES_LAT_WIDTH_DEG: f64 = 22.0;
/// Occurrence away from the temperate maximum, as a fraction of it. ANCHOR.
pub const ES_LAT_FLOOR: f64 = 0.35;

/// foEs when Es is present at its occurrence minimum, MHz. ANCHOR.
pub const ES_FOES_MIN_MHZ: f64 = 3.0;
/// foEs when Es is present at its occurrence maximum, MHz. ANCHOR. Blanketing
/// summer Es routinely exceeds this; it is a median, not a ceiling.
pub const ES_FOES_MAX_MHZ: f64 = 9.5;

/// Below this occurrence probability the Es solve is skipped entirely: it costs
/// a full second pass over the homing, and a result that would be reported as
/// "1 % chance" is not worth that. Surfaced in the assumptions.
pub const ES_NEGLIGIBLE_PROBABILITY: f64 = 0.02;

/// Circular distance between two local solar times, hours (never more than 12).
fn lst_separation(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(24.0);
    d.min(24.0 - d)
}

/// The shared 0..1 shape that drives both occurrence and foEs: a season factor,
/// a two-peaked diurnal cycle, and a temperate-latitude weighting. Kept as one
/// function because foEs and occurrence rise and fall together - when Es is
/// more likely it is also, on average, stronger - and splitting them would
/// invite the two to disagree.
fn es_shape(season: Season, lst_h: f64, lat_deg: f64) -> f64 {
    let seasonal = match season {
        Season::Summer => 1.0,
        Season::Equinox => ES_EQUINOX_FRACTION,
        Season::Winter => ES_WINTER_FRACTION,
    };

    let peak = |centre: f64| {
        let u = lst_separation(lst_h, centre) / ES_PEAK_WIDTH_H;
        (-u * u).exp()
    };
    let diurnal = ES_DIURNAL_FLOOR
        + (1.0 - ES_DIURNAL_FLOOR) * peak(ES_MORNING_PEAK_LST_H).max(peak(ES_EVENING_PEAK_LST_H));

    let u = (lat_deg.abs() - ES_PEAK_LAT_DEG) / ES_LAT_WIDTH_DEG;
    let latitude = ES_LAT_FLOOR + (1.0 - ES_LAT_FLOOR) * (-u * u).exp();

    seasonal * diurnal * latitude
}

/// What the Es model says about a place and time. Everything here is reported
/// in the assumptions panel; nothing is used without being shown.
#[derive(Clone)]
pub struct SporadicE {
    /// Critical frequency of the Es layer when it is present, MHz.
    pub foes_mhz: f64,
    /// Probability that a usable Es layer is present, 0..1.
    pub probability: f64,
    pub height_km: f64,
    pub semi_thickness_km: f64,
    /// Where the two numbers above came from, for display.
    pub source: String,
}

impl SporadicE {
    /// Derive foEs and its occurrence probability from local season, local
    /// solar time and latitude.
    ///
    /// `foes_max_mhz` and `peak_probability` are the [`ES_FOES_MAX_MHZ`] and
    /// [`ES_PEAK_PROBABILITY`] anchors, passed in rather than read from the
    /// constants so a calibration run can move them. The SHAPE they scale -
    /// [`es_shape`], the summer maximum, the twin diurnal peaks and the
    /// temperate-latitude weighting - is the documented climatology and is not a
    /// calibration target.
    #[must_use]
    pub fn derive(
        season: Season,
        lst_h: f64,
        lat_deg: f64,
        foes_max_mhz: f64,
        peak_probability: f64,
    ) -> Self {
        let shape = es_shape(season, lst_h, lat_deg).clamp(0.0, 1.0);
        let probability = peak_probability * shape;
        let foes_mhz = ES_FOES_MIN_MHZ + (foes_max_mhz - ES_FOES_MIN_MHZ) * shape;
        Self {
            foes_mhz,
            probability,
            height_km: ES_HEIGHT_KM,
            semi_thickness_km: ES_SEMI_THICKNESS_KM,
            source: format!(
                "derived from local {} at LST {lst_h:.1} h, latitude {lat_deg:.1} deg: \
                 occurrence {:.0} % of the {:.0} % summer-afternoon peak, foEs scaled on the \
                 same shape. Quasi-parabolic sheet {ES_SEMI_THICKNESS_KM} km semi-thick at \
                 {ES_HEIGHT_KM:.0} km - NOT a Chapman layer, because Es is wind-shear \
                 metallic-ion concentration, not photochemical equilibrium. All magnitudes are \
                 order-of-magnitude anchors, not a fitted model; equatorial and auroral Es are \
                 not modelled",
                season.label(),
                100.0 * shape,
                100.0 * peak_probability,
            ),
        }
    }

    /// A manual override: the operator's own foEs and probability, used
    /// verbatim.
    #[must_use]
    pub fn manual(foes_mhz: f64, probability: f64) -> Self {
        Self {
            foes_mhz,
            probability: probability.clamp(0.0, 1.0),
            height_km: ES_HEIGHT_KM,
            semi_thickness_km: ES_SEMI_THICKNESS_KM,
            source: "manual override".to_string(),
        }
    }

    /// True when the layer is likely enough to be worth a second solve.
    #[must_use]
    pub fn is_worth_solving(&self) -> bool {
        self.probability >= ES_NEGLIGIBLE_PROBABILITY && self.foes_mhz > 0.0
    }

    /// Peak electron density of the sheet, m^-3.
    #[must_use]
    pub fn peak_ne(&self) -> f64 {
        density_at_critical_frequency(Hertz::new(self.foes_mhz * 1e6)).get()
    }

    /// The engine layer.
    ///
    /// # Errors
    /// Propagates the engine's own rejection of unphysical geometry.
    pub fn layer(&self, earth_radius_m: f64) -> Result<QuasiParabolicLayer, ProfileError> {
        QuasiParabolicLayer::new(
            PerCubicMeter::new(self.peak_ne()),
            Meters::new(earth_radius_m + self.height_km * 1e3),
            Meters::new(self.semi_thickness_km * 1e3),
        )
    }

    /// Power fraction that TUNNELS THROUGH the sheet instead of reflecting from
    /// it, for a ray whose turning point the tracer placed at `apex_alt_km`.
    ///
    /// # Why an Es reflection is not automatically total
    ///
    /// The tracer is a geometric-optics tracer: it turns a ray where the local
    /// plasma condition is met and calls that a reflection, with no loss at all.
    /// For a *thick* layer that is right. For a thin sheet it is not, because the
    /// evanescent region above the turning point has FINITE width - the density
    /// falls back through the same value on the far side of the peak - so the
    /// wave does not decay forever. It decays across a barrier and re-emerges
    /// above the sheet. That transmitted fraction is energy the reflected ray
    /// never carries, and it is the one reflection loss an Es sheet has that
    /// follows from the model's own geometry rather than from a fitted constant.
    ///
    /// # Derivation
    ///
    /// Take the sheet in its plane-stratified limit, which is where the barrier
    /// integral is analytic (the engine's [`QuasiParabolicLayer`] is the
    /// spherical-Earth form of the same profile):
    ///
    /// ```text
    ///   fp^2(z) = foEs^2 [1 - u^2],   u = (z - h) / a
    /// ```
    ///
    /// with `h` the peak height and `a` the semi-thickness. By the Martyn
    /// equivalence an oblique ray at incidence `i` behaves like a vertical wave
    /// at the equivalent vertical frequency `f_v = f cos i`, and it turns where
    /// `fp = f_v`. Writing `r = f_v / foEs`, the turning point sits at
    /// `u_t = -sqrt(1 - r^2)` and the evanescent region is `|u| < sqrt(1 - r^2)`,
    /// symmetric about the peak. In the WKB (Gamow) approximation the
    /// transmitted power fraction across it is `T = exp(-2 int kappa dz)` with
    /// `kappa = (2 pi / c) sqrt(fp^2 - f_v^2)`, and that integral is a
    /// semicircle:
    ///
    /// ```text
    ///   2 int kappa dz = (4 pi / c) foEs a int_-s^s sqrt(s^2 - u^2) du
    ///                  = (2 pi^2 / c) foEs a (1 - r^2),      s = sqrt(1 - r^2)
    /// ```
    ///
    /// so the barrier opacity is [`Self::barrier_opacity`] and the reflection
    /// loss is `-10 log10(1 - T)`.
    ///
    /// The one thing needed from the ray is `1 - r^2`, and that is read straight
    /// off the traced geometry rather than re-derived from a secant law: in the
    /// parabolic profile `1 - r^2 = u_t^2`, i.e. the turning point's depth below
    /// the peak measured in semi-thicknesses. So the apex altitude the ENGINE
    /// reported is the whole input, and no incidence angle has to be
    /// reconstructed. (`tests::opacity_from_apex_matches_the_frequency_form`
    /// pins that identity against the `f_v/foEs` route.)
    ///
    /// # What it measures out at, which is the finding
    ///
    /// `(2 pi^2 / c) foEs a` is about 790 for a 8 MHz sheet 1.5 km semi-thick,
    /// so the barrier is hundreds of e-foldings opaque unless the ray turns
    /// within ~0.1 % of the peak density. Over the geometries the tracer
    /// actually produces this term is therefore ZERO to many decimal places, and
    /// that is a result rather than a disappointment: **a 1.5 km Es sheet really
    /// is a near-perfect mirror at HF**, so an Es bias measured against real
    /// spots cannot be blamed on a missing reflection loss. It has to come from
    /// the selection rule, from [`Self::foes_mhz`], or from
    /// [`Self::probability`]. The term is kept because it is derived, costs
    /// nothing, and makes the Es MUF cut off with the sharpness the sheet
    /// thickness implies instead of by fiat.
    #[must_use]
    pub fn tunnelling_fraction(&self, apex_alt_km: f64) -> f64 {
        (-self.barrier_opacity(apex_alt_km)).exp()
    }

    /// The dimensionless barrier opacity `2 int kappa dz` of
    /// [`Self::tunnelling_fraction`], for a turning point at `apex_alt_km`.
    ///
    /// Zero (fully transparent) when the ray turned at or above the peak, where
    /// there is no barrier left to cross.
    #[must_use]
    pub fn barrier_opacity(&self, apex_alt_km: f64) -> f64 {
        if !apex_alt_km.is_finite() || self.semi_thickness_km <= 0.0 {
            return f64::INFINITY;
        }
        // Depth of the turning point below the peak, in semi-thicknesses. This
        // is |u_t|, and u_t^2 is the `1 - r^2` of the derivation.
        let depth = ((self.height_km - apex_alt_km) / self.semi_thickness_km).clamp(0.0, 1.0);
        2.0 * std::f64::consts::PI.powi(2) / SPEED_OF_LIGHT
            * (self.foes_mhz * 1e6)
            * (self.semi_thickness_km * 1e3)
            * depth
            * depth
    }

    /// Reflection loss at ONE bounce off the sheet, dB, for a ray the tracer
    /// turned at `apex_alt_km`. See [`Self::tunnelling_fraction`] for the
    /// derivation and for why this is almost always 0.
    #[must_use]
    pub fn reflection_loss_db(&self, apex_alt_km: f64) -> f64 {
        let reflected = 1.0 - self.tunnelling_fraction(apex_alt_km);
        -10.0 * reflected.clamp(1e-12, 1.0).log10()
    }

    /// Altitude band a reflection has to sit in to be attributed to Es, km.
    /// Deliberately wider than the sheet: a ray turns a little below the peak,
    /// and the QP layer's upper zero sits above it.
    #[must_use]
    pub fn attribution_band_km(&self) -> (f64, f64) {
        (
            self.height_km - 3.0 * self.semi_thickness_km,
            self.height_km + 4.0 * self.semi_thickness_km,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skipzone::density::ElectronDensity;
    use skipzone::geo::SphericalPoint;
    use skipzone::units::Radians;

    const R0: f64 = 6_371_000.0;

    /// The documented climatology: summer beats winter, the diurnal cycle has
    /// its two peaks in the right places, and temperate latitudes beat both the
    /// equator and the pole.
    #[test]
    fn occurrence_trends_are_as_documented() {
        let at = |season, lst, lat| SporadicE::derive(season, lst, lat, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY).probability;

        // Season, at the summer diurnal peak and latitude.
        let summer = at(Season::Summer, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        let equinox = at(Season::Equinox, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        let winter = at(Season::Winter, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        assert!(summer > equinox && equinox > winter);
        assert!(
            (summer - ES_PEAK_PROBABILITY).abs() < 1e-9,
            "the peak anchor should be reachable exactly, got {summer}"
        );

        // Diurnal: both named peaks stand above the small hours, and the
        // afternoon lull between them is genuinely a lull.
        let night = at(Season::Summer, 3.0, ES_PEAK_LAT_DEG);
        let morning = at(Season::Summer, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        let evening = at(Season::Summer, ES_EVENING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        let lull = at(Season::Summer, 14.5, ES_PEAK_LAT_DEG);
        assert!(morning > night && evening > night);
        assert!(lull < morning && lull < evening, "no afternoon lull");
        assert!(night > 0.0, "Es does occur at night, just less often");

        // Latitude: temperate maximum, falling towards equator and pole.
        let temperate = at(Season::Summer, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG);
        assert!(temperate > at(Season::Summer, ES_MORNING_PEAK_LST_H, 0.0));
        assert!(temperate > at(Season::Summer, ES_MORNING_PEAK_LST_H, 85.0));
        // Symmetric about the equator.
        assert!((at(Season::Summer, 10.0, 38.0) - at(Season::Summer, 10.0, -38.0)).abs() < 1e-12);

        // Probabilities are probabilities.
        for season in [Season::Summer, Season::Equinox, Season::Winter] {
            for lst in 0..24 {
                for lat in [-88.0, -40.0, 0.0, 40.0, 88.0] {
                    let p = at(season, f64::from(lst), lat);
                    assert!((0.0..=1.0).contains(&p), "probability {p} out of range");
                }
            }
        }
    }

    /// The diurnal cycle wraps: 23:30 and 00:30 LST are neighbours, so the
    /// evening peak does not fall off a cliff at midnight.
    #[test]
    fn diurnal_cycle_wraps_at_midnight() {
        assert!((lst_separation(23.5, 0.5) - 1.0).abs() < 1e-12);
        assert!((lst_separation(1.0, 23.0) - 2.0).abs() < 1e-12);
        let before = SporadicE::derive(Season::Summer, 23.9, 40.0, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY).probability;
        let after = SporadicE::derive(Season::Summer, 0.1, 40.0, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY).probability;
        assert!((before - after).abs() < 0.02, "{before} vs {after}");
    }

    /// foEs tracks occurrence and stays in the anchored band.
    #[test]
    fn foes_tracks_occurrence_within_its_band() {
        let strong = SporadicE::derive(Season::Summer, ES_MORNING_PEAK_LST_H, ES_PEAK_LAT_DEG, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY);
        let weak = SporadicE::derive(Season::Winter, 3.0, 5.0, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY);
        assert!(strong.foes_mhz > weak.foes_mhz);
        for s in [&strong, &weak] {
            assert!(
                (ES_FOES_MIN_MHZ..=ES_FOES_MAX_MHZ).contains(&s.foes_mhz),
                "foEs {} outside the anchored band",
                s.foes_mhz
            );
        }
        assert!(strong.is_worth_solving());
        assert!(
            !weak.is_worth_solving(),
            "deep winter night should be skipped"
        );
    }

    /// The engine accepts the geometry, and the built layer really is a thin
    /// sheet at the stated height with the stated critical frequency: a
    /// vertically incident wave at foEs must find the plasma condition there
    /// and nowhere else.
    #[test]
    fn layer_is_a_thin_sheet_at_the_stated_height() {
        let es = SporadicE::derive(Season::Summer, 10.0, 45.0, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY);
        let layer = es.layer(R0).expect("engine accepts the Es geometry");
        let at = |alt_km: f64| {
            layer
                .sample(&SphericalPoint::new(
                    Meters::new(R0 + alt_km * 1e3),
                    Radians::new(1.1),
                    Radians::new(0.2),
                ))
                .ne
        };
        // Peak at the stated height, matching foEs.
        let peak = at(ES_HEIGHT_KM);
        assert!(
            (peak - es.peak_ne()).abs() < 1e-6 * es.peak_ne(),
            "{peak} vs {}",
            es.peak_ne()
        );
        // Thin: gone a few km either side, which is what lets Es reflect far
        // above foE while the E layer below it does not.
        assert_eq!(at(ES_HEIGHT_KM - 2.0 * ES_SEMI_THICKNESS_KM), 0.0);
        assert_eq!(at(ES_HEIGHT_KM + 4.0 * ES_SEMI_THICKNESS_KM), 0.0);
        // Spherically symmetric, so no horizontal gradient to get wrong.
        let s = layer.sample(&SphericalPoint::new(
            Meters::new(R0 + ES_HEIGHT_KM * 1e3),
            Radians::new(1.1),
            Radians::new(0.2),
        ));
        assert_eq!(s.d_ne[1], 0.0);
        assert_eq!(s.d_ne[2], 0.0);
    }

    /// The attribution band contains the whole sheet, so a reflection off Es
    /// can never be misfiled as an E-layer reflection.
    #[test]
    fn attribution_band_contains_the_sheet() {
        let es = SporadicE::derive(Season::Summer, 10.0, 45.0, ES_FOES_MAX_MHZ, ES_PEAK_PROBABILITY);
        let (lo, hi) = es.attribution_band_km();
        assert!(lo < ES_HEIGHT_KM - ES_SEMI_THICKNESS_KM);
        assert!(hi > ES_HEIGHT_KM + ES_SEMI_THICKNESS_KM);
        // ...and it does not reach the E layer's own peak at 105-110 km.
        assert!(hi < 110.0, "band top {hi} would swallow the E layer");
    }

    /// The barrier opacity read off the traced apex must equal the one derived
    /// from the equivalent vertical frequency. Two routes to `1 - r^2`: the
    /// turning point's depth below the peak (what the code uses, because the
    /// engine reports the apex) and `1 - (f_v/foEs)^2` (what the derivation is
    /// written in). In a parabolic sheet they are the same number, and if they
    /// ever stop being the same the profile assumption has drifted.
    #[test]
    fn opacity_from_apex_matches_the_frequency_form() {
        let es = SporadicE::manual(8.0, 0.4);
        for depth in [0.1, 0.372, 0.5, 0.928, 1.0] {
            let apex = es.height_km - depth * es.semi_thickness_km;
            // Parabolic profile: fp(apex)/foEs = sqrt(1 - depth^2), and the ray
            // turns where f_v = fp, so r = sqrt(1 - depth^2) and 1 - r^2 = depth^2.
            let r = (1.0 - depth * depth).sqrt();
            let expected = 2.0 * std::f64::consts::PI.powi(2) / SPEED_OF_LIGHT
                * (es.foes_mhz * 1e6)
                * (es.semi_thickness_km * 1e3)
                * (1.0 - r * r);
            let got = es.barrier_opacity(apex);
            assert!(
                (got - expected).abs() < 1e-9 * expected.max(1.0),
                "depth {depth}: {got} vs {expected}"
            );
        }
    }

    /// The magnitude that makes this a FINDING rather than a knob: a sheet of
    /// the modelled thickness is hundreds of e-foldings opaque, so the loss is
    /// zero everywhere except within a whisker of the peak. If a future change
    /// to the sheet geometry makes this term start to bite, this test is where
    /// that shows up.
    #[test]
    fn the_sheet_is_an_almost_perfect_mirror() {
        let es = SporadicE::manual(8.0, 0.4);
        // The scale factor of the derivation, quoted in the doc comment as ~790.
        let full = es.barrier_opacity(es.height_km - es.semi_thickness_km);
        assert!((700.0..900.0).contains(&full), "opacity scale {full}");

        // A ray turning where the tracer actually puts them - a good fraction of
        // a semi-thickness below the peak - loses nothing measurable.
        let typical = es.reflection_loss_db(98.6);
        assert!(typical < 1e-9, "{typical} dB at a typical Es apex");

        // The loss only reaches 1 dB when the turning point is within about
        // 0.1 % of the peak DENSITY, i.e. ~5 % of a semi-thickness below the
        // peak in height. That sharpness is the sheet thickness talking.
        let depth_for_1db = (1.58 / full).sqrt();
        let apex_1db = es.height_km - depth_for_1db * es.semi_thickness_km;
        assert!(
            (es.reflection_loss_db(apex_1db) - 1.0).abs() < 0.2,
            "{} dB at apex {apex_1db}",
            es.reflection_loss_db(apex_1db)
        );
        assert!(depth_for_1db < 0.06, "transition width {depth_for_1db}");

        // Monotone: turning deeper can only mean a thicker barrier.
        let mut previous = f64::INFINITY;
        for depth in [0.0, 0.02, 0.05, 0.1, 0.3, 1.0] {
            let loss = es.reflection_loss_db(es.height_km - depth * es.semi_thickness_km);
            assert!(loss <= previous + 1e-12, "not monotone at depth {depth}");
            previous = loss;
        }
        // Turning at or above the peak leaves no barrier: total transmission.
        assert!(es.tunnelling_fraction(es.height_km) > 0.999);
        assert!(es.tunnelling_fraction(es.height_km + 1.0) > 0.999);
    }

    /// A manual override is used verbatim and says so.
    #[test]
    fn manual_override_is_verbatim() {
        let es = SporadicE::manual(12.0, 0.8);
        assert!((es.foes_mhz - 12.0).abs() < 1e-12);
        assert!((es.probability - 0.8).abs() < 1e-12);
        assert!(es.source.contains("manual"));
        // Probabilities are still clamped to a probability.
        assert!((SporadicE::manual(5.0, 3.0).probability - 1.0).abs() < 1e-12);
    }
}
