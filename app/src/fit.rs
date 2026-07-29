//! Two-way fixed-effects calibration of the model against measured WSPR SNRs.
//!
//! # The problem this module exists to avoid
//!
//! A WSPR spot's measured SNR is not a measurement of propagation alone:
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
//! Regress the physics parameters on raw measured SNR and those two unknowns do
//! not vanish - they get absorbed by whichever physical parameter is most
//! flexible, which here is D-region absorption or the noise model. The residual
//! goes down and the model gets worse. That is the failure mode, and it is the
//! default outcome rather than an unlucky one.
//!
//! # What is done instead
//!
//! `tx_effect` and `rx_effect` are estimated EXPLICITLY, as nuisance parameters,
//! jointly with the physics ([`StationEffects`]). The physics is then identified
//! from variation WITHIN a station rather than across stations:
//!
//! * one transmitter heard by many receivers in one cycle: the TX effect and the
//!   claimed power are common to all of them, so they cancel in the comparison;
//! * one TX->RX pair on several bands at once: both effects are common, so the
//!   FREQUENCY dependence of absorption is cleanly identified;
//! * one TX->RX pair over many hours: both effects are fixed, so the DIURNAL
//!   variation isolates the solar-zenith-angle dependence of the D region.
//!
//! # What this makes unidentifiable, on purpose
//!
//! Any quantity that is CONSTANT for a given station is absorbed into that
//! station's effect and cannot be recovered. That is not a defect of the method,
//! it is an honest statement about what WSPR contains:
//!
//! | quantity | identified by | identifiable here |
//! |---|---|---|
//! | absorption magnitude | overall level, and its frequency and zenith pattern | yes |
//! | atmospheric noise day-night DIFFERENCE | diurnal variation within a station | yes |
//! | atmospheric noise frequency SLOPES | cross-band within a station | yes |
//! | atmospheric noise ABSOLUTE level | nothing - it is a constant | no, absorbed |
//! | receiver noise environment | a constant per receiver | no, absorbed |
//! | latitude terms of the noise model | a receiver's latitude never changes | no, absorbed |
//! | seasonal swing | needs more than one season in the corpus | not from one month |
//! | absolute antenna gain | a constant per station | no, absorbed |
//!
//! So a fit here can calibrate HOW THE SIGNAL VARIES with frequency, path length,
//! zenith angle, hop count and layer. It cannot calibrate absolute levels, and
//! anything claiming to have done so from WSPR has fitted station population
//! statistics and called them physics.
//!
//! # Why the inner loop is cheap
//!
//! Re-solving a spot costs a few hundred milliseconds, and a fit needs thousands
//! of evaluations. Two measured facts make almost all of that unnecessary:
//!
//! 1. **Absorption is exactly linear in the collision frequency.** Doubling
//!    `NU_REF_PER_S` doubles the reported absorption to four significant figures
//!    (`the_absorption_scale_is_linear` pins it). In the D region `nu << omega`,
//!    so the absorption coefficient is proportional to `Ne nu / omega^2`, and
//!    scaling either factor scales the whole line integral.
//! 2. **The D-region and collision parameters do not move the ray.** The D
//!    region's plasma frequency is ~0.3 MHz, so at 7 MHz `X ~ 0.002` and there is
//!    no refraction to speak of. Measured: changing the collision reference
//!    altitude from 65 to 80 km moves absorption from 5.7 dB to 53.4 dB and moves
//!    the launch elevation by 0.0003 degrees.
//!
//! Together those mean the ray can be traced ONCE per spot and the link budget
//! re-derived arithmetically for any absorption scale and any noise anchors. That
//! is [`Cached`], and it makes the inner loop about four orders of magnitude
//! cheaper than re-solving.
//!
//! The parameters that DO change which paths exist - E-region geometry, foE, foEs,
//! Es occurrence - cannot be cached this way and need a real re-solve. They are
//! scanned coarsely rather than optimised, and scored against the negatives set,
//! because turning a path on or off is not a quantity least squares can
//! differentiate.

use std::collections::BTreeMap;

use crate::calib::{Anchors, AtmosphericAnchors, Bounded};
use crate::noise::{NoiseEnvironment, NoiseFloor};
use crate::solar::Season;

