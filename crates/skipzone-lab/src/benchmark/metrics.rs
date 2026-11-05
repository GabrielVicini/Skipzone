//! The metrics of docs/notes/accuracy-roadmap.md §2.3, on scored rows.
//!
//! Everything here is a pure function of [`Scored`] rows, so the bootstrap
//! can call it on resampled sets and the paired comparison can call it on
//! two models' rows alike.
//!
//! Signs follow the rest of the lab crate: errors are model minus measured,
//! so a positive bias means the model reads high.

/// One row as a model scored it. Path rows fill the SNR and decode fields,
/// ionosonde rows the MHz/km ones.
#[derive(Clone, Debug, Default)]
pub struct Scored {
    /// The corpus row's id, the pairing key.
    pub id: String,
    /// UTC day, the bootstrap block.
    pub day: (i32, u32, u32),
    /// Cut labels: band, distance bucket, midpoint day or night. Ionosonde
    /// rows use the station and local hour instead.
    pub cuts: Vec<String>,
    /// Measured SNR, dB, for a decode that reported one.
    pub measured_db: Option<f64>,
    /// Modelled SNR, dB, when the model found a path.
    pub model_db: Option<f64>,
    /// Transmitter plus receiver effect, dB, re-estimated on the fold's
    /// scored rows. Figure 1 of the roadmap's two SNR figures subtracts it.
    pub effect_db: Option<f64>,
    /// Predicted decode probability; 0 when no path was found.
    pub p_decode: Option<f64>,
    /// Observed outcome: true for a decode.
    pub decoded: Option<bool>,
    /// Modelled SNR at or over the decode threshold.
    pub decodable: Option<bool>,
    /// `(model, measured)` pairs for the ionosonde metrics.
    pub fof2: Option<(f64, f64)>,
    pub hmf2: Option<(f64, f64)>,
    pub muf: Option<(f64, f64)>,
}

/// Every metric, in the order they are written. The suffix `_none` is the
/// roadmap's figure 2 (no station effects), `_eff` figure 1 (effects
/// re-estimated on the scored rows).
pub const METRICS: [&str; 30] = [
    "snr_bias_mean_none",
    "snr_bias_median_none",
    "snr_rms_none",
    "snr_median_abs_none",
    "snr_iqr_none",
    "snr_slope_none",
    "snr_bias_mean_eff",
    "snr_bias_median_eff",
    "snr_rms_eff",
    "snr_median_abs_eff",
    "snr_iqr_eff",
    "snr_slope_eff",
    "path_found_rate",
    "brier",
    "brier_skill",
    "brier_reliability",
    "brier_resolution",
    "brier_uncertainty",
    "auc",
    "base_rate",
    "false_positive_rate",
    "fof2_bias",
    "fof2_rms",
    "hmf2_bias",
    "hmf2_rms",
    "muf_bias",
    "muf_rms",
    "muf_median_abs",
    "n_decodes",
    "n_non_decodes",
];

/// A metric value and the number of rows it was computed from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub value: f64,
    pub n: usize,
}

const NONE: Value = Value {
    value: f64::NAN,
    n: 0,
};

#[allow(clippy::cast_precision_loss)]
fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Linear-interpolated quantile of an unsorted sample.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn quantile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    quantile_sorted(&s, q)
}

/// [`quantile`] of a sample already sorted ascending.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn quantile_sorted(s: &[f64], q: f64) -> f64 {
    if s.is_empty() {
        return f64::NAN;
    }
    let pos = q.clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    s[lo] + (pos - lo as f64) * (s[hi] - s[lo])
}

/// Bias, RMS, median absolute error and IQR of a residual, plus the slope
/// of model on measured.
fn residual_block(pairs: &[(f64, f64)]) -> [Value; 6] {
    if pairs.is_empty() {
        return [NONE; 6];
    }
    let n = pairs.len();
    // Sorted once: the bootstrap calls this a thousand times per group.
    let mut r: Vec<f64> = pairs.iter().map(|(m, y)| m - y).collect();
    r.sort_by(f64::total_cmp);
    let mut abs: Vec<f64> = r.iter().map(|e| e.abs()).collect();
    abs.sort_by(f64::total_cmp);
    let rms = mean(&r.iter().map(|e| e * e).collect::<Vec<_>>()).sqrt();
    let v = |value: f64| Value { value, n };
    [
        v(mean(&r)),
        v(quantile_sorted(&r, 0.5)),
        v(rms),
        v(quantile_sorted(&abs, 0.5)),
        v(quantile_sorted(&r, 0.75) - quantile_sorted(&r, 0.25)),
        v(slope(pairs)),
    ]
}

