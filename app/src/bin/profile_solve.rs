//! Profiling harness for one point-to-point solve.
//!
//! Counts, for a chosen scenario, where the time in `solve()` actually goes:
//! traces, integrator steps, density/field/collision evaluations, and the raw
//! cost of one evaluation of each model. Instrumentation is entirely app-side:
//! the engine models are wrapped in counting decorators, so nothing in the
//! engine changes and the physics is bit-identical.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use skipzone::collision::{CollisionFrequency, CollisionSample};
use skipzone::density::{DensitySample, ElectronDensity};
use skipzone::geo::{SphericalPoint, bearing, central_angle};
use skipzone::homing::{Homing, HomingConfig};
use skipzone::mag::{FieldSample, MagneticField};
use skipzone::magnetoionic::Mode;
use skipzone::trace::{Outcome, TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};

use skipzone_app::scenario::{self, EARTH_RADIUS_M, Inputs, ground_point};

static N_DENSITY: AtomicU64 = AtomicU64::new(0);
static N_FIELD: AtomicU64 = AtomicU64::new(0);
static N_COLL: AtomicU64 = AtomicU64::new(0);
static N_TRACE: AtomicU64 = AtomicU64::new(0);

fn counts() -> (u64, u64, u64) {
    (
        N_DENSITY.load(Ordering::Relaxed),
        N_FIELD.load(Ordering::Relaxed),
        N_COLL.load(Ordering::Relaxed),
    )
}

struct CountingDensity<'a>(&'a (dyn ElectronDensity + Sync));
impl ElectronDensity for CountingDensity<'_> {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        N_DENSITY.fetch_add(1, Ordering::Relaxed);
        self.0.sample(p)
    }
}

struct CountingField<'a>(&'a (dyn MagneticField + Sync));
impl MagneticField for CountingField<'_> {
    fn sample(&self, p: &SphericalPoint) -> FieldSample {
        N_FIELD.fetch_add(1, Ordering::Relaxed);
        self.0.sample(p)
    }
}

struct CountingCollisions<'a>(&'a (dyn CollisionFrequency + Sync));
impl CollisionFrequency for CountingCollisions<'_> {
    fn sample(&self, p: &SphericalPoint) -> CollisionSample {
        N_COLL.fetch_add(1, Ordering::Relaxed);
        self.0.sample(p)
    }
}

/// Density wrapper that also counts calls made at the launch radius, which is
/// one per `initial_state`, i.e. one per trace attempt.
struct TraceCountingDensity<'a> {
    inner: &'a (dyn ElectronDensity + Sync),
    launch_r: f64,
}
impl ElectronDensity for TraceCountingDensity<'_> {
    fn sample(&self, p: &SphericalPoint) -> DensitySample {
        N_DENSITY.fetch_add(1, Ordering::Relaxed);
        if p.r.get() == self.launch_r {
            N_TRACE.fetch_add(1, Ordering::Relaxed);
        }
        self.inner.sample(p)
    }
}

fn scenarios() -> Vec<(&'static str, Inputs)> {
    vec![
        ("default (Denver->London 14.1 MHz)", Inputs::default()),
        (
            "no-connect (45 MHz, triggers near-miss sweep)",
            Inputs {
                freq_mhz: 45.0,
                ..Inputs::default()
            },
        ),
        (
            "short summer path 18.1 MHz (Es stack active)",
            Inputs {
                freq_mhz: 18.1,
                month: 7,
                day_of_month: 24,
                utc_hours: 3.37,
                tx_lat: 40.0,
                tx_lon: -105.0,
                rx_lat: 43.6,
                rx_lon: -105.0,
                max_hops: 2,
                ..Inputs::default()
            },
        ),
        (
            "default + Es forced on (two stacks, 4 hops)",
            Inputs {
                es_manual: true,
                foes_mhz: 5.0,
                es_probability: 0.4,
                ..Inputs::default()
            },
        ),
        (
            "7.1 MHz daytime (E-screened, no bracket anywhere)",
            Inputs {
                freq_mhz: 7.1,
                month: 6,
                day_of_month: 21,
                utc_hours: 19.0,
                rx_lat: 40.7,
                rx_lon: -74.0,
                max_hops: 3,
                ..Inputs::default()
            },
        ),
    ]
}