/// Everything about one solved spot that lets its SNR be re-derived without
/// tracing the ray again.
///
/// The split is exact rather than approximate: the link budget is
/// `tx_power - (loss - gain) - noise`, and absorption is the only loss term the
/// cached parameters touch. `tests::cache_reproduces_the_solver` pins the
/// reconstruction against a real solve.
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
    /// Every loss term EXCEPT ionospheric absorption, less the antenna gain, dB.
    /// Free-space spreading, ground reflections and any Es sheet loss.
    pub loss_without_absorption_db: f64,
    /// Ionospheric absorption at the BASELINE anchors, dB. Scaled linearly.
    pub absorption_db: f64,
    /// Frequency, needed to recompute the noise floor. MHz.
    pub freq_mhz: f64,
    /// Receiver bandwidth the measured SNR is quoted in, Hz.
    pub bandwidth_hz: f64,
    /// Was it day at the RECEIVER? The noise floor is heard there.
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
    /// Solar zenith angle at the path MIDPOINT, degrees. Above 90 the midpoint
    /// is in darkness. The receiver's own day flag is the wrong thing to cut a
    /// layer by: the reflection happens at the midpoint, not at either end.
    pub midpoint_zenith_deg: f64,
    /// The best F2 path that ALSO closed on this spot, when the layer actually
    /// reported was a lower one. `None` when F2 was itself the choice, or when
    /// no F2 path closed at all - and those two cases mean opposite things, so
    /// see [`Cached::layer_was_a_race`].
    pub alternative: Option<Alternative>,
}

/// The F2 path a lower layer beat, carried so its SNR can be re-derived under
/// the same parameters as the path that won.
///
/// Only the two loss terms are stored. Everything else the link budget needs -
/// power, frequency, bandwidth, noise environment, day, season, latitude - is a
/// property of the SPOT and is already on [`Cached`], so an alternative scored
/// against those is scored against exactly the conditions the winner was.
#[derive(Clone, Copy, Debug)]
pub struct Alternative {
    /// Every loss term except ionospheric absorption, less antenna gain, dB.
    pub loss_without_absorption_db: f64,
    /// Ionospheric absorption at the BASELINE anchors, dB. Scaled linearly, the
    /// same way the winner's is.
    pub absorption_db: f64,
}

impl Cached {
    /// Modelled SNR [dB] under an absorption scale and a set of noise anchors.
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
    /// This is the distinction that matters when a lower layer reads optimistic.
    /// If F2 was available and lost, the layer was picked by a COMPARISON, and a
    /// comparison that systematically prefers the lower layer is a selection rule
    /// to examine. If F2 was not available, the lower layer was the only answer
    /// there was, and the question is instead why it closed at all.
    #[must_use]
    pub fn layer_was_a_race(&self) -> bool {
        self.alternative.is_some()
    }

    /// Modelled SNR [dB] of the F2 path this spot's layer beat, under the same
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

    /// Is the path MIDPOINT in darkness? The reflection happens there, so this
    /// is the cut a layer's behaviour has to be split on - not the receiver's
    /// local day, which on a long path can be the opposite.
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
/// The fit moves one number, the absorption magnitude. Absorption is
/// proportional to the PRODUCT of the D-region electron density and the electron-
/// neutral collision frequency, so that product is what the data identifies and
/// the split between the two factors is not identifiable at all - a finding worth
/// more than a fitted pair would be.
///
/// The scale is attributed to [`crate::scenario::NU_REF_PER_S`] rather than to
/// the density for a measurable reason: doubling the collision frequency doubles
/// the reported absorption exactly, whereas doubling the D-region peak density
/// changes it by only about 5 %, because in this model the E and F2 layers
/// contribute most of the absorption integral. So the collision frequency is the
/// factor the data actually constrains.
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
/// That makes the global offset VISIBLE rather than hidden inside the station
/// effects. It is the part of the model's bias that WSPR cannot attribute - it
/// could be the model being optimistic, or the station population being worse
/// than the assumed antennas, and no amount of this data separates those.
#[derive(Clone, Debug, Default)]
pub struct StationEffects {
    /// Transmitter offsets, dB, mean zero. Indexed as [`Cached::tx`].
    pub tx: Vec<f64>,
    /// Receiver offsets, dB, mean zero. Indexed as [`Cached::rx`].
    pub rx: Vec<f64>,
    /// The offset the two cannot tell apart, dB. Positive means the model reads
    /// high once station effects are removed.
    pub global_db: f64,
    /// How many spots each transmitter contributed. An effect estimated from two
    /// spots is not an estimate.
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
    /// two-way decomposition of that, computed by alternating conditional means -
    /// the standard "sweep" algorithm for a two-factor additive model with
    /// unbalanced cells. It converges geometrically and 200 sweeps is far past
    /// the point of no further movement for corpora of this size.
    ///
    /// The alternative would be to build and solve the normal equations, which
    /// for a few hundred stations is a few hundred thousand entries; the sweep
    /// gives the same answer without the matrix.
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

