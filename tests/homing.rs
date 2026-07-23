//! Validation for homing (build-order step 9): both methods must agree
//! within stated tolerances, and against the QP closed form where it exists.

mod common;
use common::{R0, ground, qp_closed_form};
use skipzone::collision::{ExponentialCollisions, ZeroCollisions};
use skipzone::density::{ChapmanLayer, QuasiParabolicLayer, density_at_critical_frequency};
use skipzone::geo::SphericalPoint;
use skipzone::homing::{Homing, HomingConfig, HomingError};
use skipzone::mag::{Dipole, ZeroField};
use skipzone::magnetoionic::Mode;
use skipzone::trace::{TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, PerCubicMeter, PerSecond, Radians, Tesla};

fn qp_layer() -> (QuasiParabolicLayer, f64, f64, f64) {
    let nm = density_at_critical_frequency(Hertz::new(7e6)).get();
    let (rm, ym) = (R0 + 300e3, 100e3);
    (
        QuasiParabolicLayer::new(PerCubicMeter::new(nm), Meters::new(rm), Meters::new(ym)).unwrap(),
        nm,
        rm,
        ym,
    )
}

fn target_at(range_m: f64) -> SphericalPoint {
    // Due east along the equator from the common ground() point.
    SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(90.0),
        Radians::new(range_m / R0),
    )
}

#[test]
fn qp_field_free_agrees_with_closed_form_and_between_methods() {
    let (layer, nm, rm, ym) = qp_layer();
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(8e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 700e3)),
    );
    let homing = Homing {
        tracer: &tracer,
        config: HomingConfig::default(),
    };
    let target = target_at(1.4e6);
    let scan = homing.home_scan(&ground(), &target).unwrap();
    let newton = homing.home_newton(&ground(), &target).unwrap();
    assert!(!scan.is_empty());
    assert_eq!(
        scan.len(),
        newton.len(),
        "methods found different ray counts"
    );
    for (s, n) in scan.iter().zip(&newton) {
        assert!(s.miss_m < 30.0, "scan miss {}", s.miss_m);
        assert!(n.miss_m < 30.0, "newton miss {}", n.miss_m);
        // Stated cross-method tolerance: 0.02 deg in elevation, and both
        // land within the 30 m miss tolerance of the same target.
        assert!(
            (s.elevation.to_degrees() - n.elevation.to_degrees()).abs() < 0.02,
            "elevations {} vs {}",
            s.elevation.to_degrees(),
            n.elevation.to_degrees()
        );
        // Independent check: the closed form maps the found elevation to a
        // ground range that must equal the target range within the miss
        // tolerance (field-free: azimuth is exactly the bearing).
        let (delta, _, _, _) = qp_closed_form(&layer, nm, rm, ym, 8e6, R0, n.elevation.get());
        assert!(
            (delta * R0 - 1.4e6).abs() < 35.0,
            "closed-form range at found elevation: {}",
            delta * R0
        );
    }
    // The QP range-elevation curve is strictly monotone here (verified from
    // the closed form: 2447 km at 4 deg falling through 570 km at 44.5 deg
    // with no upturn - the QP topside is stretched, so no high-ray branch
    // exists at this f/fc): exactly one solution.
    assert_eq!(scan.len(), 1, "monotone QP range curve admits one ray");
}

#[test]
fn homing_with_field_and_collisions_methods_agree() {
    let layer = ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(9e6)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap();
    let field = Dipole::new(
        Meters::new(6_371_200.0),
        Tesla::new(-2.935e-5),
        Tesla::new(-1.41e-6),
        Tesla::new(4.55e-6),
    );
    let coll = ExponentialCollisions::new(
        PerSecond::new(1e5),
        Meters::new(R0 + 100e3),
        Meters::new(30e3),
    )
    .unwrap();
    let tracer = Tracer::new(
        &layer,
        &field,
        &coll,
        Hertz::new(7.5e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 800e3)),
    );
    let homing = Homing {
        tracer: &tracer,
        config: HomingConfig::default(),
    };
    let from = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(55.0),
        Radians::from_degrees(10.0),
    );
    let to = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(47.0),
        Radians::from_degrees(22.0),
    );
    let newton = homing.home_newton(&from, &to).unwrap();
    let scan = homing.home_scan(&from, &to).unwrap();
    assert!(!newton.is_empty());
    assert_eq!(scan.len(), newton.len());
    for (s, n) in scan.iter().zip(&newton) {
        assert!(n.miss_m < 30.0);
        assert!(s.miss_m < 30.0);
        assert!((s.elevation.to_degrees() - n.elevation.to_degrees()).abs() < 0.05);
        assert!((s.azimuth.to_degrees() - n.azimuth.to_degrees()).abs() < 0.05);
        // The magnetized ray is deflected out of the great-circle plane;
        // absorption must be positive through the collisional D region.
        assert!(n.result.absorption.get() > 0.0);
    }
}

#[test]
fn skip_zone_target_returns_no_bracket() {
    let (layer, ..) = qp_layer();
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(8e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 700e3)),
    );
    let homing = Homing {
        tracer: &tracer,
        config: HomingConfig::default(),
    };
    // 8 MHz over a 7 MHz-critical QP layer cannot land 150 km away: that
    // range needs steep elevations, which penetrate.
    let err = homing.home_scan(&ground(), &target_at(150e3)).unwrap_err();
    assert!(matches!(err, HomingError::NoBracket { .. }), "{err}");
}
