//! Geocentric spherical coordinates and directions on the sphere.
//!
//! Convention (crate-wide): `(r, theta, phi)` = radius from Earth's centre,
//! colatitude from the geographic north pole, east longitude. Local
//! right-handed orthonormal basis `(r_hat, theta_hat, phi_hat)` = (up, south,
//! east). The Earth is treated as a sphere throughout the core; geodetic
//! conversion, if ever needed, happens outside this crate's physics.

use crate::units::{Meters, Radians};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphericalPoint {
    pub r: Meters,
    pub colat: Radians,
    pub lon: Radians,
}

impl SphericalPoint {
    #[must_use]
    pub const fn new(r: Meters, colat: Radians, lon: Radians) -> Self {
        Self { r, colat, lon }
    }
}

/// Unit vector, as components on the local `(r_hat, theta_hat, phi_hat)`
/// basis, of a ray launched at `elevation` above the local horizontal along
/// compass `azimuth` (clockwise from north: 0 = north, pi/2 = east).
///
/// Derivation: the horizontal unit vector at azimuth A is
/// `cos A * north + sin A * east` with `north = -theta_hat`,
/// `east = phi_hat`; tilting toward `r_hat` by elevation beta gives
/// `sin(beta) r_hat + cos(beta) (cos A * north + sin A * east)`.
#[must_use]
pub fn launch_direction(elevation: Radians, azimuth: Radians) -> [f64; 3] {
    let (sb, cb) = elevation.get().sin_cos();
    let (sa, ca) = azimuth.get().sin_cos();
    [sb, -cb * ca, cb * sa]
}

/// Central angle between two points on the sphere (radii ignored).
///
/// Derivation: the spherical law of cosines with colatitudes theta_1, theta_2
/// and longitude difference dphi reads
/// `cos D = cos(theta_1) cos(theta_2) + sin(theta_1) sin(theta_2) cos(dphi)`.
/// Rewriting `cos(theta_1) cos(theta_2) = cos(theta_1 - theta_2) - sin(theta_1) sin(theta_2)`
/// and using `1 - cos x = 2 sin^2(x/2)` on both sides gives the haversine form
/// `sin^2(D/2) = sin^2((theta_1 - theta_2)/2) + sin(theta_1) sin(theta_2) sin^2(dphi/2)`,
/// which keeps full precision for small separations where `cos D ~ 1`.
#[must_use]
pub fn central_angle(a: &SphericalPoint, b: &SphericalPoint) -> Radians {
    let dt = 0.5 * (a.colat.get() - b.colat.get());
    let dp = 0.5 * (a.lon.get() - b.lon.get());
    let h = dt.sin().powi(2) + a.colat.get().sin() * b.colat.get().sin() * dp.sin().powi(2);
    // Clamp antipodal roundoff (h marginally above 1) without using min(),
    // which would silently map a NaN from out-of-convention input to pi;
    // this form propagates NaN honestly.
    let root = h.sqrt();
    let root = if root > 1.0 { 1.0 } else { root };
    Radians::new(2.0 * root.asin())
}

/// Initial great-circle bearing from `a` toward `b`, clockwise from north.
///
/// Derivation: spherical trig on the triangle (pole, a, b). With colatitudes
/// theta and longitude difference dphi = lon_b - lon_a, the standard bearing
/// formula in latitude translates via cos(lat) = sin(colat),
/// sin(lat) = cos(colat) to
/// `atan2(sin(dphi) sin(theta_b), sin(theta_a) cos(theta_b) - cos(theta_a) sin(theta_b) cos(dphi))`.
/// Checks: b due north of an equatorial a gives 0; b due east gives pi/2.
#[must_use]
pub fn bearing(a: &SphericalPoint, b: &SphericalPoint) -> Radians {
    let (sa, ca) = a.colat.get().sin_cos();
    let (sb, cb) = b.colat.get().sin_cos();
    let dp = b.lon.get() - a.lon.get();
    Radians::new((dp.sin() * sb).atan2(sa * cb - ca * sb * dp.cos()))
}

/// Decompose the position of `landing` relative to the great-circle track
/// that starts at `origin` with bearing `track`: returns (along, cross) in
/// radians of arc, cross positive to the right of the track.
///
/// Derivation: with d = central angle origin->landing and db = bearing
/// difference (bearing(origin->landing) - track), the spherical right
/// triangle with the foot of the perpendicular gives
/// `sin(cross) = sin(d) sin(db)` and `tan(along) = tan(d) cos(db)`,
/// implemented as `along = atan2(sin d cos db, cos d)` (exact for db = 0:
/// along = d).
#[must_use]
pub fn track_errors(
    origin: &SphericalPoint,
    track: Radians,
    landing: &SphericalPoint,
) -> (Radians, Radians) {
    let d = central_angle(origin, landing).get();
    let db = bearing(origin, landing).get() - track.get();
    let cross = (d.sin() * db.sin()).asin();
    let along = (d.sin() * db.cos()).atan2(d.cos());
    (Radians::new(along), Radians::new(cross))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn launch_direction_is_unit_and_oriented() {
        let d = launch_direction(Radians::from_degrees(30.0), Radians::from_degrees(90.0));
        let norm = d.iter().map(|c| c * c).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-15);
        // Due east, 30 deg up: no north-south component, positive east and up.
        assert!((d[0] - 0.5).abs() < 1e-15);
        assert!(d[1].abs() < 1e-15);
        assert!(d[2] > 0.8);
        // Due north at zero elevation is -theta_hat exactly.
        let n = launch_direction(Radians::new(0.0), Radians::new(0.0));
        assert_eq!(n[0], 0.0);
        assert!((n[1] + 1.0).abs() < 1e-15);
        assert!(n[2].abs() < 1e-15);
    }

    #[test]
    fn central_angle_matches_known_configurations() {
        let r = Meters::from_km(6371.0);
        let equator0 = SphericalPoint::new(r, Radians::new(FRAC_PI_2), Radians::new(0.0));
        let equator90 = SphericalPoint::new(r, Radians::new(FRAC_PI_2), Radians::new(FRAC_PI_2));
        let pole = SphericalPoint::new(r, Radians::new(0.0), Radians::new(1.234));
        assert!((central_angle(&equator0, &equator90).get() - FRAC_PI_2).abs() < 1e-15);
        assert!((central_angle(&equator0, &pole).get() - FRAC_PI_2).abs() < 1e-15);
        let antipode = SphericalPoint::new(r, Radians::new(FRAC_PI_2), Radians::new(PI));
        assert!((central_angle(&equator0, &antipode).get() - PI).abs() < 1e-7);
        // Small-separation precision: 1 m of arc at Earth scale.
        let tiny = SphericalPoint::new(r, Radians::new(FRAC_PI_2), Radians::new(1.0 / 6_371_000.0));
        let got = central_angle(&equator0, &tiny).get();
        assert!((got - 1.0 / 6_371_000.0).abs() < 1e-22);
    }
}
