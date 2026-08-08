//! Assembly of the Haselgrove ray equations from the medium models.
//! Every equation here is derived in docs/derivations/haselgrove.md; the
//! section numbers in comments refer to that file.
//!
//! State vector layout (SI, f64, fixed size - no allocation in the loop):
//!
//! ```text
//! y[0] r [m]              y[1] theta [rad]      y[2] phi [rad]
//! y[3..6] m = (c/omega) k, physical components (r, theta, phi)
//! y[6] group path P' [m]  y[7] phase path P [m]
//! y[8] absorption A [Np]  y[9] arc length s [m]
//! ```
//!
//! The independent variable sigma has units of metres (section 2).

use crate::collision::CollisionFrequency;
use crate::constants::{OMEGA_H_PER_TESLA, OMEGA_P_SQ_PER_DENSITY, SPEED_OF_LIGHT};
use crate::density::ElectronDensity;
use crate::error::TraceError;
use crate::geo::{SphericalPoint, launch_direction};
use crate::mag::MagneticField;
use crate::magnetoionic::{Mode, appleton_hartree};
use crate::units::{Hertz, Meters, Radians};

pub const STATE_DIM: usize = 10;
pub type State = [f64; STATE_DIM];

/// Below this |sin(colatitude)| the cot(theta) and 1/sin(theta) terms
/// amplify roundoff beyond any useful tolerance; rays this close to the
/// coordinate pole (< ~6 mm off-axis) are a coordinate-system limitation
/// and abort with a typed error rather than degrade silently.
pub const SIN_COLAT_MIN: f64 = 1e-9;

pub struct RayEquations<'a, D: ?Sized, B: ?Sized, C: ?Sized> {
    density: &'a D,
    field: &'a B,
    collisions: &'a C,
    mode: Mode,
    /// Angular wave frequency, rad/s.
    omega: f64,
    /// X per unit electron density: e^2/(eps0 m_e omega^2).
    x_per_ne: f64,
    /// Y per tesla: e/(m_e omega).
    y_per_tesla: f64,
    /// omega/c, for the absorption integrand.
    k0: f64,
}

