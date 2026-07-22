//! `hfpredict` — a first-order HF path predictor that WIRES TOGETHER the
//! already-validated Skipzone engine (Chapman electron-density layer, the
//! Haselgrove ray tracer, and the shooting/Newton homing). It adds no new
//! physics: every propagation number comes from library code that is covered
//! by the analytic-solution and invariant test suites.
//!
//! What this tool does: given a transmitter, a receiver, an operating
//! frequency, and a time of day, it assumes a single Chapman F2 layer whose
//! peak frequency (foF2) and height (hmF2) are looked up from a COARSE,
//! explicitly-representative midlatitude climatology table (not live data,
//! not a path-specific prediction), then uses the validated homing to decide
//! whether a reflected ray path exists and reports its geometry.
//!
//! Honesty boundaries, stated plainly because the engine's whole purpose is
//! defensible numbers:
//!   * The foF2/hmF2 table is order-of-magnitude climatology. It is consistent
//!     with published midlatitude ranges (e.g. Davies, "Ionospheric Radio",
//!     1990; the diurnal/seasonal/solar behaviour and the winter anomaly are
//!     standard), but it is NOT a measured or forecast value for the specific
//!     path and time. The assumed numbers are printed so they can be checked,
//!     and `--fof2`/`--hmf2` override them with real ionosonde or prediction
//!     data — that is the defensible path for real work.
//!   * A single Chapman F2 layer only. No E/F1 layers, no horizontal gradients,
//!     no sporadic-E, no tilts.
//!   * Field-free O-mode by default (the most heavily validated path; O and X
//!     are bit-identical without a field). No magnetoionic splitting.
//!   * "Connects" here means a ray reflects and reaches the receiver
//!     geometrically. Absorption/signal strength is NOT modelled, so this is a
//!     Maximum-Usable-Frequency-style reachability check, not a link budget.
//!   * Multi-hop uses the equal-hop assumption: because the assumed ionosphere
//!     depends only on height, every hop is geometrically identical, so an
//!     N-hop path exists exactly when a single hop of 1/N the ground distance
//!     reflects. This reuses the single-hop homing unchanged.

use std::process::ExitCode;

use skipzone::collision::ZeroCollisions;
use skipzone::density::{ChapmanLayer, density_at_critical_frequency};
use skipzone::geo::{SphericalPoint, bearing, central_angle};
use skipzone::homing::{HomedRay, Homing, HomingConfig, HomingError};
use skipzone::mag::ZeroField;
use skipzone::magnetoionic::Mode;
use skipzone::trace::{TraceConfig, Tracer};
use skipzone::units::{Hertz, Meters, Radians};

/// Spherical Earth radius used consistently across the engine's test suites.
const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// Speed of light, m/s, for group-path -> time-of-flight (exact SI value; the
/// same constant the library uses internally).
const C_M_PER_S: f64 = 299_792_458.0;
/// Domain top for the tracer: well above any assumed hmF2 plus several scale
/// heights, so a penetrating ray cleanly escapes rather than being clipped.
const DOMAIN_TOP_M: f64 = EARTH_RADIUS_M + 600_000.0;
/// Default Chapman scale height, km. Representative bottomside-F2 value; real
/// scale heights run ~40-80 km. Override with `--scale-height`.
const DEFAULT_SCALE_HEIGHT_KM: f64 = 50.0;
/// Default maximum number of equal hops to try before declaring no path.
const DEFAULT_MAX_HOPS: u32 = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    match parse_args(&args).and_then(|a| run(&a)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!("try `hfpredict --help`");
            ExitCode::from(2)
        }
    }
}

