//! Turns UI inputs into the engine's model objects, and records every assumed
//! value so the UI can display it. No physics here: `ChapmanLayer`,
//! `IgrfModel` and `ExponentialCollisions` are all engine types used through
//! their public constructors.

use skipzone::collision::{CollisionFrequency, ExponentialCollisions};
use skipzone::density::{
    ChapmanLayer, ElectronDensity, MultiLayer, critical_frequency, density_at_critical_frequency,
};
use skipzone::geo::SphericalPoint;
use skipzone::mag::{Igrf, IgrfModel, MagneticField};
use skipzone::units::{Hertz, Meters, PerCubicMeter, PerSecond, Radians};

use crate::antenna::{AntennaConfig, Ground};
use crate::dregion::SolarChapmanD;
use crate::noise::{NoiseEnvironment, NoiseFloor, OperatingMode};
use crate::solar::{self, SolarGeometry};

/// Spherical Earth radius, matching the engine's validation suites.
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;

// --- D-region absorbing layer -------------------------------------------
//
// Absorption needs electrons where the collision frequency is high, i.e. the
// D region (roughly 60-90 km). The F2 layer alone has almost no density down
// there, so before this existed the "absorption" figure was really just the
// F2 tail and had no day/night behaviour at all.
//
// The SHAPE and the zenith-angle dependence are derived, not guessed: an
// alpha-Chapman layer in photochemical equilibrium has peak density
// proportional to sqrt(cos chi), rising by H ln(sec chi) as the sun sets
// (see ChapmanLayer's docs in the engine).
//
// The ABSOLUTE ANCHORS below are order-of-magnitude textbook values for the
// mid-latitude daytime D region, NOT a fitted model and NOT traceable to a
// single citable table. They are surfaced in the UI on every run and are
// overridable. Treat absorption magnitudes as indicative; the day/night and
// seasonal *trends* are the defensible part.

/// Overhead-sun (chi = 0) D-region peak electron density, m^-3.
pub const D_REGION_PEAK_NE_OVERHEAD: f64 = 1.0e9;
/// Overhead-sun D-region peak height, km.
pub const D_REGION_PEAK_ALT_KM: f64 = 85.0;
/// D-region Chapman scale height, km.
pub const D_REGION_SCALE_HEIGHT_KM: f64 = 6.0;

// --- Electron-neutral collision frequency --------------------------------
//
// nu_e tracks NEUTRAL density, so it is essentially a property of the neutral
// atmosphere and barely changes between day and night. It is therefore NOT a
// function of solar zenith angle: the day/night swing in absorption comes
// from the D-region electron density above, not from nu. The exponential form
// follows an isothermal neutral atmosphere; the magnitude and scale height
// are again textbook order-of-magnitude anchors, surfaced and overridable.

/// Representative electron-neutral collision frequency at the reference
/// altitude, s^-1.
pub const NU_REF_PER_S: f64 = 5.0e6;
/// Reference altitude for `NU_REF_PER_S`, km.
pub const NU_REF_ALT_KM: f64 = 70.0;
/// Neutral scale height controlling the fall-off of nu, km.
pub const NU_SCALE_HEIGHT_KM: f64 = 6.7;

#[derive(Clone, Copy, PartialEq)]
pub enum PlaceMode {
    Tx,
    Rx,
}

/// Surface type at the intermediate ground reflections, for the ground-loss
/// term of the link budget.
///
/// The `(relative permittivity, conductivity [S/m])` pairs are the standard
/// HF-band (low-frequency-limit) representative constants tabulated in
/// ITU-R P.527 / P.368 and the classic radio-propagation literature (they are
/// the values NEC/antenna tools ship as ground presets). They are surfaced in
/// the UI and user-selectable; one manual choice approximates a whole path that
/// really crosses several surface types.
///
/// [`GroundType::AutoDetect`] is the alternative to that approximation: it
/// picks one of the manual presets *per hop* from the reflection point's
/// position against the Natural Earth coastline data (see
/// [`crate::coastline`]). It never introduces new electrical constants - it
/// only chooses among the ones below.
#[derive(Clone, Copy, PartialEq)]
pub enum GroundType {
    SeaWater,
    FreshWater,
    WetGround,
    MediumGround,
    DryGround,
    /// Water vs. land decided per hop from the coastline datasets; land hops
    /// take [`Inputs::ground_land_fallback`].
    AutoDetect,
}

