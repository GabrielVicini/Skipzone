//! Two-way fixed-effects calibration of the model against measured WSPR SNRs.
//!
//! # Station effects
//!
//! A WSPR spot's measured SNR mixes propagation with station properties:
//!
//! ```text
//!   measured = physics + tx_effect + rx_effect + fading + error
//! ```
//!
//! `tx_effect` bundles the transmitting antenna (unknown, worth tens of dB) with
//! the accuracy of the claimed power. `rx_effect` bundles the receiving antenna
//! with the receiver site's local noise floor. Neither is in the archive, and
//! both are large.
//!
//! Fitting the physics to raw measured SNR lets those two unknowns leak into
//! the most flexible physical parameter (D-region absorption or the noise
//! model): the residual falls while the model gets worse.
//!
//! So `tx_effect` and `rx_effect` are estimated as nuisance parameters jointly
//! with the physics ([`crate::fit::StationEffects`]), and the physics is identified from
//! variation within a station:
//!
//! * one transmitter heard by many receivers in one cycle: the TX effect and the
//!   claimed power cancel;
//! * one TX->RX pair on several bands at once: both effects cancel, which
//!   identifies the frequency dependence of absorption;
//! * one TX->RX pair over many hours: both effects are fixed, so the diurnal
//!   variation isolates the solar-zenith-angle dependence of the D region.
//!
//! # What cannot be identified
//!
//! Anything constant for a given station is absorbed into that station's
//! effect:
//!
//! | quantity | identified by | identifiable here |
//! |---|---|---|
//! | absorption magnitude | overall level, and its frequency and zenith pattern | yes |
//! | atmospheric noise day-night difference | diurnal variation within a station | yes |
//! | atmospheric noise frequency slopes | cross-band within a station | yes |
//! | atmospheric noise absolute level | nothing - it is a constant | no, absorbed |
//! | receiver noise environment | a constant per receiver | no, absorbed |
//! | latitude terms of the noise model | a receiver's latitude never changes | no, absorbed |
//! | seasonal swing | needs more than one season in the corpus | not from one month |
//! | absolute antenna gain | a constant per station | no, absorbed |
//!
//! A fit can therefore calibrate how the signal varies with frequency, path
//! length, zenith angle, hop count and layer, but not absolute levels.
//!
//! # Caching
//!
//! Re-solving a spot costs a few hundred milliseconds and a fit needs thousands
//! of evaluations. Two measured facts avoid that:
//!
//! 1. Absorption is linear in the collision frequency. Doubling `NU_REF_PER_S`
//!    doubles the reported absorption to four significant figures
//!    (`the_absorption_scale_is_linear` checks it). In the D region
//!    `nu << omega`, so the absorption coefficient is proportional to
//!    `Ne nu / omega^2`.
//! 2. The D-region and collision parameters do not move the ray. The D region's
//!    plasma frequency is ~0.3 MHz, so at 7 MHz `X ~ 0.002`. Moving the
//!    collision reference altitude from 65 to 80 km changes absorption from
//!    5.7 dB to 53.4 dB but the launch elevation by only 0.0003 degrees.
//!
//! So each spot is traced once and the link budget is re-derived
//! arithmetically for any absorption scale and noise anchors ([`crate::fit::Cached`]).
//!
//! Parameters that change which paths exist (E-region geometry, foE, foEs, Es
//! occurrence) need a real re-solve. They are scanned coarsely and scored
//! against the negatives set, since least squares cannot differentiate a path
//! turning on or off.

use std::collections::BTreeMap;

use crate::calib::{Anchors, AtmosphericAnchors, Bounded};
use crate::noise::{NoiseEnvironment, NoiseFloor};
use skipzone_iono::solar::Season;

/// Everything about one solved spot that lets its SNR be re-derived without
/// tracing the ray again.
///
/// The split is exact: the link budget is `tx_power - (loss - gain) - noise`,
/// and absorption is the only loss term the cached parameters touch.
/// `tests::cache_reproduces_the_solver` checks it against a real solve.
#[derive(Clone, Debug)]
pub struct Cached {
    /// Index of the transmitting station in the fit's station table.
    pub tx: usize,
    /// Index of the receiving station.
    pub rx: usize,
    /// Measured SNR in the WSPR 2500 Hz reference bandwidth, dB.
    pub measured_db: f64,
    /// Transmitter power as claimed, dBm.
    pub tx_power_dbm: f64,
    /// Every loss term except ionospheric absorption, less the antenna gain, dB.
    /// Free-space spreading, ground reflections and any Es sheet loss.
    pub loss_without_absorption_db: f64,
    /// Ionospheric absorption at the baseline anchors, dB. Scaled linearly.
    pub absorption_db: f64,
    /// Frequency, needed to recompute the noise floor. MHz.
    pub freq_mhz: f64,
    /// Receiver bandwidth the measured SNR is quoted in, Hz.
    pub bandwidth_hz: f64,
    /// Whether it was day at the receiver, where the noise floor is heard.
    pub rx_is_day: bool,
    pub rx_season: Season,
    pub rx_lat: f64,
    pub noise_env: NoiseEnvironment,
    /// Which layer carried the path, for per-layer reporting.
    pub layer: &'static str,
    pub hops: u32,
    pub range_km: f64,
    /// Es occurrence probability if this was an Es path, else 1.
    pub probability: f64,
    /// Day of the corpus this spot came from, for the hold-out split.
    pub date: (i32, u32, u32),
    /// Solar zenith angle at the path midpoint, degrees; above 90 the midpoint
    /// is dark. Layers are split on this because the reflection happens at the
    /// midpoint.
    pub midpoint_zenith_deg: f64,
    /// The best F2 path that also closed on this spot, when a lower layer was
    /// reported. `None` when F2 was itself the choice or no F2 path closed;
    /// [`Cached::layer_was_a_race`] tells the two apart.
    pub alternative: Option<Alternative>,
}

/// The F2 path a lower layer beat, carried so its SNR can be re-derived under
/// the same parameters as the path that won.
///
/// Only the two loss terms are stored; everything else the link budget needs
/// is a property of the spot and is already on [`Cached`].
#[derive(Clone, Copy, Debug)]
pub struct Alternative {
    /// Every loss term except ionospheric absorption, less antenna gain, dB.
    pub loss_without_absorption_db: f64,
    /// Ionospheric absorption at the baseline anchors, dB. Scaled linearly, as
    /// the winner's is.
    pub absorption_db: f64,
}

impl Cached {
    /// Modelled SNR (dB) under an absorption scale and a set of noise anchors.
    ///
    /// `absorption_scale` multiplies the baseline absorption; 1.0 reproduces the
    /// solve exactly. It stands for the product of the D-region electron density
    /// and the collision frequency, which is the only combination of the two that
    /// absorption depends on - see [`absorption_scale_as_nu`].
    #[must_use]
    pub fn modelled_db(&self, absorption_scale: f64, atm: AtmosphericAnchors) -> f64 {
        let noise = NoiseFloor::compute(
            self.freq_mhz,
            self.bandwidth_hz,
            self.noise_env,
            self.rx_is_day,
            self.rx_season,
            self.rx_lat,
            atm,
        );
        self.tx_power_dbm
            - self.loss_without_absorption_db
            - absorption_scale * self.absorption_db
            - noise.power_dbm
    }

    /// Was the reported layer chosen over an F2 path that also closed?
    ///
    /// When a lower layer reads optimistic, this separates a selection rule
    /// that prefers it over F2 from a lower layer that was the only path.
    #[must_use]
    pub fn layer_was_a_race(&self) -> bool {
        self.alternative.is_some()
    }

