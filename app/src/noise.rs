//! Radio-noise floor and the received-signal / SNR judgment layer.
//!
//! This module answers the question the ray tracer cannot: *given that a path
//! geometrically closes, would anyone actually hear it?* It touches no
//! ray-tracing, absorption or ground-reflection maths - it consumes the total
//! system loss those produce and adds a transmitter power and a noise floor.
//!
//! # Provenance of every formula here
//!
//! VERIFIED against Recommendation ITU-R P.372-9 (Radio noise, 08/2007), whose
//! text was read directly, not recalled:
//!
//!   * Eq. (2): the external noise factor `fa = pn / (k t0 b)`, with `pn` the
//!     available noise power (W) from an equivalent lossless antenna,
//!     `k = 1.38e-23 J/K`, `t0` the reference temperature "taken as 290 K", and
//!     `b` the noise power bandwidth (Hz). Note 1: `Fa = 10 log10(fa)` dB.
//!   * Eq. (6): the available noise power
//!         `Pn = Fa + B - 204   dBW`
//!     where `B = 10 log10(b)` and `-204 = 10 log10(k t0)`.
//!   * Eq. (11) + Table 1: man-made noise median
//!         `Fam = c - d log10(f)`,  f in MHz,
//!     valid 0.3-250 MHz, with (c, d) = City (76.8, 27.7), Residential
//!     (72.5, 27.7), Rural (67.2, 27.7), Quiet rural (53.6, 28.6), Galactic
//!     noise (52.0, 23.0).
//!
//! NOT from any reference - a documented approximation of my own construction:
//!
//!   * The ATMOSPHERIC noise term. P.372 supplies atmospheric noise as a set of
//!     world maps of `Fam` at 1 MHz (four-hour time blocks x season, derived
//!     from CCIR Report 322) plus separate frequency-dependence curves. There
//!     is NO closed-form equation for it in the Recommendation, and this
//!     project does not ship the map coefficient data. What
//!     [`atmospheric_noise_figure_db`] implements is a log-linear surrogate
//!     with day/night, season and latitude terms whose SHAPE follows the
//!     published curves but whose absolute anchors are unverified. Treat its
//!     magnitudes as indicative; the day/night, seasonal and frequency TRENDS
//!     are the defensible part. This is surfaced in the UI on every run.
//!
//! Everything is combined on a power basis and converted once, explicitly, at
//! the boundaries - see [`dbm_from_watts`] and [`noise_power_dbm`].

use crate::scenario::Season;

/// Boltzmann's constant, J/K. The value P.372-9 itself states under eq. (2).
pub const BOLTZMANN_J_PER_K: f64 = 1.38e-23;

/// Reference temperature `t0`, K. P.372-9 under eq. (2): "taken as 290 K".
pub const REFERENCE_TEMP_K: f64 = 290.0;

/// `10 log10(k t0)` in dBW, i.e. the `-204` of P.372-9 eq. (6) computed rather
/// than pasted in:
///   `k t0 = 1.38e-23 * 290 = 4.002e-21 W/Hz`  ->  `10 log10 = -203.978 dBW`.
/// The Recommendation rounds this to -204; we keep the exact figure so the
/// unit chain is visible, and [`tests::kt0_matches_p372_minus_204`] pins it to
/// the published constant.
#[must_use]
pub fn kt0_dbw() -> f64 {
    10.0 * (BOLTZMANN_J_PER_K * REFERENCE_TEMP_K).log10()
}

/// Transmitter power, watts -> dBm.
///
/// `P[dBm] = 10 log10(P[W] / 1 mW) = 10 log10(P[W] * 1000) = 30 + 10 log10 P[W]`.
/// So 1 W = 30.00 dBm, 100 W = 50.00 dBm, 1500 W = 61.76 dBm.
#[must_use]
pub fn dbm_from_watts(power_w: f64) -> f64 {
    30.0 + power_w.max(1e-12).log10() * 10.0
}