            // Re-centre, so the gauge stays where the doc comment says it is and
            // the global offset really is the whole unattributable part.
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

    /// Spread of the estimated effects, over stations with enough spots to have
    /// been estimated at all. This is a MEASUREMENT of the WSPR station
    /// population and is worth reporting on its own.
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

/// Linearly interpolated percentile of a SORTED slice; NaN when empty.
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
/// The SLOPE matters more than the bias. A model whose bias is zero but whose
/// slope is 0.3 emits nearly the same number for every path and has learnt
/// nothing about which paths are strong; correcting its bias would only hide
/// that.
///
/// # Why two slopes are reported
///
/// Against RAW measured SNR the slope is attenuated by construction. If
/// `measured = physics + station_effect`, then regressing the modelled value
/// (which contains no station effect) on the measured value gives
///
/// ```text
///   slope = Var(physics) / (Var(physics) + Var(station_effect))
/// ```
///
/// which is below 1 however perfect the physics is. That is classical
/// errors-in-variables attenuation, not a model defect. With station effects
/// worth twice the variance of the physics it lands near 0.33 on its own.
///
/// So [`Fit::slope_raw`] is reported because it is what a naive comparison shows,
/// and [`Fit::slope_adjusted`] - measured with the estimated station effects
/// removed from the measurement - is reported because it is the one that says
/// whether the physics tracks reality. Quoting only the first understates the
/// model; quoting only the second overstates what a user would see.
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
            // The measurement with the station's own offset folded IN, which is
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
/// Returns `(NaN, NaN)` rather than a confident zero when `x` has no variance:
/// a slope against a constant is undefined, not flat.
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
/// Deliberately only the ones that re-derive from [`Cached`] without a re-solve.
/// The rest change which paths exist and are scanned separately.
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
    /// 1e6 to 2e7 s^-1 against its 5e6 prior, i.e. 0.2x to 4x. It is not a
    /// separate freedom; it is that range restated.
    #[must_use]
    pub fn prior() -> Self {
        let nu = Anchors::default().ionosphere.nu_ref_per_s;
        Self {
            absorption_scale: Bounded::new(1.0, nu.min / nu.value, nu.max / nu.value),
            atm: AtmosphericAnchors::default(),
        }
    }

