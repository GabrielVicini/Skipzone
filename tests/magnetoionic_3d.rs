//! Validation B for the full 3D magnetoionic tracer: physical invariants
//! with a magnetic field present. References: docs/derivations/
//! appleton-hartree.md (mode structure, reciprocity basis) and
//! haselgrove.md (equations of motion).

use skipzone::collision::ZeroCollisions;
use skipzone::density::{
    ChapmanLayer, QuasiParabolicLayer, ZeroDensity, density_at_critical_frequency,
};
use skipzone::geo::{SphericalPoint, central_angle};
use skipzone::mag::{Dipole, MagneticField, ZeroField};
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, TraceResult, Tracer};
use skipzone::units::{Hertz, Meters, PerCubicMeter, Radians, Tesla};

const R0: f64 = 6_371_000.0;

fn launch_point() -> SphericalPoint {
    SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(60.0),
        Radians::from_degrees(10.0),
    )
}

fn chapman(fc_hz: f64) -> ChapmanLayer {
    ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(fc_hz)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap()
}

fn earthlike_dipole() -> Dipole {
    // IGRF-14 2025 first-degree coefficients rounded to 3 digits; any
    // Earth-magnitude tilted dipole serves - invariants cannot depend on the
    // exact values.
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

/// Arrival direction reversed into a new launch (elevation, azimuth):
/// u = -m_hat; elevation = asin(u_r), azimuth = atan2(east, north)
/// = atan2(u_phi, -u_theta).
fn reversed_launch(res: &TraceResult) -> (Radians, Radians) {
    let m = res.end_m;
    let n = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
    let u = [-m[0] / n, -m[1] / n, -m[2] / n];
    (Radians::new(u[0].asin()), Radians::new(u[2].atan2(-u[1])))
}

#[test]
fn zero_field_makes_modes_bit_identical() {
    let layer = chapman(9e6);
    let elev = Radians::from_degrees(30.0);
    let az = Radians::from_degrees(40.0);
    let run = |mode: Mode| {
        let t = Tracer::new(
            &layer,
            &ZeroField,
            &ZeroCollisions,
            Hertz::new(7e6),
            mode,
            config(),
        );
        t.trace(&launch_point(), elev, az).unwrap()
    };
    let (o, x) = (run(Mode::Ordinary), run(Mode::Extraordinary));
    // Bit-identical, not merely close: the zero-field code path must be
    // exactly shared (validation invariant from the task spec).
    assert_eq!(o.end.r.get().to_bits(), x.end.r.get().to_bits());
    assert_eq!(o.end.colat.get().to_bits(), x.end.colat.get().to_bits());
    assert_eq!(o.end.lon.get().to_bits(), x.end.lon.get().to_bits());
    assert_eq!(o.group_path.get().to_bits(), x.group_path.get().to_bits());
    assert_eq!(o.phase_path.get().to_bits(), x.phase_path.get().to_bits());
    for i in 0..3 {
        assert_eq!(o.end_m[i].to_bits(), x.end_m[i].to_bits());
    }
    assert_eq!(o.steps, x.steps);
}

#[test]
fn zero_density_rays_straight_despite_field() {
    let density = ZeroDensity;
    let field = earthlike_dipole();
    let tracer = Tracer::new(
        &density,
        &field,
        &ZeroCollisions,
        Hertz::new(10e6),
        Mode::Extraordinary,
        config(),
    );
    let elev = Radians::from_degrees(25.0);
    let res = tracer
        .trace(&launch_point(), elev, Radians::from_degrees(70.0))
        .unwrap();
    assert_eq!(res.outcome, Outcome::Escaped);
    // X = 0 makes n = 1 exactly for any Y (AH: n^2 = 1 - X * (...)), so the
    // ray must be geometrically straight; wander is integrator error.
    let dir = |colat: f64, lon: f64, m: [f64; 3]| -> [f64; 3] {
        let (st, ct) = colat.sin_cos();
        let (sp, cp) = lon.sin_cos();
        let nm = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
        [
            (m[0] * st * cp + m[1] * ct * cp - m[2] * sp) / nm,
            (m[0] * st * sp + m[1] * ct * sp + m[2] * cp) / nm,
            (m[0] * ct - m[1] * st) / nm,
        ]
    };
    let p0 = launch_point();
    let d0 = {
        let (sb, cb) = elev.get().sin_cos();
        let (sa, ca) = Radians::from_degrees(70.0).get().sin_cos();
        dir(p0.colat.get(), p0.lon.get(), [sb, -cb * ca, cb * sa])
    };
    let d1 = dir(res.end.colat.get(), res.end.lon.get(), res.end_m);
    let dot: f64 = d0.iter().zip(&d1).map(|(a, b)| a * b).sum();
    assert!(
        dot.min(1.0).acos() < 1e-10,
        "curvature residual {}",
        dot.min(1.0).acos()
    );
    let c = R0 * elev.get().cos();
    let r_top = R0 + 800e3;
    let chord = (r_top * r_top - c * c).sqrt() - (R0 * R0 - c * c).sqrt();
    assert!((res.group_path.get() - chord).abs() < 1e-9 * chord + 1e-5);
}

/// n depends on the wave normal only through Y_L^2 and Y_T^2
/// (appleton-hartree.md), so n(r, -k_hat) = n(r, k_hat) and the ray flow is
/// time-reversible: the reversed ray retraces the path with identical group
/// and phase paths. This holds separately for each mode.
#[test]
fn reciprocity_forward_backward() {
    let layer = chapman(9e6);
    let field = earthlike_dipole();
    for mode in [Mode::Ordinary, Mode::Extraordinary] {
        let tracer = Tracer::new(
            &layer,
            &field,
            &ZeroCollisions,
            Hertz::new(7.5e6),
            mode,
            config(),
        );
        let fwd = tracer
            .trace(
                &launch_point(),
                Radians::from_degrees(28.0),
                Radians::from_degrees(40.0),
            )
            .unwrap();
        assert_eq!(fwd.outcome, Outcome::Landed, "{mode:?}");
        let (elev_b, az_b) = reversed_launch(&fwd);
        let bwd = tracer.trace(&fwd.end, elev_b, az_b).unwrap();
        assert_eq!(bwd.outcome, Outcome::Landed, "{mode:?}");
        // Round-trip budget: two traces at rtol 1e-10 over ~2 Mm each plus
        // 1e-6 m event refinement: metres at worst.
        let gap = central_angle(&bwd.end, &launch_point()).get() * R0;
        assert!(gap < 5.0, "{mode:?}: reverse ray missed start by {gap} m");
        let dg = (fwd.group_path.get() - bwd.group_path.get()).abs();
        assert!(dg < 1.0, "{mode:?}: group paths differ by {dg} m");
        let dp = (fwd.phase_path.get() - bwd.phase_path.get()).abs();
        assert!(dp < 1.0, "{mode:?}: phase paths differ by {dp} m");
        assert!(fwd.hamiltonian_drift < 1e-10);
        assert!(bwd.hamiltonian_drift < 1e-10);
    }
}

#[test]
fn modes_split_with_field() {
    let layer = chapman(9e6);
    let field = earthlike_dipole();
    let run = |mode: Mode| {
        let t = Tracer::new(
            &layer,
            &field,
            &ZeroCollisions,
            Hertz::new(7.5e6),
            mode,
            config(),
        );
        t.trace(
            &launch_point(),
            Radians::from_degrees(28.0),
            Radians::from_degrees(40.0),
        )
        .unwrap()
    };
    let (o, x) = (run(Mode::Ordinary), run(Mode::Extraordinary));
    // With Y ~ 0.15 the magnetoionic splitting is macroscopic: the X mode
    // reflects at X = 1 - Y, below the O-mode X = 1 level.
    assert!(
        o.apexes[0].r.get() - x.apexes[0].r.get() > 1e3,
        "apex split"
    );
    assert!(
        (o.group_path.get() - x.group_path.get()).abs() > 100.0,
        "group split"
    );
    assert!(
        o.apexes[0].x > x.apexes[0].x,
        "O reflects at higher X than X mode"
    );
}

/// Near-vertical incidence: the mode cutoffs are the classic X = 1 (O) and
/// X = 1 - Y (X mode). Launched 0.5 deg off vertical and due EAST, i.e.
/// out of the magnetic meridian: a meridian-plane near-vertical O-mode ray
/// runs into the Spitze cusp (wave normal driven into field alignment at
/// X -> 1, the Ellis-window degeneracy) where the tracer correctly reports
/// step collapse - see haselgrove.md section 7 and the companion test
/// below. Off-meridian, the apex wave normal is nearly transverse to B and
/// the cutoff residual is O(m_horizontal^2) ~ 1e-4; tolerance 3e-3 also
/// covers the position drift entering Y.
#[test]
fn vertical_reflection_conditions_with_field() {
    let layer = chapman(9e6);
    let field = Dipole::axial(Meters::new(6_371_200.0), Tesla::new(-3.0e-5));
    let start = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(50.0),
        Radians::new(0.0),
    );
    let run = |mode: Mode| {
        let t = Tracer::new(
            &layer,
            &field,
            &ZeroCollisions,
            Hertz::new(5e6),
            mode,
            config(),
        );
        t.trace(
            &start,
            Radians::from_degrees(89.5),
            Radians::from_degrees(90.0),
        )
        .unwrap()
    };
    let o = run(Mode::Ordinary);
    assert!(!o.apexes.is_empty());
    let xo = o.apexes[0].x;
    assert!((xo - 1.0).abs() < 3e-3, "O-mode apex X = {xo}");
    let x = run(Mode::Extraordinary);
    let apex = &x.apexes[0];
    let p_apex = SphericalPoint::new(apex.r, Radians::from_degrees(50.0), Radians::new(0.0));
    let b = field.sample(&p_apex).b;
    let b_mag = (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt();
    let y = skipzone::constants::OMEGA_H_PER_TESLA * b_mag / Hertz::new(5e6).angular();
    assert!(
        (apex.x - (1.0 - y)).abs() < 3e-3,
        "X-mode apex X = {} vs 1 - Y = {}",
        apex.x,
        1.0 - y
    );
}

/// The Spitze: a near-vertical O-mode ray in the magnetic meridian drives
/// its wave normal into field alignment at X -> 1 (the Ellis-window
/// degeneracy, appleton-hartree.md section 6); the physical ray has a cusp
/// and smooth integration is impossible there. Contract: the tracer fails
/// with the typed step-collapse error - never a silently wrong path
/// (haselgrove.md section 7).
#[test]
fn spitze_meridian_ray_fails_typed() {
    let layer = chapman(9e6);
    let field = Dipole::axial(Meters::new(6_371_200.0), Tesla::new(-3.0e-5));
    let start = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(50.0),
        Radians::new(0.0),
    );
    let t = Tracer::new(
        &layer,
        &field,
        &ZeroCollisions,
        Hertz::new(5e6),
        Mode::Ordinary,
        config(),
    );
    let err = t
        .trace(&start, Radians::from_degrees(89.5), Radians::new(0.0))
        .expect_err("meridian-plane Spitze ray must not produce a path");
    assert!(
        matches!(err, skipzone::error::TraceError::StepSizeCollapse { .. }),
        "unexpected error kind: {err}"
    );
}