fn print_usage() {
    println!(
        "hfpredict — first-order HF path predictor (Skipzone engine)\n\
\n\
USAGE:\n\
    hfpredict --tx LAT,LON --rx LAT,LON --freq MHZ --utc HOURS [options]\n\
\n\
REQUIRED:\n\
    --tx LAT,LON     transmitter geographic latitude,longitude in degrees\n\
    --rx LAT,LON     receiver geographic latitude,longitude in degrees\n\
    --freq MHZ       operating frequency in MHz\n\
    --utc HOURS      time of day, UTC, decimal hours (0..24)\n\
\n\
IONOSPHERE (coarse climatology unless overridden):\n\
    --month N        month 1..12, sets season (default 6)\n\
    --solar low|high solar-cycle level (default low)\n\
    --fof2 MHZ       override the looked-up F2 critical frequency\n\
    --hmf2 KM        override the looked-up F2 peak height\n\
    --scale-height KM   Chapman scale height (default {DEFAULT_SCALE_HEIGHT_KM})\n\
\n\
OTHER:\n\
    --max-hops N     equal-hop path attempts (default {DEFAULT_MAX_HOPS})\n\
    -h, --help       show this help\n\
\n\
The assumed foF2/hmF2 are coarse representative midlatitude values, NOT a\n\
prediction for the specific path/time. Override with --fof2/--hmf2 for\n\
defensible results. Absorption is not modelled; connectivity is geometric."
    );
}

#[derive(Clone, Copy)]
enum Solar {
    Low,
    High,
}

#[derive(Clone, Copy)]
enum Season {
    Summer,
    Winter,
    Equinox,
}

struct Args {
    tx: (f64, f64),
    rx: (f64, f64),
    freq_hz: f64,
    utc_hours: f64,
    month: u32,
    solar: Solar,
    fof2_override_mhz: Option<f64>,
    hmf2_override_km: Option<f64>,
    scale_height_km: f64,
    max_hops: u32,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut tx = None;
    let mut rx = None;
    let mut freq_mhz = None;
    let mut utc = None;
    let mut month = 6u32;
    let mut solar = Solar::Low;
    let mut fof2 = None;
    let mut hmf2 = None;
    let mut scale_height_km = DEFAULT_SCALE_HEIGHT_KM;
    let mut max_hops = DEFAULT_MAX_HOPS;

    let mut i = 0;
    while i < args.len() {
        let key = &args[i];
        let val = || {
            args.get(i + 1)
                .ok_or_else(|| format!("missing value after {key}"))
        };
        match key.as_str() {
            "--tx" => tx = Some(parse_lat_lon(val()?)?),
            "--rx" => rx = Some(parse_lat_lon(val()?)?),
            "--freq" => freq_mhz = Some(parse_f64(val()?, "--freq")?),
            "--utc" => utc = Some(parse_f64(val()?, "--utc")?),
            "--month" => month = parse_month(val()?)?,
            "--solar" => solar = parse_solar(val()?)?,
            "--fof2" => fof2 = Some(parse_f64(val()?, "--fof2")?),
            "--hmf2" => hmf2 = Some(parse_f64(val()?, "--hmf2")?),
            "--scale-height" => scale_height_km = parse_f64(val()?, "--scale-height")?,
            "--max-hops" => max_hops = parse_hops(val()?)?,
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }

    let freq_mhz = freq_mhz.ok_or("--freq is required")?;
    let utc_hours = utc.ok_or("--utc is required")?;
    if !(0.0..=24.0).contains(&utc_hours) {
        return Err("--utc must be in [0, 24]".into());
    }
    if freq_mhz <= 0.0 {
        return Err("--freq must be positive".into());
    }
    if scale_height_km <= 0.0 {
        return Err("--scale-height must be positive".into());
    }
    Ok(Args {
        tx: tx.ok_or("--tx is required")?,
        rx: rx.ok_or("--rx is required")?,
        freq_hz: freq_mhz * 1e6,
        utc_hours,
        month,
        solar,
        fof2_override_mhz: fof2,
        hmf2_override_km: hmf2,
        scale_height_km,
        max_hops,
    })
}

fn parse_lat_lon(s: &str) -> Result<(f64, f64), String> {
    let (a, b) = s
        .split_once(',')
        .ok_or_else(|| format!("expected LAT,LON but got '{s}'"))?;
    let lat = parse_f64(a.trim(), "latitude")?;
    let lon = parse_f64(b.trim(), "longitude")?;
    if !(-90.0..=90.0).contains(&lat) {
        return Err(format!("latitude {lat} out of range [-90, 90]"));
    }
    if !(-180.0..=360.0).contains(&lon) {
        return Err(format!("longitude {lon} out of range [-180, 360]"));
    }
    Ok((lat, lon))
}

fn parse_f64(s: &str, what: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .map_err(|_| format!("{what}: '{s}' is not a number"))
        .and_then(|v| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err(format!("{what}: value must be finite"))
            }
        })
}

