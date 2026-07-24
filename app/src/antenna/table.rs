//! EXTENSION POINT: antennas whose pattern is measured or NEC-modelled data
//! rather than a formula.
//!
//! A Yagi, log-periodic, multi-band trap dipole, random wire or loop cannot be
//! written as a closed form the way the types in [`super::image`] can. Their
//! patterns depend on element lengths, spacings, trap resonances and feed
//! geometry, and the honest way to carry them is as tabulated gain against
//! elevation, per frequency and per height, from a NEC run or a measurement.
//!
//! This module exists so that adding such a type does not require touching the
//! solver, the link budget, or any UI code beyond the picker. It is deliberately
//! NOT wired up: no data ships with the app, and no table-backed antenna appears
//! in [`super::AntennaKind`] yet.
//!
//! # Adding one
//!
//! 1. Get the data: a set of `(frequency, height, [(elevation, gain)])` blocks,
//!    from NEC or a measurement, with the ground model recorded.
//! 2. Build a [`TabulatedPattern`] from them, most likely deserialised from a
//!    file under `data/antennas/` and loaded once at startup.
//! 3. Add an [`super::AntennaKind`] variant and return the pattern from
//!    [`super::AntennaConfig::build`].
//! 4. Write the type's [`super::ElevationPattern::provenance`] to say where the
//!    data came from and what it excludes - the same standard the closed-form
//!    types are held to.
//!
//! # Interpolation
//!
//! Bilinear in `(frequency, height)` between the nearest bracketing blocks, and
//! linear in elevation within a block, all in dB. Outside the tabulated range
//! the nearest block is used unchanged and [`TabulatedPattern::in_range`]
//! reports false, so the UI can say the pattern is being extrapolated rather
//! than quietly presenting an edge value as if it were data. Real antenna data
//! is sampled far too coarsely in height and frequency for anything fancier to
//! be honest.

// Unused until the first table-backed antenna ships. That is the point of the
// module: the machinery is in place and tested, so adding one is a data file
// plus an `AntennaKind` variant, not a redesign.
#![allow(dead_code)]

use super::{ElevationPattern, GainCurve};

/// One tabulated pattern: gain against elevation at a single frequency and
/// height.
#[derive(Clone)]
pub struct PatternBlock {
    pub freq_hz: f64,
    pub height_m: f64,
    /// `(elevation [deg], gain [dBi])`, ascending in elevation. Need not be
    /// evenly spaced; it is resampled onto the [`GainCurve`] grid.
    pub samples: Vec<(f64, f64)>,
}

impl PatternBlock {
    /// Gain [dBi] at `elev_deg`, linearly interpolated between samples and
    /// clamped to the ends of the tabulated range.
    #[must_use]
    pub fn gain_dbi(&self, elev_deg: f64) -> f64 {
        let s = &self.samples;
        match s.len() {
            0 => super::GAIN_FLOOR_DBI,
            1 => s[0].1,
            _ => {
                if elev_deg <= s[0].0 {
                    return s[0].1;
                }
                if elev_deg >= s[s.len() - 1].0 {
                    return s[s.len() - 1].1;
                }
                let i = s.partition_point(|&(e, _)| e < elev_deg).max(1) - 1;
                let (e0, g0) = s[i];
                let (e1, g1) = s[i + 1];
                let t = if (e1 - e0).abs() < 1e-12 {
                    0.0
                } else {
                    (elev_deg - e0) / (e1 - e0)
                };
                g0 * (1.0 - t) + g1 * t
            }
        }
    }
}

/// An antenna described by tabulated pattern data.
pub struct TabulatedPattern {
    pub name: String,
    /// Where the data came from, surfaced in the UI verbatim.
    pub provenance: &'static str,
    /// At least one block. More blocks in frequency and height give a better
    /// interpolation; a single block is treated as valid everywhere with
    /// [`Self::in_range`] false off its own point.
    pub blocks: Vec<PatternBlock>,
}

impl TabulatedPattern {
    /// Is `(freq_hz, height_m)` inside the tabulated range, or is the result an
    /// extrapolation from the nearest block?
    #[must_use]
    pub fn in_range(&self, freq_hz: f64, height_m: f64) -> bool {
        let within = |get: fn(&PatternBlock) -> f64, v: f64| {
            let lo = self.blocks.iter().map(get).fold(f64::INFINITY, f64::min);
            let hi = self
                .blocks
                .iter()
                .map(get)
                .fold(f64::NEG_INFINITY, f64::max);
            (lo..=hi).contains(&v)
        };
        !self.blocks.is_empty()
            && within(|b| b.freq_hz, freq_hz)
            && within(|b| b.height_m, height_m)
    }