/// Available noise power in dBm from an external noise figure and a bandwidth.
///
/// P.372-9 eq. (6) is in dBW; this returns dBm, so the explicit chain is
///   `Pn[dBW] = Fa + 10 log10(b) + 10 log10(k t0)`
///   `Pn[dBm] = Pn[dBW] + 30`
/// With `Fa = 0` and `b = 1 Hz` this is `-203.98 + 30 = -173.98 dBm`, the
/// familiar `kT0 = -174 dBm/Hz` thermal floor - an independent cross-check
/// that the constant and the unit conversion are both right.
#[must_use]
pub fn noise_power_dbm(fa_db: f64, bandwidth_hz: f64) -> f64 {
    fa_db + 10.0 * bandwidth_hz.max(1e-12).log10() + kt0_dbw() + 30.0
}

/// Man-made noise environment: the categories of P.372-9 Table 1.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoiseEnvironment {
    City,
    Residential,
    Rural,
    QuietRural,
}

impl NoiseEnvironment {
    pub const ALL: [Self; 4] = [Self::City, Self::Residential, Self::Rural, Self::QuietRural];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::City => "city (business)",
            Self::Residential => "residential",
            Self::Rural => "rural",
            Self::QuietRural => "quiet rural",
        }
    }

    /// `(c, d)` of P.372-9 Table 1, for `Fam = c - d log10(f_MHz)`.
    #[must_use]
    pub fn constants(self) -> (f64, f64) {
        match self {
            Self::City => (76.8, 27.7),
            Self::Residential => (72.5, 27.7),
            Self::Rural => (67.2, 27.7),
            Self::QuietRural => (53.6, 28.6),
        }
    }
}

/// Lower edge of the validity range of P.372-9 eq. (11), MHz.
pub const MAN_MADE_VALID_MIN_MHZ: f64 = 0.3;
/// Upper edge of the validity range of P.372-9 eq. (11), MHz (250 MHz for
/// curves A-C, 1000 MHz for quiet rural; the tighter bound is used).
pub const MAN_MADE_VALID_MAX_MHZ: f64 = 250.0;

/// Is `f_mhz` inside the range P.372-9 declares eq. (11) valid over?
/// Outside it the man-made and galactic figures are extrapolations of the fit,
/// which the UI says out loud rather than hiding.
#[must_use]
pub fn man_made_range_is_valid(f_mhz: f64) -> bool {
    (MAN_MADE_VALID_MIN_MHZ..=MAN_MADE_VALID_MAX_MHZ).contains(&f_mhz)
}

/// Man-made noise figure `Fam` [dB above kT0b], P.372-9 eq. (11) + Table 1.
#[must_use]
pub fn man_made_noise_figure_db(env: NoiseEnvironment, f_mhz: f64) -> f64 {
    let (c, d) = env.constants();
    c - d * f_mhz.max(1e-6).log10()
}

/// Galactic noise `(c, d)`: curve E of P.372-9 Table 1.
pub const GALACTIC_C: f64 = 52.0;
pub const GALACTIC_D: f64 = 23.0;

/// Galactic noise figure [dB above kT0b], P.372-9 Table 1 curve E.
///
/// This is the noise reaching the ground; below the ionospheric cut-off it is
/// screened out, but there atmospheric and man-made noise dominate by tens of
/// dB anyway, so no explicit cut-off is applied.
#[must_use]
pub fn galactic_noise_figure_db(f_mhz: f64) -> f64 {
    GALACTIC_C - GALACTIC_D * f_mhz.max(1e-6).log10()
}

