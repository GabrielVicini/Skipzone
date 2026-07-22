//! Turns UI inputs into the engine's model objects, and records every assumed
//! value so the UI can display it. No physics here: `ChapmanLayer`,
//! `IgrfModel` and `ExponentialCollisions` are all engine types used through
//! their public constructors.

use skipzone::collision::{CollisionFrequency, ExponentialCollisions};
use skipzone::density::{ChapmanLayer, ElectronDensity, critical_frequency, density_at_critical_frequency};
use skipzone::geo::SphericalPoint;
use skipzone::mag::{Igrf, IgrfModel, MagneticField};
use skipzone::units::{Hertz, Meters, PerSecond, PerCubicMeter, Radians};

/// Spherical Earth radius, matching the engine's validation suites.
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

#[derive(Clone, Copy, PartialEq)]
pub enum PlaceMode {
    Tx,
    Rx,
}

#[derive(Clone)]
pub struct Inputs {
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
    pub freq_mhz: f64,
    pub utc_hours: f64,
    pub month: u32,
    pub solar_high: bool,
    /// Manual overrides; when set, climatology is bypassed entirely.
    pub fof2_override: Option<f64>,
    pub hmf2_override: Option<f64>,
    pub scale_height_km: f64,
    /// Collision-frequency profile. The engine deliberately ships no default
    /// magnitude, so these are the user's numbers and are always displayed.
    pub nu0_per_s: f64,
    pub nu_ref_alt_km: f64,
    pub nu_scale_height_km: f64,
    pub use_field: bool,
    pub igrf_epoch: f64,
    pub max_hops: u32,
    pub domain_top_km: f64,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            tx_lat: 39.74,
            tx_lon: -104.99,
            rx_lat: 51.50,
            rx_lon: -0.13,
            freq_mhz: 14.1,
            utc_hours: 18.0,
            month: 1,
            solar_high: false,
            fof2_override: None,
            hmf2_override: None,
            scale_height_km: 50.0,
            // ~1e5 s^-1 at 100 km with a 30 km scale height: gives Z ~ 2e-3 at
            // HF, the magnitude used in the engine's own absorption tests.
            nu0_per_s: 1.0e5,
            nu_ref_alt_km: 100.0,
            nu_scale_height_km: 30.0,
            use_field: true,
            igrf_epoch: 2026.5,
            max_hops: 4,
            domain_top_km: 800.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Season {
    Summer,
    Winter,
    Equinox,
}

impl Season {
    pub fn label(self) -> &'static str {
        match self {
            Self::Summer => "summer",
            Self::Winter => "winter",
            Self::Equinox => "equinox",
        }
    }
}

/// Every assumed / derived value, for display. Nothing here is hidden from the
/// UI: the whole point is that the operator can see what was fed to the engine.
#[derive(Clone)]
pub struct Assumptions {
    pub fof2_mhz: f64,
    pub fof2_source: String,
    pub hmf2_km: f64,
    pub hmf2_source: String,
    pub scale_height_km: f64,
    pub nm_per_m3: f64,
    pub midpoint_lat: f64,
    pub midpoint_lon: f64,
    pub lst_hours: f64,
    pub is_day: bool,
    pub season: Season,
    pub nu0_per_s: f64,
    pub nu_ref_alt_km: f64,
    pub nu_scale_height_km: f64,
    pub field_desc: String,
    pub r_ground_m: f64,
    pub r_top_m: f64,
    pub freq_mhz: f64,
}

/// Local mean solar time, hours in [0,24). Exact by definition (15 deg/hour);
/// a crude day/night proxy, not a solar-position calculation.
pub fn local_solar_time(utc_hours: f64, longitude_deg: f64) -> f64 {
    (utc_hours + longitude_deg / 15.0).rem_euclid(24.0)
}

pub fn is_daytime(lst_hours: f64) -> bool {
    (6.0..18.0).contains(&lst_hours)
}

pub fn season_at(month: u32, latitude_deg: f64) -> Season {
    let north = match month {
        12 | 1 | 2 => Season::Winter,
        6 | 7 | 8 => Season::Summer,
        _ => Season::Equinox,
    };
    if latitude_deg >= 0.0 {
        north
    } else {
        match north {
            Season::Winter => Season::Summer,
            Season::Summer => Season::Winter,
            Season::Equinox => Season::Equinox,
        }
    }
}