/// Least-squares slope of model on measured. Below 1 means the model
/// compresses the measured range.
#[must_use]
pub fn slope(pairs: &[(f64, f64)]) -> f64 {
    let xs: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    let ys: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let (mx, my) = (mean(&xs), mean(&ys));
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (x, y) in xs.iter().zip(&ys) {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    if sxx > 0.0 { sxy / sxx } else { f64::NAN }
}

/// Number of equal-width probability bins for the reliability diagram and
/// the Murphy decomposition.
pub const BINS: usize = 10;

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn bin_of(p: f64) -> usize {
    ((p.clamp(0.0, 1.0) * BINS as f64).floor() as usize).min(BINS - 1)
}

/// Brier score: mean of `(p - y)^2`, `y` = 1 for a decode.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn brier(py: &[(f64, bool)]) -> f64 {
    if py.is_empty() {
        return f64::NAN;
    }
    py.iter()
        .map(|&(p, y)| (p - f64::from(u8::from(y))).powi(2))
        .sum::<f64>()
        / py.len() as f64
}

/// Murphy's decomposition over [`BINS`] bins: `(reliability, resolution,
/// uncertainty)`, with Brier close to `rel - res + unc` (exact when the
/// forecasts are constant within each bin).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn murphy(py: &[(f64, bool)]) -> (f64, f64, f64) {
    if py.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let n = py.len() as f64;
    let base = py.iter().filter(|x| x.1).count() as f64 / n;
    let mut cnt = [0usize; BINS];
    let mut sp = [0.0; BINS];
    let mut so = [0.0; BINS];
    for &(p, y) in py {
        let b = bin_of(p);
        cnt[b] += 1;
        sp[b] += p;
        so[b] += f64::from(u8::from(y));
    }
    let mut rel = 0.0;
    let mut res = 0.0;
    for b in 0..BINS {
        if cnt[b] == 0 {
            continue;
        }
        let k = cnt[b] as f64;
        let (pb, ob) = (sp[b] / k, so[b] / k);
        rel += k * (pb - ob).powi(2);
        res += k * (ob - base).powi(2);
    }
    (rel / n, res / n, base * (1.0 - base))
}

/// Area under the ROC curve of `p` against the outcome, ties counted half
/// (the Mann-Whitney form).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn auc(py: &[(f64, bool)]) -> f64 {
    let npos = py.iter().filter(|x| x.1).count();
    let nneg = py.len() - npos;
    if npos == 0 || nneg == 0 {
        return f64::NAN;
    }
    let mut s: Vec<(f64, bool)> = py.to_vec();
    s.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Sum of the positives' mid-ranks.
    let mut rank_sum = 0.0;
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j < s.len() && s[j].0 == s[i].0 {
            j += 1;
        }
        let mid = (i + j + 1) as f64 / 2.0;
        rank_sum += mid * s[i..j].iter().filter(|x| x.1).count() as f64;
        i = j;
    }
    let (np, nn) = (npos as f64, nneg as f64);
    (rank_sum - np * (np + 1.0) / 2.0) / (np * nn)
}

/// One bin of a reliability diagram.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bin {
    pub lo: f64,
    pub hi: f64,
    pub n: usize,
    pub mean_p: f64,
    pub observed: f64,
    /// Wilson 95 % interval on `observed`.
    pub wilson: (f64, f64),
}

/// Wilson score interval at 95 % for `k` successes in `n`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    let z = 1.959_963_984_540_054;
    let n = n as f64;
    let p = k as f64 / n;
    let d = 1.0 + z * z / n;
    let c = (p + z * z / (2.0 * n)) / d;
    let h = z / d * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt();
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Ten equal-width bins of predicted probability, with the observed decode
/// fraction in each.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn reliability(py: &[(f64, bool)]) -> Vec<Bin> {
    let mut cnt = [0usize; BINS];
    let mut hit = [0usize; BINS];
    let mut sp = [0.0; BINS];
    for &(p, y) in py {
        let b = bin_of(p);
        cnt[b] += 1;
        sp[b] += p;
        hit[b] += usize::from(y);
    }
    (0..BINS)
        .map(|b| {
            let n = cnt[b];
            Bin {
                lo: b as f64 / BINS as f64,
                hi: (b + 1) as f64 / BINS as f64,
                n,
                mean_p: if n > 0 { sp[b] / n as f64 } else { f64::NAN },
                observed: if n > 0 {
                    hit[b] as f64 / n as f64
                } else {
                    f64::NAN
                },
                wilson: wilson(hit[b], n),
            }
        })
        .collect()
}