// --- Atmospheric noise: APPROXIMATION, NOT ITU-R P.372 MAP DATA -----------
//
// P.372 gives atmospheric noise as world maps of Fam at 1 MHz for six 4-hour
// time blocks x four seasons (CCIR Report 322), plus frequency-dependence
// curves; there is no equation to quote, and the coefficient files are not in
// this repo. The form below is log-linear in frequency, which is roughly how
// those published curves behave across HF, with additive day/night, season and
// latitude terms. The ANCHOR VALUES are order-of-magnitude choices, NOT read
// off the maps and NOT traceable to any table. They are surfaced in the UI.

/// Atmospheric `Fa` at 1 MHz, night, mid-latitude, equinox [dB]. Anchor.
pub const ATM_1MHZ_NIGHT_DB: f64 = 95.0;
/// Atmospheric `Fa` at 1 MHz, day, mid-latitude, equinox [dB]. Anchor.
pub const ATM_1MHZ_DAY_DB: f64 = 70.0;
/// Night fall-off of atmospheric noise with frequency, dB per decade. Anchor.
pub const ATM_SLOPE_NIGHT_DB: f64 = 50.0;
/// Day fall-off of atmospheric noise with frequency, dB per decade. Anchor.
pub const ATM_SLOPE_DAY_DB: f64 = 45.0;
/// Summer / winter offset about the equinox value [dB]. Anchor: thunderstorm
/// activity is the source, so the local summer hemisphere is noisier.
pub const ATM_SEASON_SWING_DB: f64 = 8.0;
/// Equatorial excess over the polar value [dB], applied as `boost * cos^3(lat)`
/// so it concentrates in the tropics where lightning actually is. Anchor.
pub const ATM_TROPICAL_BOOST_DB: f64 = 18.0;
/// Polar offset subtracted everywhere, so `cos(lat) -> 0` lands below the
/// mid-latitude value rather than at it [dB]. Anchor.
pub const ATM_POLAR_OFFSET_DB: f64 = 6.0;

/// Atmospheric (lightning) noise figure [dB above kT0b].
///
/// `Fa_atm = F1 + season + latitude - slope * log10(f_MHz)`
/// with `F1` and `slope` selected by day/night. **Approximation - see the
/// module docs. Not P.372 map data.**
#[must_use]
pub fn atmospheric_noise_figure_db(
    f_mhz: f64,
    is_day: bool,
    season: Season,
    latitude_deg: f64,
) -> f64 {
    let (f1, slope) = if is_day {
        (ATM_1MHZ_DAY_DB, ATM_SLOPE_DAY_DB)
    } else {
        (ATM_1MHZ_NIGHT_DB, ATM_SLOPE_NIGHT_DB)
    };
    let season_db = match season {
        Season::Summer => ATM_SEASON_SWING_DB,
        Season::Winter => -ATM_SEASON_SWING_DB,
        Season::Equinox => 0.0,
    };
    let cos_lat = latitude_deg.to_radians().cos().abs().clamp(0.0, 1.0);
    let lat_db = ATM_TROPICAL_BOOST_DB * cos_lat.powi(3) - ATM_POLAR_OFFSET_DB;
    f1 + season_db + lat_db - slope * f_mhz.max(1e-6).log10()
}

/// Combine independent noise sources. Noise POWERS add, so the figures are
/// converted out of dB, summed, and converted back:
///   `Fa_total = 10 log10( sum_i 10^(Fa_i / 10) )`.
/// Two equal sources therefore give `+3.01 dB`, as they must.
#[must_use]
pub fn combine_noise_figures_db(figures: &[f64]) -> f64 {
    let sum: f64 = figures.iter().map(|f| 10.0_f64.powf(f / 10.0)).sum();
    10.0 * sum.max(1e-30).log10()
}