    /// Inverse-distance blend of the blocks nearest in `(log frequency, height)`.
    ///
    /// Frequency is weighted logarithmically because pattern shape tracks
    /// height in WAVELENGTHS, which is a ratio, not a difference.
    fn blend(&self, freq_hz: f64, height_m: f64, elev_deg: f64) -> f64 {
        if self.blocks.is_empty() {
            return super::GAIN_FLOOR_DBI;
        }
        let mut num = 0.0;
        let mut den = 0.0;
        for b in &self.blocks {
            let df = (freq_hz.max(1.0).ln() - b.freq_hz.max(1.0).ln()) * 4.0;
            let dh = (height_m - b.height_m) / 5.0;
            let d2 = df * df + dh * dh;
            if d2 < 1e-12 {
                return b.gain_dbi(elev_deg);
            }
            let w = 1.0 / d2;
            num += w * b.gain_dbi(elev_deg);
            den += w;
        }
        num / den
    }
}

impl ElevationPattern for TabulatedPattern {
    fn label(&self, height_m: f64, freq_hz: f64) -> String {
        let tag = if self.in_range(freq_hz, height_m) {
            ""
        } else {
            " [EXTRAPOLATED]"
        };
        format!("{} at {height_m:.1} m{tag}", self.name)
    }

    fn provenance(&self) -> &'static str {
        self.provenance
    }

    fn curve(&self, height_m: f64, freq_hz: f64) -> GainCurve {
        GainCurve::from_fn(self.label(height_m, freq_hz), |elev_rad| {
            self.blend(freq_hz, height_m, elev_rad.to_degrees())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(freq_hz: f64, height_m: f64, samples: &[(f64, f64)]) -> PatternBlock {
        PatternBlock {
            freq_hz,
            height_m,
            samples: samples.to_vec(),
        }
    }

    /// A table-backed antenna must satisfy exactly the same trait as a
    /// closed-form one, so the solver cannot tell them apart. That is the whole
    /// point of the extension point.
    #[test]
    fn a_tabulated_antenna_is_just_an_elevation_pattern() {
        let ant = TabulatedPattern {
            name: "3-element Yagi (test fixture)".to_string(),
            provenance: "Synthetic fixture. Not real antenna data.",
            blocks: vec![block(
                14.1e6,
                15.0,
                &[
                    (0.0, -10.0),
                    (10.0, 8.0),
                    (20.0, 11.0),
                    (45.0, 4.0),
                    (90.0, -6.0),
                ],
            )],
        };
        let dynamic: &dyn ElevationPattern = &ant;
        // Interpolates between tabulated points.
        assert!((dynamic.gain_dbi(f64::to_radians(15.0), 15.0, 14.1e6) - 9.5).abs() < 1e-6);
        // Lands on them exactly.
        assert!((dynamic.gain_dbi(f64::to_radians(20.0), 15.0, 14.1e6) - 11.0).abs() < 1e-6);
        let (g, deg) = dynamic.curve(15.0, 14.1e6).peak();
        assert!((g - 11.0).abs() < 1e-6 && (deg - 20.0).abs() < 0.6);
    }

    /// Off the tabulated range the pattern still answers, but says it is
    /// extrapolating rather than passing an edge value off as data.
    #[test]
    fn extrapolation_is_declared() {
        let ant = TabulatedPattern {
            name: "fixture".to_string(),
            provenance: "Synthetic fixture.",
            blocks: vec![
                block(14.0e6, 10.0, &[(0.0, 0.0), (90.0, 0.0)]),
                block(21.0e6, 10.0, &[(0.0, 3.0), (90.0, 3.0)]),
            ],
        };
        assert!(ant.in_range(18.0e6, 10.0));
        assert!(!ant.in_range(50.0e6, 10.0));
        assert!(!ant.in_range(18.0e6, 40.0));
        assert!(ant.label(10.0, 18.0e6).contains("18") || !ant.label(10.0, 18.0e6).is_empty());
        assert!(ant.label(40.0, 18.0e6).contains("EXTRAPOLATED"));
        assert!(!ant.label(10.0, 18.0e6).contains("EXTRAPOLATED"));
        // Blending between the two blocks lands between their values.
        let mid = ant.gain_dbi(f64::to_radians(20.0), 10.0, 17.1e6);
        assert!((0.0..3.0).contains(&mid), "blended gain {mid}");
    }

    /// Elevation interpolation handles the ends and unevenly spaced samples.
    #[test]
    fn block_interpolation_clamps_and_handles_uneven_spacing() {
        let b = block(7.0e6, 10.0, &[(5.0, 0.0), (7.0, 4.0), (50.0, 4.0)]);
        assert!(
            (b.gain_dbi(0.0) - 0.0).abs() < 1e-9,
            "below the range clamps"
        );
        assert!(
            (b.gain_dbi(90.0) - 4.0).abs() < 1e-9,
            "above the range clamps"
        );
        assert!(
            (b.gain_dbi(6.0) - 2.0).abs() < 1e-9,
            "midway in a short interval"
        );
        assert!(
            (b.gain_dbi(28.5) - 4.0).abs() < 1e-9,
            "flat interval stays flat"
        );
    }
}