impl GroundType {
    /// Everything the surface dropdown offers: the five manual presets, in the
    /// order they have always been listed, plus auto-detection.
    pub const ALL_SELECTABLE: [Self; 6] = [
        Self::SeaWater,
        Self::FreshWater,
        Self::WetGround,
        Self::MediumGround,
        Self::DryGround,
        Self::AutoDetect,
    ];

    /// The land types auto-detection can fall back to. Water is decided from
    /// the data; how wet the soil is is not in the data, so it stays a choice.
    pub const LAND_TYPES: [Self; 3] = [Self::WetGround, Self::MediumGround, Self::DryGround];

    pub fn label(self) -> &'static str {
        match self {
            Self::SeaWater => "sea water",
            Self::FreshWater => "fresh water",
            Self::WetGround => "wet / good ground",
            Self::MediumGround => "medium ground",
            Self::DryGround => "dry / poor ground",
            Self::AutoDetect => "auto-detect (coastline)",
        }
    }

    /// True for the coastline-driven selection, which has no single constant
    /// pair of its own.
    #[must_use]
    pub fn is_auto(self) -> bool {
        self == Self::AutoDetect
    }

    /// `(relative permittivity eps_r, conductivity sigma [S/m])`, HF band.
    ///
    /// `AutoDetect` has no constants of its own: the solver resolves it to a
    /// real surface per hop before ever asking for these. It reports the
    /// default land fallback here so the function stays total and any path that
    /// somehow reaches it gets a sane surface rather than a panic.
    #[must_use]
    pub fn constants(self) -> (f64, f64) {
        match self {
            Self::SeaWater => (80.0, 5.0),
            Self::FreshWater => (80.0, 0.003),
            Self::WetGround => (30.0, 0.01),
            Self::MediumGround | Self::AutoDetect => (15.0, 0.003),
            Self::DryGround => (5.0, 0.001),
        }
    }

    /// The same surface as the antenna models see it. One selection drives both
    /// the mid-path bounce loss and the image-theory ground under each antenna,
    /// so the two can never disagree about what the ground is made of.
    #[must_use]
    pub fn as_antenna_ground(self) -> Ground {
        let (eps_r, sigma_s_per_m) = self.constants();
        Ground::Lossy {
            eps_r,
            sigma_s_per_m,
        }
    }
}

#[derive(Clone)]
pub struct Inputs {
    pub tx_lat: f64,
    pub tx_lon: f64,
    pub rx_lat: f64,
    pub rx_lon: f64,
    pub freq_mhz: f64,
    pub utc_hours: f64,
    /// Calendar year. Carried so the UI can show and edit a real date; the
    /// solar geometry deliberately ignores it (see [`crate::solar`]: Cooper's
    /// declination takes only the day of year, and leap years shift that by
    /// under 0.4 deg of declination - far below this model's accuracy).
    pub year: i32,
    pub month: u32,
    /// Sunspot number. foF2 is derived from this (see `fof2_from_ssn`); it is
    /// the solar-activity input, replacing the old day/season/solar-high table.
    pub ssn: f64,
    /// F2 peak height, km. A direct user input (no climatology table).
    pub hmf2_km: f64,
    pub scale_height_km: f64,
    pub day_of_month: u32,
    /// When false (the default) the collision profile comes from the module
    /// constants above and the D region is driven by solar zenith angle. When
    /// true the three fields below are used verbatim instead.
    pub collision_manual: bool,
    pub nu0_per_s: f64,
    pub nu_ref_alt_km: f64,
    pub nu_scale_height_km: f64,
    pub use_field: bool,
    pub igrf_epoch: f64,
    pub max_hops: u32,
    pub domain_top_km: f64,
    /// Surface at the intermediate ground reflections (link-budget ground loss),
    /// and under both antennas (their image-theory ground reflection).
    pub ground_type: GroundType,
    /// Which land preset a hop that lands on land takes when `ground_type` is
    /// [`GroundType::AutoDetect`]. The coastline data says water or land; it
    /// carries nothing about soil moisture, so that half stays the operator's
    /// call. Ignored entirely for a manual `ground_type`.
    pub ground_land_fallback: GroundType,