/// The three noise components and the floor they produce, all kept separately
/// so the debug panel can show where the floor came from.
#[derive(Clone, Copy)]
pub struct NoiseFloor {
    /// Atmospheric noise figure [dB above kT0b]. APPROXIMATION.
    pub atmospheric_db: f64,
    /// Man-made noise figure [dB above kT0b]. P.372-9 Table 1.
    pub man_made_db: f64,
    /// Galactic noise figure [dB above kT0b]. P.372-9 Table 1 curve E.
    pub galactic_db: f64,
    /// Power-sum of the three [dB above kT0b].
    pub total_fa_db: f64,
    /// Noise power in the receiver bandwidth [dBm], P.372-9 eq. (6) + 30.
    pub power_dbm: f64,
    /// Bandwidth the floor was computed in [Hz].
    pub bandwidth_hz: f64,
}

impl NoiseFloor {
    /// Build the floor at one frequency for one scenario.
    #[must_use]
    pub fn compute(
        f_mhz: f64,
        bandwidth_hz: f64,
        env: NoiseEnvironment,
        is_day: bool,
        season: Season,
        latitude_deg: f64,
    ) -> Self {
        let atmospheric_db = atmospheric_noise_figure_db(f_mhz, is_day, season, latitude_deg);
        let man_made_db = man_made_noise_figure_db(env, f_mhz);
        let galactic_db = galactic_noise_figure_db(f_mhz);
        let total_fa_db = combine_noise_figures_db(&[atmospheric_db, man_made_db, galactic_db]);
        Self {
            atmospheric_db,
            man_made_db,
            galactic_db,
            total_fa_db,
            power_dbm: noise_power_dbm(total_fa_db, bandwidth_hz),
            bandwidth_hz,
        }
    }
}

/// Operating mode preset: an occupied noise bandwidth and the SNR needed in
/// THAT bandwidth for the mode to be copyable.
///
/// These are engineering conventions from operating practice, not a cited
/// standard, which is exactly why both numbers stay editable in the UI - the
/// threshold is a setting, never a constant baked into the verdict.
///
/// The FT8 figure deserves its conversion shown: FT8's famous "-21 dB" decode
/// threshold is quoted in a 2500 Hz reference bandwidth. Referred to its own
/// ~50 Hz occupied bandwidth that is
///   `-21 + 10 log10(2500 / 50) = -21 + 16.99 = -4.0 dB`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OperatingMode {
    Cw,
    Ssb,
    Rtty,
    Ft8,
}

impl OperatingMode {
    pub const ALL: [Self; 4] = [Self::Cw, Self::Ssb, Self::Rtty, Self::Ft8];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Cw => "CW",
            Self::Ssb => "SSB voice",
            Self::Rtty => "RTTY (45 baud)",
            Self::Ft8 => "FT8",
        }
    }

    /// `(noise bandwidth [Hz], required SNR in that bandwidth [dB])`.
    #[must_use]
    pub fn defaults(self) -> (f64, f64) {
        match self {
            Self::Cw => (500.0, 3.0),
            Self::Ssb => (2400.0, 10.0),
            Self::Rtty => (300.0, 6.0),
            Self::Ft8 => (50.0, -4.0),
        }
    }
}

/// What the sweep and the status chip report per frequency. Replaces the old
/// two-state "connects" boolean: geometry closing is necessary but not
/// sufficient for a signal to be heard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathState {
    /// Ray tracing found no path at all.
    NoPath,
    /// Geometry closes, but the signal does not clear the SNR threshold.
    BelowThreshold,
    /// Geometry closes and the SNR clears the threshold.
    Usable,
}

impl PathState {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NoPath => "NO PATH",
            Self::BelowThreshold => "PATH FOUND, BELOW THRESHOLD",
            Self::Usable => "USABLE",
        }
    }

    #[must_use]
    pub fn found_path(self) -> bool {
        !matches!(self, Self::NoPath)
    }
}

/// Everything the judgment layer needs that the ray tracer does not produce.
/// Carried into the solution assembler so the loss terms stay untouched and
/// only the verdict is added on top.
#[derive(Clone, Copy)]
pub struct LinkSettings {
    pub tx_power_w: f64,
    pub noise: NoiseFloor,
    pub threshold_db: f64,
}