impl<'a, D, B, C> RayEquations<'a, D, B, C>
where
    D: ElectronDensity + ?Sized,
    B: MagneticField + ?Sized,
    C: CollisionFrequency + ?Sized,
{
    pub fn new(density: &'a D, field: &'a B, collisions: &'a C, f: Hertz, mode: Mode) -> Self {
        let omega = f.angular();
        Self {
            density,
            field,
            collisions,
            mode,
            omega,
            x_per_ne: OMEGA_P_SQ_PER_DENSITY / (omega * omega),
            y_per_tesla: OMEGA_H_PER_TESLA / omega,
            k0: omega / SPEED_OF_LIGHT,
        }
    }

    /// The ray right-hand side dy/dsigma (haselgrove.md sections 2-4).
    ///
    /// # Errors
    /// `PoleProximity` if the state is within `SIN_COLAT_MIN` of a
    /// coordinate pole.
    #[allow(clippy::too_many_lines)] // one derivation, one function; splitting would scatter it
    pub fn rhs(&self, y: &State, dy: &mut State) -> Result<(), TraceError> {
        let (r, theta, phi) = (y[0], y[1], y[2]);
        let m = [y[3], y[4], y[5]];
        let (sin_t, cos_t) = theta.sin_cos();
        if sin_t.abs() < SIN_COLAT_MIN {
            return Err(TraceError::PoleProximity { sin_colat: sin_t });
        }
        let cot_t = cos_t / sin_t;
        let p = SphericalPoint::new(Meters::new(r), Radians::new(theta), Radians::new(phi));

        let dens = self.density.sample(&p);
        let coll = self.collisions.sample(&p);
        let fld = self.field.sample(&p);

        let x = self.x_per_ne * dens.ne;
        let z = coll.nu / self.omega;
        let b_mag = (fld.b[0] * fld.b[0] + fld.b[1] * fld.b[1] + fld.b[2] * fld.b[2]).sqrt();
        let y_par = self.y_per_tesla * b_mag;

        let m_norm = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2])
            .sqrt()
            .max(f64::MIN_POSITIVE); // apex: |m| -> 0 smoothly; exact 0 has measure zero
        let m_hat = [m[0] / m_norm, m[1] / m_norm, m[2] / m_norm];

        // b_hat and cos(Theta) only exist in a magnetised medium; y_par == 0.0
        // routes through the isotropic AH short-circuit where d_y = d_cos = 0.
        let (b_hat, cos_th) = if y_par == 0.0 {
            ([0.0; 3], 0.0)
        } else {
            let bh = [fld.b[0] / b_mag, fld.b[1] / b_mag, fld.b[2] / b_mag];
            let c = (m_hat[0] * bh[0] + m_hat[1] * bh[1] + m_hat[2] * bh[2]).clamp(-1.0, 1.0);
            (bh, c)
        };

        let ri = appleton_hartree(self.mode, x, y_par, z, cos_th);
        let (dx_re, dy_re, dcos_re, dz_re) = (ri.d_x.re, ri.d_y.re, ri.d_cos.re, ri.d_z.re);

        // v = dH/dm (section 2): m plus the wave-normal-direction term.
        let v = if y_par == 0.0 {
            m
        } else {
            let f = 0.5 * dcos_re / m_norm;
            [
                m[0] - f * (b_hat[0] - cos_th * m_hat[0]),
                m[1] - f * (b_hat[1] - cos_th * m_hat[1]),
                m[2] - f * (b_hat[2] - cos_th * m_hat[2]),
            ]
        };

        // G_q = d(Re n^2)/dq at fixed physical m (section 2).
        let mut g = [0.0; 3];
        for (q, gq) in g.iter_mut().enumerate() {
            let db_q = [fld.db[0][q], fld.db[1][q], fld.db[2][q]];
            *gq = dx_re * self.x_per_ne * dens.d_ne[q] + dz_re * coll.d_nu[q] / self.omega;
            if y_par > 0.0 {
                let dbmag_q = b_hat[0] * db_q[0] + b_hat[1] * db_q[1] + b_hat[2] * db_q[2];
                let dcos_q = (m_hat[0] * db_q[0] + m_hat[1] * db_q[1] + m_hat[2] * db_q[2]
                    - cos_th * dbmag_q)
                    / b_mag;
                *gq += dy_re * self.y_per_tesla * dbmag_q + dcos_re * dcos_q;
            }
        }

        dy[0] = v[0];
        dy[1] = v[1] / r;
        dy[2] = v[2] / (r * sin_t);
        dy[3] = 0.5 * g[0] + (m[1] * v[1] + m[2] * v[2]) / r;
        dy[4] = (0.5 * g[1] + m[2] * v[2] * cot_t - m[1] * v[0]) / r;
        dy[5] = (0.5 * g[2] / sin_t - m[2] * v[0] - m[2] * v[1] * cot_t) / r;

        // Group path integrand (section 3): m.m + (omega/2) d(Re n^2)/domega.
        let m_sq = m[0] * m[0] + m[1] * m[1] + m[2] * m[2];
        dy[6] = m_sq - (dx_re * x + 0.5 * dy_re * y_par + 0.5 * dz_re * z);
        // Phase path integrand (section 4): m.v.
        dy[7] = m[0] * v[0] + m[1] * v[1] + m[2] * v[2];
        // Absorption integrand (section 4); exactly zero without collisions.
        let v_norm = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        dy[8] = if z == 0.0 {
            0.0
        } else {
            self.k0 * ri.n_sq.sqrt().im * v_norm
        };
        dy[9] = v_norm;
        Ok(())
    }

    /// The conserved Hamiltonian H = (m.m - Re n^2)/2 (diagnostic; zero on
    /// shell, drift measures integrator error).
    ///
    /// # Errors
    /// `PoleProximity` as for `rhs`.
    pub fn hamiltonian(&self, y: &State) -> Result<f64, TraceError> {
        let (sin_t, _) = y[1].sin_cos();
        if sin_t.abs() < SIN_COLAT_MIN {
            return Err(TraceError::PoleProximity { sin_colat: sin_t });
        }
        let p = SphericalPoint::new(Meters::new(y[0]), Radians::new(y[1]), Radians::new(y[2]));
        let x = self.x_per_ne * self.density.sample(&p).ne;
        let z = self.collisions.sample(&p).nu / self.omega;
        let fld = self.field.sample(&p);
        let b_mag = (fld.b[0] * fld.b[0] + fld.b[1] * fld.b[1] + fld.b[2] * fld.b[2]).sqrt();
        let y_par = self.y_per_tesla * b_mag;
        let m = [y[3], y[4], y[5]];
        let m_sq = m[0] * m[0] + m[1] * m[1] + m[2] * m[2];
        let cos_th = if y_par == 0.0 {
            0.0
        } else {
            let mn = m_sq.sqrt().max(f64::MIN_POSITIVE);
            ((m[0] * fld.b[0] + m[1] * fld.b[1] + m[2] * fld.b[2]) / (mn * b_mag)).clamp(-1.0, 1.0)
        };
        let n_sq = appleton_hartree(self.mode, x, y_par, z, cos_th).n_sq.re;
        Ok(0.5 * (m_sq - n_sq))
    }

    /// X = (f_p/f)^2 from the density model at a point (apex observable).
    #[must_use]
    pub fn x_at(&self, p: &SphericalPoint) -> f64 {
        self.x_per_ne * self.density.sample(p).ne
    }

    /// State exactly on the dispersion shell at a launch point (section 6).
    ///
    /// # Errors
    /// `EvanescentLaunch` if Re n^2 <= 0 for this mode at the launch point;
    /// `PoleProximity` at a coordinate pole.
    pub fn initial_state(
        &self,
        launch: &SphericalPoint,
        elevation: Radians,
        azimuth: Radians,
    ) -> Result<State, TraceError> {
        if launch.colat.get().sin().abs() < SIN_COLAT_MIN {
            return Err(TraceError::PoleProximity {
                sin_colat: launch.colat.get().sin(),
            });
        }
        let k_hat = launch_direction(elevation, azimuth);
        let x = self.x_per_ne * self.density.sample(launch).ne;
        let z = self.collisions.sample(launch).nu / self.omega;
        let fld = self.field.sample(launch);
        let b_mag = (fld.b[0] * fld.b[0] + fld.b[1] * fld.b[1] + fld.b[2] * fld.b[2]).sqrt();
        let y_par = self.y_per_tesla * b_mag;
        let cos_th = if y_par == 0.0 {
            0.0
        } else {
            ((k_hat[0] * fld.b[0] + k_hat[1] * fld.b[1] + k_hat[2] * fld.b[2]) / b_mag)
                .clamp(-1.0, 1.0)
        };
        let n_sq = appleton_hartree(self.mode, x, y_par, z, cos_th).n_sq.re;
        if n_sq <= 0.0 {
            return Err(TraceError::EvanescentLaunch { n_squared: n_sq });
        }
        let n = n_sq.sqrt();
        Ok([
            launch.r.get(),
            launch.colat.get(),
            launch.lon.get(),
            n * k_hat[0],
            n * k_hat[1],
            n * k_hat[2],
            0.0,
            0.0,
            0.0,
            0.0,
        ])
    }
}
