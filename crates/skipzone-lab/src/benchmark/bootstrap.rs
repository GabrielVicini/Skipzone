//! Error bars by block bootstrap over UTC days, and the paired version for a
//! before/after comparison (docs/notes/accuracy-roadmap.md §2.3).
//!
//! Spots on one day share one ionosphere, so the day is the unit that is
//! resampled. Everything is seeded: the same rows, seed and resample count
//! give the same intervals on every machine.
//!
//! Station effects are estimated once per fold and held fixed inside the
//! resamples. Re-estimating them per resample would widen the `_eff`
//! intervals a little and cost a two-way solve per resample.

use std::collections::BTreeMap;

use super::metrics::{self, METRICS, Scored};

/// Resamples when none is given, as the roadmap specifies.
pub const DEFAULT_RESAMPLES: usize = 1000;

/// Seed when none is given. Recorded in every report.
pub const DEFAULT_SEED: u64 = 20_261_105;

/// SplitMix64: small, fast, and identical everywhere.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`, by the multiply-shift reduction.
    #[allow(clippy::cast_possible_truncation)]
    pub fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
}

/// A seed for one group, from the run seed and the group's name. Models
/// scored on the same corpus, split and cut draw the same days.
#[must_use]
pub fn seed_for(seed: u64, key: &str) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    seed ^ h
}

/// Row indices grouped by UTC day, in day order.
#[must_use]
pub fn day_blocks(rows: &[&Scored]) -> Vec<Vec<usize>> {
    let mut by_day: BTreeMap<(i32, u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_day.entry(r.day).or_default().push(i);
    }
    by_day.into_values().collect()
}

/// 2.5 % and 97.5 % points of the finite values, or NaN when fewer than
/// half the resamples gave a finite value (a metric with no rows on most
/// resampled day sets has no interval worth printing).
fn interval(values: &[f64]) -> (f64, f64) {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.len() * 2 < values.len() || finite.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    (
        metrics::quantile(&finite, 0.025),
        metrics::quantile(&finite, 0.975),
    )
}

/// One resample: as many days as there are, drawn with replacement.
fn draw(blocks: &[Vec<usize>], rng: &mut Rng) -> Vec<usize> {
    let mut picked = Vec::new();
    for _ in 0..blocks.len() {
        picked.extend_from_slice(&blocks[rng.below(blocks.len())]);
    }
    picked
}

/// 95 % day-block bootstrap interval for every metric, in
/// [`METRICS`] order.
#[must_use]
pub fn intervals(rows: &[&Scored], seed: u64, resamples: usize) -> Vec<(f64, f64)> {
    let blocks = day_blocks(rows);
    if blocks.is_empty() || resamples == 0 {
        return vec![(f64::NAN, f64::NAN); METRICS.len()];
    }
    let mut rng = Rng::new(seed);
    let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(resamples); METRICS.len()];
    for _ in 0..resamples {
        let idx = draw(&blocks, &mut rng);
        let set: Vec<&Scored> = idx.iter().map(|&i| rows[i]).collect();
        for (s, v) in samples.iter_mut().zip(metrics::compute(&set)) {
            s.push(v.value);
        }
    }
    samples.iter().map(|s| interval(s)).collect()
}

/// A paired difference, after minus before, with its bootstrap interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paired {
    pub before: f64,
    pub after: f64,
    pub delta: f64,
    pub lo: f64,
    pub hi: f64,
    /// Rows present in both.
    pub n: usize,
}