    /// Modelled SNR (dB) of the F2 path this spot's layer beat, under the same
    /// parameters and the same noise floor. `None` when there was no such path.
    #[must_use]
    pub fn alternative_modelled_db(
        &self,
        absorption_scale: f64,
        atm: AtmosphericAnchors,
    ) -> Option<f64> {
        let alt = self.alternative?;
        Some(
            self.tx_power_dbm
                - alt.loss_without_absorption_db
                - absorption_scale * alt.absorption_db
                - self.noise_dbm(atm),
        )
    }

    /// Whether the path midpoint is dark. The reflection happens there, and on
    /// a long path the receiver's local day can differ.
    #[must_use]
    pub fn midpoint_is_night(&self) -> bool {
        self.midpoint_zenith_deg > 90.0
    }

    /// The noise floor this spot was judged against under given anchors, dBm.
    #[must_use]
    pub fn noise_dbm(&self, atm: AtmosphericAnchors) -> f64 {
        NoiseFloor::compute(
            self.freq_mhz,
            self.bandwidth_hz,
            self.noise_env,
            self.rx_is_day,
            self.rx_season,
            self.rx_lat,
            atm,
        )
        .power_dbm
    }
}

/// The absorption scale expressed back as a collision frequency, s^-1.
///
/// Absorption depends on the product of D-region electron density and
/// collision frequency, so the data cannot split the two. The scale is
/// attributed to [`crate::scenario::NU_REF_PER_S`] because doubling the
/// collision frequency doubles the reported absorption, while doubling the
/// D-region peak density changes it by only about 5 % (the E and F2 layers
/// contribute most of the absorption integral).
#[must_use]
pub fn absorption_scale_as_nu(scale: f64, prior: Bounded) -> (Bounded, bool) {
    prior.clamped(prior.value * scale)
}

/// Per-station additive offsets, plus the global offset neither of them can
/// distinguish from the other.
///
/// # Gauge
///
/// `tx + rx` is identified but `tx` and `rx` separately are not: adding a
/// constant to every transmitter effect and subtracting it from every receiver
/// effect leaves every prediction unchanged. And a constant added to both is
/// indistinguishable from a global bias in the model. So the fit pins the gauge
/// by forcing both sets of effects to average zero and carrying the leftover
/// explicitly in [`Self::global_db`].
///
/// The global offset is the part of the bias WSPR cannot attribute: the model
/// being optimistic and the station population being worse than the assumed
/// antennas look the same.
#[derive(Clone, Debug, Default)]
pub struct StationEffects {
    /// Transmitter offsets, dB, mean zero. Indexed as [`Cached::tx`].
    pub tx: Vec<f64>,
    /// Receiver offsets, dB, mean zero. Indexed as [`Cached::rx`].
    pub rx: Vec<f64>,
    /// The offset the two cannot tell apart, dB. Positive means the model reads
    /// high once station effects are removed.
    pub global_db: f64,
    /// How many spots each transmitter contributed.
    pub tx_counts: Vec<usize>,
    pub rx_counts: Vec<usize>,
}

impl StationEffects {
    /// This spot's total nuisance offset, dB.
    #[must_use]
    pub fn offset_for(&self, c: &Cached) -> f64 {
        self.global_db + self.tx[c.tx] + self.rx[c.rx]
    }

    /// Solve the two-way layout for a fixed set of model residuals.
    ///
    /// `residual_i = modelled_i - measured_i`. The effects are the least-squares
    /// two-way decomposition of that, computed by alternating conditional means
    /// (the standard sweep algorithm for a two-factor additive model with
    /// unbalanced cells). It converges geometrically, and 200 sweeps is far more
    /// than corpora of this size need. It gives the normal-equations answer
    /// without building the matrix.
    #[must_use]
    pub fn solve(residuals: &[f64], spots: &[Cached], n_tx: usize, n_rx: usize) -> Self {
        let mut tx = vec![0.0; n_tx];
        let mut rx = vec![0.0; n_rx];
        let mut tx_counts = vec![0usize; n_tx];
        let mut rx_counts = vec![0usize; n_rx];
        for c in spots {
            tx_counts[c.tx] += 1;
            rx_counts[c.rx] += 1;
        }
        let mut global = 0.0;
        if spots.is_empty() {
            return Self {
                tx,
                rx,
                global_db: 0.0,
                tx_counts,
                rx_counts,
            };
        }

        for _ in 0..200 {
            // Global: the mean of what the station effects do not explain.
            let mut sum = 0.0;
            for (c, r) in spots.iter().zip(residuals) {
                sum += r - tx[c.tx] - rx[c.rx];
            }
            #[allow(clippy::cast_precision_loss)]
            let n = spots.len() as f64;
            global = sum / n;

            // Transmitters, holding receivers fixed.
            let mut acc = vec![0.0; n_tx];
            for (c, r) in spots.iter().zip(residuals) {
                acc[c.tx] += r - global - rx[c.rx];
            }
            for t in 0..n_tx {
                if tx_counts[t] > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let k = tx_counts[t] as f64;
                    tx[t] = acc[t] / k;
                }
            }

            // Receivers, holding transmitters fixed.
            let mut acc = vec![0.0; n_rx];
            for (c, r) in spots.iter().zip(residuals) {
                acc[c.rx] += r - global - tx[c.tx];
            }
            for x in 0..n_rx {
                if rx_counts[x] > 0 {
                    #[allow(clippy::cast_precision_loss)]
                    let k = rx_counts[x] as f64;
                    rx[x] = acc[x] / k;
                }
            }

            // Re-centre so the global offset holds the whole unattributable part.
            recentre(&mut tx, &tx_counts);
            recentre(&mut rx, &rx_counts);
        }

        Self {
            tx,
            rx,
            global_db: global,
            tx_counts,
            rx_counts,
        }
    }

    /// Spread of the estimated effects over stations with enough spots. This
    /// measures the WSPR station population and is reported on its own.
    #[must_use]
    pub fn distribution(&self, min_spots: usize) -> EffectDistribution {
        let pick = |v: &[f64], counts: &[usize]| -> Vec<f64> {
            v.iter()
                .zip(counts)
                .filter(|(_, n)| **n >= min_spots)
                .map(|(e, _)| *e)
                .collect()
        };
        EffectDistribution {
            tx: Spread::of(&pick(&self.tx, &self.tx_counts)),
            rx: Spread::of(&pick(&self.rx, &self.rx_counts)),
        }
    }
}

/// Shift a set of effects to mean zero, weighting by how many spots each one was
/// estimated from. An unweighted centring would let a station with two spots move
/// the gauge as much as one with two hundred.
fn recentre(effects: &mut [f64], counts: &[usize]) {
    let mut total = 0.0;
    let mut weight = 0.0;
    for (e, &n) in effects.iter().zip(counts) {
        #[allow(clippy::cast_precision_loss)]
        let w = n as f64;
        total += e * w;
        weight += w;
    }
    if weight > 0.0 {
        let mean = total / weight;
        for e in effects.iter_mut() {
            *e -= mean;
        }
    }
}

/// The distribution of estimated station effects at each end.
#[derive(Clone, Debug)]
pub struct EffectDistribution {
    pub tx: Spread,
    pub rx: Spread,
}

/// Order statistics of a sample.
#[derive(Clone, Debug, Default)]
pub struct Spread {
    pub n: usize,
    pub median: f64,
    pub p10: f64,
    pub p25: f64,
    pub p75: f64,
    pub p90: f64,
    pub min: f64,
    pub max: f64,
}

