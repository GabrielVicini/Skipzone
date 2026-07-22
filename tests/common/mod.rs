//! Shared reference machinery for the validation suites, computed from
//! docs/derivations/analytic-solutions.md independently of the tracer.
#![allow(dead_code)] // each test binary uses a subset

use skipzone::density::{
    ElectronDensity, QuasiParabolicLayer, critical_frequency, density_at_critical_frequency,
};
use skipzone::geo::SphericalPoint;
use skipzone::units::{Hertz, Meters, PerCubicMeter, Radians};

pub const R0: f64 = 6_371_000.0;

pub fn ground() -> SphericalPoint {
    SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(90.0),
        Radians::new(0.0),
    )
}

/// X(r) and dX/dr for a spherically symmetric density model at frequency f.
pub fn x_of_r(model: &dyn ElectronDensity, f_hz: f64) -> impl Fn(f64) -> (f64, f64) + '_ {
    let nm_per_x = density_at_critical_frequency(Hertz::new(f_hz)).get();
    move |r: f64| {
        let s = model.sample(&SphericalPoint::new(
            Meters::new(r),
            Radians::from_degrees(90.0),
            Radians::new(0.0),
        ));
        (s.ne / nm_per_x, s.d_ne[0] / nm_per_x)
    }
}

pub struct BouguerRef {
    pub delta: f64,
    pub group: f64,
    pub phase: f64,
    pub r_apex: f64,
    /// Nepers; zero unless a collision profile was supplied.
    pub absorption: f64,
}