fn parse_month(s: &str) -> Result<u32, String> {
    let m = s
        .parse::<u32>()
        .map_err(|_| format!("--month: '{s}' is not an integer"))?;
    if (1..=12).contains(&m) {
        Ok(m)
    } else {
        Err("--month must be 1..12".into())
    }
}

fn parse_hops(s: &str) -> Result<u32, String> {
    let n = s
        .parse::<u32>()
        .map_err(|_| format!("--max-hops: '{s}' is not an integer"))?;
    if (1..=10).contains(&n) {
        Ok(n)
    } else {
        Err("--max-hops must be 1..10".into())
    }
}

fn parse_solar(s: &str) -> Result<Solar, String> {
    match s.to_ascii_lowercase().as_str() {
        "low" | "min" => Ok(Solar::Low),
        "high" | "max" => Ok(Solar::High),
        other => Err(format!("--solar must be low|high, got '{other}'")),
    }
}

/// Season at a geographic latitude and month. Northern-hemisphere convention,
/// flipped south of the equator. Equinox months bundle spring and autumn.
fn season_at(month: u32, latitude_deg: f64) -> Season {
    let northern = matches!(month, 12 | 1 | 2 | 6 | 7 | 8);
    // Determine the astronomical bucket in northern-hemisphere terms first.
    let north_season = match month {
        12 | 1 | 2 => Season::Winter,
        6..=8 => Season::Summer,
        _ => Season::Equinox,
    };
    let _ = northern;
    if latitude_deg >= 0.0 {
        north_season
    } else {
        match north_season {
            Season::Winter => Season::Summer,
            Season::Summer => Season::Winter,
            Season::Equinox => Season::Equinox,
        }
    }
}

/// Local solar time, hours in [0, 24): UTC shifted by 15 deg/hour of east
/// longitude. Exact by definition of mean solar time; a crude day/night proxy,
/// not a solar-position calculation.
fn local_solar_time(utc_hours: f64, longitude_deg: f64) -> f64 {
    let lst = utc_hours + longitude_deg / 15.0;
    lst.rem_euclid(24.0)
}

/// Daytime proxy: local solar time between 06:00 and 18:00.
fn is_daytime(lst_hours: f64) -> bool {
    (6.0..18.0).contains(&lst_hours)
}

/// Coarse representative midlatitude F2 critical frequency, MHz.
///
/// These are order-of-magnitude values consistent with standard published
/// climatology (diurnal drop at night, solar-cycle scaling, and the
/// midlatitude winter anomaly where noon foF2 is higher in winter than
/// summer). They are NOT a prediction for a specific path/time; the tool
/// prints them and they are overridable via `--fof2`.
fn representative_fof2_mhz(day: bool, season: Season, solar: Solar) -> f64 {
    match (day, solar) {
        (true, Solar::High) => match season {
            Season::Winter => 12.0,
            Season::Equinox => 11.0,
            Season::Summer => 8.0,
        },
        // Winter and equinox coincide at solar minimum in this coarse table;
        // the winter anomaly only separates them at solar maximum.
        (true, Solar::Low) => match season {
            Season::Winter | Season::Equinox => 7.0,
            Season::Summer => 5.0,
        },
        // Night foF2 is much less season-dependent at midlatitudes.
        (false, Solar::High) => 4.5,
        (false, Solar::Low) => 2.8,
    }
}

