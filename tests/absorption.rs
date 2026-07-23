//! Validation for absorption (build-order step 8): the integral of
//! (omega/c) Im(n) along the ray. References: docs/derivations/
//! haselgrove.md section 4 and the collisional sign conventions in
//! conventions.md. Collision magnitudes in these scenarios are
//! test-arbitrary (documented in collision.rs: the crate provides no
//! default); every assertion here is an invariant that cannot depend on
//! the chosen magnitude.

mod common;
use common::{R0, bouguer_reference, ground, x_of_r};
use skipzone::collision::{CollisionFrequency, ExponentialCollisions, ZeroCollisions};
use skipzone::density::{ChapmanLayer, ZeroDensity, density_at_critical_frequency};
use skipzone::geo::SphericalPoint;
use skipzone::mag::{Dipole, ZeroField};
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, PerSecond, Radians, Tesla};

fn chapman() -> ChapmanLayer {
    ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(9e6)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap()
}

fn collisions() -> ExponentialCollisions {
    // nu ~ 1e5 s^-1 at 100 km falling with a 30 km scale height: Z ~ 2e-3
    // at 7 MHz. Magnitude is test-arbitrary by design.
    ExponentialCollisions::new(
        PerSecond::new(1e5),
        Meters::new(R0 + 100e3),
        Meters::new(30e3),
    )
    .unwrap()
}

fn dipole() -> Dipole {
    Dipole::new(
        Meters::new(6_371_200.0),
        Tesla::new(-2.935e-5),
        Tesla::new(-1.41e-6),
        Tesla::new(4.55e-6),
    )
}

fn config() -> TraceConfig {
    TraceConfig::new(Meters::new(R0), Meters::new(R0 + 800e3))
}

fn start() -> SphericalPoint {
    SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(60.0),
        Radians::from_degrees(10.0),
    )
}

/// No electrons: n = 1 exactly for any Y, Z, so collisions alone must
/// produce exactly zero absorption.
#[test]
fn no_electrons_no_absorption() {
    let density = ZeroDensity;
    let field = dipole();
    let coll = collisions();
    let t = Tracer::new(
        &density,
        &field,
        &coll,
        Hertz::new(7e6),
        Mode::Ordinary,
        config(),
    );
    let res = t
        .trace(
            &start(),
            Radians::from_degrees(20.0),
            Radians::from_degrees(40.0),
        )
        .unwrap();
    assert_eq!(res.outcome, Outcome::Escaped);
    assert_eq!(res.absorption.get().to_bits(), 0.0_f64.to_bits());
}

/// Zero collision frequency: absorption exactly zero even through a dense
/// magnetised layer (bit-exact, from the Z = 0 short-circuit).
#[test]
fn zero_collisions_zero_absorption() {
    let layer = chapman();
    let field = dipole();
    let t = Tracer::new(
        &layer,
        &field,
        &ZeroCollisions,
        Hertz::new(7e6),
        Mode::Ordinary,
        config(),
    );
    let res = t
        .trace(
            &start(),
            Radians::from_degrees(30.0),
            Radians::from_degrees(40.0),
        )
        .unwrap();
    assert_eq!(res.outcome, Outcome::Landed);
    assert_eq!(res.absorption.get().to_bits(), 0.0_f64.to_bits());
}

/// Absorption is monotone non-decreasing along the ray and strictly
/// positive at landing when both Ne and nu are positive along the path.
#[test]
fn absorption_monotone_and_positive() {
    let layer = chapman();
    let field = dipole();
    let coll = collisions();
    let t = Tracer::new(
        &layer,
        &field,
        &coll,
        Hertz::new(7e6),
        Mode::Ordinary,
        config(),
    );
    let mut last = 0.0_f64;
    let mut monotone = true;
    let res = t
        .trace_with_observer(
            &start(),
            Radians::from_degrees(30.0),
            Radians::from_degrees(40.0),
            &mut |_s, y| {
                if y[8] < last {
                    monotone = false;
                }
                last = y[8];
            },
        )
        .unwrap();
    assert!(monotone, "absorption decreased along the ray");
    assert!(res.absorption.get() > 0.0);
    assert_eq!(res.outcome, Outcome::Landed);
}

/// Reciprocity extends to absorption: n(-k) = n(k) makes the attenuation
/// integral direction-independent (appleton-hartree.md; the Y_L^2/Y_T^2
/// structure covers the complex index, not just its real part).
#[test]
fn absorption_reciprocity() {
    let layer = chapman();
    let field = dipole();
    let coll = collisions();
    for mode in [Mode::Ordinary, Mode::Extraordinary] {
        let t = Tracer::new(&layer, &field, &coll, Hertz::new(7.5e6), mode, config());
        let fwd = t
            .trace(
                &start(),
                Radians::from_degrees(28.0),
                Radians::from_degrees(40.0),
            )
            .unwrap();
        assert_eq!(fwd.outcome, Outcome::Landed);
        let m = fwd.end_m;
        let n = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
        let (elev, az) = (
            Radians::new((-m[0] / n).asin()),
            Radians::new((-m[2] / n).atan2(m[1] / n)),
        );
        let bwd = t.trace(&fwd.end, elev, az).unwrap();
        assert_eq!(bwd.outcome, Outcome::Landed);
        let (a1, a2) = (fwd.absorption.get(), bwd.absorption.get());
        // Budget: two rtol-1e-10 traces; absorption carries atol 1e-9 Np.
        assert!(
            (a1 - a2).abs() < 1e-6 * a1.max(1.0),
            "{mode:?}: absorption {a1} vs reversed {a2}"
        );
    }
}

/// Quantitative check: field-free absorption against the Bouguer-quadrature
/// reference, which shares the tracer's real-part Hamiltonian geometry by
/// construction, so agreement is limited only by integration accuracy.
#[test]
fn field_free_absorption_matches_quadrature() {
    let layer = chapman();
    let coll = collisions();
    let f_hz = 7e6;
    let omega = Hertz::new(f_hz).angular();
    let t = Tracer::new(
        &layer,
        &ZeroField,
        &coll,
        Hertz::new(f_hz),
        Mode::Ordinary,
        config(),
    );
    let beta = Radians::from_degrees(30.0);
    let res = t
        .trace(&common::ground(), beta, Radians::from_degrees(90.0))
        .unwrap();
    assert_eq!(res.outcome, Outcome::Landed);
    let x = x_of_r(&layer, f_hz);
    let z_of_r = |r: f64| {
        coll.sample(&SphericalPoint::new(
            Meters::new(r),
            Radians::from_degrees(90.0),
            Radians::new(0.0),
        ))
        .nu / omega
    };
    let refq = bouguer_reference(&x, Some((&z_of_r, omega)), R0, beta.get(), R0 + 800e3, &[]);
    let (got, want) = (res.absorption.get(), refq.absorption);
    assert!(
        want > 1e-3,
        "scenario should absorb measurably, got {want} Np"
    );
    assert!(
        (got - want).abs() < 1e-6 * want,
        "absorption {got} vs quadrature {want} Np"
    );
    let _ = ground(); // keep the shared helper exercised in this binary
}