/// Rays approach the vacuum geodesic as f^-2: n^2 - 1 = -X(1 -+ Y_L + ...)
/// with X ~ f^-2 and the magnetic correction one order higher (f^-3), so
/// the range deviation from the vacuum chord must fall at observed order 2.
#[test]
fn high_frequency_geometric_limit() {
    let nm = density_at_critical_frequency(Hertz::new(7e6));
    let layer = QuasiParabolicLayer::new(
        PerCubicMeter::new(nm.get()),
        Meters::new(R0 + 300e3),
        Meters::new(100e3),
    )
    .unwrap();
    let field = earthlike_dipole();
    let elev = Radians::from_degrees(30.0);
    let dev = |f_hz: f64| -> f64 {
        let t = Tracer::new(
            &layer,
            &field,
            &ZeroCollisions,
            Hertz::new(f_hz),
            Mode::Ordinary,
            config(),
        );
        let res = t
            .trace(&launch_point(), elev, Radians::from_degrees(40.0))
            .unwrap();
        assert_eq!(res.outcome, Outcome::Escaped);
        let c = R0 * elev.get().cos();
        let r_top = R0 + 800e3;
        let vac_delta = (c / r_top).acos() - (c / R0).acos();
        (central_angle(&launch_point(), &res.end).get() - vac_delta).abs()
    };
    let (d1, d2, d3) = (dev(30e6), dev(60e6), dev(120e6));
    let (p1, p2) = ((d1 / d2).log2(), (d2 / d3).log2());
    assert!(
        (1.8..=2.2).contains(&p1),
        "order {p1} (dev {d1:e} -> {d2:e})"
    );
    assert!(
        (1.8..=2.2).contains(&p2),
        "order {p2} (dev {d2:e} -> {d3:e})"
    );
}
