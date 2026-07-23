//! Validation A-C for the field-free tracer against solutions derived in
//! docs/derivations/analytic-solutions.md. Every reference here is computed
//! from that document's formulas, independently of the tracer.

use skipzone::collision::ZeroCollisions;
use skipzone::density::{
    ChapmanLayer, ElectronDensity, LinearLayer, ParabolicLayer, QuasiParabolicLayer, ZeroDensity,
    density_at_critical_frequency,
};
use skipzone::geo::central_angle;
use skipzone::mag::ZeroField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, PerCubicMeter, Radians};

mod common;
use common::{R0, bouguer_reference, cartesian_direction, ground, qp_closed_form, x_of_r};

#[test]
fn vacuum_rays_are_straight() {
    let density = ZeroDensity;
    let tracer = Tracer::new(
        &density,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(10e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 500e3)),
    );
    for elev_deg in [10.0, 30.0, 60.0, 89.0] {
        let beta = Radians::from_degrees(elev_deg);
        let res = tracer
            .trace(&ground(), beta, Radians::from_degrees(90.0))
            .unwrap();
        assert_eq!(res.outcome, Outcome::Escaped);
        let r_top = R0 + 500e3;
        assert!((res.end.r.get() - r_top).abs() < 2e-6, "top refinement");
        let c = R0 * beta.get().cos();
        let chord = (r_top * r_top - c * c).sqrt() - (R0 * R0 - c * c).sqrt();
        let delta = (c / r_top).acos() - (c / R0).acos();
        let got_delta = central_angle(&ground(), &res.end).get();
        // Sub-micrometre agreement over hundreds of km: 1e-9 relative covers
        // the event tolerance (1e-6 m) plus integrator error at rtol 1e-10.
        assert!(
            (res.group_path.get() - chord).abs() < 1e-9 * chord + 1e-5,
            "group vs chord"
        );
        assert!(
            (res.phase_path.get() - chord).abs() < 1e-9 * chord + 1e-5,
            "phase vs chord"
        );
        assert!(
            (res.arc_length.get() - chord).abs() < 1e-9 * chord + 1e-5,
            "arc vs chord"
        );
        assert!((got_delta - delta).abs() < 1e-9, "central angle");
        // Residual curvature = angle between launch and landing directions
        // in a fixed Cartesian frame; reported bound is integrator error.
        let d0 = cartesian_direction(
            ground().colat.get(),
            ground().lon.get(),
            [beta.get().sin(), 0.0, beta.get().cos()],
        );
        let d1 = cartesian_direction(res.end.colat.get(), res.end.lon.get(), res.end_m);
        let dot: f64 = d0.iter().zip(&d1).map(|(a, b)| a * b).sum();
        assert!(
            dot.min(1.0).acos() < 1e-10,
            "residual curvature {}",
            dot.min(1.0).acos()
        );
        assert!(
            res.hamiltonian_drift < 1e-13,
            "H drift {}",
            res.hamiltonian_drift
        );
    }
}

#[test]
fn quasi_parabolic_closed_form_agreement() {
    let (nm, rm, ym) = (
        density_at_critical_frequency(Hertz::new(7e6)).get(),
        R0 + 300e3,
        100e3,
    );
    let layer =
        QuasiParabolicLayer::new(PerCubicMeter::new(nm), Meters::new(rm), Meters::new(ym)).unwrap();
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(8e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 700e3)),
    );
    for elev_deg in [12.0, 20.0, 28.0, 35.0] {
        let beta = Radians::from_degrees(elev_deg);
        let res = tracer
            .trace(&ground(), beta, Radians::from_degrees(90.0))
            .unwrap();
        assert_eq!(res.outcome, Outcome::Landed, "elev {elev_deg}");
        let (delta, group, phase, rt) = qp_closed_form(&layer, nm, rm, ym, 8e6, R0, beta.get());
        let got_delta = central_angle(&ground(), &res.end).get();
        // Tolerances: ground range to 0.5 m over ~1-3 Mm, paths to 0.5 m,
        // apex to 0.1 m; roughly 10x the observed rtol=1e-10 error so the
        // test fails on any real regression but not on roundoff jitter.
        assert!(
            (got_delta - delta).abs() * R0 < 0.5,
            "elev {elev_deg}: range {} vs {}",
            got_delta * R0,
            delta * R0
        );
        assert!(
            (res.group_path.get() - group).abs() < 0.5,
            "elev {elev_deg}: group {} vs {group}",
            res.group_path.get()
        );
        assert!(
            (res.phase_path.get() - phase).abs() < 0.5,
            "elev {elev_deg}: phase {} vs {phase}",
            res.phase_path.get()
        );
        assert_eq!(res.apexes.len(), 1);
        assert!(
            (res.apexes[0].r.get() - rt).abs() < 0.1,
            "elev {elev_deg}: apex {} vs {rt}",
            res.apexes[0].r.get()
        );
        // Phase path <= group path strictly inside a plasma.
        assert!(res.phase_path.get() < res.group_path.get());
        // Zero collisions => exactly zero absorption (bit-exact invariant).
        assert_eq!(res.absorption.get(), 0.0);
    }
}