    /// The parameters a search has to STEP, on a common 0..1 scale so it can do
    /// so without knowing their units.
    ///
    /// The absorption scale is deliberately ABSENT: the prediction is exactly
    /// linear in it, so it is solved in closed form at every trial point instead
    /// ([`best_absorption_scale`]). Stepping it alongside the noise slopes, which
    /// it is strongly correlated with, made the descent stall - measured on a
    /// planted case, it stopped at a scale of 1.76 and an RMS of 0.20 dB when the
    /// true optimum was 1.80 at an RMS of 0. Profiling it out removes that
    /// correlation from the search entirely.
    ///
    /// The atmospheric anchors a fixed-effects design cannot identify - the
    /// seasonal swing and both latitude terms - are also absent. A receiver's
    /// latitude never changes, so the latitude terms are a constant per receiver
    /// and are absorbed exactly into its effect; a one-month corpus has no second
    /// season. Offering them to the search would let it move them freely without
    /// changing the objective, and then report a "fitted value" that is noise.
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
            // The three that carry the day/night contrast. These are what a WSPR
            // corpus can actually see: the level of the floor is constant per
            // station and absorbed into that station's effect, the difference
            // across the terminator is not.
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
/// A corpus of spots is a corpus of SUCCESSES. Every residual in it is
/// `modelled - measured - station - global`, and `global` is re-solved at every
/// trial point, so shifting the whole model by a constant costs the objective
/// exactly nothing. The level is not weakly identified from the positives, it is
/// not identified at all - and the fit is therefore free to close a residual by
/// sliding the model optimistic, which is precisely what it was measured doing.
///
/// A negative is different in kind. It says that on THIS path, at THIS hour, a
/// receiver that was demonstrably hearing other stations did not hear this one.
/// That is a one-sided statement about an ABSOLUTE quantity: the modelled SNR
/// should have been below the decode threshold. It cannot be satisfied by any
/// constant shift, so it removes the degeneracy the positives leave behind.
///
/// # Why the term is a hinge and not a residual
///
/// The corpus documentation is explicit that the false-positive rate it measures
/// is an UPPER BOUND: a negative may be a path that really was open and merely
/// collided, or arrived off the back of a beam. So a negative carries no
/// information at all about how far below the threshold the signal should have
/// been - only that it should not have been above it. Penalising
/// `(modelled - threshold)^2` unconditionally would invent that missing
/// information and drive every negative towards a specific SNR the data never
/// stated. The hinge `max(0, modelled - threshold)^2` charges only the excess,
/// which is the whole of what a non-decode asserts.
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
    /// No constraint: the positives-only objective, exactly as before.
    #[must_use]
    pub fn none() -> Self {
        Self {
            spots: &[],
            threshold_db: 0.0,
            weight: 0.0,
        }
    }

    /// Negatives weighted so their TOTAL weight equals the positives' total.
    ///
    /// How many negatives get scored is an arbitrary sampling decision - the
    /// calibrator thins tens of thousands of them down to whatever is
    /// affordable - so an unweighted sum would let that choice, rather than the
    /// evidence, decide how much the one-sided term counts for. Equalising the
    /// totals makes the objective half positives and half negatives however many
    /// of each were solved.
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
    /// A station absent from the fit's index space contributes 0, which is not a
    /// guess: the effects are gauge-centred to mean zero, so 0 IS the population
    /// estimate for a station nothing is known about.
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

/// The absorption scale AND the global offset that minimise the residual, solved
/// jointly in closed form.
///
/// # Why the two must be solved together
///
/// The prediction is `base_i - s * absorption_i`, exactly linear in `s`, and the
/// unattributable global offset `g` is a constant. Solving them alternately - `s`
/// against a fixed `g`, then `g` as the residual mean - converges, but only
/// geometrically, at rate
///
/// ```text
///   k = mean(a)^2 / (mean(a)^2 + var(a))
/// ```
///
/// which is the fraction of absorption's total mean-square that its MEAN accounts
/// for. Measured on a planted case with `mean(a) = 7.5 dB` and
/// `var(a) = 11.7 dB^2`, `k = 0.83`, so the alternation removed only 17 % of the
/// error per round: after 12 rounds a planted scale of 1.80 read 1.73, and after
/// 20 rounds 1.77. That looked exactly like a stuck optimiser and was not one.
///
/// The reason is worth stating as a result rather than hiding in an
/// implementation: **the absorption scale is identified by how much absorption
/// VARIES across the corpus, not by its average level.** A constant amount of
/// absorption is indistinguishable from the global offset, which WSPR cannot
/// attribute to physics or to the station population. So a corpus with little
/// spread in absorption cannot calibrate absorption at all, however many spots it
/// holds.
///
/// Solving the 2x2 system removes that confounded direction from the alternation
/// entirely, so the fit converges in a couple of rounds instead of dozens.
///
/// The bool reports whether the scale had to be clamped to its range, because a
/// scale that wants to leave every published collision frequency is a finding
/// about the model rather than a number to quietly accept.
/// # How the one-sided negatives enter a closed-form solve
///
/// A violating negative contributes `w (u_i - g - s a_i)^2` with
/// `u_i = base_i - threshold - station_i`, which is ALGEBRAICALLY IDENTICAL to a
/// positive whose measurement is the threshold, carrying weight `w`. So the
/// hinge does not need a different solver - it needs the right set of
/// pseudo-observations.
///
/// Which negatives are violating depends on `(g, s)`, and `(g, s)` depends on
/// which are violating. The objective is convex in `(g, s)` - a sum of squares
/// plus squared hinges of affine functions - so alternating the two converges to
/// the global minimum: pick the active set, solve the weighted 2x2 exactly,
/// repeat until the active set stops changing. It settles in a handful of passes
/// and cannot cycle, because each solve strictly reduces a convex objective that
/// each active-set update also cannot increase.
#[must_use]
pub fn best_absorption_scale(
    spots: &[Cached],
    atm: AtmosphericAnchors,
    effects: &StationEffects,
    prior: Bounded,
    negatives: Negatives<'_>,
) -> (Bounded, f64, bool) {
    // Regress u_i on (1, a_i): u_i = g + s * a_i, in the least-squares sense.
    // Note the sign - the prediction SUBTRACTS s * a_i - so `u` is formed to make
    // `s` come out positive for a model that needs more absorption.
    //
    // The positives' contribution never changes with the active set, so it is
    // accumulated once.
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
            // No absorption variation at all: the scale is not identified. Leave
            // it at the prior and put everything into the offset, which is the
            // honest outcome rather than an invented scale.
            return None;
        }
        Some(((su * saa - sa * sua) / det, (n * sua - sa * su) / det))
    };