/// Coarse representative F2 peak height, km. Higher at night and at solar
/// maximum, consistent with published midlatitude behaviour. Overridable via
/// `--hmf2`.
fn representative_hmf2_km(day: bool, solar: Solar) -> f64 {
    match (day, solar) {
        (true, Solar::High) => 300.0,
        (true, Solar::Low) => 280.0,
        (false, Solar::High) => 350.0,
        (false, Solar::Low) => 330.0,
    }
}

/// Geographic (lat, lon) in degrees -> engine `SphericalPoint` at ground.
fn ground_point(lat_deg: f64, lon_deg: f64) -> SphericalPoint {
    SphericalPoint::new(
        Meters::new(EARTH_RADIUS_M),
        Radians::from_degrees(90.0 - lat_deg),
        Radians::from_degrees(lon_deg),
    )
}

/// Point at central angle `arc` (radians) from `start` along initial bearing
/// `brng`. Standard great-circle destination formula, in geographic latitude:
///   lat2 = asin(sin lat1 cos d + cos lat1 sin d cos brng)
///   lon2 = lon1 + atan2(sin brng sin d cos lat1, cos d - sin lat1 sin lat2)
/// Used only to place the intermediate target of an equal-hop path and the
/// path midpoint for the climatology lookup.
fn destination_point(start: &SphericalPoint, brng: Radians, arc: Radians) -> SphericalPoint {
    let lat1 = std::f64::consts::FRAC_PI_2 - start.colat.get();
    let lon1 = start.lon.get();
    let (sd, cd) = arc.get().sin_cos();
    let (sb, cb) = brng.get().sin_cos();
    let (sl1, cl1) = lat1.sin_cos();
    let lat2 = (sl1 * cd + cl1 * sd * cb).clamp(-1.0, 1.0).asin();
    let lon2 = lon1 + (sb * sd * cl1).atan2(cd - sl1 * lat2.sin());
    SphericalPoint::new(
        Meters::new(EARTH_RADIUS_M),
        Radians::new(std::f64::consts::FRAC_PI_2 - lat2),
        Radians::new(lon2),
    )
}

struct Ionosphere {
    fof2_mhz: f64,
    hmf2_km: f64,
    scale_height_km: f64,
    /// Source label for the assumed values, for the report.
    provenance: String,
}

fn run(args: &Args) -> Result<(), String> {
    let tx = ground_point(args.tx.0, args.tx.1);
    let rx = ground_point(args.rx.0, args.rx.1);

    let total_arc = central_angle(&tx, &rx);
    let distance_m = total_arc.get() * EARTH_RADIUS_M;
    if distance_m < 1.0 {
        return Err("transmitter and receiver are the same point".into());
    }
    let brng = bearing(&tx, &rx);

    // Climatology is looked up at the great-circle midpoint: more
    // representative of the reflection region than either endpoint.
    let mid = destination_point(&tx, brng, Radians::new(0.5 * total_arc.get()));
    let mid_lat = 90.0 - mid.colat.get().to_degrees();
    let mid_lon = mid.lon.get().to_degrees();
    let lst = local_solar_time(args.utc_hours, mid_lon);
    let day = is_daytime(lst);
    let season = season_at(args.month, mid_lat);

    let iono = resolve_ionosphere(args, day, season, mid_lat, mid_lon, lst);

    print_header(args, &tx, &rx, distance_m, brng, &iono, day, lst);

    // Build the assumed Chapman layer once; it is height-only, so every hop
    // sees the same ionosphere.
    let nm = density_at_critical_frequency(Hertz::new(iono.fof2_mhz * 1e6));
    let layer = ChapmanLayer::new(
        nm,
        Meters::new(EARTH_RADIUS_M + iono.hmf2_km * 1e3),
        Meters::new(iono.scale_height_km * 1e3),
    )
    .map_err(|e| format!("invalid Chapman layer: {e}"))?;

    let mut config = TraceConfig::new(Meters::new(EARTH_RADIUS_M), Meters::new(DOMAIN_TOP_M));
    // Kinks are absent in a Chapman layer, but keep the default budget generous.
    config.max_step = 25_000.0;
    let tracer = Tracer::new(
        &layer,
        &ZeroField,
        &ZeroCollisions,
        Hertz::new(args.freq_hz),
        Mode::Ordinary,
        config,
    );
    // Field-free, so the near-vertical Spitze that motivates the default 80 deg
    // scan cap cannot occur; raise the cap to reach NVIS (near-vertical) short
    // paths. Everything else stays at the validated defaults.
    let homing_config = HomingConfig {
        elev_max: Radians::from_degrees(88.0),
        ..Default::default()
    };
    let homing = Homing {
        tracer: &tracer,
        config: homing_config,
    };

    search_and_report(
        &homing,
        &tx,
        brng,
        total_arc,
        distance_m,
        args.max_hops,
        args.freq_hz,
    )
}