/// Paired bootstrap of every metric over the rows both sides scored,
/// matched by id. Each resample draws one set of days and scores both sides
/// on it, so the interval is on the difference itself.
#[must_use]
pub fn paired(before: &[&Scored], after: &[&Scored], seed: u64, resamples: usize) -> Vec<Paired> {
    let by_id: BTreeMap<&str, &Scored> = after.iter().map(|r| (r.id.as_str(), *r)).collect();
    let mut b_rows: Vec<&Scored> = Vec::new();
    let mut a_rows: Vec<&Scored> = Vec::new();
    for r in before {
        if let Some(a) = by_id.get(r.id.as_str()) {
            b_rows.push(r);
            a_rows.push(a);
        }
    }
    let n = b_rows.len();
    let vb = metrics::compute(&b_rows);
    let va = metrics::compute(&a_rows);
    let blocks = day_blocks(&b_rows);
    let mut deltas: Vec<Vec<f64>> = vec![Vec::with_capacity(resamples); METRICS.len()];
    if !blocks.is_empty() {
        let mut rng = Rng::new(seed);
        for _ in 0..resamples {
            let idx = draw(&blocks, &mut rng);
            let sb: Vec<&Scored> = idx.iter().map(|&i| b_rows[i]).collect();
            let sa: Vec<&Scored> = idx.iter().map(|&i| a_rows[i]).collect();
            let (mb, ma) = (metrics::compute(&sb), metrics::compute(&sa));
            for (k, d) in deltas.iter_mut().enumerate() {
                d.push(ma[k].value - mb[k].value);
            }
        }
    }
    (0..METRICS.len())
        .map(|k| {
            let (lo, hi) = interval(&deltas[k]);
            Paired {
                before: vb[k].value,
                after: va[k].value,
                delta: va[k].value - vb[k].value,
                lo,
                hi,
                n,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(offset: f64) -> Vec<Scored> {
        (0..40)
            .map(|i| Scored {
                id: format!("r{i}"),
                day: (2026, 1, 1 + (i % 8) as u32),
                measured_db: Some(f64::from(i % 7) - 3.0),
                model_db: Some(f64::from(i % 5) + offset),
                p_decode: Some(f64::from(i % 10) / 10.0),
                decoded: Some(i % 3 != 0),
                decodable: Some(i % 2 == 0),
                ..Scored::default()
            })
            .collect()
    }

    #[test]
    fn same_seed_same_intervals() {
        let r = rows(0.0);
        let refs: Vec<&Scored> = r.iter().collect();
        let a = intervals(&refs, 7, 200);
        let b = intervals(&refs, 7, 200);
        // Bit-identical, NaN included.
        let bits = |v: &[(f64, f64)]| -> Vec<(u64, u64)> {
            v.iter().map(|x| (x.0.to_bits(), x.1.to_bits())).collect()
        };
        assert_eq!(bits(&a), bits(&b));
        let c = intervals(&refs, 8, 200);
        assert_ne!(bits(&a), bits(&c));
    }

    #[test]
    fn interval_brackets_the_point_estimate() {
        let r = rows(0.0);
        let refs: Vec<&Scored> = r.iter().collect();
        let iv = intervals(&refs, 1, 400);
        let point = metrics::compute(&refs);
        let k = metrics::index_of("brier").unwrap();
        assert!(iv[k].0 <= point[k].value && point[k].value <= iv[k].1);
    }

    #[test]
    fn a_constant_shift_is_paired_exactly() {
        // The after model reads 2 dB higher on every row, so the paired bias
        // difference is exactly 2 on every resample: a zero-width interval.
        let b = rows(0.0);
        let a = rows(2.0);
        let (rb, ra): (Vec<&Scored>, Vec<&Scored>) = (b.iter().collect(), a.iter().collect());
        let p = paired(&rb, &ra, 3, 100);
        let k = metrics::index_of("snr_bias_mean_none").unwrap();
        assert!((p[k].delta - 2.0).abs() < 1e-12);
        assert!((p[k].lo - 2.0).abs() < 1e-9 && (p[k].hi - 2.0).abs() < 1e-9);
        assert_eq!(p[k].n, 40);
        let again = paired(&rb, &ra, 3, 100);
        let bits = |v: &[Paired]| -> Vec<[u64; 3]> {
            v.iter()
                .map(|x| [x.delta.to_bits(), x.lo.to_bits(), x.hi.to_bits()])
                .collect()
        };
        assert_eq!(bits(&p), bits(&again));
    }

    #[test]
    fn rows_missing_on_one_side_are_left_out_of_the_pairing() {
        let b = rows(0.0);
        let a = rows(0.0);
        let rb: Vec<&Scored> = b.iter().collect();
        let ra: Vec<&Scored> = a.iter().take(30).collect();
        assert_eq!(paired(&rb, &ra, 1, 10)[0].n, 30);
    }
}