    // --- Antennas (see `crate::antenna`) ---------------------------------
    /// Transmitting antenna. Its gain at the launch elevation of the first hop
    /// enters the link budget.
    pub tx_antenna: AntennaConfig,
    /// Receiving antenna. Its gain at the arrival elevation of the last hop
    /// enters the link budget.
    pub rx_antenna: AntennaConfig,

    // --- Received-signal judgment (see `crate::noise`) -------------------
    /// Transmitter power, watts. Converted to dBm for the link budget.
    pub tx_power_w: f64,
    /// Receiving-site man-made noise category, ITU-R P.372 Table 1.
    pub noise_env: NoiseEnvironment,
    /// Operating-mode preset the bandwidth and threshold were last taken from.
    /// Selecting one overwrites the two fields below; both stay editable, so
    /// the threshold is a setting rather than a constant.
    pub op_mode: OperatingMode,
    /// Receiver noise bandwidth, Hz.
    pub bandwidth_hz: f64,
    /// SNR in `bandwidth_hz` required to call the path usable, dB.
    pub snr_threshold_db: f64,
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
            year: 2026,
            month: 1,
            ssn: 70.0,
            hmf2_km: 300.0,
            scale_height_km: 50.0,
            day_of_month: 15,
            collision_manual: false,
            nu0_per_s: NU_REF_PER_S,
            nu_ref_alt_km: NU_REF_ALT_KM,
            nu_scale_height_km: NU_SCALE_HEIGHT_KM,
            use_field: true,
            igrf_epoch: 2026.5,
            max_hops: 4,
            domain_top_km: 800.0,
            ground_type: GroundType::AutoDetect,
            ground_land_fallback: GroundType::MediumGround,
            tx_antenna: AntennaConfig::default(),
            rx_antenna: AntennaConfig::default(),
            tx_power_w: 100.0,
            noise_env: NoiseEnvironment::Rural,
            op_mode: OperatingMode::Ssb,
            bandwidth_hz: OperatingMode::Ssb.defaults().0,
            snr_threshold_db: OperatingMode::Ssb.defaults().1,
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
    pub collision_source: String,
    pub field_desc: String,
    pub r_ground_m: f64,
    pub r_top_m: f64,
    pub freq_mhz: f64,
    /// Solar geometry at the path midpoint.
    pub solar: SolarGeometry,
    /// Display flag: the D region is producing at the path midpoint (sun up and
    /// non-negligible). The layer itself is always built and varies along the
    /// path; this only drives the panel label.
    pub d_region_active: bool,
    /// Realised D-region peak at the midpoint, `Nm / sqrt(Ch)` from the Chapman
    /// grazing function, m^-3.
    pub d_region_peak_ne: f64,
    /// Realised D-region peak altitude at the midpoint, `+H ln(Ch)`, km.
    pub d_region_peak_alt_km: f64,
    pub d_region_source: String,
    /// Sun above the horizon AT THE RECEIVER. The noise floor is a property of
    /// the receiving site, so it uses this rather than the midpoint's `is_day`.
    pub rx_is_day: bool,
    /// Season at the receiver's latitude, for the atmospheric noise term.
    pub rx_season: Season,
}

/// The noise floor for a scenario at one frequency.
///
/// Split out from [`resolve`] because the frequency sweep re-solves at many
/// frequencies against a single `Assumptions`: the floor must follow the
/// frequency actually being tried, not the tuned one.
///
/// Latitude, day/night and season are all taken at the RECEIVER: noise is what
/// the listener's antenna hears, not a path-average quantity.
#[must_use]
pub fn noise_floor_at(inputs: &Inputs, a: &Assumptions, f_mhz: f64) -> NoiseFloor {
    NoiseFloor::compute(
        f_mhz,
        inputs.bandwidth_hz,
        inputs.noise_env,
        a.rx_is_day,
        a.rx_season,
        inputs.rx_lat,
    )
}