fn resolve_ionosphere(
    args: &Args,
    day: bool,
    season: Season,
    mid_lat: f64,
    mid_lon: f64,
    lst: f64,
) -> Ionosphere {
    let overridden = args.fof2_override_mhz.is_some() || args.hmf2_override_km.is_some();
    let fof2 = args
        .fof2_override_mhz
        .unwrap_or_else(|| representative_fof2_mhz(day, season, args.solar));
    let hmf2 = args
        .hmf2_override_km
        .unwrap_or_else(|| representative_hmf2_km(day, args.solar));
    let solar = match args.solar {
        Solar::Low => "low",
        Solar::High => "high",
    };
    let season_str = match season {
        Season::Summer => "summer",
        Season::Winter => "winter",
        Season::Equinox => "equinox",
    };
    let daynight = if day { "day" } else { "night" };
    let provenance = if overridden {
        "user-supplied via --fof2/--hmf2".to_string()
    } else {
        format!(
            "coarse representative midlatitude climatology [{daynight}, {season_str}, solar {solar}]; \
             midpoint {mid_lat:.1} deg lat, LST {lst:.1} h; NOT a path-specific prediction"
        )
    };
    let _ = mid_lon;
    Ionosphere {
        fof2_mhz: fof2,
        hmf2_km: hmf2,
        scale_height_km: args.scale_height_km,
        provenance,
    }
}

#[allow(clippy::too_many_arguments)] // a report header; grouping into a struct would not clarify it
fn print_header(
    args: &Args,
    tx: &SphericalPoint,
    rx: &SphericalPoint,
    distance_m: f64,
    brng: Radians,
    iono: &Ionosphere,
    day: bool,
    lst: f64,
) {
    let _ = (tx, rx);
    println!("HF path prediction  (first-order: single Chapman F2 layer, field-free O-mode)");
    println!("============================================================================");
    println!(
        "TX {}   ->   RX {}",
        fmt_lat_lon(args.tx.0, args.tx.1),
        fmt_lat_lon(args.rx.0, args.rx.1)
    );
    println!(
        "Great-circle distance: {:.0} km   initial bearing: {:.1} deg",
        distance_m / 1e3,
        brng.to_degrees().rem_euclid(360.0)
    );
    println!();
    println!("Assumed ionosphere (see caveat):");
    println!("  foF2        = {:.2} MHz", iono.fof2_mhz);
    println!("  hmF2        = {:.0} km", iono.hmf2_km);
    println!("  scale height= {:.0} km", iono.scale_height_km);
    println!("  B-field     = none (O-mode)     absorption = not modelled");
    println!("  source      = {}", iono.provenance);
    let daynight = if day { "day" } else { "night" };
    println!("  (path-midpoint local solar time {lst:.1} h -> {daynight})");
    println!();
    println!("Operating frequency: {:.3} MHz", args.freq_hz / 1e6);
    println!();
}