fn pair_block(pairs: &[(f64, f64)]) -> [Value; 2] {
    if pairs.is_empty() {
        return [NONE; 2];
    }
    let r: Vec<f64> = pairs.iter().map(|(m, y)| m - y).collect();
    let n = r.len();
    [
        Value { value: mean(&r), n },
        Value {
            value: mean(&r.iter().map(|e| e * e).collect::<Vec<_>>()).sqrt(),
            n,
        },
    ]
}

/// The probability pairs of the rows that have both.
#[must_use]
pub fn prob_pairs(rows: &[&Scored]) -> Vec<(f64, bool)> {
    rows.iter()
        .filter_map(|r| Some((r.p_decode?, r.decoded?)))
        .collect()
}

/// Every metric in [`METRICS`] order.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute(rows: &[&Scored]) -> Vec<Value> {
    let none: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| Some((r.model_db?, r.measured_db?)))
        .collect();
    let eff: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| Some((r.model_db? - r.effect_db?, r.measured_db?)))
        .collect();
    let mut out = Vec::with_capacity(METRICS.len());
    out.extend(residual_block(&none));
    out.extend(residual_block(&eff));

    // Of the decodes, how many the model found any path for.
    let decodes: Vec<&&Scored> = rows.iter().filter(|r| r.decoded == Some(true)).collect();
    let found = decodes.iter().filter(|r| r.model_db.is_some()).count();
    out.push(if decodes.is_empty() {
        NONE
    } else {
        Value {
            value: found as f64 / decodes.len() as f64,
            n: decodes.len(),
        }
    });

    let py = prob_pairs(rows);
    let n = py.len();
    let v = |value: f64| if n == 0 { NONE } else { Value { value, n } };
    let b = brier(&py);
    let (rel, res, unc) = murphy(&py);
    out.push(v(b));
    out.push(v(if unc > 0.0 { 1.0 - b / unc } else { f64::NAN }));
    out.push(v(rel));
    out.push(v(res));
    out.push(v(unc));
    out.push(v(auc(&py)));
    out.push(v(if n == 0 {
        f64::NAN
    } else {
        py.iter().filter(|x| x.1).count() as f64 / n as f64
    }));
    // Upper bound, as `fit::NegativeScore` reports it: the share of
    // non-decodes the model called decodable.
    let negs: Vec<bool> = rows
        .iter()
        .filter(|r| r.decoded == Some(false))
        .filter_map(|r| r.decodable)
        .collect();
    out.push(if negs.is_empty() {
        NONE
    } else {
        Value {
            value: negs.iter().filter(|d| **d).count() as f64 / negs.len() as f64,
            n: negs.len(),
        }
    });

    let pairs = |f: fn(&Scored) -> Option<(f64, f64)>| -> Vec<(f64, f64)> {
        rows.iter().filter_map(|r| f(r)).collect()
    };
    out.extend(pair_block(&pairs(|r| r.fof2)));
    out.extend(pair_block(&pairs(|r| r.hmf2)));
    let muf = pairs(|r| r.muf);
    out.extend(pair_block(&muf));
    out.push(if muf.is_empty() {
        NONE
    } else {
        let abs: Vec<f64> = muf.iter().map(|(m, y)| (m - y).abs()).collect();
        Value {
            value: quantile(&abs, 0.5),
            n: muf.len(),
        }
    });
    let count = |want: bool| {
        let k = rows.iter().filter(|r| r.decoded == Some(want)).count();
        if k == 0 {
            NONE
        } else {
            Value {
                value: k as f64,
                n: k,
            }
        }
    };
    out.push(count(true));
    out.push(count(false));
    debug_assert_eq!(out.len(), METRICS.len());
    out
}