pub fn season_at(month: u32, latitude_deg: f64) -> Season {
    let north = match month {
        1..=2 | 12 => Season::Winter,
        6..=8 => Season::Summer,
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

/// foF2 [MHz] derived from sunspot number. The peak plasma density NmF2 (and
/// hence foF2^2, since NmF2 proportional to foF2^2) is taken linear in SSN -
/// the standard first-order statement that peak ionisation scales with solar
/// activity - calibrated to representative midlatitude anchors: foF2 ~ 4.5 MHz
/// at SSN 0 and ~10 MHz at SSN 150.
///
/// This is a coarse climatological anchor, NOT a path-, season-, or
/// time-specific prediction (that is the job of the CCIR maps, which are not
/// implemented here), and it is always surfaced in the UI. There is
/// deliberately no hidden day/night branch: SSN is the only driver, so the
/// operator sees exactly the foF2 the engine is given.
#[must_use]
pub fn fof2_from_ssn(ssn: f64) -> f64 {
    // foF2(0)^2 = 4.5^2 = 20.25; slope (10^2 - 20.25)/150 = 0.5317.
    (20.25 + 0.531_666_7 * ssn.max(0.0)).sqrt()
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
    /// F2 layer plus, in daylight, the D-region absorbing layer. Composed with
    /// the engine's existing validated `MultiLayer`.
    pub density: MultiLayer,
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

    // Real solar geometry now replaces the old "LST between 06 and 18" proxy.
    let solar = solar::solar_geometry(
        mid_lat,
        mid_lon,
        inputs.month,
        inputs.day_of_month,
        inputs.utc_hours,
    );
    let lst = solar.local_solar_time_h;
    let season = season_at(inputs.month, mid_lat);

    // Noise is heard at the receiver, so its day/night and season come from
    // the receiver's own solar geometry, not the path midpoint's.
    let rx_solar = solar::solar_geometry(
        inputs.rx_lat,
        inputs.rx_lon,
        inputs.month,
        inputs.day_of_month,
        inputs.utc_hours,
    );
    let rx_is_day = rx_solar.is_day();
    let rx_season = season_at(inputs.month, inputs.rx_lat);

    // D region: day/night-aware alpha-Chapman layer on the Chapman grazing
    // function (docs/derivations/chapman-grazing.md). The layer is ALWAYS built
    // (build_models) and is evaluated at the LOCAL solar zenith angle at every
    // point along the ray; these numbers are the realised peak at the path
    // midpoint, for display only.
    let chi_deg = solar.zenith_angle_deg;
    let d_disp = SolarChapmanD::new(
        D_REGION_PEAK_NE_OVERHEAD,
        EARTH_RADIUS_M + D_REGION_PEAK_ALT_KM * 1e3,
        D_REGION_SCALE_HEIGHT_KM * 1e3,
        solar.declination_deg,
        inputs.utc_hours,
    );
    let chi_rad = chi_deg.to_radians();
    let d_region_peak_ne = d_disp.realised_peak_ne(chi_rad);
    // "Active" now means "producing at the midpoint", tied to the SAME
    // sun-above-horizon test as is_day(), so the panel can no longer disagree
    // with itself about day vs night (the old 85 deg / 90 deg split).
    let d_region_active = solar.is_day() && d_region_peak_ne > 1e-3 * D_REGION_PEAK_NE_OVERHEAD;
    let rise_km = d_disp.realised_peak_rise(chi_rad) / 1e3;
    let d_region_peak_alt_km = if rise_km.is_finite() {
        D_REGION_PEAK_ALT_KM + rise_km
    } else {
        D_REGION_PEAK_ALT_KM
    };
    let d_region_source = format!(
        "alpha-Chapman with Chapman grazing function Ch(X, chi) at midpoint \
         chi = {chi_deg:.2} deg: realised peak Nm/sqrt(Ch) at +H ln(Ch), staying finite \
         through the terminator. Evaluated at the LOCAL zenith angle at every point on \
         the ray, so a path crossing the terminator is absorbed only on its sunlit part. \
         Overhead anchor {D_REGION_PEAK_NE_OVERHEAD:.1e} m^-3 at {D_REGION_PEAK_ALT_KM:.0} km \
         (order-of-magnitude, not a fitted model)"
    );

    let fof2 = fof2_from_ssn(inputs.ssn);
    let fof2_source = format!(
        "derived from SSN = {:.0} (NmF2 linear in SSN, coarse midlat anchor; \
         no day/night branch)",
        inputs.ssn
    );
    let hmf2 = inputs.hmf2_km;
    let hmf2_source = "direct user input".to_string();
    let nm = density_at_critical_frequency(Hertz::new(fof2 * 1e6));

    let (nu0_per_s, nu_ref_alt_km, nu_scale_height_km, collision_source) = if inputs
        .collision_manual
    {
        (
            inputs.nu0_per_s,
            inputs.nu_ref_alt_km,
            inputs.nu_scale_height_km,
            "manual override".to_string(),
        )
    } else {
        (
            NU_REF_PER_S,
            NU_REF_ALT_KM,
            NU_SCALE_HEIGHT_KM,
            format!(
                "neutral-atmosphere exponential, {NU_REF_PER_S:.1e} /s at {NU_REF_ALT_KM:.0} km, \
                 scale {NU_SCALE_HEIGHT_KM:.1} km (order-of-magnitude anchor). nu follows neutral \
                 density and is NOT a function of solar zenith angle"
            ),
        )
    };

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
        is_day: solar.is_day(),
        season,
        nu0_per_s,
        nu_ref_alt_km,
        nu_scale_height_km,
        collision_source,
        field_desc,
        r_ground_m: EARTH_RADIUS_M,
        r_top_m: EARTH_RADIUS_M + inputs.domain_top_km * 1e3,
        freq_mhz: inputs.freq_mhz,
        solar,
        d_region_active,
        d_region_peak_ne,
        d_region_peak_alt_km,
        d_region_source,
        rx_is_day,
        rx_season,
    }
}

pub fn build_models(inputs: &Inputs, a: &Assumptions) -> Result<Models, String> {
    let f2 = ChapmanLayer::new(
        PerCubicMeter::new(a.nm_per_m3),
        Meters::new(EARTH_RADIUS_M + a.hmf2_km * 1e3),
        Meters::new(a.scale_height_km * 1e3),
    )
    .map_err(|e| format!("F2 Chapman layer rejected: {e}"))?;

    // The D region is always present and is day/night-aware: it evaluates the
    // Chapman grazing function at the local solar zenith angle of each sampled
    // point, self-zeroing smoothly on the night side rather than being switched
    // off at a midpoint zenith-angle threshold (docs/derivations/chapman-grazing.md).
    let d = SolarChapmanD::new(
        D_REGION_PEAK_NE_OVERHEAD,
        EARTH_RADIUS_M + D_REGION_PEAK_ALT_KM * 1e3,
        D_REGION_SCALE_HEIGHT_KM * 1e3,
        a.solar.declination_deg,
        inputs.utc_hours,
    );
    let layers: Vec<Box<dyn ElectronDensity + Send + Sync>> = vec![Box::new(f2), Box::new(d)];
    let density = MultiLayer::new(layers);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// foF2(SSN) hits its calibration anchors, rises monotonically with solar
    /// activity, and clamps negative SSN rather than producing NaN.
    #[test]
    fn fof2_from_ssn_monotonic_and_anchored() {
        assert!(
            (fof2_from_ssn(0.0) - 4.5).abs() < 0.02,
            "{}",
            fof2_from_ssn(0.0)
        );
        assert!(
            (fof2_from_ssn(150.0) - 10.0).abs() < 0.05,
            "{}",
            fof2_from_ssn(150.0)
        );
        let mut prev = fof2_from_ssn(0.0);
        for s in 1..=300 {
            let v = fof2_from_ssn(f64::from(s));
            assert!(v > prev, "not monotonic at SSN {s}: {v} <= {prev}");
            prev = v;
        }
        // Negative SSN is clamped to the SSN = 0 value (never NaN).
        assert_eq!(fof2_from_ssn(-40.0), fof2_from_ssn(0.0));
    }
}