/// The received-signal verdict for one solution: what arrives, what it has to
/// compete with, and whether it wins by enough.
#[derive(Clone, Copy)]
pub struct LinkBudget {
    pub tx_power_dbm: f64,
    /// `P_rx = P_tx - total system loss` [dBm]. No antenna gains: the loss the
    /// tracer produces is a basic transmission loss between isotropic ends.
    pub rx_power_dbm: f64,
    pub noise: NoiseFloor,
    /// `SNR = P_rx - P_noise` [dB].
    pub snr_db: f64,
    /// Threshold this SNR was judged against [dB].
    pub threshold_db: f64,
}

impl LinkBudget {
    /// Combine a traced path's total system loss with the transmitter power and
    /// the noise floor.
    ///
    /// Unit chain, kept explicit:
    ///   `P_tx[dBm] = 30 + 10 log10 P[W]`
    ///   `P_rx[dBm] = P_tx[dBm] - L_total[dB]`
    ///   `P_n[dBm]  = Fa[dB] + 10 log10 b[Hz] + 10 log10(k t0) + 30`
    ///   `SNR[dB]   = P_rx[dBm] - P_n[dBm]`
    #[must_use]
    pub fn new(
        tx_power_w: f64,
        total_system_loss_db: f64,
        noise: NoiseFloor,
        threshold_db: f64,
    ) -> Self {
        let tx_power_dbm = dbm_from_watts(tx_power_w);
        let rx_power_dbm = tx_power_dbm - total_system_loss_db;
        Self {
            tx_power_dbm,
            rx_power_dbm,
            noise,
            snr_db: rx_power_dbm - noise.power_dbm,
            threshold_db,
        }
    }

    /// Same as [`LinkBudget::new`], reading the scenario-level settings from a
    /// [`LinkSettings`].
    #[must_use]
    pub fn from_settings(s: LinkSettings, total_system_loss_db: f64) -> Self {
        Self::new(s.tx_power_w, total_system_loss_db, s.noise, s.threshold_db)
    }

    /// Margin above the threshold [dB]; negative means the path is too weak.
    #[must_use]
    pub fn margin_db(self) -> f64 {
        self.snr_db - self.threshold_db
    }