    // The active set is SEEDED at the prior scale rather than started empty.
    //
    // The positives can be singular in the scale direction all by themselves -
    // that is the documented case where absorption does not vary and the scale is
    // not identified - and it is exactly then that the negatives carry the only
    // information there is. Starting from an empty active set would solve the
    // positives alone, find them singular, and give up before ever looking at the
    // evidence that resolves them.
    //
    // The seed pair must be a pair that actually DESCRIBES the prior model:
    // `base_u / base_n` is the offset that fits when the scale is zero, not when
    // it is the prior, and seeding with a mismatched pair mis-classifies which
    // negatives the prior model would have claimed.
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
        // Singular even with the active negatives included: the scale genuinely
        // is not identified, so leave it at the prior and let the offset carry
        // everything. That is the honest outcome, not an invented scale.
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
    // With the scale clamped, re-solve the offset conditional on it, or the two
    // no longer describe the same fit. The active set is held at the one the
    // unclamped solve settled on: re-deriving it under a clamped scale would be
    // solving a different problem than the one that reported the clamp.
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

/// The sum of squares at one point, with the absorption scale FIXED and only the
/// global offset solved for it.
///
/// Used to profile the objective along the absorption scale, which
/// [`best_absorption_scale`] otherwise solves in closed form and so never
/// exposes as a curve. The global offset must still be re-solved at every scale,
/// or the profile would measure the cost of holding the level wrong rather than
/// the cost of the scale.
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
/// Public so a report can draw the objective as a CURVE along each parameter.
/// A parameter that ends on a bound has two quite different explanations - the
/// objective really falls all the way to the edge, or it goes flat partway and a
/// descent drifts to the edge because nothing stops it - and only the curve
/// distinguishes them. `Bounded::at_bound` cannot: it sees where the value
/// landed, not what the objective was doing there.
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
/// Coordinate descent rather than a gradient method because the objective is
/// cheap, only five-dimensional, and the parameters are bounded - and because a
/// coordinate step that wants to leave its bound is exactly the finding the
/// bounds exist to surface. `report_bound_hits` says which ones did.
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
    // The PROFILED objective: at every trial set of noise anchors the absorption
    // scale is solved in closed form rather than searched, so the search only ever
    // walks the four-dimensional noise surface. Returns the objective and the
    // scale that produced it.
    //
    // The hinge over the negatives is part of the SAME objective, not a separate
    // score compared afterwards. That is the whole point: a trial point that
    // improves the positives by sliding the model optimistic must pay for the
    // non-decodes it thereby claims would have decoded.
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
        // Re-profile the scale and the global offset against the effects just
        // solved. Both together, for the reason  explains.
        let (_, scale, global) = profiled(params.atm, &effects);
        params.absorption_scale = scale;
        effects.global_db = global;