/// Coarse representative midlatitude foF2 [MHz]. Order-of-magnitude values
/// consistent with published climatology (night drop, solar-cycle scaling,
/// midlatitude winter anomaly). NOT a path-specific prediction; always shown
/// in the UI and overridable.
pub fn representative_fof2_mhz(day: bool, season: Season, solar_high: bool) -> f64 {
    match (day, solar_high) {
        (true, true) => match season {
            Season::Winter => 12.0,
            Season::Equinox => 11.0,
            Season::Summer => 8.0,
        },
        (true, false) => match season {
            Season::Winter => 7.0,
            Season::Equinox => 7.0,
            Season::Summer => 5.0,
        },
        // Night foF2 is much less season-dependent at midlatitudes.
        (false, true) => 4.5,
        (false, false) => 2.8,
    }
}

/// Coarse representative F2 peak height [km]: higher at night and solar max.
pub fn representative_hmf2_km(day: bool, solar_high: bool) -> f64 {
    match (day, solar_high) {
        (true, true) => 300.0,
        (true, false) => 280.0,
        (false, true) => 350.0,
        (false, false) => 330.0,
    }
}

pub fn ground_point(lat_deg: f64, lon_deg: f64) -> SphericalPoint {
    SphericalPoint::new(
        Meters::new(EARTH_RADIUS_M),
        Radians::from_degrees(90.0 - lat_deg),
        Radians::from_degrees(lon_deg),
    )
}

/// (lat, lon) in degrees from an engine point; longitude wrapped to [-180,180].
pub fn to_lat_lon(p: &SphericalPoint) -> (f64, f64) {
    let lat = 90.0 - p.colat.get().to_degrees();
    let mut lon = p.lon.get().to_degrees();
    lon = ((lon + 180.0).rem_euclid(360.0)) - 180.0;
    (lat, lon)
}