    #[must_use]
    pub fn state(self) -> PathState {
        if self.snr_db >= self.threshold_db {
            PathState::Usable
        } else {
            PathState::BelowThreshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P.372-9 eq. (6) writes the constant as `-204 = 10 log10(k t0)`.
    /// Recomputing it from the k and t0 the same Recommendation states must
    /// reproduce that, or one of the three numbers is wrong.
    #[test]
    fn kt0_matches_p372_minus_204() {
        let kt0 = kt0_dbw();
        // 1.38e-23 * 290 = 4.002e-21 W/Hz -> -203.978 dBW.
        assert!(
            (kt0 - (-203.978)).abs() < 0.001,
            "10 log10(k t0) = {kt0}, expected -203.978"
        );
        assert!(
            (kt0 - (-204.0)).abs() < 0.03,
            "must round to the published -204 dBW, got {kt0}"
        );
    }

    /// Independent cross-check of the constant AND the dBW->dBm conversion:
    /// a 0 dB noise figure in 1 Hz is the textbook -174 dBm/Hz thermal floor.
    #[test]
    fn thermal_floor_is_minus_174_dbm_per_hz() {
        let p = noise_power_dbm(0.0, 1.0);
        assert!((p - (-173.978)).abs() < 0.001, "kT0 floor = {p} dBm/Hz");
        // And it scales as 10 log10(b): 2400 Hz adds 33.80 dB.
        let wide = noise_power_dbm(0.0, 2400.0);
        assert!(
            (wide - p - 33.802).abs() < 0.002,
            "2400 Hz should add 33.802 dB, added {}",
            wide - p
        );
    }

    /// W -> dBm at three hand-computed points.
    #[test]
    fn watts_to_dbm_hand_checked() {
        assert!((dbm_from_watts(1.0) - 30.0).abs() < 1e-9);
        assert!((dbm_from_watts(100.0) - 50.0).abs() < 1e-9);
        // 1500 W: 30 + 10 log10(1500) = 30 + 31.7609 = 61.7609 dBm.
        assert!((dbm_from_watts(1500.0) - 61.760_913).abs() < 1e-5);
        // 5 W QRP: 30 + 6.9897 = 36.9897 dBm.
        assert!((dbm_from_watts(5.0) - 36.989_700).abs() < 1e-5);
    }

    /// P.372-9 eq. (11) with Table 1, evaluated by hand at points where the
    /// arithmetic is checkable.
    #[test]
    fn man_made_matches_p372_table_1() {
        // log10(1) = 0, so at 1 MHz every category returns its own c exactly.
        for env in NoiseEnvironment::ALL {
            let (c, _) = env.constants();
            assert!((man_made_noise_figure_db(env, 1.0) - c).abs() < 1e-12);
        }
        // Residential at 10 MHz: 72.5 - 27.7 * 1 = 44.8 dB.
        assert!((man_made_noise_figure_db(NoiseEnvironment::Residential, 10.0) - 44.8).abs() < 1e-9);
        // City at 10 MHz: 76.8 - 27.7 = 49.1 dB.
        assert!((man_made_noise_figure_db(NoiseEnvironment::City, 10.0) - 49.1).abs() < 1e-9);
        // Rural at 3 MHz: 67.2 - 27.7 * 0.4771213 = 67.2 - 13.2163 = 53.9837 dB.
        assert!(
            (man_made_noise_figure_db(NoiseEnvironment::Rural, 3.0) - 53.983_7).abs() < 1e-4,
            "{}",
            man_made_noise_figure_db(NoiseEnvironment::Rural, 3.0)
        );
        // Quiet rural at 100 MHz: 53.6 - 28.6 * 2 = -3.6 dB (below kT0b).
        assert!(
            (man_made_noise_figure_db(NoiseEnvironment::QuietRural, 100.0) - (-3.6)).abs() < 1e-9
        );
        // The published ordering: city noisiest, quiet rural quietest.
        let f = 7.0;
        assert!(
            man_made_noise_figure_db(NoiseEnvironment::City, f)
                > man_made_noise_figure_db(NoiseEnvironment::Residential, f)
        );
        assert!(
            man_made_noise_figure_db(NoiseEnvironment::Residential, f)
                > man_made_noise_figure_db(NoiseEnvironment::Rural, f)
        );
        assert!(
            man_made_noise_figure_db(NoiseEnvironment::Rural, f)
                > man_made_noise_figure_db(NoiseEnvironment::QuietRural, f)
        );
    }

    /// Table 1 curve E, hand-evaluated.
    #[test]
    fn galactic_matches_p372_curve_e() {
        // 1 MHz: 52.0 - 0 = 52.0 dB.
        assert!((galactic_noise_figure_db(1.0) - 52.0).abs() < 1e-12);
        // 10 MHz: 52.0 - 23.0 = 29.0 dB.
        assert!((galactic_noise_figure_db(10.0) - 29.0).abs() < 1e-9);
        // 30 MHz: 52.0 - 23.0 * 1.4771213 = 52.0 - 33.9738 = 18.0262 dB.
        assert!(
            (galactic_noise_figure_db(30.0) - 18.026_2).abs() < 1e-4,
            "{}",
            galactic_noise_figure_db(30.0)
        );
        // Sanity in temperature terms: Ta = t0 (fa - 1) at 100 MHz is ~864 K,
        // the right order for the galactic background (roughly 0.8-3 kK).
        let fa = 10.0_f64.powf(galactic_noise_figure_db(100.0) / 10.0);
        let t_sky = REFERENCE_TEMP_K * (fa - 1.0);
        assert!((600.0..3000.0).contains(&t_sky), "sky temp {t_sky} K");
    }

    /// Powers add, not decibels.
    #[test]
    fn combining_equal_sources_adds_3_db() {
        assert!((combine_noise_figures_db(&[20.0, 20.0]) - 23.010_3).abs() < 1e-4);
        // Ten equal sources: +10 dB exactly.
        let ten = [30.0_f64; 10];
        assert!((combine_noise_figures_db(&ten) - 40.0).abs() < 1e-9);
        // A source 20 dB below another contributes ~0.04 dB.
        let d = combine_noise_figures_db(&[40.0, 20.0]) - 40.0;
        assert!((d - 0.043_2).abs() < 1e-3, "{d}");
        // The total is never below the loudest contributor.
        let parts = [12.0, 31.5, 8.0];
        let total = combine_noise_figures_db(&parts);
        assert!(total >= 31.5 && total < 32.0, "{total}");
    }

    /// The atmospheric surrogate has no reference value to check against, so
    /// only its documented BEHAVIOUR is pinned: falling with frequency, louder
    /// at night, louder in summer, louder in the tropics.
    #[test]
    fn atmospheric_trends_are_as_documented() {
        let (day, night) = (true, false);
        let mid = 50.0;
        // Falls monotonically with frequency across HF.
        let mut prev = f64::INFINITY;
        let mut f = 2.0;
        while f <= 30.0 {
            let v = atmospheric_noise_figure_db(f, night, Season::Equinox, mid);
            assert!(v < prev, "not falling at {f} MHz");
            prev = v;
            f += 0.5;
        }
        // Night is louder than day at the same frequency.
        for f in [3.5, 7.0, 14.0, 28.0] {
            assert!(
                atmospheric_noise_figure_db(f, night, Season::Equinox, mid)
                    > atmospheric_noise_figure_db(f, day, Season::Equinox, mid),
                "night should exceed day at {f} MHz"
            );
        }
        // Summer louder than winter, by the full documented swing.
        let s = atmospheric_noise_figure_db(7.0, night, Season::Summer, mid);
        let w = atmospheric_noise_figure_db(7.0, night, Season::Winter, mid);
        assert!((s - w - 2.0 * ATM_SEASON_SWING_DB).abs() < 1e-9);
        // Tropics louder than poles.
        assert!(
            atmospheric_noise_figure_db(7.0, night, Season::Equinox, 0.0)
                > atmospheric_noise_figure_db(7.0, night, Season::Equinox, 80.0)
        );
        // Sign of latitude must not matter.
        let n = atmospheric_noise_figure_db(7.0, night, Season::Equinox, 35.0);
        let s = atmospheric_noise_figure_db(7.0, night, Season::Equinox, -35.0);
        assert!((n - s).abs() < 1e-12);
    }

    /// The one qualitative claim the composed floor must get right: at the low
    /// end of HF the floor is atmospheric, at the top of HF it is galactic (in
    /// a quiet-rural location). If the surrogate's slope were badly wrong this
    /// crossover would land outside the band.
    #[test]
    fn atmospheric_dominates_low_hf_galactic_dominates_high_hf() {
        let env = NoiseEnvironment::QuietRural;
        let low = NoiseFloor::compute(2.0, 2400.0, env, false, Season::Equinox, 50.0);
        assert!(
            low.atmospheric_db > low.galactic_db + 10.0,
            "at 2 MHz atmospheric {} should dominate galactic {}",
            low.atmospheric_db,
            low.galactic_db
        );
        let high = NoiseFloor::compute(28.0, 2400.0, env, true, Season::Equinox, 50.0);
        assert!(
            high.galactic_db > high.atmospheric_db,
            "at 28 MHz daytime galactic {} should dominate atmospheric {}",
            high.galactic_db,
            high.atmospheric_db
        );
    }

    /// The whole chain on numbers that can be followed by hand.
    ///
    ///   P_tx  = 30 + 10 log10(100)        = 50.00 dBm
    ///   P_rx  = 50.00 - 140.00            = -90.00 dBm
    ///   P_n   = Fa + 10 log10(2400) - 173.978
    ///   SNR   = P_rx - P_n
    #[test]
    fn link_budget_chain_is_hand_followable() {
        let noise = NoiseFloor::compute(
            14.0,
            2400.0,
            NoiseEnvironment::Rural,
            true,
            Season::Equinox,
            50.0,
        );
        let lb = LinkBudget::new(100.0, 140.0, noise, 10.0);
        assert!((lb.tx_power_dbm - 50.0).abs() < 1e-9);
        assert!((lb.rx_power_dbm - (-90.0)).abs() < 1e-9);
        // Noise power must equal the formula applied to the composed figure.
        let expect = noise.total_fa_db + 10.0 * 2400.0_f64.log10() + kt0_dbw() + 30.0;
        assert!((lb.noise.power_dbm - expect).abs() < 1e-12);
        assert!((lb.snr_db - (lb.rx_power_dbm - lb.noise.power_dbm)).abs() < 1e-12);
        assert!((lb.margin_db() - (lb.snr_db - 10.0)).abs() < 1e-12);
        // 100 W over a 140 dB path on 20 m in a rural daytime location is a
        // workable SSB contact, so the verdict must be Usable here.
        assert_eq!(lb.state(), PathState::Usable);
    }

    /// The state boundary is exactly the threshold, and more loss eventually
    /// pushes a found path below it - the behaviour the whole change exists for.
    #[test]
    fn threshold_decides_usable_versus_below() {
        let noise = NoiseFloor::compute(
            14.0,
            2400.0,
            NoiseEnvironment::Rural,
            true,
            Season::Equinox,
            50.0,
        );
        // Pick the loss that lands SNR exactly on a 10 dB threshold.
        let exact_loss = dbm_from_watts(100.0) - noise.power_dbm - 10.0;
        let on = LinkBudget::new(100.0, exact_loss, noise, 10.0);
        assert!((on.snr_db - 10.0).abs() < 1e-9);
        assert_eq!(on.state(), PathState::Usable, "at threshold counts as usable");
        let under = LinkBudget::new(100.0, exact_loss + 0.5, noise, 10.0);
        assert_eq!(under.state(), PathState::BelowThreshold);
        assert!(under.margin_db() < 0.0);
        // A 190 dB path is dead air however good the geometry is.
        let dead = LinkBudget::new(100.0, 190.0, noise, 10.0);
        assert_eq!(dead.state(), PathState::BelowThreshold);
        // ...but FT8's threshold in its own 50 Hz bandwidth may still take it.
        let (bw, thr) = OperatingMode::Ft8.defaults();
        let quiet = NoiseFloor::compute(
            14.0,
            bw,
            NoiseEnvironment::Rural,
            true,
            Season::Equinox,
            50.0,
        );
        assert!(
            LinkBudget::new(100.0, 165.0, quiet, thr).state() == PathState::Usable,
            "narrow bandwidth plus a low threshold should recover a weak path"
        );
    }

    /// The FT8 threshold conversion quoted in `OperatingMode`'s docs.
    #[test]
    fn ft8_threshold_conversion_is_consistent() {
        let (bw, thr) = OperatingMode::Ft8.defaults();
        // -21 dB in 2500 Hz referred to 50 Hz: -21 + 10 log10(2500/50).
        let referred = -21.0 + 10.0 * (2500.0_f64 / bw).log10();
        assert!((referred - thr).abs() < 0.05, "{referred} vs {thr}");
        // Wider modes need more SNR in their own bandwidth than FT8 does.
        for m in [OperatingMode::Cw, OperatingMode::Ssb, OperatingMode::Rtty] {
            assert!(m.defaults().1 > thr);
        }
    }
}