fn main() {
    println!("== SCENARIO WALL TIMES (full solve) ==");
    for (name, inputs) in scenarios() {
        let a = scenario::resolve(&inputs);
        let models = scenario::build_models(&inputs, &a).expect("models");
        // Repeat and report the median: the first solve in a process pays for
        // the rayon pool's threads being created.
        let mut runs = Vec::new();
        let mut out = skipzone_app::solve::solve(&inputs, &a, &models);
        for _ in 0..7 {
            let t = Instant::now();
            out = skipzone_app::solve::solve(&inputs, &a, &models);
            runs.push(t.elapsed().as_secs_f64() * 1e3);
        }
        runs.sort_by(f64::total_cmp);
        println!(
            "  {:<44} {:>8.1} ms  (min {:>7.1}, max {:>7.1})  solutions {}  es {}  near-miss {}",
            name,
            runs[runs.len() / 2],
            runs[0],
            runs[runs.len() - 1],
            out.solutions.len(),
            out.es_solutions.len(),
            out.near_misses.len(),
        );
    }

    let inputs = Inputs::default();
    let a = scenario::resolve(&inputs);
    let models = scenario::build_models(&inputs, &a).expect("models");

    println!("\n== FIXED PER-SOLVE OVERHEAD (not ray tracing) ==");
    {
        use skipzone_app::antenna::Ground;
        let f_hz = inputs.freq_mhz * 1e6;
        let g = Ground::Lossy {
            eps_r: 13.0,
            sigma_s_per_m: 0.005,
        };
        let t = Instant::now();
        let mut guard = 0.0;
        for _ in 0..20 {
            guard += inputs.tx_antenna.curve(g, f_hz).gain_dbi(0.3);
            guard += inputs.rx_antenna.curve(g, f_hz).gain_dbi(0.3);
        }
        println!(
            "  both antenna curves   {:>8.3} ms   (guard {guard:.3})",
            t.elapsed().as_secs_f64() * 1e3 / 20.0
        );
        let t = Instant::now();
        for _ in 0..20 {
            let _ = scenario::build_models(&inputs, &a);
        }
        println!(
            "  build_models          {:>8.3} ms",
            t.elapsed().as_secs_f64() * 1e3 / 20.0
        );
    }

    println!(
        "\nes_solved = {}, max_hops = {}, use_field = {}",
        a.es_solved, inputs.max_hops, inputs.use_field
    );

    let tx = ground_point(inputs.tx_lat, inputs.tx_lon);
    let rx = ground_point(inputs.rx_lat, inputs.rx_lon);
    let brng = bearing(&tx, &rx);
    let total_arc = central_angle(&tx, &rx);

    let cd = TraceCountingDensity {
        inner: &models.density,
        launch_r: tx.r.get(),
    };
    let cf = CountingField(models.field.as_ref().expect("igrf"));
    let cc = CountingCollisions(&models.collisions);

    let config = TraceConfig::new(Meters::new(a.r_ground_m), Meters::new(a.r_top_m));
    let tracer = Tracer::new(
        &cd,
        &cf,
        &cc,
        Hertz::new(inputs.freq_mhz * 1e6),
        Mode::Ordinary,
        config,
    );

    // ---------------------------------------------------------------- one trace
    println!("\n== ONE TRACE (default scenario, elev 20 deg) ==");
    let before = counts();
    let t = Instant::now();
    let r = tracer
        .trace(&tx, Radians::from_degrees(20.0), brng)
        .expect("trace");
    let one = t.elapsed();
    let after = counts();
    let (dd, df, dc) = (after.0 - before.0, after.1 - before.1, after.2 - before.2);
    #[allow(clippy::cast_precision_loss)]
    let steps = r.steps as f64;
    println!("  wall              {:>10.3} ms", one.as_secs_f64() * 1e3);
    println!("  accepted steps    {:>10}", r.steps);
    println!("  outcome           {:?}", r.outcome);
    #[allow(clippy::cast_precision_loss)]
    {
        println!("  density samples   {dd:>10}   ({:.2} / step)", dd as f64 / steps);
        println!("  field samples     {df:>10}   ({:.2} / step)", df as f64 / steps);
        println!("  collision samples {dc:>10}   ({:.2} / step)", dc as f64 / steps);
    }

    // ---------------------------------------------------------------- raw model cost
    println!("\n== RAW MODEL EVALUATION COST ==");
    let p = SphericalPoint::new(
        Meters::new(EARTH_RADIUS_M + 250e3),
        Radians::from_degrees(45.0),
        Radians::from_degrees(-60.0),
    );
    let n = 200_000u32;
    macro_rules! bench {
        ($name:expr, $e:expr) => {{
            let t = Instant::now();
            let mut acc = 0.0f64;
            for i in 0..n {
                let pp =
                    SphericalPoint::new(Meters::new(p.r.get() + f64::from(i % 100)), p.colat, p.lon);
                acc += $e(&pp);
            }
            let el = t.elapsed();
            println!(
                "  {:<22} {:>8.1} ns/call   (guard {:.3e})",
                $name,
                el.as_secs_f64() * 1e9 / f64::from(n),
                acc
            );
            el
        }};
    }
    let d: &dyn ElectronDensity = &models.density;
    let fl: &dyn MagneticField = models.field.as_ref().expect("igrf");
    let co: &dyn CollisionFrequency = &models.collisions;
    let t_d = bench!("density.sample", |q: &SphericalPoint| d.sample(q).ne);
    let t_f = bench!("field.sample", |q: &SphericalPoint| fl.sample(q).b[0]);
    let t_c = bench!("collisions.sample", |q: &SphericalPoint| co.sample(q).nu);
    let per_rhs = (t_d + t_f + t_c).as_secs_f64() * 1e9 / f64::from(n);
    println!("  => one RHS-worth of model evaluation: {per_rhs:.0} ns");
    println!(
        "     field share {:.0} %, density share {:.0} %",
        100.0 * t_f.as_secs_f64() / (t_d + t_f + t_c).as_secs_f64(),
        100.0 * t_d.as_secs_f64() / (t_d + t_f + t_c).as_secs_f64(),
    );

    // ---------------------------------------------------------------- scan phase
    println!("\n== HOMING: SCAN PHASE, replicated (4..=80 deg, 1 deg step) ==");
    let before = counts();
    let t = Instant::now();
    let (mut n_scan, mut n_landed, mut steps_total) = (0u32, 0u32, 0usize);
    let mut e = 4.0f64;
    while e <= 80.0 {
        n_scan += 1;
        if let Ok(res) = tracer.trace(&tx, Radians::from_degrees(e), brng) {
            steps_total += res.steps;
            if res.outcome == Outcome::Landed {
                n_landed += 1;
            }
        }
        e += 1.0;
    }
    let scan_time = t.elapsed();
    let after = counts();
    #[allow(clippy::cast_precision_loss)]
    {
        println!("  traces            {n_scan:>10}   ({n_landed} landed)");
        println!(
            "  total steps       {steps_total:>10}   ({:.0} / trace)",
            steps_total as f64 / f64::from(n_scan)
        );
        println!("  density samples   {:>10}", after.0 - before.0);
        println!(
            "  wall              {:>10.3} s   ({:.2} ms / trace)",
            scan_time.as_secs_f64(),
            scan_time.as_secs_f64() * 1e3 / f64::from(n_scan)
        );
    }

    // ---------------------------------------------------------------- home_scan per hop count
    println!("\n== HOMING: home_scan PER HOP COUNT (one mode, deterministic stack) ==");
    let homing = Homing {
        tracer: &tracer,
        config: HomingConfig {
            miss_tolerance_m: 2000.0,
            ..HomingConfig::default()
        },
    };
    let mut grand = std::time::Duration::ZERO;
    let mut grand_traces = 0u64;
    for hops in 1..=inputs.max_hops {
        let target = if hops == 1 {
            rx
        } else {
            scenario::destination_point(&tx, brng, Radians::new(total_arc.get() / f64::from(hops)))
        };
        let before_t = N_TRACE.load(Ordering::Relaxed);
        let t = Instant::now();
        let res = homing.home_scan(&tx, &target);
        let el = t.elapsed();
        grand += el;
        let traces = N_TRACE.load(Ordering::Relaxed) - before_t;
        grand_traces += traces;
        println!(
            "  hops {hops}: {:>7.3} s   traces {traces:>5}  (scan 77, refine {:>4})  -> {}",
            el.as_secs_f64(),
            traces.saturating_sub(77),
            match &res {
                Ok(v) => format!("{} ray(s)", v.len()),
                Err(err) => format!("{err}"),
            }
        );
    }
    println!(
        "  1 mode, 1 stack, {} hop counts: {:.3} s, {grand_traces} traces",
        inputs.max_hops,
        grand.as_secs_f64()
    );
    println!(
        "  of which SCAN traces: {} ({} of them are exact duplicates of the first hop count's scan)",
        77 * inputs.max_hops,
        77 * (inputs.max_hops - 1)
    );
}

// Appended probe: fixed per-solve overhead that is not ray tracing.
#[allow(dead_code)]
fn probe_overhead() {}
