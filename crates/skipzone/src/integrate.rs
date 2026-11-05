//! Dormand-Prince 5(4) embedded Runge-Kutta pair (RK5(4)7M, Dormand & Prince
//! 1980; the standard DOPRI5 tableau as given by Hairer, Norsett & Wanner).
//! Seven stages, FSAL (the seventh stage is the first stage of the next
//! step). Fixed-size state, zero allocation per step.
//!
//! The tableau constants are exact rationals from the published method; the
//! convergence-order validation test (observed order 5) would expose any
//! transcription error, which is the project's required check on them.

use crate::error::TraceError;

/// Stage abscissae, kept for documentation of the tableau; the ray RHS is
/// autonomous so they never enter the computation.
#[allow(dead_code)]
const C: [f64; 7] = [0.0, 0.2, 0.3, 0.8, 8.0 / 9.0, 1.0, 1.0];
const A2: [f64; 1] = [0.2];
const A3: [f64; 2] = [3.0 / 40.0, 9.0 / 40.0];
const A4: [f64; 3] = [44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0];
const A5: [f64; 4] = [
    19372.0 / 6561.0,
    -25360.0 / 2187.0,
    64448.0 / 6561.0,
    -212.0 / 729.0,
];
const A6: [f64; 5] = [
    9017.0 / 3168.0,
    -355.0 / 33.0,
    46732.0 / 5247.0,
    49.0 / 176.0,
    -5103.0 / 18656.0,
];
/// Fifth-order weights; also row 7 of A (FSAL property).
const B5: [f64; 7] = [
    35.0 / 384.0,
    0.0,
    500.0 / 1113.0,
    125.0 / 192.0,
    -2187.0 / 6784.0,
    11.0 / 84.0,
    0.0,
];
/// Embedded fourth-order weights.
const B4: [f64; 7] = [
    5179.0 / 57600.0,
    0.0,
    7571.0 / 16695.0,
    393.0 / 640.0,
    -92_097.0 / 339_200.0,
    187.0 / 2100.0,
    1.0 / 40.0,
];

/// Workspace for one state size. `try_step` writes the fifth-order solution,
/// the FSAL derivative at it, and the per-component embedded error estimate.
pub struct Dopri5<const N: usize> {
    k: [[f64; N]; 7],
    y_tmp: [f64; N],
}

impl<const N: usize> Default for Dopri5<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Dopri5<N> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            k: [[0.0; N]; 7],
            y_tmp: [0.0; N],
        }
    }

    /// One trial step of size `h` from `y` where `k1` = f(y) (FSAL input).
    /// On success `y5` holds the fifth-order solution, `k7` = f(y5), and
    /// `err[i]` the embedded (5th minus 4th order) error component.
    ///
    /// # Errors
    /// Propagates the first RHS failure (e.g. pole proximity in a trial
    /// stage) unchanged; the caller decides whether that aborts the ray.
    #[allow(clippy::needless_range_loop)] // index loops mirror the tableau sums
    #[allow(clippy::too_many_arguments)] // in/out buffers of one step; a struct would obscure the FSAL flow
    pub fn try_step<F>(
        &mut self,
        f: &mut F,
        y: &[f64; N],
        k1: &[f64; N],
        h: f64,
        y5: &mut [f64; N],
        k7: &mut [f64; N],
        err: &mut [f64; N],
    ) -> Result<(), TraceError>
    where
        F: FnMut(&[f64; N], &mut [f64; N]) -> Result<(), TraceError>,
    {
        self.k[0] = *k1;
        let rows: [&[f64]; 5] = [&A2, &A3, &A4, &A5, &A6];
        for (s, row) in rows.iter().enumerate() {
            for i in 0..N {
                let mut acc = 0.0;
                for (j, a) in row.iter().enumerate() {
                    acc += a * self.k[j][i];
                }
                self.y_tmp[i] = y[i] + h * acc;
            }
            let stage = s + 1;
            let (_, tail) = self.k.split_at_mut(stage);
            f(&self.y_tmp, &mut tail[0])?;
        }
        for i in 0..N {
            let mut acc = 0.0;
            for j in 0..6 {
                acc += B5[j] * self.k[j][i];
            }
            y5[i] = y[i] + h * acc;
        }
        f(y5, k7)?;
        self.k[6] = *k7;
        for i in 0..N {
            let mut acc = 0.0;
            for j in 0..7 {
                acc += (B5[j] - B4[j]) * self.k[j][i];
            }
            err[i] = h * acc;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exponential test problem y' = y: one step must reproduce the degree-5
    /// Taylor polynomial of exp(h) exactly (definition of a 5th-order step),
    /// and the embedded estimate must scale like h^5.
    #[test]
    fn single_step_matches_exp_taylor() {
        let mut rk = Dopri5::<1>::new();
        let mut f = |y: &[f64; 1], dy: &mut [f64; 1]| {
            dy[0] = y[0];
            Ok(())
        };
        for &h in &[0.1, 0.05, 0.025] {
            let y = [1.0];
            let k1 = [1.0];
            let (mut y5, mut k7, mut err) = ([0.0], [0.0], [0.0]);
            rk.try_step(&mut f, &y, &k1, h, &mut y5, &mut k7, &mut err)
                .unwrap();
            let taylor: f64 = (0..=5).map(|k| h.powi(k) / factorial(k)).sum();
            // A 5th-order one-step method on y'=y differs from the Taylor-5
            // sum only at O(h^6) with an O(1) constant.
            assert!(
                (y5[0] - taylor).abs() < 2.0 * h.powi(6),
                "h={h}: {} vs {taylor}",
                y5[0]
            );
            assert!((k7[0] - y5[0]).abs() < 1e-15);
        }
        // Error estimator order: err(h)/err(h/2) ~ 2^5.
        let mut est = |h: f64| {
            let (mut y5, mut k7, mut err) = ([0.0], [0.0], [0.0]);
            rk.try_step(&mut f, &[1.0], &[1.0], h, &mut y5, &mut k7, &mut err)
                .unwrap();
            err[0].abs()
        };
        let ratio = est(0.1) / est(0.05);
        assert!(
            (ratio.log2() - 5.0).abs() < 0.15,
            "estimator order {}",
            ratio.log2()
        );
    }

    fn factorial(k: i32) -> f64 {
        (1..=k).map(f64::from).product::<f64>().max(1.0)
    }
}
