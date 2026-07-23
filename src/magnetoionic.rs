//! Full complex Appleton-Hartree refractive index, O and X modes, with
//! analytic partial derivatives. No quasi-longitudinal or quasi-transverse
//! approximation anywhere. Derivation and all conventions:
//! docs/derivations/appleton-hartree.md; time convention exp(-i omega t),
//! so Im(n^2) >= 0 in absorbing regions.
//!
//! Known degeneracies (derivation section 6): the Ellis-window sliver around
//! (X = 1, transverse Y_T = 0) and exact root coalescence S_m = 0 are guarded
//! to return finite continuations rather than NaN; mode labels are not
//! globally continuable there and mode-conversion physics is out of scope.

use num_complex::Complex64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ordinary,
    Extraordinary,
}

/// `n^2` and its complex partials with respect to the magnetoionic
/// parameters, at fixed values of the other three.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefractiveIndexSq {
    pub n_sq: Complex64,
    /// d(n^2)/dX, X = omega_p^2/omega^2.
    pub d_x: Complex64,
    /// d(n^2)/dY, Y = omega_H/omega.
    pub d_y: Complex64,
    /// d(n^2)/d(cos theta), theta the wave-normal/field angle.
    pub d_cos: Complex64,
    /// d(n^2)/dZ, Z = nu/omega.
    pub d_z: Complex64,
}

const ZERO: Complex64 = Complex64::new(0.0, 0.0);
const ONE: Complex64 = Complex64::new(1.0, 0.0);
const I: Complex64 = Complex64::new(0.0, 1.0);

/// Derivative-direction order: (X, Y, cos theta, Z).
type Diff = [Complex64; 4];

#[must_use]
pub fn appleton_hartree(mode: Mode, x: f64, y: f64, z: f64, cos_theta: f64) -> RefractiveIndexSq {
    let u = Complex64::new(1.0, z);
    // Unmagnetised short-circuit: exact isotropic limit, and the code path
    // that makes O and X bit-identical when |B| = 0 (validation invariant).
    // The Y-dependence of n^2 is quadratic at Y = 0, so d/dY = d/dcos = 0.
    if y == 0.0 {
        let n_sq = ONE - x / u;
        return RefractiveIndexSq {
            n_sq,
            d_x: -ONE / u,
            d_y: ZERO,
            d_cos: ZERO,
            d_z: I * x / (u * u),
        };
    }

    let c = cos_theta;
    let w = u - x;
    let a1 = Complex64::from(y * y * c * c); // Y_L^2
    let a2 = Complex64::from(y * y * (1.0 - c * c)); // Y_T^2

    // Differentials in the (X, Y, cos, Z) directions; derivation section 7.
    let du: Diff = [ZERO, ZERO, ZERO, I];
    let dx: Diff = [ONE, ZERO, ZERO, ZERO];
    let dw: Diff = [-ONE, ZERO, ZERO, I];
    let da1: Diff = [
        ZERO,
        Complex64::from(2.0 * y * c * c),
        Complex64::from(2.0 * y * y * c),
        ZERO,
    ];
    let da2: Diff = [
        ZERO,
        Complex64::from(2.0 * y * (1.0 - c * c)),
        Complex64::from(-2.0 * y * y * c),
        ZERO,
    ];

    // S_m = sqrt(Y_T^4/4 + Y_L^2 W^2), principal branch (derivation
    // section 4: real non-negative in the collisionless case for all X).
    let s_m = (0.25 * a2 * a2 + a1 * w * w).sqrt();
    let g = s_m + 0.5 * a2;

    // Exact root coalescence (S_m = 0 with Y_T > 0): branch point of the
    // dispersion surface; derivatives of the sqrt are singular there. A
    // float-exact hit has measure zero; we return the degenerate root with
    // the singular dS_m term omitted rather than poison the state with NaN.
    let ds_m: Diff = if s_m == ZERO {
        [ZERO; 4]
    } else {
        core::array::from_fn(|i| {
            (0.5 * a2 * da2[i] + w * w * da1[i] + 2.0 * a1 * w * dw[i]) / (2.0 * s_m)
        })
    };

    match mode {
        Mode::Ordinary => {
            // Ellis-window point (W = 0, Y_T = 0): G = 0. Return the
            // quasi-transverse limit 1 - X/U (derivation section 6) with its
            // isotropic-limit derivatives.
            if g == ZERO {
                let n_sq = ONE - x / u;
                return RefractiveIndexSq {
                    n_sq,
                    d_x: -ONE / u,
                    d_y: ZERO,
                    d_cos: ZERO,
                    d_z: I * x / (u * u),
                };
            }
            let f = u + a1 * w / g;
            let n_sq = ONE - x / f;
            let d = core::array::from_fn(|i| {
                let dg = ds_m[i] + 0.5 * da2[i];
                let df = du[i] + (w * da1[i] + a1 * dw[i]) / g - a1 * w * dg / (g * g);
                -dx[i] / f + x * df / (f * f)
            });
            pack(n_sq, d)
        }
        Mode::Extraordinary => {
            let f = u * w - 0.5 * a2 - s_m;
            let n_sq = ONE - x * w / f;
            let d = core::array::from_fn(|i| {
                let df = u * dw[i] + w * du[i] - 0.5 * da2[i] - ds_m[i];
                -(w * dx[i] + x * dw[i]) / f + x * w * df / (f * f)
            });
            pack(n_sq, d)
        }
    }
}