impl Spread {
    #[must_use]
    pub fn of(values: &[f64]) -> Self {
        let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
        v.sort_by(f64::total_cmp);
        Self {
            n: v.len(),
            median: percentile(&v, 0.5),
            p10: percentile(&v, 0.10),
            p25: percentile(&v, 0.25),
            p75: percentile(&v, 0.75),
            p90: percentile(&v, 0.90),
            min: v.first().copied().unwrap_or(f64::NAN),
            max: v.last().copied().unwrap_or(f64::NAN),
        }
    }

    #[must_use]
    pub fn iqr(&self) -> f64 {
        self.p75 - self.p25
    }
}

/// Linearly interpolated percentile of a sorted slice; NaN when empty.
#[must_use]
pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    #[allow(clippy::cast_precision_loss)]
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = lo as usize;
    let frac = pos - lo;
    if i + 1 >= sorted.len() {
        sorted[i]
    } else {
        sorted[i] + (sorted[i + 1] - sorted[i]) * frac
    }
}

/// How well modelled tracks measured: the headline the calibration is judged on.
///
/// The slope matters more than the bias: a slope of 0.3 means the model gives
/// nearly the same number for every path.
///
/// # Why two slopes are reported
///
/// Against raw measured SNR the slope is attenuated by construction. If
/// `measured = physics + station_effect`, then regressing the modelled value
/// (which contains no station effect) on the measured value gives
///
/// ```text
///   slope = Var(physics) / (Var(physics) + Var(station_effect))
/// ```
///
/// which is below 1 even for perfect physics (errors-in-variables
/// attenuation). With station effects worth twice the variance of the physics
/// it lands near 0.33.
///
/// [`Fit::slope_raw`] is what a naive comparison shows. [`Fit::slope_adjusted`]
/// removes the estimated station effects from the measurement and shows
/// whether the physics tracks reality.
#[derive(Clone, Debug, Default)]
pub struct Fit {
    pub n: usize,
    /// Slope of modelled on raw measured.
    pub slope_raw: f64,
    pub r2_raw: f64,
    /// Slope of modelled on measured-plus-estimated-station-effects.
    pub slope_adjusted: f64,
    pub r2_adjusted: f64,
    /// `modelled - measured` before station effects are removed.
    pub residual: Spread,
    /// `modelled - measured - station effects`: what the physics still cannot
    /// explain.
    pub adjusted_residual: Spread,
    /// Root mean square of the adjusted residual, the quantity the fit minimises.
    pub rms_db: f64,
}

impl Fit {
    /// Score a set of cached spots under given parameters and station effects.
    #[must_use]
    pub fn of(
        spots: &[Cached],
        absorption_scale: f64,
        atm: AtmosphericAnchors,
        effects: &StationEffects,
    ) -> Self {
        let mut modelled = Vec::with_capacity(spots.len());
        let mut measured = Vec::with_capacity(spots.len());
        let mut explained = Vec::with_capacity(spots.len());
        let mut residual = Vec::with_capacity(spots.len());
        let mut adjusted = Vec::with_capacity(spots.len());
        for c in spots {
            let m = c.modelled_db(absorption_scale, atm);
            let offset = effects.offset_for(c);
            modelled.push(m);
            measured.push(c.measured_db);
            // The measurement with the station's offset folded in, which is
            // what the physics should be compared against.
            explained.push(c.measured_db + offset);
            residual.push(m - c.measured_db);
            adjusted.push(m - c.measured_db - offset);
        }
        let (slope_raw, r2_raw) = regress(&measured, &modelled);
        let (slope_adjusted, r2_adjusted) = regress(&explained, &modelled);
        #[allow(clippy::cast_precision_loss)]
        let n = spots.len().max(1) as f64;
        Self {
            n: spots.len(),
            slope_raw,
            r2_raw,
            slope_adjusted,
            r2_adjusted,
            residual: Spread::of(&residual),
            adjusted_residual: Spread::of(&adjusted),
            rms_db: (adjusted.iter().map(|e| e * e).sum::<f64>() / n).sqrt(),
        }
    }
}

/// Ordinary least squares slope of `y` on `x`, and the R^2 of that line.
///
/// Returns `(NaN, NaN)` when `x` has no variance, since a slope against a
/// constant is undefined.
#[must_use]
pub fn regress(x: &[f64], y: &[f64]) -> (f64, f64) {
    let pairs: Vec<(f64, f64)> = x
        .iter()
        .zip(y)
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .map(|(a, b)| (*a, *b))
        .collect();
    if pairs.len() < 3 {
        return (f64::NAN, f64::NAN);
    }
    #[allow(clippy::cast_precision_loss)]
    let n = pairs.len() as f64;
    let mx = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let my = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum();
    let syy: f64 = pairs.iter().map(|p| (p.1 - my).powi(2)).sum();
    let sxy: f64 = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    if sxx <= 0.0 || syy <= 0.0 {
        return (f64::NAN, f64::NAN);
    }
    let slope = sxy / sxx;
    let r = sxy / (sxx * syy).sqrt();
    (slope, r * r)
}

/// The parameters a cached fit can move, and the range each may move in.
///
/// Only the ones that re-derive from [`Cached`] without a re-solve. The rest
/// change which paths exist and are scanned separately.
#[derive(Clone, Copy, Debug)]
pub struct CachedParams {
    /// Multiplier on the baseline absorption. Stands for the product of D-region
    /// electron density and collision frequency; see [`absorption_scale_as_nu`].
    pub absorption_scale: Bounded,
    pub atm: AtmosphericAnchors,
}

impl CachedParams {
    /// The prior: absorption exactly as solved, noise anchors at the module
    /// constants.
    ///
    /// The absorption bound follows from `NU_REF_PER_S`'s own plausible range of
    /// 1e6 to 2e7 s^-1 against its 5e6 prior, i.e. 0.2x to 4x.
    #[must_use]
    pub fn prior() -> Self {
        let nu = Anchors::default().ionosphere.nu_ref_per_s;
        Self {
            absorption_scale: Bounded::new(1.0, nu.min / nu.value, nu.max / nu.value),
            atm: AtmosphericAnchors::default(),
        }
    }

    /// The parameters a search steps, on a common 0..1 scale.
    ///
    /// The absorption scale is not here: the prediction is linear in it, so it
    /// is solved in closed form at every trial point
    /// ([`best_absorption_scale`]). Stepping it alongside the correlated noise
    /// slopes made the descent stall (1.76 instead of a planted 1.80).
    ///
    /// The seasonal swing and both latitude terms are also left out. Latitude
    /// terms are constant per receiver and absorbed into its effect, and a
    /// one-month corpus has one season, so the search could move them without
    /// changing the objective.
    #[must_use]
    #[allow(clippy::type_complexity)]
    pub fn fields() -> Vec<(&'static str, fn(&Self) -> Bounded, fn(&mut Self, Bounded))> {
        vec![
            (
                "atm Fa 1 MHz day [dB]",
                |p: &Self| p.atm.f1_day_db,
                |p: &mut Self, v| p.atm.f1_day_db = v,
            ),
            (
                "atm slope day [dB/dec]",
                |p: &Self| p.atm.slope_day_db,
                |p: &mut Self, v| p.atm.slope_day_db = v,
            ),
            // The three that carry the day/night contrast. A WSPR corpus can see
            // the difference across the terminator, while the floor level is
            // absorbed into each station's effect.
            (
                "atm day->night step [dB]",
                |p: &Self| p.atm.step_1mhz_db,
                |p: &mut Self, v| p.atm.step_1mhz_db = v,
            ),
            (
                "atm step slope [dB/dec]",
                |p: &Self| p.atm.step_slope_db,
                |p: &mut Self, v| p.atm.step_slope_db = v,
            ),
            (
                "atm step curve [dB/dec2]",
                |p: &Self| p.atm.step_curve_db,
                |p: &mut Self, v| p.atm.step_curve_db = v,
            ),
        ]
    }
}

