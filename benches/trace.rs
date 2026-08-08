//! Criterion baseline (build-order step 10). Scenarios chosen to be the
//! production shape: full magnetoionic ray through IGRF-14 + Chapman +
//! exponential collisions, landing after one hop.

use criterion::{Criterion, criterion_group, criterion_main};
use skipzone::collision::ExponentialCollisions;
use skipzone::density::{ChapmanLayer, density_at_critical_frequency};
use skipzone::geo::SphericalPoint;
use skipzone::mag::Igrf;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, PerSecond, Radians};
use std::hint::black_box;

const R0: f64 = 6_371_000.0;

fn bench_trace(c: &mut Criterion) {
    let layer = ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(9e6)),
        Meters::new(R0 + 300e3),
        Meters::new(60e3),
    )
    .unwrap();
    let field = Igrf::from_embedded().unwrap().model_at(2026.5).unwrap();
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
    let start = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(55.0),
        Radians::from_degrees(10.0),
    );

    c.bench_function("single_hop_igrf_chapman", |b| {
        b.iter(|| {
            let res = tracer
                .trace(
                    black_box(&start),
                    Radians::from_degrees(28.0),
                    Radians::from_degrees(40.0),
                )
                .unwrap();
            black_box(res.group_path)
        });
    });

    // A fan of launches, traced one after another. The engine is deliberately
    // single-threaded: parallelism is the app's `compute` layer, which fans
    // whole solves out rather than individual rays. This measures the serial
    // cost that layer is dividing up.
    let fan: Vec<_> = (0..64)
        .map(|i| {
            (
                start,
                Radians::from_degrees(10.0 + 0.5 * f64::from(i)),
                Radians::from_degrees(40.0),
            )
        })
        .collect();
    c.bench_function("fan_64_rays_serial", |b| {
        b.iter(|| {
            let out: Vec<_> = black_box(&fan)
                .iter()
                .map(|(p, elev, az)| tracer.trace(p, *elev, *az))
                .collect();
            black_box(out.len())
        });
    });
}

criterion_group!(benches, bench_trace);
criterion_main!(benches);
