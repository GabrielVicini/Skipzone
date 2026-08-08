//! Day-level jackknife: refit with each day held out, to see how much any one
//! day is carrying the answer.

use crate::solving::*;

use skipzone_app::fit::{self, Cached};
use skipzone_app::wspr::WSPR_DECODE_THRESHOLD_DB;
/// Leave-one-day-out refits, as the interval that actually matters.
pub(crate) struct Jackknife {
    pub(crate) n: usize,
    pub(crate) min: f64,
    pub(crate) max: f64,
}

pub(crate) fn day_jackknife(set: &Solved, rounds: usize, negatives: &[Cached]) -> Jackknife {
    let days: Vec<(i32, u32, u32)> = {
        let mut d: Vec<_> = set.spots.iter().map(|c| c.date).collect();
        d.sort_unstable();
        d.dedup();
        d
    };
    let mut values = Vec::new();
    for drop in &days {
        let kept: Vec<Cached> = set
            .spots
            .iter()
            .filter(|c| c.date != *drop)
            .cloned()
            .collect();
        if kept.len() < 50 {
            continue;
        }
        // The negatives are dropped for the same day, so each refit sees one
        // consistent ionosphere's worth of evidence on both sides.
        let kept_negatives: Vec<Cached> = negatives
            .iter()
            .filter(|c| c.date != *drop)
            .cloned()
            .collect();
        let (p, _, _) = fit::fit_cached(
            &kept,
            set.tx_names.len(),
            set.rx_names.len(),
            rounds,
            fit::Negatives::balanced(&kept_negatives, WSPR_DECODE_THRESHOLD_DB, kept.len()),
        );
        values.push(p.absorption_scale.value);
    }
    Jackknife {
        n: values.len(),
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}
