//! IGRF-14: the International Geomagnetic Reference Field, 14th generation,
//! evaluated from its defining Schmidt semi-normalised spherical-harmonic
//! expansion (docs/derivations/magnetic-field.md sections 1-3).
//!
//! Coefficient source: `data/igrf14coeffs.txt`, the published IAGA V-MOD
//! working group file (fetched from NOAA NCEI), embedded verbatim. Main-field
//! epochs 1900.0..2025.0 in 5-year steps, linear interpolation between
//! epochs, linear secular-variation extrapolation for 2025.0..2030.0 - this
//! time behaviour is part of the IGRF definition, not a modelling choice here.
//!
//! Not valid at the coordinate poles (B_phi terms divide by sin theta); the
//! tracer's pole guard is the enforcement point.

use super::legendre::{NMAX, TABLE_LEN, idx, schmidt};
use super::{FieldSample, MagneticField};
use crate::geo::SphericalPoint;
use thiserror::Error;

/// Reference radius of the IGRF expansion, m. Part of the model definition.
pub const IGRF_REFERENCE_RADIUS: f64 = 6_371_200.0;

const COEFFS_TXT: &str = include_str!("../../data/igrf14coeffs.txt");
const FIRST_EPOCH: f64 = 1900.0;
const LAST_EPOCH: f64 = 2025.0;
const EPOCH_STEP: f64 = 5.0;
const N_EPOCHS: usize = 26;
/// Years past the last main-field epoch for which the published secular
/// variation defines the model (the IGRF is prospective for one 5-year cycle).
const SV_HORIZON: f64 = 5.0;
const NANOTESLA: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum IgrfError {
    /// The embedded coefficient file failed structural validation. This can
    /// only happen if the data file is replaced with a malformed one.
    #[error("IGRF coefficient file malformed at line {line}: {reason}")]
    Malformed { line: usize, reason: &'static str },

    /// Requested epoch is outside the defined range 1900.0..=2030.0.
    #[error("epoch {epoch} outside IGRF-14 validity [{FIRST_EPOCH}, {}]", LAST_EPOCH + SV_HORIZON)]
    EpochOutOfRange { epoch: f64 },
}

/// The full coefficient set: all main-field epochs plus secular variation.
pub struct Igrf {
    /// `g[e][idx(n,m)]`, tesla, per epoch.
    g: Vec<[f64; TABLE_LEN]>,
    h: Vec<[f64; TABLE_LEN]>,
    /// Secular variation, tesla per year, valid after the last epoch.
    sv_g: [f64; TABLE_LEN],
    sv_h: [f64; TABLE_LEN],
}

impl Igrf {
    /// # Errors
    /// `IgrfError::Malformed` if the embedded coefficient file fails
    /// structural validation (possible only if the data file is replaced).
    pub fn from_embedded() -> Result<Self, IgrfError> {
        Self::parse(COEFFS_TXT)
    }

    fn parse(text: &str) -> Result<Self, IgrfError> {
        let mut igrf = Self {
            g: vec![[0.0; TABLE_LEN]; N_EPOCHS],
            h: vec![[0.0; TABLE_LEN]; N_EPOCHS],
            sv_g: [0.0; TABLE_LEN],
            sv_h: [0.0; TABLE_LEN],
        };
        let mut rows = 0usize;
        let mut epochs_checked = false;
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut tok = line.split_whitespace();
            let first = tok.next().unwrap_or("");
            if first == "c/s" {
                continue;
            }
            let err = |reason| IgrfError::Malformed {
                line: lineno + 1,
                reason,
            };
            if first == "g/h" {
                // Header row carries the epoch years; validate our constants
                // against the file instead of trusting them.
                tok.next();
                tok.next();
                let years: Vec<f64> = tok.filter_map(|t| t.parse().ok()).collect();
                if years.len() != N_EPOCHS {
                    return Err(err("unexpected number of epoch columns"));
                }
                for (i, y) in years.iter().enumerate() {
                    #[allow(clippy::cast_precision_loss)]
                    let want = FIRST_EPOCH + EPOCH_STEP * i as f64;
                    if (y - want).abs() > 1e-9 {
                        return Err(err("epoch grid is not 1900:5:2025"));
                    }
                }
                epochs_checked = true;
                continue;
            }
            let is_g = match first {
                "g" => true,
                "h" => false,
                _ => return Err(err("row must start with g or h")),
            };
            let n: usize = tok
                .next()
                .and_then(|t| t.parse().ok())
                .ok_or_else(|| err("bad degree"))?;
            let m: usize = tok
                .next()
                .and_then(|t| t.parse().ok())
                .ok_or_else(|| err("bad order"))?;
            if n == 0 || n > NMAX || m > n {
                return Err(err("degree/order out of range"));
            }
            let vals: Result<Vec<f64>, _> = tok.map(str::parse::<f64>).collect();
            let vals = vals.map_err(|_| err("non-numeric coefficient"))?;
            if vals.len() != N_EPOCHS + 1 {
                return Err(err("wrong number of value columns"));
            }
            let i = idx(n, m);
            for (e, v) in vals[..N_EPOCHS].iter().enumerate() {
                if is_g {
                    igrf.g[e][i] = v * NANOTESLA;
                } else {
                    igrf.h[e][i] = v * NANOTESLA;
                }
            }
            if is_g {
                igrf.sv_g[i] = vals[N_EPOCHS] * NANOTESLA;
            } else {
                igrf.sv_h[i] = vals[N_EPOCHS] * NANOTESLA;
            }
            rows += 1;
        }
        // g rows: one per (n,m) with m=0..=n; h rows only for m >= 1;
        // total sum over n of (2n+1) = NMAX(NMAX+2) = 195.
        let expect = NMAX * (NMAX + 2);
        if !epochs_checked {
            return Err(IgrfError::Malformed {
                line: 0,
                reason: "missing epoch header row",
            });
        }
        if rows != expect {
            return Err(IgrfError::Malformed {
                line: 0,
                reason: "wrong number of coefficient rows",
            });
        }
        Ok(igrf)
    }

    /// Coefficients interpolated to `epoch` (decimal year), per the IGRF
    /// definition: piecewise-linear between 5-year epochs, secular-variation
    /// extrapolation for the final prospective interval.
    ///
    /// # Errors
    /// `IgrfError::EpochOutOfRange` outside 1900.0..=2030.0.
    pub fn model_at(&self, epoch: f64) -> Result<IgrfModel, IgrfError> {
        if !epoch.is_finite() || !(FIRST_EPOCH..=LAST_EPOCH + SV_HORIZON).contains(&epoch) {
            return Err(IgrfError::EpochOutOfRange { epoch });
        }
        let mut m = IgrfModel {
            g: [0.0; TABLE_LEN],
            h: [0.0; TABLE_LEN],
        };
        if epoch >= LAST_EPOCH {
            let dt = epoch - LAST_EPOCH;
            for i in 0..TABLE_LEN {
                m.g[i] = self.g[N_EPOCHS - 1][i] + dt * self.sv_g[i];
                m.h[i] = self.h[N_EPOCHS - 1][i] + dt * self.sv_h[i];
            }
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let k = (((epoch - FIRST_EPOCH) / EPOCH_STEP) as usize).min(N_EPOCHS - 2);
            #[allow(clippy::cast_precision_loss)]
            let w = (epoch - (FIRST_EPOCH + EPOCH_STEP * k as f64)) / EPOCH_STEP;
            for i in 0..TABLE_LEN {
                m.g[i] = self.g[k][i] * (1.0 - w) + self.g[k + 1][i] * w;
                m.h[i] = self.h[k][i] * (1.0 - w) + self.h[k + 1][i] * w;
            }
        }
        Ok(m)
    }
}