#[test]
fn qp_closed_form_cross_checked_by_quadrature() {
    // The two references must agree with each other far more tightly than
    // either agrees with the tracer: catches algebra slips in the closed
    // forms independently of the integrator.
    let (nm, rm, ym) = (
        density_at_critical_frequency(Hertz::new(7e6)).get(),
        R0 + 300e3,
        100e3,
    );
    let layer =
        QuasiParabolicLayer::new(PerCubicMeter::new(nm), Meters::new(rm), Meters::new(ym)).unwrap();
    for elev_deg in [12.0, 20.0, 22.0, 28.0, 35.0] {
        let beta = Radians::from_degrees(elev_deg).get();
        let x = x_of_r(&layer, 8e6);
        let refq = bouguer_reference(&x, None, R0, beta, R0 + 700e3, &[layer.base_radius().get()]);
        let (delta, group, phase, rt) = qp_closed_form(&layer, nm, rm, ym, 8e6, R0, beta);
        assert!(
            (refq.delta - delta).abs() * R0 < 1e-4,
            "elev {elev_deg} delta: quadrature {} vs closed {delta} (x R0: {})",
            refq.delta,
            (refq.delta - delta).abs() * R0
        );
        assert!(
            (refq.group - group).abs() < 1e-4,
            "elev {elev_deg} group: quadrature {} vs closed {group}, apex q {} vs closed {rt}",
            refq.group,
            refq.r_apex
        );
        assert!(
            (refq.phase - phase).abs() < 1e-4,
            "elev {elev_deg} phase: quadrature {} vs closed {phase}",
            refq.phase
        );
        assert!((refq.r_apex - rt).abs() < 1e-4, "elev {elev_deg} apex");
    }
}

#[test]
fn chapman_linear_parabolic_match_bouguer_quadrature() {
    let scenarios: Vec<(Box<dyn ElectronDensity>, Vec<f64>, f64)> = vec![
        (
            Box::new(
                ChapmanLayer::new(
                    density_at_critical_frequency(Hertz::new(9e6)),
                    Meters::new(R0 + 300e3),
                    Meters::new(60e3),
                )
                .unwrap(),
            ),
            vec![],
            7e6,
        ),
        (
            // Slope 6e6 m^-4: X = 1 for 7 MHz about 100 km above the base -
            // an ionosphere-scale gradient (6e9 would put the reflection
            // 100 m above the base: an unresolvable wall, not a layer).
            Box::new(LinearLayer::new(Meters::new(R0 + 180e3), 6e6).unwrap()),
            vec![R0 + 180e3],
            7e6,
        ),
        (
            Box::new(
                ParabolicLayer::new(
                    density_at_critical_frequency(Hertz::new(9e6)),
                    Meters::new(R0 + 300e3),
                    Meters::new(100e3),
                )
                .unwrap(),
            ),
            vec![R0 + 200e3, R0 + 400e3],
            7e6,
        ),
    ];
    for (model, breaks, f_hz) in &scenarios {
        // Kinked profiles (linear/parabolic edges) force the controller to
        // localise the gradient jump with steps ~ sqrt(rtol/jump) ~ mm;
        // allow it to go there instead of declaring collapse.
        let mut config = TraceConfig::new(Meters::new(R0), Meters::new(R0 + 800e3));
        config.min_step = 1e-5;
        let tracer = Tracer::new(
            model.as_ref(),
            &ZeroField,
            &ZeroCollisions,
            Hertz::new(*f_hz),
            Mode::Ordinary,
            config,
        );
        let beta = Radians::from_degrees(25.0);
        let res = tracer
            .trace(&ground(), beta, Radians::from_degrees(90.0))
            .unwrap();
        assert_eq!(res.outcome, Outcome::Landed);
        let x = x_of_r(model.as_ref(), *f_hz);
        let refq = bouguer_reference(&x, None, R0, beta.get(), R0 + 800e3, breaks);
        let got_delta = central_angle(&ground(), &res.end).get();
        assert!(
            (got_delta - refq.delta).abs() * R0 < 1.0,
            "range {} vs {}",
            got_delta * R0,
            refq.delta * R0
        );
        assert!(
            (res.group_path.get() - refq.group).abs() < 1.0,
            "group {} vs {}",
            res.group_path.get(),
            refq.group
        );
        assert!(
            (res.phase_path.get() - refq.phase).abs() < 1.0,
            "phase {} vs {}",
            res.phase_path.get(),
            refq.phase
        );
        assert!((res.apexes[0].r.get() - refq.r_apex).abs() < 0.5);
    }
}

