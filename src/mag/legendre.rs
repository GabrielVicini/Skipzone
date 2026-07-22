//! Schmidt semi-normalised associated Legendre functions `S_n^m(theta)` with
//! first and second theta-derivatives, computed by the recurrences derived in
//! docs/derivations/magnetic-field.md section 3.
//!
//! Geomagnetic convention: no Condon-Shortley phase. Valid for all theta, but
//! callers dividing by sin(theta) (the B_phi terms) must stay away from the
//! coordinate poles.

/// Maximum spherical-harmonic degree carried by the crate (IGRF-14 truncates
/// at 13; nothing here needs more).
pub const NMAX: usize = 13;

/// Triangular table length for degrees 0..=NMAX.
pub const TABLE_LEN: usize = (NMAX + 1) * (NMAX + 2) / 2;

/// Index of (n, m) in the triangular tables, m <= n <= NMAX.
#[inline]
#[must_use]
pub const fn idx(n: usize, m: usize) -> usize {
    n * (n + 1) / 2 + m
}

/// `S_n^m`, `dS/dtheta`, `d2S/dtheta2` for all n <= NMAX, m <= n.
pub struct SchmidtTable {
    pub p: [f64; TABLE_LEN],
    pub dp: [f64; TABLE_LEN],
    pub d2p: [f64; TABLE_LEN],
}

#[must_use]
#[allow(clippy::cast_precision_loss)] // n, m <= 13: exact in f64
pub fn schmidt(theta: f64) -> SchmidtTable {
    let mut t = SchmidtTable {
        p: [0.0; TABLE_LEN],
        dp: [0.0; TABLE_LEN],
        d2p: [0.0; TABLE_LEN],
    };
    let (s, x) = theta.sin_cos();

    t.p[idx(0, 0)] = 1.0;
    // Diagonal: S_m^m = alpha_m sin(theta) S_{m-1}^{m-1}, alpha_1 = 1,
    // alpha_m = sqrt((2m-1)/(2m)); derivatives from the product rule.
    for m in 1..=NMAX {
        let alpha = if m == 1 {
            1.0
        } else {
            ((2 * m - 1) as f64 / (2 * m) as f64).sqrt()
        };
        let i = idx(m, m);
        let j = idx(m - 1, m - 1);
        t.p[i] = alpha * s * t.p[j];
        t.dp[i] = alpha * (x * t.p[j] + s * t.dp[j]);
        t.d2p[i] = alpha * (-s * t.p[j] + 2.0 * x * t.dp[j] + s * t.d2p[j]);
    }
    // Vertical: S_n^m = [(2n-1) cos(theta) S_{n-1}^m - beta S_{n-2}^m]/gamma,
    // beta = sqrt((n-1)^2 - m^2) (zero when n = m+1, so the missing
    // S_{m-1}^m never contributes), gamma = sqrt(n^2 - m^2).
    for m in 0..=NMAX {
        for n in (m + 1)..=NMAX {
            let a = (2 * n - 1) as f64;
            let beta = (((n - 1) * (n - 1)) as f64 - (m * m) as f64).sqrt();
            let gamma = ((n * n - m * m) as f64).sqrt();
            let i = idx(n, m);
            let j = idx(n - 1, m);
            let (pk, dpk, d2pk) = if n >= m + 2 {
                let k = idx(n - 2, m);
                (t.p[k], t.dp[k], t.d2p[k])
            } else {
                (0.0, 0.0, 0.0)
            };
            t.p[i] = (a * x * t.p[j] - beta * pk) / gamma;
            t.dp[i] = (a * (-s * t.p[j] + x * t.dp[j]) - beta * dpk) / gamma;
            t.d2p[i] = (a * (-x * t.p[j] - 2.0 * s * t.dp[j] + x * t.d2p[j]) - beta * d2pk) / gamma;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-expanded Schmidt functions (derivation doc, section 3 checks).
    #[test]
    fn low_degree_closed_forms() {
        for &theta in &[0.3, 1.0, std::f64::consts::FRAC_PI_2, 2.4] {
            let (s, x) = theta.sin_cos();
            let t = schmidt(theta);
            let cases = [
                (idx(1, 0), x),
                (idx(1, 1), s),
                (idx(2, 0), 0.5 * (3.0 * x * x - 1.0)),
                (idx(2, 1), 3.0_f64.sqrt() * s * x),
                (idx(2, 2), 3.0_f64.sqrt() / 2.0 * s * s),
                (idx(3, 1), 6.0_f64.sqrt() / 4.0 * (5.0 * x * x - 1.0) * s),
            ];
            for (i, want) in cases {
                assert!(
                    (t.p[i] - want).abs() < 1e-14,
                    "theta={theta}: slot {i}: got {}, want {want}",
                    t.p[i]
                );
            }
        }
    }

    /// d/dtheta and d2/dtheta2 against central differences of the recurrence
    /// values themselves (finite differences appear only as a test oracle).
    /// Tolerance budget: truncation ~ h^2/6 * |S'''| with |S'''| <= n^3 ~ 2e3
    /// gives ~4e-8 for S'; for S'' ~ h^2/12 * n^4 plus eps/h^2 roundoff
    /// amplification ~2e-6 relative; the asserted bounds carry ~5x margin.
    #[test]
    fn derivatives_match_finite_differences() {
        let h = 1e-5;
        for &theta in &[0.2, 0.9, 1.7, 2.8] {
            let t0 = schmidt(theta);
            let tp = schmidt(theta + h);
            let tm = schmidt(theta - h);
            for i in 0..TABLE_LEN {
                let fd1 = (tp.p[i] - tm.p[i]) / (2.0 * h);
                let fd2 = (tp.p[i] - 2.0 * t0.p[i] + tm.p[i]) / (h * h);
                assert!(
                    (t0.dp[i] - fd1).abs() < 1e-8 * (1.0 + fd1.abs()),
                    "dp[{i}] at {theta}: {} vs {fd1}",
                    t0.dp[i]
                );
                assert!(
                    (t0.d2p[i] - fd2).abs() < 2e-4 * (1.0 + fd2.abs()),
                    "d2p[{i}] at {theta}: {} vs {fd2}",
                    t0.d2p[i]
                );
            }
        }
    }

    /// Schmidt normalisation: integral of (S_n^m cos m phi)^2 over the sphere
    /// is 4 pi / (2n+1). The phi integral gives pi (m>0) or 2 pi (m=0), so
    /// the theta integral of (S_n^m)^2 sin theta must be 2(2 - delta_m0)/(2n+1).
    /// Composite Simpson with enough panels to make quadrature error
    /// negligible against the assertion tolerance.
    #[test]
    fn schmidt_normalisation() {
        let panels = 4000_i32;
        let h = std::f64::consts::PI / f64::from(panels);
        for (n, m) in [(1_i32, 0_i32), (2, 1), (5, 3), (8, 0), (13, 13), (13, 5)] {
            #[allow(clippy::cast_sign_loss)]
            let f = |theta: f64| {
                let t = schmidt(theta);
                t.p[idx(n as usize, m as usize)].powi(2) * theta.sin()
            };
            let mut sum = f(0.0) + f(std::f64::consts::PI);
            for k in 1..panels {
                let w = if k % 2 == 1 { 4.0 } else { 2.0 };
                sum += w * f(h * f64::from(k));
            }
            let integral = sum * h / 3.0;
            let want = 2.0 * if m == 0 { 1.0 } else { 2.0 } / f64::from(2 * n + 1);
            assert!(
                (integral - want).abs() < 1e-9,
                "(n,m)=({n},{m}): {integral} vs {want}"
            );
        }
    }
}