/// The non-decodes the fit is allowed to be constrained by, as a one-sided term.
///
/// # Why the positives alone cannot identify the level
///
/// Every positive residual is `modelled - measured - station - global`, and
/// `global` is re-solved at every trial point, so a constant shift of the model
/// costs nothing. The positives do not identify the level, and the fit was
/// observed closing residuals by sliding the model optimistic.
///
/// A negative says a receiver that was hearing other stations did not hear
/// this one on this path at this hour, i.e. the modelled SNR should have been
/// below the decode threshold. No constant shift satisfies that, so it removes
/// the degeneracy.
///
/// # Why the term is a hinge
///
/// The measured false-positive rate is an upper bound: a negative may be an
/// open path that collided or arrived off the back of a beam. A negative says
/// nothing about how far below the threshold the signal was, so the hinge
/// `max(0, modelled - threshold)^2` charges only the excess.
#[derive(Clone, Copy)]
pub struct Negatives<'a> {
    /// Solved non-decodes. `measured_db` is meaningless on these and is never
    /// read; the threshold stands in for it.
    pub spots: &'a [Cached],
    /// Decode threshold the hinge is taken about, dB.
    pub threshold_db: f64,
    /// Weight of one negative against one positive in the sum of squares.
    pub weight: f64,
}

impl<'a> Negatives<'a> {
    /// No constraint: the positives-only objective.
    #[must_use]
    pub fn none() -> Self {
        Self {
            spots: &[],
            threshold_db: 0.0,
            weight: 0.0,
        }
    }

    /// Negatives weighted so their total weight equals the positives' total.
    ///
    /// The number of negatives scored is a sampling choice, so equal totals keep
    /// that choice from setting how much the one-sided term counts.
    #[must_use]
    pub fn balanced(spots: &'a [Cached], threshold_db: f64, n_positives: usize) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let weight = if spots.is_empty() {
            0.0
        } else {
            n_positives as f64 / spots.len() as f64
        };
        Self {
            spots,
            threshold_db,
            weight,
        }
    }

    /// This negative's predicted SNR less the station offsets that apply to it.
    ///
    /// A station absent from the fit's index space contributes 0: the effects
    /// are centred on zero, so 0 is the population estimate for an unknown
    /// station.
    fn excess(&self, c: &Cached, scale: f64, atm: AtmosphericAnchors, e: &StationEffects) -> f64 {
        let station =
            e.tx.get(c.tx).copied().unwrap_or(0.0) + e.rx.get(c.rx).copied().unwrap_or(0.0);
        c.modelled_db(scale, atm) - station - self.threshold_db
    }

    /// The hinge penalty at a trial point, given the global offset.
    #[must_use]
    pub fn penalty(
        &self,
        scale: f64,
        atm: AtmosphericAnchors,
        e: &StationEffects,
        global: f64,
    ) -> f64 {
        self.spots
            .iter()
            .map(|c| {
                let over = self.excess(c, scale, atm, e) - global;
                if over > 0.0 {
                    self.weight * over * over
                } else {
                    0.0
                }
            })
            .sum()
    }
}

/// The absorption scale and the global offset that minimise the residual,
/// solved jointly in closed form.
///
/// # Why the two are solved together
///
/// The prediction is `base_i - s * absorption_i`, linear in `s`, and the
/// global offset `g` is a constant. Solving them alternately converges only
/// geometrically, at rate
///
/// ```text
///   k = mean(a)^2 / (mean(a)^2 + var(a))
/// ```
///
/// the fraction of absorption's mean square explained by its mean. On a
/// planted case with `k = 0.83`, a scale of 1.80 still read 1.77 after 20
/// rounds. The scale is identified by how much absorption varies across the
/// corpus, not by its mean, since a constant absorption looks like the global
/// offset. Solving the 2x2 system converges in a couple of rounds.
///
/// The bool reports whether the scale was clamped to its range; a scale
/// outside every published collision frequency is a finding about the model.
///
/// # How the one-sided negatives enter
///
/// A violating negative contributes `w (u_i - g - s a_i)^2` with
/// `u_i = base_i - threshold - station_i`, the same as a positive measured at
/// the threshold with weight `w`. The active set depends on `(g, s)`, but the
/// objective is convex in `(g, s)`, so alternating between choosing the active
/// set and solving the weighted 2x2 reaches the global minimum without
/// cycling, usually in a handful of passes.
#[must_use]
pub fn best_absorption_scale(
    spots: &[Cached],
    atm: AtmosphericAnchors,
    effects: &StationEffects,
    prior: Bounded,
    negatives: Negatives<'_>,
) -> (Bounded, f64, bool) {
    // Regress u_i on (1, a_i): u_i = g + s * a_i, in the least-squares sense.
    // The prediction subtracts s * a_i, so `u` is formed to make `s` positive
    // for a model that needs more absorption. The positives' contribution does
    // not depend on the active set, so it is accumulated once.
    let mut base_n = 0.0;
    let mut base_a = 0.0;
    let mut base_aa = 0.0;
    let mut base_u = 0.0;
    let mut base_ua = 0.0;
    for c in spots {
        // `base` is the prediction with absorption switched off.
        let base = c.modelled_db(0.0, atm);
        let station = effects.tx[c.tx] + effects.rx[c.rx];
        let u = base - c.measured_db - station;
        base_n += 1.0;
        base_a += c.absorption_db;
        base_aa += c.absorption_db * c.absorption_db;
        base_u += u;
        base_ua += u * c.absorption_db;
    }
    if base_n < 3.0 {
        return (prior, effects.global_db, false);
    }

    // The pseudo-observation a violating negative contributes.
    let neg_u = |c: &Cached| -> f64 {
        let station = effects.tx.get(c.tx).copied().unwrap_or(0.0)
            + effects.rx.get(c.rx).copied().unwrap_or(0.0);
        c.modelled_db(0.0, atm) - negatives.threshold_db - station
    };

    let solve = |active: &[usize]| -> Option<(f64, f64)> {
        let (mut n, mut sa, mut saa, mut su, mut sua) = (base_n, base_a, base_aa, base_u, base_ua);
        for &i in active {
            let c = &negatives.spots[i];
            let w = negatives.weight;
            let a = c.absorption_db;
            let u = neg_u(c);
            n += w;
            sa += w * a;
            saa += w * a * a;
            su += w * u;
            sua += w * u * a;
        }
        let det = n * saa - sa * sa;
        if det.abs() < 1e-12 {
            // No absorption variation: the scale is not identified, so leave it
            // at the prior and put everything into the offset.
            return None;
        }
        Some(((su * saa - sa * sua) / det, (n * sua - sa * su) / det))
    };

    // Seed the active set at the prior scale. When the positives alone are
    // singular in the scale direction, only the negatives carry information,
    // and an empty active set would give up before using them. The seed offset
    // must match the prior scale: `base_u / base_n` fits a scale of zero and
    // would misclassify the negatives.
    let mut s = prior.value;
    let mut g = (base_u - s * base_a) / base_n;
    let violating = |g: f64, s: f64| -> Vec<usize> {
        negatives
            .spots
            .iter()
            .enumerate()
            .filter(|(_, c)| neg_u(c) - g - s * c.absorption_db > 0.0)
            .map(|(i, _)| i)
            .collect()
    };
    let mut active = violating(g, s);
    // The active set can only change a bounded number of times on a convex
    // piecewise-quadratic, so this cap is for floating-point ties on the
    // threshold rather than for any real risk of cycling.
    for _ in 0..64 {
        // Singular even with the active negatives: the scale is not
        // identified, so leave it at the prior and let the offset carry it.
        let Some((gi, si)) = solve(&active) else {
            break;
        };
        g = gi;
        s = si;
        let next = violating(g, s);
        if next == active {
            break;
        }
        active = next;
    }

    let (scale, hit) = prior.clamped(s);
    // With the scale clamped, re-solve the offset for it so the pair describes
    // one fit. The active set stays at the unclamped solution.
    let g = if hit {
        let (mut n, mut su, mut sa) = (base_n, base_u, base_a);
        for &i in &active {
            let c = &negatives.spots[i];
            n += negatives.weight;
            su += negatives.weight * neg_u(c);
            sa += negatives.weight * c.absorption_db;
        }
        (su - scale.value * sa) / n
    } else {
        g
    };
    (scale, g, hit)
}