#[test]
fn bouguer_invariant_and_apex_condition_along_ray() {
    let layer = ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(9e6)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap();
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(7e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 800e3)),
    );
    let beta = Radians::from_degrees(30.0);
    let c = R0 * beta.get().cos();
    let mut max_dev_l = 0.0f64;
    let mut max_dev_pphi = 0.0f64;
    let p_phi_0 = c; // launch: r sin(colat) m_phi = r0 * 1 * cos(beta)
    let res = tracer
        .trace_with_observer(
            &ground(),
            beta,
            Radians::from_degrees(90.0),
            &mut |_s, y| {
                let l = y[0] * (y[4] * y[4] + y[5] * y[5]).sqrt();
                max_dev_l = max_dev_l.max((l - c).abs() / c);
                let p_phi = y[0] * y[1].sin() * y[5];
                max_dev_pphi = max_dev_pphi.max((p_phi - p_phi_0).abs() / p_phi_0);
            },
        )
        .unwrap();
    // Bouguer L = n r sin(chi) and the azimuthal momentum are exact
    // invariants of the continuous equations (analytic-solutions.md sec. 1);
    // deviation is pure integrator error, budget ~ rtol * path/step scale.
    assert!(max_dev_l < 1e-9, "Bouguer deviation {max_dev_l:e}");
    assert!(max_dev_pphi < 1e-9, "p_phi deviation {max_dev_pphi:e}");
    // Apex: n^2 r^2 = C^2 (chi = 90 deg) and the recorded X obeys it.
    let apex = &res.apexes[0];
    let n_sq_apex = 1.0 - apex.x;
    let lhs = n_sq_apex * apex.r.get() * apex.r.get();
    assert!(
        (lhs - c * c).abs() / (c * c) < 1e-9,
        "apex condition: n^2 r^2 = {lhs} vs C^2 = {}",
        c * c
    );
    // Symmetric medium => symmetric ray: arrival elevation mirrors launch.
    let m_h_end = (res.end_m[1] * res.end_m[1] + res.end_m[2] * res.end_m[2]).sqrt();
    let arrival = (-res.end_m[0]).atan2(m_h_end);
    assert!(
        (arrival - beta.get()).abs() < 1e-9,
        "arrival elevation {arrival} vs launch {}",
        beta.get()
    );
    // Ray launched due east from the equator stays in the equatorial plane.
    assert!((res.end.colat.get() - ground().colat.get()).abs() < 1e-9);
}

#[test]
fn convergence_order_is_five() {
    let layer = ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(9e6)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap();
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(7e6),
        Mode::Ordinary,
        TraceConfig::new(Meters::new(R0), Meters::new(R0 + 800e3)),
    );
    let beta = Radians::from_degrees(35.0);
    let az = Radians::from_degrees(90.0);
    // Span crosses the apex (smooth in sigma; haselgrove.md section 7).
    let span = 900e3;
    let sol = |n: usize| {
        tracer
            .integrate_fixed(&ground(), beta, az, span, n)
            .unwrap()
    };
    let err = |a: &[f64; 10], b: &[f64; 10]| -> f64 {
        (0..10)
            .map(|i| ((a[i] - b[i]) / b[i].abs().max(1.0)).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let (y1, y2, y3, y4) = (sol(200), sol(400), sol(800), sol(1600));
    let (e1, e2, e3) = (err(&y1, &y2), err(&y2, &y3), err(&y3, &y4));
    let (p1, p2) = ((e1 / e2).log2(), (e2 / e3).log2());
    // Theoretical order 5 for DOPRI5 on a C-infinity RHS. Window +-0.7
    // covers the error-constant wobble between halvings; a wrong tableau
    // or gradient shows up as order ~1-4 and fails hard.
    assert!(e1 > 1e-13, "errors already at roundoff; enlarge steps");
    assert!(
        (4.3..=5.7).contains(&p1),
        "observed order {p1} (e1={e1:e}, e2={e2:e})"
    );
    assert!(
        (4.3..=5.7).contains(&p2),
        "observed order {p2} (e2={e2:e}, e3={e3:e})"
    );
}