/// A single-epoch coefficient set, ready for evaluation.
pub struct IgrfModel {
    g: [f64; TABLE_LEN],
    h: [f64; TABLE_LEN],
}

impl MagneticField for IgrfModel {
    fn sample(&self, p: &SphericalPoint) -> FieldSample {
        let r = p.r.get();
        let theta = p.colat.get();
        let t = schmidt(theta);
        let (st, ct) = theta.sin_cos();
        let (sp, cp) = p.lon.get().sin_cos();
        // cos(m phi), sin(m phi) by the angle-addition recurrence (exact
        // identity; error growth over m <= 13 is a few ulp).
        let mut cm = [0.0; NMAX + 1];
        let mut sm = [0.0; NMAX + 1];
        cm[0] = 1.0;
        for m in 1..=NMAX {
            cm[m] = cm[m - 1] * cp - sm[m - 1] * sp;
            sm[m] = sm[m - 1] * cp + cm[m - 1] * sp;
        }

        let ar = IGRF_REFERENCE_RADIUS / r;
        let mut b = [0.0; 3];
        let mut db = [[0.0; 3]; 3];
        // rn = (a/r)^{n+2}; the derivation's component formulas, with the
        // radial derivative multiplying each degree-n term by -(n+2)/r.
        let mut rn = ar * ar * ar;
        for n in 1..=NMAX {
            #[allow(clippy::cast_precision_loss)]
            let np1 = (n + 1) as f64;
            #[allow(clippy::cast_precision_loss)]
            let dfr = -((n + 2) as f64) / r;
            for m in 0..=n {
                let i = idx(n, m);
                let (gg, hh) = (self.g[i], self.h[i]);
                if gg == 0.0 && hh == 0.0 {
                    continue;
                }
                let e = gg * cm[m] + hh * sm[m];
                let o = gg * sm[m] - hh * cm[m];
                #[allow(clippy::cast_precision_loss)]
                let mf = m as f64;
                let (pp, dp, d2p) = (t.p[i], t.dp[i], t.d2p[i]);

                let br_t = np1 * rn * e * pp;
                b[0] += br_t;
                db[0][0] += dfr * br_t;
                db[0][1] += np1 * rn * e * dp;
                db[0][2] += np1 * rn * (-mf * o) * pp;

                let bt_t = -rn * e * dp;
                b[1] += bt_t;
                db[1][0] += dfr * bt_t;
                db[1][1] += -rn * e * d2p;
                db[1][2] += rn * mf * o * dp;

                if m > 0 {
                    let pos = pp / st;
                    let bp_t = rn * mf * o * pos;
                    b[2] += bp_t;
                    db[2][0] += dfr * bp_t;
                    db[2][1] += rn * mf * o * (dp / st - pp * ct / (st * st));
                    db[2][2] += rn * mf * mf * e * pos;
                }
            }
            rn *= ar;
        }
        FieldSample { b, db }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mag::{Dipole, div_curl};
    use crate::units::{Meters, Radians, Tesla};

    fn points() -> Vec<SphericalPoint> {
        let mut v = Vec::new();
        for &rk in &[6371.2, 6521.2, 6871.2] {
            for &colat in &[0.4, 1.0, 1.6, 2.1, 2.7] {
                for &lon in &[0.1, 1.3, 2.9, 4.4, 6.0] {
                    v.push(SphericalPoint::new(
                        Meters::from_km(rk),
                        Radians::new(colat),
                        Radians::new(lon),
                    ));
                }
            }
        }
        v
    }

    #[test]
    fn parses_and_field_is_potential() {
        let model = Igrf::from_embedded().unwrap().model_at(2026.5).unwrap();
        for p in points() {
            let s = model.sample(&p);
            let (div, curl) = div_curl(&p, &s);
            let scale = s.b.iter().map(|c| c.abs()).fold(0.0, f64::max) / p.r.get();
            assert!(div.abs() < 1e-10 * scale, "div {div:e} at {p:?}");
            for c in curl {
                assert!(c.abs() < 1e-10 * scale, "curl {c:e} at {p:?}");
            }
        }
    }

    #[test]
    fn jacobian_matches_finite_differences() {
        let model = Igrf::from_embedded().unwrap().model_at(2020.0).unwrap();
        for p in points() {
            let s = model.sample(&p);
            for (j, h) in [(0usize, 5.0), (1, 1e-6), (2, 1e-6)] {
                let mut pp = p;
                let mut pm = p;
                match j {
                    0 => {
                        pp.r = Meters::new(p.r.get() + h);
                        pm.r = Meters::new(p.r.get() - h);
                    }
                    1 => {
                        pp.colat = Radians::new(p.colat.get() + h);
                        pm.colat = Radians::new(p.colat.get() - h);
                    }
                    _ => {
                        pp.lon = Radians::new(p.lon.get() + h);
                        pm.lon = Radians::new(p.lon.get() - h);
                    }
                }
                let (sp, sm) = (model.sample(&pp), model.sample(&pm));
                for i in 0..3 {
                    let fd = (sp.b[i] - sm.b[i]) / (2.0 * h);
                    assert!(
                        (s.db[i][j] - fd).abs() < 1e-6 * fd.abs().max(1e-9),
                        "db[{i}][{j}] {} vs {fd} at {p:?}",
                        s.db[i][j]
                    );
                }
            }
        }
    }

    /// Truncating IGRF to degree 1 must reproduce the analytic tilted dipole
    /// exactly (same expansion, closed form vs recurrence).
    #[test]
    fn degree_one_truncation_matches_analytic_dipole() {
        let igrf = Igrf::from_embedded().unwrap();
        let full = igrf.model_at(2010.0).unwrap();
        let mut trunc = IgrfModel {
            g: [0.0; TABLE_LEN],
            h: [0.0; TABLE_LEN],
        };
        for m in 0..=1 {
            trunc.g[idx(1, m)] = full.g[idx(1, m)];
            trunc.h[idx(1, m)] = full.h[idx(1, m)];
        }
        let dip = Dipole::new(
            Meters::new(IGRF_REFERENCE_RADIUS),
            Tesla::new(full.g[idx(1, 0)]),
            Tesla::new(full.g[idx(1, 1)]),
            Tesla::new(full.h[idx(1, 1)]),
        );
        for p in points() {
            let a = trunc.sample(&p);
            let d = dip.sample(&p);
            for i in 0..3 {
                assert!((a.b[i] - d.b[i]).abs() < 1e-13 * a.b[i].abs().max(1e-9));
                for j in 0..3 {
                    // Absolute floor 1e-19: the two paths compute e.g.
                    // db[2][1] as algebraically-zero expressions that differ
                    // by rounding residue up to ~5e-21 T/rad at unlucky
                    // colatitudes; a real term/sign error shows up at the
                    // entry scale (>= ~5e-12 for the smallest, radial,
                    // entries), seven orders above the floor.
                    assert!(
                        (a.db[i][j] - d.db[i][j]).abs() < 1e-12 * a.db[i][j].abs() + 1e-19,
                        "db[{i}][{j}]"
                    );
                }
            }
        }
    }

    /// The defined time behaviour: linear between epochs, linear SV after the
    /// last epoch. Field values are linear in the coefficients, so midpoints
    /// must average exactly.
    #[test]
    fn epoch_interpolation_is_piecewise_linear() {
        let igrf = Igrf::from_embedded().unwrap();
        let p = SphericalPoint::new(
            Meters::from_km(6471.2),
            Radians::new(1.1),
            Radians::new(0.6),
        );
        let b = |e: f64| igrf.model_at(e).unwrap().sample(&p).b;
        let (b0, bm, b1) = (b(1980.0), b(1982.5), b(1985.0));
        let (c0, cm, c1) = (b(2025.0), b(2027.5), b(2030.0));
        // Bound: both sides are the same linear combination up to f64
        // rounding, ~1 ulp of a 1e-5 T field = 1e-21; allow a margin.
        for i in 0..3 {
            assert!((bm[i] - 0.5 * (b0[i] + b1[i])).abs() < 5e-20);
            assert!((cm[i] - 0.5 * (c0[i] + c1[i])).abs() < 5e-20);
        }
        assert!(igrf.model_at(1899.9).is_err());
        assert!(igrf.model_at(2030.1).is_err());
        assert!(igrf.model_at(f64::NAN).is_err());
    }

    /// Sanity anchor (not validation): surface field magnitude everywhere in
    /// the observed range of Earth's field, roughly 20-70 microtesla.
    #[test]
    fn surface_magnitude_in_observed_range() {
        let model = Igrf::from_embedded().unwrap().model_at(2025.0).unwrap();
        for p in points() {
            if (p.r.get() - 6_371_200.0).abs() > 1.0 {
                continue;
            }
            let s = model.sample(&p);
            let mag = s.b.iter().map(|c| c * c).sum::<f64>().sqrt();
            assert!((2.0e-5..7.0e-5).contains(&mag), "|B| = {mag} at {p:?}");
        }
    }
}
