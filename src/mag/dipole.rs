//! Centered tilted dipole: the n = 1 spherical-harmonic field in closed form.
//! Derivation: docs/derivations/magnetic-field.md section 4.

use super::{FieldSample, MagneticField};
use crate::geo::SphericalPoint;
use crate::units::{Meters, Tesla};

pub struct Dipole {
    /// Reference radius a of the expansion, m.
    a: f64,
    /// First-degree Gauss coefficients, tesla.
    g10: f64,
    g11: f64,
    h11: f64,
}

impl Dipole {
    #[must_use]
    pub const fn new(reference_radius: Meters, g10: Tesla, g11: Tesla, h11: Tesla) -> Self {
        Self {
            a: reference_radius.get(),
            g10: g10.get(),
            g11: g11.get(),
            h11: h11.get(),
        }
    }

    /// Axis-aligned dipole (g11 = h11 = 0). Earth-like fields have g10 < 0.
    #[must_use]
    pub const fn axial(reference_radius: Meters, g10: Tesla) -> Self {
        Self::new(reference_radius, g10, Tesla::new(0.0), Tesla::new(0.0))
    }
}

impl MagneticField for Dipole {
    fn sample(&self, p: &SphericalPoint) -> FieldSample {
        let r = p.r.get();
        let (st, ct) = p.colat.get().sin_cos();
        let (sp, cp) = p.lon.get().sin_cos();
        let q = (self.a / r).powi(3);
        // G(phi) = g11 cos + h11 sin; dG/dphi = -(g11 sin - h11 cos) = -o
        let g = self.g11 * cp + self.h11 * sp;
        let o = self.g11 * sp - self.h11 * cp;

        let br = 2.0 * q * (self.g10 * ct + g * st);
        let bt = q * (self.g10 * st - g * ct);
        let bp = q * o;

        // Every term carries (a/r)^3, so d/dr = -3/r x component.
        let m3r = -3.0 / r;
        FieldSample {
            b: [br, bt, bp],
            db: [
                [
                    m3r * br,
                    2.0 * q * (-self.g10 * st + g * ct),
                    2.0 * q * st * (-o),
                ],
                [m3r * bt, q * (self.g10 * ct + g * st), q * ct * o],
                [m3r * bp, 0.0, q * g],
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mag::div_curl;
    use crate::units::Radians;

    fn sample_points() -> Vec<SphericalPoint> {
        let mut v = Vec::new();
        for &rk in &[6371.2, 6671.2, 7371.2] {
            for &colat in &[0.3, 1.1, std::f64::consts::FRAC_PI_2, 2.2, 2.9] {
                for &lon in &[0.0, 0.7, 3.0, 5.5] {
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
    fn axial_magnitude_matches_closed_form() {
        // |B| = (a/r)^3 |g10| sqrt(1 + 3 cos^2 theta), derivation section 4.
        let a = Meters::from_km(6371.2);
        let d = Dipole::axial(a, Tesla::new(-3.0e-5));
        for p in sample_points() {
            let s = d.sample(&p);
            let mag = s.b.iter().map(|c| c * c).sum::<f64>().sqrt();
            let x = p.colat.get().cos();
            let want = (a.get() / p.r.get()).powi(3) * 3.0e-5 * (1.0 + 3.0 * x * x).sqrt();
            assert!((mag - want).abs() < 1e-18 + 1e-14 * want);
            // Northern hemisphere: field points downward (B_r < 0) for g10 < 0.
            if x > 0.1 {
                assert!(s.b[0] < 0.0);
            }
        }
    }

    #[test]
    fn tilted_dipole_is_divergence_and_curl_free() {
        let d = Dipole::new(
            Meters::from_km(6371.2),
            Tesla::new(-2.94e-5),
            Tesla::new(-1.4e-6),
            Tesla::new(4.5e-6),
        );
        for p in sample_points() {
            let s = d.sample(&p);
            let (div, curl) = div_curl(&p, &s);
            let scale = s.b.iter().map(|c| c.abs()).fold(0.0, f64::max) / p.r.get();
            assert!(div.abs() < 1e-12 * scale, "div {div:e} at {p:?}");
            for c in curl {
                assert!(c.abs() < 1e-12 * scale, "curl {c:e} at {p:?}");
            }
        }
    }

    /// Tolerance form 1e-7*(|fd| + 1e-12/h): relative h^2 truncation plus an
    /// absolute floor covering eps*|B|/h central-difference roundoff for
    /// entries that are exactly zero analytically.
    #[test]
    fn jacobian_matches_finite_differences() {
        let d = Dipole::new(
            Meters::from_km(6371.2),
            Tesla::new(-2.94e-5),
            Tesla::new(-1.4e-6),
            Tesla::new(4.5e-6),
        );
        for p in sample_points() {
            let s = d.sample(&p);
            let hr = 10.0; // m
            let ha = 1e-6; // rad
            for (j, h) in [(0usize, hr), (1, ha), (2, ha)] {
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
                let sp = d.sample(&pp);
                let sm = d.sample(&pm);
                for i in 0..3 {
                    let fd = (sp.b[i] - sm.b[i]) / (2.0 * h);
                    let tol = 1e-7 * (fd.abs() + 1e-12 / h);
                    assert!(
                        (s.db[i][j] - fd).abs() < tol.max(1e-20),
                        "db[{i}][{j}] {} vs {fd}",
                        s.db[i][j]
                    );
                }
            }
        }
    }
}