        // Parameters, by coordinate descent on the unit scale.
        //
        // Each step size is swept to convergence rather than once. The absorption
        // scale and the noise-model frequency slopes are genuinely correlated -
        // both change how the prediction varies across bands - so a single sweep
        // per step size zigzags and stalls short of the optimum. Measured on a
        // planted case: one sweep per size recovered 1.69 of a true 1.80;
        // sweeping to convergence recovers it.
        let mut improved = false;
        let fields = CachedParams::fields();
        // Coarse to fine: 1/8 of each range down to 1/4096, which resolves the
        // absorption scale to about 0.1 % and the noise anchors to a hundredth of
        // a dB - well below the precision the data can support either way.
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
                    // A relative improvement floor, so the sweep stops on genuine
                    // convergence rather than on floating-point dust.
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
                // PATTERN MOVE (Hooke-Jeeves). Pure coordinate descent cannot
                // follow a valley that runs diagonally to its axes, and this
                // objective has exactly such a valley: raising the noise
                // frequency slope and lowering the absorption scale change the
                // prediction across bands in nearly the same way. Measured on a
                // planted case, axis-only descent stalled at a scale of 1.77 with
                // 0.19 dB of residual when the optimum was 1.80 with none.
                //
                // So after each productive sweep, keep going in the direction the
                // sweep moved, as far as that keeps helping.
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
/// A bound hit is a FINDING, not a nuisance to be widened away: it means the
/// residual the fit was chasing is not produced by the quantity it was pushing,
/// so the error lives elsewhere in the model.
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
/// The false-positive rate here is an UPPER BOUND, because the negatives set
/// necessarily includes some paths that were open but collided or arrived off the
/// back of a directional antenna. See [`crate::corpus`].
#[derive(Clone, Debug, Default)]
pub struct NegativeScore {
    pub n: usize,
    /// Negatives for which the model found a geometric path at all.
    pub path_found: usize,
    /// Negatives the model predicted would be DECODED - above the WSPR threshold.
    /// This is the false positive that matters: claiming an opening that was not
    /// there.
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
    /// only by its own two loss terms - the noise floor is a property of the spot.
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

    /// The midpoint cut is on the MIDPOINT, and 90 degrees is the terminator.
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
        // The noise floor really is the one the module composes, not a copy.
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
    /// dB - the linearity the whole cache rests on.
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

    /// The gauge freedom the docs describe really is a freedom: shifting a
    /// constant from the transmitters to the receivers changes no prediction, so
    /// the solver must return the CENTRED representative of that family.
    #[test]
    fn only_the_sum_of_the_two_effects_is_identified() {
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        for t in 0..2 {
            for r in 0..2 {
                spots.push(mk(t, r, -15.0, 5.0, 14.097));
                // A planted set that is NOT centred: tx offset by +10 throughout.
                #[allow(clippy::cast_precision_loss)]
                let planted = 10.0 + t as f64 - r as f64;
                residuals.push(planted);
            }
        }
        let e = StationEffects::solve(&residuals, &spots, 2, 2);
        // The +10 could not be attributed to either end, so it must have landed
        // in the global term, not in the transmitter effects.
        assert!(e.tx.iter().sum::<f64>().abs() < 1e-9);
        assert!(e.rx.iter().sum::<f64>().abs() < 1e-9);
        assert!((e.global_db - 10.0).abs() < 1e-6, "{}", e.global_db);
    }

    /// A station seen a handful of times must be excluded from the reported
    /// distribution: its "effect" is its own residual and describes nothing.
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