/// Index of a metric in [`METRICS`].
#[must_use]
pub fn index_of(name: &str) -> Option<usize> {
    METRICS.iter().position(|m| *m == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
    }

    #[test]
    fn brier_by_hand() {
        // (0.9-1)^2 + (0.2-0)^2 + (0.6-1)^2 + (0.6-0)^2 = 0.01+0.04+0.16+0.36
        let py = [(0.9, true), (0.2, false), (0.6, true), (0.6, false)];
        close(brier(&py), 0.57 / 4.0);
    }

    #[test]
    fn murphy_terms_by_hand() {
        // Bins: 0.9 -> bin 9 (obs 1), 0.2 -> bin 2 (obs 0), 0.6 twice -> bin 6
        // (obs 0.5). Base rate 0.5.
        // REL = (1*(0.9-1)^2 + 1*(0.2-0)^2 + 2*(0.6-0.5)^2)/4 = (0.01+0.04+0.02)/4
        // RES = (1*0.25 + 1*0.25 + 2*0)/4 = 0.125
        // UNC = 0.25
        let py = [(0.9, true), (0.2, false), (0.6, true), (0.6, false)];
        let (rel, res, unc) = murphy(&py);
        close(rel, 0.07 / 4.0);
        close(res, 0.125);
        close(unc, 0.25);
        // Exact here, since forecasts are constant within each bin.
        close(rel - res + unc, brier(&py));
    }

    #[test]
    fn reliability_bins_by_hand() {
        let py = [(0.05, false), (0.05, true), (0.95, true), (1.0, true)];
        let bins = reliability(&py);
        assert_eq!(bins.len(), 10);
        assert_eq!(bins[0].n, 2);
        close(bins[0].mean_p, 0.05);
        close(bins[0].observed, 0.5);
        // p = 1.0 lands in the top bin.
        assert_eq!(bins[9].n, 2);
        close(bins[9].observed, 1.0);
        assert_eq!(bins[5].n, 0);
        assert!(bins[5].observed.is_nan());
    }

    #[test]
    fn wilson_by_hand() {
        // 5 of 10: centre 0.5, half-width z/(1+z^2/10) * sqrt(0.025 + z^2/400).
        let (lo, hi) = wilson(5, 10);
        let z: f64 = 1.959_963_984_540_054;
        let d = 1.0 + z * z / 10.0;
        let h = z / d * (0.025 + z * z / 400.0).sqrt();
        close(lo, 0.5 - h);
        close(hi, 0.5 + h);
        // Published value for 5/10: about 0.237 to 0.763.
        assert!((lo - 0.2366).abs() < 1e-3 && (hi - 0.7634).abs() < 1e-3);
    }

    #[test]
    fn auc_by_hand() {
        // Positives 0.9, 0.6; negatives 0.6, 0.2. Pairs: (0.9>0.6) 1, (0.9>0.2) 1,
        // (0.6=0.6) 0.5, (0.6>0.2) 1 -> 3.5/4.
        let py = [(0.9, true), (0.2, false), (0.6, true), (0.6, false)];
        close(auc(&py), 0.875);
    }

    #[test]
    fn snr_metrics_with_and_without_effects() {
        let rows: Vec<Scored> = [(1.0, 0.0, 1.0), (3.0, 0.0, 1.0), (-2.0, 0.0, -1.0)]
            .iter()
            .map(|&(m, y, e)| Scored {
                model_db: Some(m),
                measured_db: Some(y),
                effect_db: Some(e),
                decoded: Some(true),
                p_decode: Some(1.0),
                ..Scored::default()
            })
            .collect();
        let refs: Vec<&Scored> = rows.iter().collect();
        let v = compute(&refs);
        let get = |name: &str| v[index_of(name).unwrap()].value;
        close(get("snr_bias_mean_none"), 2.0 / 3.0);
        close(get("snr_rms_none"), (14.0_f64 / 3.0).sqrt());
        close(get("snr_bias_median_none"), 1.0);
        close(get("snr_median_abs_none"), 2.0);
        // Residuals after effects: 0, 2, -1.
        close(get("snr_bias_mean_eff"), 1.0 / 3.0);
        close(get("snr_rms_eff"), (5.0_f64 / 3.0).sqrt());
        close(get("path_found_rate"), 1.0);
        close(get("n_decodes"), 3.0);
    }
}
