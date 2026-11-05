//! Hold-out splits: by month, by region of the path midpoint, and both.
//!
//! Never random: adjacent spots share an ionosphere, so a random split
//! scores interpolation between near-duplicates
//! (docs/notes/accuracy-roadmap.md §2.4).
//!
//! A fold names the rows it scores (`test`) and the rows anything fitted may
//! see (`train`). Every row is scored exactly once per split, so the pooled
//! test rows of a split are the whole corpus and splits differ only in what
//! a fitted method was allowed to learn from, and in how finely station
//! effects are re-estimated.

use std::collections::BTreeSet;

/// Region of a path, from its midpoint, where the ionosphere being tested
/// sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Region {
    Europe,
    NorthAmerica,
    SouthAmerica,
    Africa,
    Asia,
    Oceania,
    /// Ends in opposite hemispheres, whatever the midpoint.
    TransEquatorial,
}

impl Region {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Europe => "europe",
            Self::NorthAmerica => "north-america",
            Self::SouthAmerica => "south-america",
            Self::Africa => "africa",
            Self::Asia => "asia",
            Self::Oceania => "oceania",
            Self::TransEquatorial => "trans-equatorial",
        }
    }

    /// The region of a path.
    ///
    /// The roadmap names the regions but not their borders, so these are
    /// plain longitude sectors split by latitude, chosen so ocean midpoints
    /// fall to the nearest continent's side:
    ///
    /// | longitude | north of the line | south of it |
    /// |---|---|---|
    /// | 170 W to 30 W | North America (12 N) | South America |
    /// | 30 W to 40 E | Europe (35 N) | Africa |
    /// | 40 E to 170 W | Asia (10 S) | Oceania |
    ///
    /// A path whose two ends lie in opposite hemispheres is trans-equatorial
    /// whatever its midpoint.
    #[must_use]
    pub fn of_path(tx_lat: f64, rx_lat: f64, mid_lat: f64, mid_lon: f64) -> Self {
        if tx_lat * rx_lat < 0.0 {
            return Self::TransEquatorial;
        }
        let lon = (mid_lon + 180.0).rem_euclid(360.0) - 180.0;
        if (-170.0..-30.0).contains(&lon) {
            if mid_lat >= 12.0 {
                Self::NorthAmerica
            } else {
                Self::SouthAmerica
            }
        } else if (-30.0..40.0).contains(&lon) {
            if mid_lat >= 35.0 {
                Self::Europe
            } else {
                Self::Africa
            }
        } else if mid_lat >= -10.0 {
            Self::Asia
        } else {
            Self::Oceania
        }
    }
}

/// Which hold-out a fold belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SplitKind {
    /// Everything scored together, nothing held out. In-sample reference.
    All,
    /// Leave one calendar month out.
    Month,
    /// Leave one midpoint region out.
    Region,
    /// Leave a month and a region out together: the headline cell.
    MonthRegion,
    /// Leave one ionosonde station out.
    Station,
}

impl SplitKind {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Month => "month",
            Self::Region => "region",
            Self::MonthRegion => "month+region",
            Self::Station => "station",
        }
    }

    /// The splits path corpora are scored under, headline first.
    pub const PATH_SPLITS: [Self; 4] = [Self::MonthRegion, Self::Month, Self::Region, Self::All];
}

/// What a row contributes to the split: its calendar month and region.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SplitKey {
    pub month: (i32, u32),
    pub region: Region,
}

/// One fold: rows scored and rows a fitted method may train on, as indices
/// into the corpus.
#[derive(Clone, Debug)]
pub struct Fold {
    pub name: String,
    pub train: Vec<usize>,
    pub test: Vec<usize>,
}

/// Folds for a path corpus.
///
/// For a month fold, training excludes that month. For a region fold, it
/// excludes that region. For the doubly held-out cell (month m, region r)
/// the test rows are those in both, and training excludes every row that
/// shares either, so nothing fitted has seen that month anywhere or that
/// region in any month.
#[must_use]
pub fn path_folds(keys: &[SplitKey], kind: SplitKind) -> Vec<Fold> {
    let all: Vec<usize> = (0..keys.len()).collect();
    let months: BTreeSet<(i32, u32)> = keys.iter().map(|k| k.month).collect();
    let regions: BTreeSet<Region> = keys.iter().map(|k| k.region).collect();
    let pick = |f: &dyn Fn(&SplitKey) -> bool| -> Vec<usize> {
        all.iter().copied().filter(|&i| f(&keys[i])).collect()
    };
    match kind {
        SplitKind::All | SplitKind::Station => vec![Fold {
            name: "all".to_string(),
            train: all.clone(),
            test: all.clone(),
        }],
        SplitKind::Month => months
            .iter()
            .map(|&m| Fold {
                name: format!("{}-{:02}", m.0, m.1),
                train: pick(&|k| k.month != m),
                test: pick(&|k| k.month == m),
            })
            .collect(),
        SplitKind::Region => regions
            .iter()
            .map(|&r| Fold {
                name: r.label().to_string(),
                train: pick(&|k| k.region != r),
                test: pick(&|k| k.region == r),
            })
            .collect(),
        SplitKind::MonthRegion => {
            let mut out = Vec::new();
            for &m in &months {
                for &r in &regions {
                    let test = pick(&|k| k.month == m && k.region == r);
                    if test.is_empty() {
                        continue;
                    }
                    out.push(Fold {
                        name: format!("{}-{:02}/{}", m.0, m.1, r.label()),
                        train: pick(&|k| k.month != m && k.region != r),
                        test,
                    });
                }
            }
            out
        }
    }
}