/// The sum of squares with the absorption scale fixed and only the global
/// offset solved for it.
///
/// Used to draw the objective along the absorption scale, which
/// [`best_absorption_scale`] otherwise solves in closed form. The offset is
/// re-solved at every scale so the profile measures the scale alone.
#[must_use]
pub fn objective_at_scale(
    spots: &[Cached],
    scale: f64,
    atm: AtmosphericAnchors,
    effects: &StationEffects,
    negatives: Negatives<'_>,
) -> f64 {
    // The offset that minimises the same convex objective at this fixed scale,
    // by the same active-set argument `best_absorption_scale` documents.
    let residual = |c: &Cached, target: f64| {
        let station = effects.tx.get(c.tx).copied().unwrap_or(0.0)
            + effects.rx.get(c.rx).copied().unwrap_or(0.0);
        c.modelled_db(scale, atm) - target - station
    };
    let mut global = 0.0;
    for _ in 0..64 {
        let mut sum = 0.0;
        let mut weight = 0.0;
        for c in spots {
            sum += residual(c, c.measured_db);
            weight += 1.0;
        }
        for c in negatives.spots {
            if residual(c, negatives.threshold_db) - global > 0.0 {
                sum += negatives.weight * residual(c, negatives.threshold_db);
                weight += negatives.weight;
            }
        }
        let next = if weight > 0.0 { sum / weight } else { 0.0 };
        if (next - global).abs() < 1e-12 {
            break;
        }
        global = next;
    }
    let mut total = 0.0;
    for c in spots {
        let d = residual(c, c.measured_db) - global;
        total += d * d;
    }
    total + negatives.penalty(scale, atm, effects, global)
}

/// The objective with the absorption scale and the global offset profiled out,
/// exactly as the fit's inner loop does it.
///
/// Public so a report can draw the objective along each parameter. A parameter
/// on a bound may sit where the objective keeps falling, or where it went flat
/// and the descent drifted; only the curve tells these apart.
#[must_use]
pub fn profiled_objective(
    spots: &[Cached],
    atm: AtmosphericAnchors,
    effects: &StationEffects,
    prior_scale: Bounded,
    negatives: Negatives<'_>,
) -> (f64, Bounded, f64) {
    let (scale, global, _hit) = best_absorption_scale(spots, atm, effects, prior_scale, negatives);
    let mut sum = 0.0;
    for c in spots {
        let station = effects.tx[c.tx] + effects.rx[c.rx];
        let d = c.modelled_db(scale.value, atm) - c.measured_db - station - global;
        sum += d * d;
    }
    sum += negatives.penalty(scale.value, atm, effects, global);
    (sum, scale, global)
}