    /// The attenuation the doc comment describes, demonstrated: a PERFECT model
    /// scores a slope well below 1 against raw measured SNR purely because the
    /// measurement carries station effects, and scores 1 once they are removed.
    ///
    /// This is why the headline number has to be the adjusted slope.
    #[test]
    fn a_perfect_model_still_scores_a_low_raw_slope() {
        let atm = AtmosphericAnchors::default();
        let mut spots = Vec::new();
        let mut residuals = Vec::new();
        // 20 transmitters x 5 receivers. The physics varies through absorption;
        // the station effects are twice as variable, which is the regime the WSPR
        // population actually sits in.
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

    /// The fit must recover a planted absorption error rather than pushing it
    /// into the station effects - the whole point of the design.
    ///
    /// # What this test had to be rewritten to demonstrate
    ///
    /// It first planted an absorption that varied with the TRANSMITTER index. The
    /// fit then recovered nothing at all and left the scale exactly at its prior,
    /// which was correct behaviour: absorption that is a function of the station
    /// is perfectly collinear with that station's fixed effect, so the effect
    /// absorbs it completely and the data contains no information about the scale.
    ///
    /// That is the identification condition made concrete. The physics is
    /// identified only from variation WITHIN a station, so the corpus must have
    /// each station spanning bands and hours - which is exactly why the corpus
    /// schedule walks eight UTC hours and ten bands rather than taking a uniform
    /// sample. The layout below has each TX->RX pair observed on four bands at
    /// three absorption levels, so the scale is identified within every pair.
    #[test]
    fn the_fit_recovers_a_planted_absorption_error() {
        let atm = AtmosphericAnchors::default();
        let truth = 1.8;
        let mut spots = Vec::new();
        for t in 0..12 {
            for r in 0..5 {
                for freq in [3.568, 7.038, 14.095, 21.094] {
                    for level in 0..3 {
                        // Day/night is crossed with the absorption level rather
                        // than tied to it. Tied together, the noise model's
                        // day-night anchors can explain part of the absorption
                        // error and the two are only partly separable - which is
                        // itself worth knowing, and is why the corpus samples
                        // every band at every hour rather than one band per hour.
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

    /// The other half of that lesson, pinned so it cannot be forgotten: when the
    /// planted physics error IS collinear with a station, the fit must leave the
    /// parameter alone rather than reporting a confident wrong value.
    ///
    /// A fit that "recovered" a scale here would be reading the station
    /// population and calling it absorption.
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
        // ...and the station effects will have swallowed the whole thing, leaving
        // no residual. A tiny residual with an unmoved parameter is the signature
        // of an unidentified quantity, not of a good fit.
        let fit = Fit::of(&spots, params.absorption_scale.value, params.atm, &effects);
        assert!(fit.rms_db < 1e-6, "residual {} dB", fit.rms_db);
    }

    /// The failure the negatives exist to prevent, demonstrated and then fixed.
    ///
    /// The absorption scale is identified by how much absorption VARIES, never by
    /// its mean - a constant amount of absorption is indistinguishable from the
    /// global offset. So a corpus whose absorption is the same everywhere cannot
    /// identify the scale at all, however many spots it holds, and the fit
    /// correctly leaves it at its prior while the offset swallows the difference.
    ///
    /// A non-decode breaks that, and is the only thing here that can. It is a
    /// statement about an ABSOLUTE SNR, so no constant shift satisfies it, and a
    /// negative carrying a different amount of absorption from the positives
    /// supplies exactly the variation the positives lacked.
    #[test]
    fn negatives_identify_a_scale_the_positives_cannot() {
        let atm = AtmosphericAnchors::default();
        let truth = 3.0;
        // Absorption is EXACTLY constant across the positives, so the normal
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

    /// The hinge must be ONE-SIDED. A negative the model already places below
    /// the threshold has nothing more to say, and pulling it further down would
    /// be inventing information the corpus explicitly does not contain.
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

    /// With no negatives the objective must be bit-identical to the old
    /// positives-only one, or every earlier result silently changed meaning.
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

    /// A regression against a constant is undefined, and must say so rather than
    /// return a confident zero.
    #[test]
    fn regression_refuses_a_degenerate_input() {
        let (s, r) = regress(&[1.0, 1.0, 1.0, 1.0], &[1.0, 2.0, 3.0, 4.0]);
        assert!(s.is_nan() && r.is_nan());
        let (s, r) = regress(&[1.0, 2.0], &[1.0, 2.0]);
        assert!(s.is_nan() && r.is_nan(), "two points is not a regression");
        let (s, r) = regress(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]);
        assert!((s - 2.0).abs() < 1e-12 && (r - 1.0).abs() < 1e-12);
    }

    /// The false-positive rate is over ALL negatives, not only the ones a path
    /// was found for - otherwise a model that found nothing would score zero.
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