/// Point at central angle `arc` from `start` along initial bearing `brng`.
/// Standard great-circle destination formula in geographic latitude:
///   lat2 = asin(sin lat1 cos d + cos lat1 sin d cos brng)
///   lon2 = lon1 + atan2(sin brng sin d cos lat1, cos d - sin lat1 sin lat2)
pub fn destination_point(start: &SphericalPoint, brng: Radians, arc: Radians) -> SphericalPoint {
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

/// The engine model objects. Held together so the tracer can borrow them.
pub struct Models {
    pub density: ChapmanLayer,
    pub field: Option<IgrfModel>,
    pub collisions: ExponentialCollisions,
}

impl Models {
    /// Trait-object views, so the tracer can be built uniformly whether or not
    /// a magnetic field is enabled (the engine's generics are `?Sized`).
    pub fn density_dyn(&self) -> &dyn ElectronDensity {
        &self.density
    }

    pub fn field_dyn(&self) -> &dyn MagneticField {
        match &self.field {
            Some(igrf) => igrf,
            None => &skipzone::mag::ZeroField,
        }
    }

    pub fn collisions_dyn(&self) -> &dyn CollisionFrequency {
        &self.collisions
    }
}

pub fn resolve(inputs: &Inputs) -> Assumptions {
    let tx = ground_point(inputs.tx_lat, inputs.tx_lon);
    let rx = ground_point(inputs.rx_lat, inputs.rx_lon);
    let arc = skipzone::geo::central_angle(&tx, &rx);
    let brng = skipzone::geo::bearing(&tx, &rx);
    let mid = destination_point(&tx, brng, Radians::new(0.5 * arc.get()));
    let (mid_lat, mid_lon) = to_lat_lon(&mid);
    let lst = local_solar_time(inputs.utc_hours, mid_lon);
    let day = is_daytime(lst);
    let season = season_at(inputs.month, mid_lat);

    let (fof2, fof2_source) = match inputs.fof2_override {
        Some(v) => (v, "manual override".to_string()),
        None => (
            representative_fof2_mhz(day, season, inputs.solar_high),
            format!(
                "coarse midlat climatology [{}, {}, solar {}] @ path midpoint",
                if day { "day" } else { "night" },
                season.label(),
                if inputs.solar_high { "high" } else { "low" }
            ),
        ),
    };
    let (hmf2, hmf2_source) = match inputs.hmf2_override {
        Some(v) => (v, "manual override".to_string()),
        None => (
            representative_hmf2_km(day, inputs.solar_high),
            "coarse midlat climatology".to_string(),
        ),
    };
    let nm = density_at_critical_frequency(Hertz::new(fof2 * 1e6));
    let field_desc = if inputs.use_field {
        format!("IGRF-14 @ epoch {:.1}", inputs.igrf_epoch)
    } else {
        "zero field (O and X degenerate)".to_string()
    };

    Assumptions {
        fof2_mhz: fof2,
        fof2_source,
        hmf2_km: hmf2,
        hmf2_source,
        scale_height_km: inputs.scale_height_km,
        nm_per_m3: nm.get(),
        midpoint_lat: mid_lat,
        midpoint_lon: mid_lon,
        lst_hours: lst,
        is_day: day,
        season,
        nu0_per_s: inputs.nu0_per_s,
        nu_ref_alt_km: inputs.nu_ref_alt_km,
        nu_scale_height_km: inputs.nu_scale_height_km,
        field_desc,
        r_ground_m: EARTH_RADIUS_M,
        r_top_m: EARTH_RADIUS_M + inputs.domain_top_km * 1e3,
        freq_mhz: inputs.freq_mhz,
    }
}

pub fn build_models(inputs: &Inputs, a: &Assumptions) -> Result<Models, String> {
    let density = ChapmanLayer::new(
        PerCubicMeter::new(a.nm_per_m3),
        Meters::new(EARTH_RADIUS_M + a.hmf2_km * 1e3),
        Meters::new(a.scale_height_km * 1e3),
    )
    .map_err(|e| format!("Chapman layer rejected: {e}"))?;

    let field = if inputs.use_field {
        let igrf = Igrf::from_embedded().map_err(|e| format!("IGRF load failed: {e}"))?;
        Some(
            igrf.model_at(inputs.igrf_epoch)
                .map_err(|e| format!("IGRF epoch rejected: {e}"))?,
        )
    } else {
        None
    };

    let collisions = ExponentialCollisions::new(
        PerSecond::new(a.nu0_per_s),
        Meters::new(EARTH_RADIUS_M + a.nu_ref_alt_km * 1e3),
        Meters::new(a.nu_scale_height_km * 1e3),
    )
    .map_err(|e| format!("collision profile rejected: {e}"))?;

    Ok(Models {
        density,
        field,
        collisions,
    })
}

/// One sampled row of the vertical profile actually in use, for UI display.
pub struct ProfileRow {
    pub alt_km: f64,
    pub ne_per_m3: f64,
    pub plasma_mhz: f64,
    pub nu_per_s: f64,
    /// X = (fp/f)^2 at the operating frequency.
    pub x: f64,
    /// Z = nu/omega at the operating frequency.
    pub z: f64,
    /// |B| in microtesla, if a field model is active.
    pub b_microtesla: Option<f64>,
}

/// Sample the profile along the vertical at the path midpoint. Uses the same
/// model objects the tracer uses, so this is the profile actually in force.
pub fn sample_profile(models: &Models, a: &Assumptions) -> Vec<ProfileRow> {
    let omega = Hertz::new(a.freq_mhz * 1e6).angular();
    let colat = Radians::from_degrees(90.0 - a.midpoint_lat);
    let lon = Radians::from_degrees(a.midpoint_lon);
    let mut rows = Vec::new();
    let mut alt = 60.0_f64;
    while alt <= 600.0 {
        let p = SphericalPoint::new(Meters::new(EARTH_RADIUS_M + alt * 1e3), colat, lon);
        let ne = models.density.sample(&p).ne;
        let nu = models.collisions.sample(&p).nu;
        let fp = critical_frequency(PerCubicMeter::new(ne)).get();
        let f_hz = a.freq_mhz * 1e6;
        let b_microtesla = models.field.as_ref().map(|igrf| {
            let b = igrf.sample(&p).b;
            (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt() * 1e6
        });
        rows.push(ProfileRow {
            alt_km: alt,
            ne_per_m3: ne,
            plasma_mhz: fp / 1e6,
            nu_per_s: nu,
            x: (fp / f_hz).powi(2),
            z: nu / omega,
            b_microtesla,
        });
        alt += 20.0;
    }
    rows
}