/// Fit the cached parameters and the station effects together.
///
/// Alternating: solve the station effects in closed form for the current
/// parameters, then improve the parameters by coordinate descent with the effects
/// held fixed, and repeat. Both halves reduce the same sum of squares, so the
/// objective is monotone and the loop cannot cycle.
///
/// Coordinate descent suits a cheap, five-dimensional, bounded objective, and
/// a step that wants to leave its bound is a finding in itself;
/// `report_bound_hits` lists them.
#[must_use]
pub fn fit_cached(
    spots: &[Cached],
    n_tx: usize,
    n_rx: usize,
    rounds: usize,
    negatives: Negatives<'_>,
) -> (CachedParams, StationEffects, Vec<String>) {
    let mut params = CachedParams::prior();
    let mut effects = StationEffects::default();
    let mut notes = Vec::new();
    if spots.is_empty() {
        return (params, effects, notes);
    }

    let prior_scale = CachedParams::prior().absorption_scale;
    // The profiled objective: the absorption scale is solved in closed form at
    // every trial set of noise anchors, so the search walks only the noise
    // surface. Returns the objective and the scale that produced it.
    //
    // The hinge over the negatives is part of this objective, so a trial point
    // that helps the positives by sliding the model optimistic pays for the
    // non-decodes it would then claim.
    let profiled = |atm: AtmosphericAnchors, e: &StationEffects| -> (f64, Bounded, f64) {
        profiled_objective(spots, atm, e, prior_scale, negatives)
    };

    for round in 0..rounds {
        // Station effects, in closed form, for the current physics.
        let residuals: Vec<f64> = spots
            .iter()
            .map(|c| c.modelled_db(params.absorption_scale.value, params.atm) - c.measured_db)
            .collect();
        effects = StationEffects::solve(&residuals, spots, n_tx, n_rx);
        // Re-profile the scale and the global offset together against the
        // effects just solved (see `best_absorption_scale`).
        let (_, scale, global) = profiled(params.atm, &effects);
        params.absorption_scale = scale;
        effects.global_db = global;

        // Parameters, by coordinate descent on the unit scale.
        //
        // Each step size is swept to convergence. The absorption scale and the
        // noise frequency slopes are correlated, so one sweep per size zigzags
        // and stalls (1.69 of a planted 1.80).
        let mut improved = false;
        let fields = CachedParams::fields();
        // Coarse to fine: 1/8 of each range down to 1/4096, which resolves the
        // absorption scale to about 0.1 % and the noise anchors to a hundredth of
        // a dB, finer than the data supports.
        let mut step = 0.125;
        while step > 1.0 / 4096.0 {
            // Cap the sweeps so a pathological objective cannot spin here; 200 is
            // far past convergence for four bounded parameters.
            for _ in 0..200 {
                let before: Vec<f64> = fields
                    .iter()
                    .map(|(_, get, _)| get(&params).unit_position())
                    .collect();
                let mut moved = false;
                for (_, get, set) in &fields {
                    let current = get(&params);
                    let (base, base_scale, base_global) = profiled(params.atm, &effects);
                    let mut best = (base, current, base_scale, base_global);
                    for delta in [step, -step] {
                        let mut trial = params;
                        set(
                            &mut trial,
                            current.at_unit_position(current.unit_position() + delta),
                        );
                        let (value, scale, global) = profiled(trial.atm, &effects);
                        if value < best.0 {
                            best = (value, get(&trial), scale, global);
                        }
                    }
                    // Relative improvement floor, so rounding noise does not keep
                    // the sweep going.
                    if best.0 < base * (1.0 - 1e-12) {
                        set(&mut params, best.1);
                        params.absorption_scale = best.2;
                        effects.global_db = best.3;
                        moved = true;
                        improved = true;
                    }
                }
                if !moved {
                    break;
                }
                // Pattern move (Hooke-Jeeves). Raising the noise frequency slope
                // and lowering the absorption scale change the prediction in
                // nearly the same way, a diagonal valley that axis-only descent
                // cannot follow. After each productive sweep, keep moving in the
                // sweep's direction while it helps.
                let after: Vec<f64> = fields
                    .iter()
                    .map(|(_, get, _)| get(&params).unit_position())
                    .collect();
                let direction: Vec<f64> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
                if direction.iter().all(|d| d.abs() < 1e-15) {
                    continue;
                }
                let mut reach = 1.0;
                loop {
                    let mut trial = params;
                    for ((_, get, set), (a, d)) in fields.iter().zip(after.iter().zip(&direction)) {
                        let b = get(&trial);
                        set(&mut trial, b.at_unit_position(a + reach * d));
                    }
                    let (value, scale, global) = profiled(trial.atm, &effects);
                    let (current, _, _) = profiled(params.atm, &effects);
                    if value < current * (1.0 - 1e-12) {
                        params = trial;
                        params.absorption_scale = scale;
                        effects.global_db = global;
                        reach *= 2.0;
                        if reach > 1024.0 {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            step *= 0.5;
        }
        if !improved && round > 0 {
            notes.push(format!(
                "coordinate descent stopped moving after {} round(s)",
                round + 1
            ));
            break;
        }
    }

    // One final station-effect solve, so the returned effects match the returned
    // parameters rather than the previous iteration's.
    let residuals: Vec<f64> = spots
        .iter()
        .map(|c| c.modelled_db(params.absorption_scale.value, params.atm) - c.measured_db)
        .collect();
    effects = StationEffects::solve(&residuals, spots, n_tx, n_rx);
    let (scale, global, hit_bound) =
        best_absorption_scale(spots, params.atm, &effects, prior_scale, negatives);
    params.absorption_scale = scale;
    effects.global_db = global;
    if hit_bound {
        notes.push(format!(
            "the absorption scale wanted to leave its range and was clamped to {:.4}              (range {:.4} to {:.4}, i.e. nu between {:.2e} and {:.2e} /s). The bound was NOT              widened: absorption the data wants outside every published collision frequency              means the error is somewhere else in the model.",
            scale.value,
            scale.min,
            scale.max,
            scale.min * Anchors::default().ionosphere.nu_ref_per_s.value,
            scale.max * Anchors::default().ionosphere.nu_ref_per_s.value,
        ));
    }
    notes.extend(report_bound_hits(&params));
    (params, effects, notes)
}

/// Which fitted parameters ended up sitting on a bound.
///
/// A bound hit means the residual the fit was chasing does not come from the
/// parameter it was pushing, so the error lives elsewhere in the model.
#[must_use]
pub fn report_bound_hits(params: &CachedParams) -> Vec<String> {
    let mut out = Vec::new();
    for (name, get, _) in CachedParams::fields() {
        let b = get(params);
        if b.at_bound() {
            out.push(format!(
                "{name} hit its bound at {:.4} (range {:.4} to {:.4}). The bound was NOT \
                 widened: the data wanting to leave a physically defensible range means the \
                 residual comes from somewhere else in the model.",
                b.value, b.min, b.max
            ));
        }
    }
    out
}

/// A per-cut breakdown of the fit, for the report.
#[derive(Clone, Debug)]
pub struct Cut {
    pub label: String,
    pub fit: Fit,
}

/// Group spots by a key and score each group.
#[must_use]
pub fn cuts_by(
    spots: &[Cached],
    absorption_scale: f64,
    atm: AtmosphericAnchors,
    effects: &StationEffects,
    key: impl Fn(&Cached) -> String,
) -> Vec<Cut> {
    let mut groups: BTreeMap<String, Vec<Cached>> = BTreeMap::new();
    for c in spots {
        groups.entry(key(c)).or_default().push(c.clone());
    }
    groups
        .into_iter()
        .map(|(label, group)| Cut {
            label,
            fit: Fit::of(&group, absorption_scale, atm, effects),
        })
        .collect()
}

/// How a set of negatives scored: paths the model claimed that did not happen.
///
/// The false-positive rate here is an upper bound, because the negatives set
/// includes some paths that were open but collided or arrived off the
/// back of a directional antenna. See [`crate::corpus`].
#[derive(Clone, Debug, Default)]
pub struct NegativeScore {
    pub n: usize,
    /// Negatives for which the model found a geometric path at all.
    pub path_found: usize,
    /// Negatives the model predicted would decode (above the WSPR threshold):
    /// claimed openings that were not there.
    pub predicted_decodable: usize,
    /// Of those, how many needed the probabilistic Es sheet.
    pub via_es: usize,
    /// Predicted SNR margin over the decode threshold, over the negatives where
    /// a path was found. A model whose false positives sit just over the
    /// threshold is failing differently from one that claims +20 dB.
    pub margin: Spread,
}

impl NegativeScore {
    /// Fraction of attempted-and-failed paths the model would have called
    /// decodable, 0..1.
    #[must_use]
    pub fn false_positive_rate(&self) -> f64 {
        if self.n == 0 {
            return f64::NAN;
        }
        #[allow(clippy::cast_precision_loss)]
        let r = self.predicted_decodable as f64 / self.n as f64;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{dbm_from_watts, noise_power_dbm};

    pub(super) fn mk(tx: usize, rx: usize, measured: f64, absorption: f64, freq: f64) -> Cached {
        Cached {
            tx,
            rx,
            measured_db: measured,
            tx_power_dbm: 23.0,
            loss_without_absorption_db: 100.0,
            absorption_db: absorption,
            freq_mhz: freq,
            bandwidth_hz: 2500.0,
            rx_is_day: true,
            rx_season: Season::Summer,
            rx_lat: 45.0,
            noise_env: NoiseEnvironment::Rural,
            layer: "F2",
            hops: 1,
            range_km: 1000.0,
            probability: 1.0,
            date: (2026, 7, 3),
            midpoint_zenith_deg: 30.0,
            alternative: None,
        }
    }

    /// An alternative scored under the same conditions as its winner must differ
    /// only by its own two loss terms, since the noise floor belongs to the spot.
    #[test]
    fn the_alternative_is_scored_against_the_same_noise() {
        let atm = AtmosphericAnchors::default();
        let mut c = mk(0, 0, -15.0, 8.0, 14.097);
        c.alternative = Some(Alternative {
            loss_without_absorption_db: 112.0,
            absorption_db: 20.0,
        });
        let winner = c.modelled_db(1.0, atm);
        let alt = c.alternative_modelled_db(1.0, atm).expect("alternative");
        // The winner has 100 dB of loss and 8 dB of absorption; the alternative
        // 112 and 20. The gap is exactly the difference in those two terms.
        assert!((winner - alt - ((112.0 - 100.0) + (20.0 - 8.0))).abs() < 1e-9);
        // And the absorption scale reaches the alternative too, linearly.
        let alt_two = c.alternative_modelled_db(2.0, atm).expect("alternative");
        assert!((alt - alt_two - 20.0).abs() < 1e-9);
        assert!(c.layer_was_a_race());
        assert!(
            mk(0, 0, 0.0, 0.0, 7.0)
                .alternative_modelled_db(1.0, atm)
                .is_none()
        );
    }

    /// The day/night cut uses the midpoint, with 90 degrees as the terminator.
    #[test]
    fn night_is_decided_at_the_midpoint() {
        let mut c = mk(0, 0, -15.0, 8.0, 14.097);
        // The receiver is in daylight throughout; only the midpoint moves.
        assert!(c.rx_is_day);
        c.midpoint_zenith_deg = 89.9;
        assert!(!c.midpoint_is_night());
        c.midpoint_zenith_deg = 90.1;
        assert!(
            c.midpoint_is_night(),
            "a dark midpoint is night whatever the receiver sees"
        );
    }

    /// The prior must reproduce the solve: absorption scale 1 and the default
    /// noise anchors have to give back exactly the SNR the link budget produced.
    #[test]
    fn the_prior_reproduces_the_link_budget() {
        let c = mk(0, 0, -15.0, 8.0, 14.097);
        let atm = AtmosphericAnchors::default();
        let expected =
            dbm_from_watts(10.0_f64.powf((23.0 - 30.0) / 10.0)) - 100.0 - 8.0 - c.noise_dbm(atm);
        assert!(
            (c.modelled_db(1.0, atm) - expected).abs() < 1e-9,
            "{} vs {expected}",
            c.modelled_db(1.0, atm)
        );
        // The noise floor is the one the module composes, not a copy.
        let direct = noise_power_dbm(
            NoiseFloor::compute(
                14.097,
                2500.0,
                NoiseEnvironment::Rural,
                true,
                Season::Summer,
                45.0,
                atm,
            )
            .total_fa_db,
            2500.0,
        );
        assert!((c.noise_dbm(atm) - direct).abs() < 1e-12);
    }

    /// Doubling the absorption scale must cost exactly the baseline absorption in
    /// dB, the linearity the cache relies on.
    #[test]
    fn the_absorption_scale_is_linear() {
        let c = mk(0, 0, -15.0, 8.0, 14.097);
        let atm = AtmosphericAnchors::default();
        let one = c.modelled_db(1.0, atm);
        let two = c.modelled_db(2.0, atm);
        assert!((one - two - 8.0).abs() < 1e-9, "{one} vs {two}");
        // And a spot with no absorption is untouched by it.
        let clear = mk(0, 0, -15.0, 0.0, 14.097);
        assert!((clear.modelled_db(1.0, atm) - clear.modelled_db(4.0, atm)).abs() < 1e-12);
    }

    /// Station effects must recover offsets that were put in by hand, and must
    /// leave the gauge where the documentation says: both sets mean zero, with
    /// the part they cannot separate in `global_db`.
    #[test]
    fn station_effects_recover_planted_offsets() {
        // Three transmitters, three receivers, fully crossed so every effect is
        // identified. Plant known offsets plus a global bias.
        let tx_true = [3.0, -1.0, -2.0]; // mean zero
        let rx_true = [5.0, 0.0, -5.0]; // mean zero
        let global = 7.0;
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        for (t, tx_effect) in tx_true.iter().enumerate() {
            for (r, rx_effect) in rx_true.iter().enumerate() {
                spots.push(mk(t, r, -15.0, 5.0, 14.097));
                residuals.push(global + tx_effect + rx_effect);
            }
        }
        let e = StationEffects::solve(&residuals, &spots, 3, 3);
        assert!(
            (e.global_db - global).abs() < 1e-6,
            "global {}",
            e.global_db
        );
        for (t, want) in tx_true.iter().enumerate() {
            assert!((e.tx[t] - want).abs() < 1e-6, "tx{t} = {}", e.tx[t]);
        }
        for (r, want) in rx_true.iter().enumerate() {
            assert!((e.rx[r] - want).abs() < 1e-6, "rx{r} = {}", e.rx[r]);
        }
        // The gauge: both sets average zero.
        assert!(e.tx.iter().sum::<f64>().abs() < 1e-9);
        assert!(e.rx.iter().sum::<f64>().abs() < 1e-9);
        // And the reconstruction is exact.
        for (c, r) in spots.iter().zip(&residuals) {
            assert!((e.offset_for(c) - r).abs() < 1e-6);
        }
    }

    /// Shifting a constant from the transmitters to the receivers changes no
    /// prediction, so the solver must return the centred representative.
    #[test]
    fn only_the_sum_of_the_two_effects_is_identified() {
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        for t in 0..2 {
            for r in 0..2 {
                spots.push(mk(t, r, -15.0, 5.0, 14.097));
                // A planted set that is not centred: tx offset by +10 throughout.
                #[allow(clippy::cast_precision_loss)]
                let planted = 10.0 + t as f64 - r as f64;
                residuals.push(planted);
            }
        }
        let e = StationEffects::solve(&residuals, &spots, 2, 2);
        // The +10 could not be attributed to either end, so it must have landed
        // in the global term.
        assert!(e.tx.iter().sum::<f64>().abs() < 1e-9);
        assert!(e.rx.iter().sum::<f64>().abs() < 1e-9);
        assert!((e.global_db - 10.0).abs() < 1e-6, "{}", e.global_db);
    }

    /// A station seen a handful of times must be excluded from the reported
    /// distribution, since its "effect" is just its own residual.
    #[test]
    fn the_distribution_excludes_barely_seen_stations() {
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        // tx 0 appears 12 times, tx 1 appears twice.
        for i in 0..12 {
            spots.push(mk(0, i % 3, -15.0, 5.0, 14.097));
            residuals.push(2.0);
        }
        for i in 0..2 {
            spots.push(mk(1, i, -15.0, 5.0, 14.097));
            residuals.push(20.0);
        }
        let e = StationEffects::solve(&residuals, &spots, 2, 3);
        let d = e.distribution(10);
        assert_eq!(d.tx.n, 1, "only the well-observed transmitter qualifies");
        assert_eq!(e.tx_counts, vec![12, 2]);
    }

    /// A perfect model scores a slope well below 1 against raw measured SNR
    /// because the measurement carries station effects, and 1 once they are
    /// removed. That is why the headline number is the adjusted slope.
    #[test]
    fn a_perfect_model_still_scores_a_low_raw_slope() {
        let atm = AtmosphericAnchors::default();
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        // 20 transmitters x 5 receivers. The physics varies through absorption;
        // the station effects are twice as variable, as in the WSPR population.
        for t in 0..20 {
            for r in 0..5 {
                #[allow(clippy::cast_precision_loss)]
                let absorption = 2.0 + 0.7 * (t % 10) as f64;
                let mut c = mk(t, r, 0.0, absorption, 14.097);
                let station = 8.0 * ((t % 7) as f64 - 3.0) + 5.0 * ((r % 5) as f64 - 2.0);
                // A perfect model: measured is exactly the model plus the
                // station effect (with the sign the fit expects).
                c.measured_db = c.modelled_db(1.0, atm) - station;
                spots.push(c);
                residuals.push(station);
            }
        }
        let effects = StationEffects::solve(&residuals, &spots, 20, 5);
        let fit = Fit::of(&spots, 1.0, atm, &effects);
        assert!(
            fit.slope_raw < 0.7,
            "raw slope should be attenuated, got {}",
            fit.slope_raw
        );
        assert!(
            (fit.slope_adjusted - 1.0).abs() < 1e-6,
            "adjusted slope should be 1 for a perfect model, got {}",
            fit.slope_adjusted
        );
        assert!(fit.rms_db < 1e-6, "a perfect model has no residual left");
    }

    /// The fit must recover a planted absorption error instead of pushing it
    /// into the station effects.
    ///
    /// The physics is identified only from variation within a station, so each
    /// TX->RX pair here is observed on four bands at three absorption levels.
    /// The same condition is why the corpus schedule walks eight UTC hours and
    /// ten bands.
    #[test]
    fn the_fit_recovers_a_planted_absorption_error() {
        let atm = AtmosphericAnchors::default();
        let truth = 1.8;
        let mut spots = Vec::new();
        for t in 0..12 {
            for r in 0..5 {
                for freq in [3.568, 7.038, 14.095, 21.094] {
                    for level in 0..3 {
                        // Day/night is crossed with the absorption level. If the
                        // two were tied, the noise model's day-night anchors could
                        // absorb part of the absorption error.
                        for is_day in [false, true] {
                            #[allow(clippy::cast_precision_loss)]
                            let absorption = 2.0 + 4.0 * level as f64 + 3.0 / (freq / 3.568);
                            let mut c = mk(t, r, 0.0, absorption, freq);
                            c.rx_is_day = is_day;
                            #[allow(clippy::cast_precision_loss)]
                            let station = 6.0 * ((t % 5) as f64 - 2.0) + 4.0 * (r as f64 - 2.0);
                            c.measured_db = c.modelled_db(truth, atm) - station;
                            spots.push(c);
                        }
                    }
                }
            }
        }
        let (params, effects, _notes) = fit_cached(&spots, 12, 5, 12, Negatives::none());
        assert!(
            (params.absorption_scale.value - truth).abs() < 0.05,
            "planted {truth}, recovered {}",
            params.absorption_scale.value
        );
        let fit = Fit::of(&spots, params.absorption_scale.value, params.atm, &effects);
        assert!(fit.rms_db < 0.5, "residual {} dB", fit.rms_db);
        assert!(
            (fit.slope_adjusted - 1.0).abs() < 0.05,
            "adjusted slope {}",
            fit.slope_adjusted
        );
    }

    /// When the planted physics error is collinear with a station, the fit must
    /// leave the parameter at its prior instead of reporting a wrong value.
    #[test]
    fn physics_collinear_with_a_station_stays_unidentified() {
        let atm = AtmosphericAnchors::default();
        let mut spots = Vec::new();
        for t in 0..25 {
            for r in 0..6 {
                // Absorption is a pure function of the transmitter, so the TX
                // fixed effect explains it exactly.
                #[allow(clippy::cast_precision_loss)]
                let absorption = 1.0 + 1.3 * t as f64;
                let mut c = mk(t, r, 0.0, absorption, 14.095);
                c.measured_db = c.modelled_db(1.8, atm);
                spots.push(c);
            }
        }
        let (params, effects, _notes) = fit_cached(&spots, 25, 6, 12, Negatives::none());
        assert!(
            (params.absorption_scale.value - 1.0).abs() < 1e-9,
            "the scale must stay at its prior, got {}",
            params.absorption_scale.value
        );
        // ...and the station effects absorb it, leaving no residual. A tiny
        // residual with an unmoved parameter marks an unidentified quantity.
        let fit = Fit::of(&spots, params.absorption_scale.value, params.atm, &effects);
        assert!(fit.rms_db < 1e-6, "residual {} dB", fit.rms_db);
    }

    /// With constant absorption the positives cannot identify the scale, and
    /// the fit leaves it at the prior. A non-decode constrains the absolute SNR,
    /// and one with different absorption supplies the missing variation.
    #[test]
    fn negatives_identify_a_scale_the_positives_cannot() {
        let atm = AtmosphericAnchors::default();
        let truth = 3.0;
        // Absorption is exactly constant across the positives, so the normal
        // equations are singular in the scale direction and only `global` moves.
        let flat_absorption = 10.0;
        let mut spots = Vec::new();
        for t in 0..8 {
            for r in 0..4 {
                for _ in 0..3 {
                    let mut c = mk(t, r, 0.0, flat_absorption, 14.095);
                    c.measured_db = c.modelled_db(truth, atm) + 18.0;
                    spots.push(c);
                }
            }
        }
        let (alone, _, _) = fit_cached(&spots, 8, 4, 12, Negatives::none());
        assert!(
            (alone.absorption_scale.value - 1.0).abs() < 1e-9,
            "with no variation the scale is unidentified and must stay at its prior, got {}",
            alone.absorption_scale.value
        );

        // A non-decode on a heavily absorbed path. At the prior scale the model
        // claims it would have been heard; at the truth it is far under.
        let mut neg = mk(0, 0, f64::NAN, 25.0, 14.095);
        neg.tx = 0;
        neg.rx = 0;
        let threshold = neg.modelled_db(1.0, atm) - 10.0;
        assert!(
            neg.modelled_db(truth, atm) < threshold - 20.0,
            "the planted negative must be comfortably inaudible under the truth"
        );
        let negs: Vec<Cached> = (0..8)
            .flat_map(|t| {
                (0..4).map(move |r| {
                    let mut c = mk(t, r, f64::NAN, 25.0, 14.095);
                    c.tx = t;
                    c.rx = r;
                    c
                })
            })
            .collect();

        let (constrained, _, _) = fit_cached(
            &spots,
            8,
            4,
            12,
            Negatives::balanced(&negs, threshold, spots.len()),
        );
        assert!(
            constrained.absorption_scale.value > 1.5,
            "the one-sided term must identify a scale the positives could not, got {}",
            constrained.absorption_scale.value
        );
    }

    /// The hinge must be one-sided: a negative already below the threshold
    /// costs nothing.
    #[test]
    fn a_negative_already_below_threshold_costs_nothing() {
        let atm = AtmosphericAnchors::default();
        let effects = StationEffects {
            tx: vec![0.0],
            rx: vec![0.0],
            global_db: 0.0,
            tx_counts: vec![1],
            rx_counts: vec![1],
        };
        let quiet = mk(0, 0, f64::NAN, 60.0, 7.038);
        let threshold = quiet.modelled_db(1.0, atm) + 5.0;
        let n = Negatives::balanced(std::slice::from_ref(&quiet), threshold, 10);
        assert!(
            n.penalty(1.0, atm, &effects, 0.0).abs() < 1e-12,
            "a comfortably-inaudible negative must be free"
        );
        // Move the threshold below the prediction and it starts costing.
        let loud = Negatives::balanced(std::slice::from_ref(&quiet), threshold - 15.0, 10);
        assert!(loud.penalty(1.0, atm, &effects, 0.0) > 0.0);
    }

    /// With no negatives the objective must equal the positives-only one bit
    /// for bit.
    #[test]
    fn no_negatives_reproduces_the_positives_only_solve() {
        let atm = AtmosphericAnchors::default();
        let spots: Vec<Cached> = (0..30)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let a = 3.0 + (i % 5) as f64;
                let mut c = mk(i % 3, i % 2, 0.0, a, 7.038);
                c.measured_db = c.modelled_db(1.4, atm) - 2.0;
                c
            })
            .collect();
        let e = StationEffects::solve(&vec![0.0; spots.len()], &spots, 3, 2);
        let prior = CachedParams::prior().absorption_scale;
        let (s, g, _) = best_absorption_scale(&spots, atm, &e, prior, Negatives::none());
        // Solved by hand from the same normal equations, with no hinge involved.
        assert!(s.value.is_finite() && g.is_finite());
        assert!((s.value - 1.4).abs() < 1e-6, "scale {}", s.value);
    }

    /// A regression against a constant is undefined and must return NaN.
    #[test]
    fn regression_refuses_a_degenerate_input() {
        let (s, r) = regress(&[1.0, 1.0, 1.0, 1.0], &[1.0, 2.0, 3.0, 4.0]);
        assert!(s.is_nan() && r.is_nan());
        let (s, r) = regress(&[1.0, 2.0], &[1.0, 2.0]);
        assert!(s.is_nan() && r.is_nan(), "two points is not a regression");
        let (s, r) = regress(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
        assert!((s - 2.0).abs() < 1e-12 && (r - 1.0).abs() < 1e-12);
    }

    /// The false-positive rate is taken over all negatives, including those
    /// where no path was found.
    #[test]
    fn false_positive_rate_is_over_every_negative() {
        let s = NegativeScore {
            n: 200,
            path_found: 150,
            predicted_decodable: 40,
            via_es: 12,
            margin: Spread::default(),
        };
        assert!((s.false_positive_rate() - 0.20).abs() < 1e-12);
        assert!(NegativeScore::default().false_positive_rate().is_nan());
    }
}