fn search_and_report(
    homing: &Homing<'_, '_, ChapmanLayer, ZeroField, ZeroCollisions>,
    tx: &SphericalPoint,
    brng: Radians,
    total_arc: Radians,
    distance_m: f64,
    max_hops: u32,
    freq_hz: f64,
) -> Result<(), String> {
    for n in 1..=max_hops {
        let hop_arc = Radians::new(total_arc.get() / f64::from(n));
        let target = if n == 1 {
            // Home to the real receiver so the reported azimuth is exact.
            destination_point(tx, brng, total_arc)
        } else {
            destination_point(tx, brng, hop_arc)
        };
        match homing.home_scan(tx, &target) {
            Ok(rays) if !rays.is_empty() => {
                report_success(n, distance_m, &rays, freq_hz);
                return Ok(());
            }
            Ok(_) | Err(HomingError::NoBracket { .. }) => {}
            // A convergence failure or trace error is a genuine numerical
            // problem, not a "no path" answer: surface it rather than hide it.
            Err(e) => return Err(format!("homing failed for {n}-hop attempt: {e}")),
        }
    }
    report_no_path(freq_hz, max_hops);
    Ok(())
}

fn report_success(hops: u32, distance_m: f64, rays: &[HomedRay], freq_hz: f64) {
    let _ = freq_hz;
    let hop_km = distance_m / 1e3 / f64::from(hops);
    println!("RESULT: LIKELY CONNECTS  ({hops} F2 hop(s))");
    println!("  ground length per hop : {hop_km:.0} km");
    // rays come back sorted by launch elevation; the lowest is the practical
    // long-skip ray, higher elevations are additional modes.
    for (i, ray) in rays.iter().enumerate() {
        let mode = if i == 0 { "primary" } else { "alt mode" };
        let group_total_m = ray.result.group_path.get() * f64::from(hops);
        let time_ms = group_total_m / C_M_PER_S * 1e3;
        println!(
            "  [{mode}] elevation {:.1} deg, azimuth {:.1} deg, apex {:.0} km, \
             group/hop {:.0} km  (total path {:.0} km, ~{:.1} ms)",
            ray.elevation.to_degrees(),
            ray.azimuth.to_degrees().rem_euclid(360.0),
            ray.result
                .apexes
                .first()
                .map_or(f64::NAN, |a| (a.r.get() - EARTH_RADIUS_M) / 1e3),
            ray.result.group_path.get() / 1e3,
            group_total_m / 1e3,
            time_ms,
        );
    }
    println!();
    println!(
        "  Note: geometric reachability only. Absorption/signal strength not modelled;\n\
         \x20 use real foF2/hmF2 (--fof2/--hmf2) for anything you need to defend."
    );
}

fn report_no_path(freq_hz: f64, max_hops: u32) {
    println!("RESULT: UNLIKELY TO CONNECT  (no F2 path found up to {max_hops} hop(s))");
    println!(
        "  {:.3} MHz is likely above the MUF for this path with the assumed foF2\n\
         \x20 (the ray penetrates the layer -> skip zone), or the path is shorter than\n\
         \x20 the minimum one-hop range. Try a lower frequency, adjust --max-hops, or\n\
         \x20 supply real foF2/hmF2 with --fof2/--hmf2.",
        freq_hz / 1e6
    );
}

fn fmt_lat_lon(lat: f64, lon: f64) -> String {
    let ns = if lat >= 0.0 { 'N' } else { 'S' };
    let ew = if lon >= 0.0 { 'E' } else { 'W' };
    format!("{:.2}{ns},{:.2}{ew}", lat.abs(), lon.abs())
}