fn pack(n_sq: Complex64, d: Diff) -> RefractiveIndexSq {
    RefractiveIndexSq {
        n_sq,
        d_x: d[0],
        d_y: d[1],
        d_cos: d[2],
        d_z: d[3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c64(re: f64) -> Complex64 {
        Complex64::from(re)
    }

    #[test]
    fn zero_field_modes_bit_identical_and_isotropic() {
        for &x in &[0.0, 0.3, 0.9999, 1.0, 1.3] {
            for &z in &[0.0, 1e-4, 0.2] {
                let o = appleton_hartree(Mode::Ordinary, x, 0.0, z, 0.37);
                let e = appleton_hartree(Mode::Extraordinary, x, 0.0, z, -0.8);
                assert_eq!(o, e);
                let u = Complex64::new(1.0, z);
                assert!((o.n_sq - (ONE - x / u)).norm() < 1e-15);
            }
        }
    }

    /// Derivation section 4 anchors, collisionless.
    #[test]
    fn transverse_and_longitudinal_limits() {
        let (x, y) = (0.4, 0.25);
        // Transverse (cos = 0): O unaffected by the field.
        let o = appleton_hartree(Mode::Ordinary, x, y, 0.0, 0.0);
        assert!((o.n_sq - c64(1.0 - x)).norm() < 1e-15);
        let e = appleton_hartree(Mode::Extraordinary, x, y, 0.0, 0.0);
        let want = 1.0 - x * (1.0 - x) / (1.0 - x - y * y);
        assert!((e.n_sq - c64(want)).norm() < 1e-14);
        // Longitudinal (cos = +-1): L and R circular waves.
        for cs in [1.0, -1.0] {
            let o = appleton_hartree(Mode::Ordinary, x, y, 0.0, cs);
            assert!((o.n_sq - c64(1.0 - x / (1.0 + y))).norm() < 1e-14);
            let e = appleton_hartree(Mode::Extraordinary, x, y, 0.0, cs);
            assert!((e.n_sq - c64(1.0 - x / (1.0 - y))).norm() < 1e-14);
        }
    }

    /// O reflects at X = 1; X-mode zeros at X = 1 -+ Y (derivation sec. 4).
    #[test]
    fn reflection_conditions() {
        for &y in &[0.05, 0.3, 0.7] {
            for &cs in &[0.9, 0.5, 0.1] {
                let o = appleton_hartree(Mode::Ordinary, 1.0, y, 0.0, cs);
                assert!(o.n_sq.norm() < 1e-13, "O at X=1: {:?}", o.n_sq);
                let e1 = appleton_hartree(Mode::Extraordinary, 1.0 - y, y, 0.0, cs);
                assert!(e1.n_sq.norm() < 1e-13, "X at X=1-Y: {:?}", e1.n_sq);
                let e2 = appleton_hartree(Mode::Extraordinary, 1.0 + y, y, 0.0, cs);
                assert!(e2.n_sq.norm() < 1e-12, "X at X=1+Y: {:?}", e2.n_sq);
            }
        }
    }

    /// Independent algebra: the classical textbook form (valid X < 1), same
    /// theory reduced differently; catches slips in the stable rewrite.
    #[test]
    fn agrees_with_classical_form() {
        let mut k = 0u32;
        for &x in &[0.1, 0.35, 0.6, 0.85] {
            for &y in &[0.03, 0.2, 0.5] {
                for &z in &[0.0, 1e-3, 0.05] {
                    for &cs in &[0.05, 0.4, 0.77, 0.99] {
                        let u = Complex64::new(1.0, z);
                        let w = u - x;
                        let yl2 = c64(y * y * cs * cs);
                        let yt2 = c64(y * y * (1.0 - cs * cs));
                        let t = yt2 / (2.0 * w);
                        let s = (t * t + yl2).sqrt();
                        let classical = |sign: f64| ONE - x / (u - t + sign * s);
                        let o = appleton_hartree(Mode::Ordinary, x, y, z, cs).n_sq;
                        let e = appleton_hartree(Mode::Extraordinary, x, y, z, cs).n_sq;
                        assert!((o - classical(1.0)).norm() < 1e-12 * o.norm().max(1.0));
                        assert!((e - classical(-1.0)).norm() < 1e-12 * e.norm().max(1.0));
                        k += 1;
                    }
                }
            }
        }
        assert_eq!(k, 144);
    }

    /// O-mode continuity through W = 0, where the classical form is 0/0:
    /// n^2 must approach 1 - X/U linearly in W with no jump.
    #[test]
    fn ordinary_stable_across_reflection() {
        let (y, cs) = (0.3, 0.6);
        for &z in &[0.0, 1e-3] {
            let mut prev: Option<Complex64> = None;
            for i in -50..=50 {
                let x = 1.0 + f64::from(i) * 1e-9;
                let o = appleton_hartree(Mode::Ordinary, x, y, z, cs).n_sq;
                assert!(o.re.is_finite() && o.im.is_finite());
                if let Some(p) = prev {
                    // Adjacent samples differ by O(dn^2/dX * 1e-9) ~ 1e-9.
                    assert!((o - p).norm() < 1e-7, "jump at x={x}: {o} vs {p}");
                }
                prev = Some(o);
            }
            // The QT-limit identity n^2(X=1) = 1 - X/U holds exactly only for
            // Z = 0 (with collisions W = iZ != 0 at X = 1 and the a1*W/G term
            // contributes at O(Z), which is physics, not error).
            if z == 0.0 {
                let at1 = appleton_hartree(Mode::Ordinary, 1.0, y, z, cs).n_sq;
                assert!((at1 - (ONE - ONE)).norm() < 1e-12);
            }
        }
    }

    /// All four partials vs central finite differences, both modes,
    /// including points close to (but not inside) the degenerate slivers.
    #[test]
    fn partials_match_finite_differences() {
        let pts = [
            (0.2, 0.1, 0.0, 0.5),
            (0.7, 0.45, 0.01, -0.3),
            (0.97, 0.3, 1e-3, 0.85),
            (1.05, 0.5, 0.02, 0.4),
            (0.5, 0.8, 0.0, 0.05),
            (0.3, 0.25, 0.15, -0.95),
            (0.999, 0.2, 1e-4, 0.3),
        ];
        for &(x, y, z, cs) in &pts {
            for mode in [Mode::Ordinary, Mode::Extraordinary] {
                let r = appleton_hartree(mode, x, y, z, cs);
                let h = 1e-6;
                let fd = |f: &dyn Fn(f64) -> Complex64| (f(h) - f(-h)) / (2.0 * h);
                let cases: [(Complex64, Complex64); 4] = [
                    (r.d_x, fd(&|d| appleton_hartree(mode, x + d, y, z, cs).n_sq)),
                    (r.d_y, fd(&|d| appleton_hartree(mode, x, y + d, z, cs).n_sq)),
                    (
                        r.d_cos,
                        fd(&|d| appleton_hartree(mode, x, y, z, cs + d).n_sq),
                    ),
                    (r.d_z, fd(&|d| appleton_hartree(mode, x, y, z + d, cs).n_sq)),
                ];
                for (i, (an, num)) in cases.iter().enumerate() {
                    let tol = 2e-5 * num.norm().max(1.0);
                    assert!(
                        (an - num).norm() < tol,
                        "{mode:?} partial {i} at ({x},{y},{z},{cs}): {an} vs {num}"
                    );
                }
            }
        }
    }

    /// Loss sign (conventions.md): in the propagating region with Z > 0,
    /// Im(n^2) > 0; with Z = 0, Im is exactly 0.0 for every parameter.
    #[test]
    fn absorption_sign_and_collisionless_reality() {
        for &x in &[0.1, 0.5, 0.9, 1.2] {
            for &y in &[0.05, 0.4] {
                for &cs in &[0.1, 0.7, 1.0] {
                    for mode in [Mode::Ordinary, Mode::Extraordinary] {
                        let r0 = appleton_hartree(mode, x, y, 0.0, cs);
                        assert_eq!(r0.n_sq.im, 0.0);
                        assert_eq!(r0.d_x.im, 0.0);
                        assert_eq!(r0.d_y.im, 0.0);
                        assert_eq!(r0.d_cos.im, 0.0);
                        let r = appleton_hartree(mode, x, y, 1e-3, cs);
                        if r.n_sq.re > 0.05 {
                            assert!(r.n_sq.im > 0.0, "{mode:?} ({x},{y},{cs}): {:?}", r.n_sq);
                        }
                    }
                }
            }
        }
    }
}
