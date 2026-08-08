//! Near-vertical-incidence skywave (NVIS) probe: where does the high-angle
//! part of the elevation fan actually land?
//!
//! # What it shows
//!
//! A field-free, collisionless Chapman layer (foF2 5 MHz, peak 280 km,
//! scale 50 km) probed at 4 MHz, which is comfortably below the layer's
//! critical frequency, so every launch angle here should reflect rather than
//! penetrate. It prints range and apex height for launch elevations from 50 to
//! 88 degrees.
//!
//! # What a good result looks like
//!
//! Every row reports `Landed`. Range falls monotonically as elevation rises,
//! down to a few tens of kilometres at 88 degrees, and apex height climbs
//! towards but never reaches the 280 km peak. A `Penetrated` row, a range that
//! turns back upward, or an error near the vertical means the high-angle
//! branch has broken - most likely the Spitze region, where the ray turns
//! almost exactly at the vertical and the apex refinement has the least to
//! bracket.
//!
//! This is a diagnostic, not a test: it has no assertions and nothing runs it
//! automatically. The equivalent invariants that ARE enforced live in
//! `tests/analytic_field_free.rs`.
//!
//! ```text
//! cargo run --release --example nvis_probe
//! ```

use skipzone::collision::ZeroCollisions;
use skipzone::density::{ChapmanLayer, density_at_critical_frequency};
use skipzone::geo::{SphericalPoint, central_angle};
use skipzone::mag::ZeroField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};
const R0: f64 = 6_371_000.0;
fn main() {
    let layer = ChapmanLayer::new(
        density_at_critical_frequency(Hertz::new(5e6)),
        Meters::new(R0 + 280e3),
        Meters::new(50e3),
    )
    .unwrap();
    let cfg = TraceConfig::new(Meters::new(R0), Meters::new(R0 + 600e3));
    let t = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(4e6),
        Mode::Ordinary,
        cfg,
    );
    let start = SphericalPoint::new(
        Meters::new(R0),
        Radians::from_degrees(90.0),
        Radians::new(0.0),
    );
    for e in [50.0, 60.0, 70.0, 75.0, 80.0, 84.0, 86.0, 88.0] {
        match t.trace(
            &start,
            Radians::from_degrees(e),
            Radians::from_degrees(90.0),
        ) {
            Ok(r) => {
                let rng = central_angle(&start, &r.end).get() * R0 / 1e3;
                let apex = r
                    .apexes
                    .first()
                    .map_or(f64::NAN, |a| (a.r.get() - R0) / 1e3);
                println!(
                    "elev {e:>4}: {:?} range {rng:8.1} km apex {apex:6.1} km",
                    r.outcome
                );
            }
            Err(err) => println!("elev {e:>4}: ERROR {err}"),
        }
    }
}