/// Bouguer-quadrature reference (derivation section 6). With a collision
/// profile the ray geometry uses the real part of the collisional index,
/// n_g^2 = 1 - X/(1+Z^2), exactly as the tracer's Hamiltonian does, and the
/// absorption integrand is (omega/c) Im(n) with the principal square root
/// Im(sqrt(re + i im)) = sqrt((|n^2| - re)/2).
#[allow(clippy::cast_precision_loss)] // panel counts <= 2^22: exact in f64
#[allow(clippy::too_many_lines)]
pub fn bouguer_reference(
    x: &dyn Fn(f64) -> (f64, f64),
    collisions: Option<(&dyn Fn(f64) -> f64, f64)>, // (Z(r), omega)
    r0: f64,
    beta: f64,
    r_max: f64,
    breaks: &[f64],
) -> BouguerRef {
    let c = r0 * beta.cos();
    let n_g_sq = |r: f64| -> f64 {
        let xv = x(r).0;
        match collisions {
            Some((z, _)) => {
                let zv = z(r);
                1.0 - xv / (1.0 + zv * zv)
            }
            None => 1.0 - xv,
        }
    };
    let ff = |r: f64| n_g_sq(r) * r * r - c * c;
    let mut lo = r0;
    let mut hi = r0;
    loop {
        hi += 1_000.0;
        assert!(hi < r_max, "reference ray does not reflect below r_max");
        if ff(hi) < 0.0 {
            break;
        }
        lo = hi;
    }
    for _ in 0..200 {
        let mid = f64::midpoint(lo, hi);
        if ff(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-9 {
            break;
        }
    }
    let r_apex = 0.5 * (lo + hi);
    // Q = F/(r_apex - r) is smooth up to the apex. Within the last
    // millimetre (where the division goes 0/0) Q is frozen at its value one
    // millimetre out: Q varies on the profile's km scale, so the relative
    // error of the freeze is ~1e-6 over a region contributing ~1e-2 of the
    // integral - far below the 1e-12 quadrature target's usefulness and the
    // 1e-4 m assertion levels.
    let q = |r: f64| {
        let d = r_apex - r;
        if d < 1e-3 {
            ff(r_apex - 1e-3) / 1e-3
        } else {
            ff(r) / d
        }
    };
    let seg = |g: &dyn Fn(f64) -> f64, a: f64, b: f64| -> f64 {
        let (ta, tb) = ((r_apex - b).max(0.0).sqrt(), (r_apex - a).sqrt());
        let integrand = |t: f64| {
            let r = r_apex - t * t;
            2.0 * g(r) / q(r).sqrt()
        };
        let simpson = |n: usize| -> f64 {
            let h = (tb - ta) / n as f64;
            let mut s = integrand(ta) + integrand(tb);
            for k in 1..n {
                s += if k % 2 == 1 { 4.0 } else { 2.0 } * integrand(ta + h * k as f64);
            }
            s * h / 3.0
        };
        let mut n = 64;
        let mut prev = simpson(n);
        loop {
            n *= 2;
            let cur = simpson(n);
            if (cur - prev).abs() <= 1e-12 * cur.abs().max(1.0) || n > 1 << 22 {
                return cur;
            }
            prev = cur;
        }
    };
    let integral = |g: &dyn Fn(f64) -> f64| -> f64 {
        let mut pts: Vec<f64> = std::iter::once(r0)
            .chain(breaks.iter().copied().filter(|&b| b > r0 && b < r_apex))
            .chain(std::iter::once(r_apex))
            .collect();
        pts.sort_by(f64::total_cmp);
        pts.windows(2).map(|w| seg(g, w[0], w[1])).sum()
    };
    let delta = 2.0 * integral(&|r| c / r);
    let group = 2.0 * integral(&|r| r);
    let phase = 2.0 * integral(&|r| n_g_sq(r) * r);
    let absorption = match collisions {
        None => 0.0,
        Some((z, omega)) => {
            let chi = |r: f64| -> f64 {
                let xv = x(r).0;
                let zv = z(r);
                let re = 1.0 - xv / (1.0 + zv * zv);
                let im = xv * zv / (1.0 + zv * zv);
                // Principal sqrt imaginary part, STABLE branch: the naive
                // sqrt((hypot - re)/2) cancels catastrophically for
                // im << re (chi rounds to 0 below ~1e-8) and silently
                // under-counts D-region absorption by ~2e-5 Np in the
                // validation scenario - found because the tracer refused
                // to match it. Compute the real part first (no
                // cancellation for re > 0), then chi = im/(2a).
                if re > 0.0 {
                    let a = f64::midpoint(re.hypot(im), re).sqrt();
                    im / (2.0 * a)
                } else {
                    (f64::midpoint(re.hypot(im), -re)).max(0.0).sqrt()
                }
            };
            let c_light = skipzone::constants::SPEED_OF_LIGHT;
            // (omega/c) chi n_g r / sqrt(F): chi per arc length, arc element
            // n_g r dr / sqrt(F).
            2.0 * integral(&|r| (omega / c_light) * chi(r) * n_g_sq(r).max(0.0).sqrt() * r)
        }
    };
    BouguerRef {
        delta,
        group,
        phase,
        r_apex,
        absorption,
    }
}

/// QP closed forms (derivation section 4): (delta, group, phase, r_apex).
#[allow(clippy::many_single_char_names)]
pub fn qp_closed_form(
    layer: &QuasiParabolicLayer,
    nm: f64,
    rm: f64,
    ym: f64,
    f_hz: f64,
    r0: f64,
    beta: f64,
) -> (f64, f64, f64, f64) {
    let rb = layer.base_radius().get();
    let fc = critical_frequency(PerCubicMeter::new(nm)).get();
    let f2 = (fc / f_hz).powi(2);
    let c = r0 * beta.cos();
    let a = 1.0 - f2 + f2 * rb * rb / (ym * ym);
    let b = -2.0 * f2 * rm * rb * rb / (ym * ym);
    let c0 = f2 * rm * rm * rb * rb / (ym * ym) - c * c;
    assert!(
        c0 > 0.0,
        "closed form used outside its C0 > 0 validity domain"
    );
    let disc = b * b - 4.0 * a * c0;
    assert!(disc > 0.0, "ray does not reflect in the QP layer");
    let rt = (-b - disc.sqrt()) / (2.0 * a);
    assert!(
        rt > rb,
        "turning point below layer base: parameters penetrate"
    );
    // acosh antiderivatives, disc > 0 branch; apex limits are exact zeros
    // (see the derivation's conditioning note - evaluating them at the
    // floating-point rt costs 0.01-100 m of noise).
    let ff = |r: f64| a * r * r + b * r + c0;
    let ach = |u: f64| u.max(1.0).acosh();
    let i1 = |r: f64| -ach(-(2.0 * a * r + b) / disc.sqrt()) / a.sqrt();
    let i2 = |r: f64| ff(r).max(0.0).sqrt() / a - b / (2.0 * a) * i1(r);
    let i3 = |r: f64| -ach((2.0 * c0 / r + b) / disc.sqrt()) / c0.sqrt();
    let i4 = |r: f64| ff(r).max(0.0).sqrt() + 0.5 * b * i1(r) + c0 * i3(r);
    let j1 = -i3(rb);
    let j2 = -i2(rb);
    let j4 = -i4(rb);
    let s0 = (rb * rb - c * c).sqrt() - (r0 * r0 - c * c).sqrt();
    let d0 = (c / rb).acos() - (c / r0).acos();
    (
        2.0 * (d0 + c * j1),
        2.0 * (s0 + j2),
        2.0 * (s0 + j4 + c * c * j1),
        rt,
    )
}

pub fn cartesian_direction(colat: f64, lon: f64, m: [f64; 3]) -> [f64; 3] {
    let (st, ct) = colat.sin_cos();
    let (sp, cp) = lon.sin_cos();
    let r_hat = [st * cp, st * sp, ct];
    let t_hat = [ct * cp, ct * sp, -st];
    let p_hat = [-sp, cp, 0.0];
    let norm = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
    core::array::from_fn(|i| (m[0] * r_hat[i] + m[1] * t_hat[i] + m[2] * p_hat[i]) / norm)
}