/// Leave-one-station-out folds for ionosonde rows, so a correction fitted to
/// ionosonde data is never scored at the station that produced it.
#[must_use]
pub fn station_folds(stations: &[String]) -> Vec<Fold> {
    let names: BTreeSet<&String> = stations.iter().collect();
    names
        .into_iter()
        .map(|s| Fold {
            name: s.clone(),
            train: (0..stations.len()).filter(|&i| &stations[i] != s).collect(),
            test: (0..stations.len()).filter(|&i| &stations[i] == s).collect(),
        })
        .collect()
}

/// Something fitted on training rows and applied to test rows. The current
/// model fits nothing, so the benchmark runs [`NoFit`]; a method that fits
/// (roadmap section 3) implements this, and the harness guarantees it never
/// sees a test row.
pub trait Fitter {
    /// Fit on these rows (indices into the corpus).
    fn fit(&mut self, train: &[usize]);
}

/// The fitter for a model with nothing to fit.
pub struct NoFit;

impl Fitter for NoFit {
    fn fit(&mut self, _train: &[usize]) {}
}

/// Run `fitter` on each fold's training rows, then call `score` with the
/// fold. The order (fit, then score) is the whole contract.
pub fn run_folds(folds: &[Fold], fitter: &mut dyn Fitter, mut score: impl FnMut(&Fold)) {
    for f in folds {
        fitter.fit(&f.train);
        score(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<SplitKey> {
        let mut v = Vec::new();
        for m in 1..=3 {
            for r in [Region::Europe, Region::Asia, Region::TransEquatorial] {
                for _ in 0..2 {
                    v.push(SplitKey {
                        month: (2026, m),
                        region: r,
                    });
                }
            }
        }
        v
    }

    /// Records every row it was trained on.
    struct Spy(Vec<Vec<usize>>);
    impl Fitter for Spy {
        fn fit(&mut self, train: &[usize]) {
            self.0.push(train.to_vec());
        }
    }

    #[test]
    fn no_held_out_row_reaches_the_fitter() {
        let k = keys();
        for kind in SplitKind::PATH_SPLITS {
            if kind == SplitKind::All {
                continue;
            }
            let folds = path_folds(&k, kind);
            let mut spy = Spy(Vec::new());
            let mut tested = Vec::new();
            run_folds(&folds, &mut spy, |f| tested.push(f.test.clone()));
            for (train, test) in spy.0.iter().zip(&tested) {
                for &t in test {
                    assert!(!train.contains(&t), "{kind:?}: row {t} leaked");
                    for &r in train {
                        match kind {
                            SplitKind::Month => assert_ne!(k[r].month, k[t].month),
                            SplitKind::Region => assert_ne!(k[r].region, k[t].region),
                            SplitKind::MonthRegion => {
                                assert_ne!(k[r].month, k[t].month);
                                assert_ne!(k[r].region, k[t].region);
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
            // Every row scored exactly once per split.
            let mut all: Vec<usize> = tested.concat();
            all.sort_unstable();
            assert_eq!(all, (0..k.len()).collect::<Vec<_>>(), "{kind:?}");
        }
    }

    #[test]
    fn doubly_held_out_cells_are_month_by_region() {
        let folds = path_folds(&keys(), SplitKind::MonthRegion);
        assert_eq!(folds.len(), 9);
        let f = &folds[0];
        assert_eq!(f.test.len(), 2);
        // 2 other months x 2 other regions x 2 rows.
        assert_eq!(f.train.len(), 8);
    }

    #[test]
    fn regions_follow_the_midpoint_and_the_equator() {
        assert_eq!(Region::of_path(51.0, 40.0, 50.0, 10.0), Region::Europe);
        assert_eq!(
            Region::of_path(40.0, 45.0, 52.0, -40.0),
            Region::NorthAmerica
        );
        assert_eq!(
            Region::of_path(-30.0, -20.0, -25.0, -50.0),
            Region::SouthAmerica
        );
        assert_eq!(Region::of_path(10.0, 20.0, 5.0, 20.0), Region::Africa);
        assert_eq!(Region::of_path(35.0, 40.0, 38.0, 120.0), Region::Asia);
        assert_eq!(Region::of_path(-33.0, -40.0, -36.0, 160.0), Region::Oceania);
        assert_eq!(
            Region::of_path(51.0, -33.0, 10.0, 20.0),
            Region::TransEquatorial
        );
        // Longitude wraps.
        assert_eq!(Region::of_path(-20.0, -30.0, -25.0, 185.0), Region::Oceania);
    }

    #[test]
    fn station_folds_hold_each_station_out() {
        let s: Vec<String> = ["A", "B", "A", "C"].iter().map(|x| x.to_string()).collect();
        let folds = station_folds(&s);
        assert_eq!(folds.len(), 3);
        assert_eq!(folds[0].test, vec![0, 2]);
        assert_eq!(folds[0].train, vec![1, 3]);
    }
}
