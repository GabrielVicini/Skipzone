//! Turns UI inputs into the engine's model objects, and records every assumed
//! value so the UI can display it. No physics here: the layers come from
//! [`crate::chapman`], [`crate::fof2`] and [`crate::sporadic_e`], and
//! `QuasiParabolicLayer`, `IgrfModel` and `ExponentialCollisions` are engine
//! types used through their public constructors.
//!
//! # The density stack
//!
//! Four layers, built from two shapes:
//!
//! | layer | shape                              | peak density              |
//! |-------|------------------------------------|---------------------------|
//! | D     | Chapman, solar grazing `Ch(X, chi)`| constant overhead anchor  |
//! | E     | Chapman, solar grazing `Ch(X, chi)`| foE from SSN, chi from Ch |
//! | Es    | quasi-parabolic sheet              | foEs, probabilistic       |
//! | F2    | Chapman, overhead (no chi law)     | foF2 climatology map      |
//!
//! Es is built into a SECOND stack rather than the main one, because it is the
//! only layer that may or may not be there. The solver runs both and keeps the
//! two verdicts apart (see [`crate::solve`]).

use skipzone::collision::{CollisionFrequency, ExponentialCollisions};
use skipzone::density::{
    ElectronDensity, MultiLayer, critical_frequency, density_at_critical_frequency,
};
use skipzone::geo::SphericalPoint;
use skipzone::mag::{Igrf, IgrfModel, MagneticField};
use skipzone::units::{Hertz, Meters, PerCubicMeter, PerSecond, Radians};

use crate::antenna::{AntennaConfig, Ground};
use crate::calib::Anchors;
use crate::chapman::{ConstantPeak, SlantFactor, SolarChapmanLayer};
use crate::fof2::{self, Fof2Backend, Fof2Grid, GriddedF2Peak};
use crate::noise::{NoiseEnvironment, NoiseFloor, OperatingMode};
use crate::solar::{self, SolarGeometry};
use crate::sporadic_e::SporadicE;

pub use crate::fof2::fof2_from_ssn;
pub use crate::solar::{Season, season_at};

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
/// Residual night-time D-region peak density, as a fraction of the overhead-sun
/// peak. ANCHOR.
///
/// An alpha-Chapman layer goes to EXACTLY zero past the terminator, because
/// `Ch(X, chi)` diverges there. The real night D region does not: galactic cosmic
/// rays and scattered Lyman-alpha on nitric oxide keep it at roughly 10^8 m^-3
/// against a daytime peak near 10^9, and neither source switches off at sunset.
/// Photochemical equilibrium under a source the sun does not supply is outside
/// what the Chapman derivation covers, so this is an input to it.
///
/// It is not cosmetic. Absorption goes as `Ne nu`, so a layer that is exactly
/// zero absorbs exactly nothing, and before this constant existed the model had
/// no night-time absorption at all on any band - measured at 0.00 dB on 7 and
/// 14 MHz over a midnight path.
pub const D_REGION_NIGHT_FLOOR_FRACTION: f64 = 0.10;

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

// --- E layer -------------------------------------------------------------
//
// The E region is the other layer close enough to photochemical equilibrium
// for an alpha-Chapman profile driven by the solar zenith angle to be the
// DERIVED answer rather than a fit: the layer produces a realised peak of
// Nm Ch^{-1/2}, i.e. foE proportional to (cos chi)^{1/4} in the plane-parallel
// regime, which is the standard foE law. The overhead-sun anchor and the
// solar-activity scaling live in `crate::fof2`.
//
// The GEOMETRY below is a textbook order-of-magnitude anchor, NOT a fitted
// model: the E peak sits near 105-110 km with a scale height of order 10 km.

/// E-layer peak height, km.
pub const E_REGION_PEAK_ALT_KM: f64 = 105.0;
/// E-layer Chapman scale height, km.
pub const E_REGION_SCALE_HEIGHT_KM: f64 = 10.0;

/// Apex altitude below which a reflection is attributed to the E region rather
/// than to F2, km. Sits above the E peak (a ray turns above the peak of the
/// layer it reflects from) and well below the F1/F2 ledge.
pub const E_ATTRIBUTION_TOP_KM: f64 = 145.0;

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
    /// Sunspot number. It drives foF2 (see [`crate::fof2`]) and foE; it is the
    /// solar-activity input, replacing the old day/season/solar-high table.
    pub ssn: f64,
    /// Which foF2 model builds the F2 layer. Defaults to the gridded
    /// climatology; the SSN-only scalar remains selectable, and is the
    /// automatic fallback if the bundled grid fails to load.
    pub fof2_backend: Fof2Backend,
    /// F2 peak height, km. A direct user input (no climatology table).
    pub hmf2_km: f64,
    pub scale_height_km: f64,
    /// Include a sporadic-E layer in the probabilistic second solve. Off makes
    /// the whole Es apparatus vanish from the output rather than reporting
    /// zero-probability paths.
    pub es_enabled: bool,
    /// When false (the default) foEs and its occurrence probability are derived
    /// from local season, local solar time and latitude. When true the two
    /// fields below are used verbatim.
    pub es_manual: bool,
    /// Critical frequency of the Es layer when present, MHz.
    pub foes_mhz: f64,
    /// Probability that a usable Es layer is present, 0..1.
    pub es_probability: f64,
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
    /// Calibrated model bias [dB], subtracted from every predicted SNR.
    ///
    /// A calibration against measured spots reports a global offset it cannot
    /// attribute to any individual station - see the `wspr_calibrate` report.
    /// Whatever its cause, it is the model's best estimate of its own bias
    /// against an UNKNOWN station, because the per-station effects are centred on
    /// zero. Shipping the prediction without it means shipping a bias that has
    /// been measured and declined.
    ///
    /// 0.0 by default, so nothing changes until a calibrated value is set.
    pub model_bias_db: f64,

    /// The unverified anchors of the ionosphere and noise models. `Default`
    /// reproduces the module constants exactly, so an ordinary run never needs
    /// to touch this; a calibration run varies it. See [`crate::calib`].
    pub anchors: Anchors,
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
            fof2_backend: Fof2Backend::Gridded,
            hmf2_km: 300.0,
            scale_height_km: 50.0,
            es_enabled: true,
            es_manual: false,
            foes_mhz: 5.0,
            es_probability: 0.15,
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
            model_bias_db: 0.0,
            anchors: Anchors::default(),
        }
    }
}

/// Every assumed / derived value, for display. Nothing here is hidden from the
/// UI: the whole point is that the operator can see what was fed to the engine.
#[derive(Clone)]
pub struct Assumptions {
    /// foF2 at the path midpoint, MHz. With the gridded backend this is one
    /// sample of a field that varies across the whole domain, not the single
    /// number the engine is given - which is exactly the change.
    pub fof2_mhz: f64,
    pub fof2_source: String,
    /// foF2 at the transmitter and at the receiver, MHz. Present so the panel
    /// can show that the field really does vary along the path; equal to
    /// `fof2_mhz` under the constant backend.
    pub fof2_tx_mhz: f64,
    pub fof2_rx_mhz: f64,
    /// Which backend actually ran, after any fallback.
    pub fof2_backend: Fof2Backend,
    /// Overhead-sun foE for this solar activity, MHz. The realised foE at any
    /// point is this times `(cos chi)^(1/4)`, produced by the layer itself.
    pub foe_overhead_mhz: f64,
    /// Realised foE at the path midpoint, MHz.
    pub foe_midpoint_mhz: f64,
    pub foe_source: String,
    pub e_region_peak_alt_km: f64,
    /// Sporadic E: foEs, occurrence probability and provenance.
    pub sporadic_e: SporadicE,
    /// Whether the probabilistic Es solve will actually be run.
    pub es_solved: bool,
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
        inputs.anchors.atmospheric,
    )
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
    /// The DETERMINISTIC density stack: D, E and F2. Every layer in it is
    /// present whenever the scenario says it is, so a path it supports is a
    /// path that is simply there. Composed with the engine's validated
    /// `MultiLayer`.
    pub density: MultiLayer,
    /// The same stack plus a sporadic-E sheet, or `None` when Es is disabled or
    /// too unlikely to be worth solving. Kept separate rather than merged
    /// because Es is probabilistic: a path that needs it is a different KIND of
    /// answer, and merging the stacks would make the two indistinguishable.
    pub density_with_es: Option<MultiLayer>,
    pub field: Option<IgrfModel>,
    pub collisions: ExponentialCollisions,
}

impl Models {
    /// Trait-object views, so the tracer can be built uniformly whether or not
    /// a magnetic field is enabled (the engine's generics are `?Sized`).
    pub fn density_dyn(&self) -> &(dyn ElectronDensity + Sync) {
        &self.density
    }

    /// The Es-bearing stack, when there is one.
    pub fn density_with_es_dyn(&self) -> Option<&(dyn ElectronDensity + Sync)> {
        self.density_with_es
            .as_ref()
            .map(|d| d as &(dyn ElectronDensity + Sync))
    }

    pub fn field_dyn(&self) -> &(dyn MagneticField + Sync) {
        match &self.field {
            Some(igrf) => igrf,
            None => &skipzone::mag::ZeroField,
        }
    }

    pub fn collisions_dyn(&self) -> &(dyn CollisionFrequency + Sync) {
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
    let mid_colat = Radians::from_degrees(90.0 - mid_lat).get();
    let mid_lon_rad = Radians::from_degrees(mid_lon).get();
    let ion = inputs.anchors.ionosphere;
    let d_disp = SolarChapmanLayer::d_region(
        ion.d_peak_ne_overhead.value,
        EARTH_RADIUS_M + ion.d_peak_alt_km.value * 1e3,
        ion.d_scale_height_km.value * 1e3,
        solar.declination_deg,
        inputs.utc_hours,
        ion.d_night_floor_fraction.value,
    );
    let d_region_peak_ne = d_disp.realised_peak_ne(mid_colat, mid_lon_rad);
    // "Active" now means "producing at the midpoint", tied to the SAME
    // sun-above-horizon test as is_day(), so the panel can no longer disagree
    // with itself about day vs night (the old 85 deg / 90 deg split).
    let d_region_active = solar.is_day() && d_region_peak_ne > 1e-3 * ion.d_peak_ne_overhead.value;
    let rise_km = d_disp.realised_peak_rise(mid_colat, mid_lon_rad) / 1e3;
    let d_region_peak_alt_km = if rise_km.is_finite() {
        ion.d_peak_alt_km.value + rise_km
    } else {
        ion.d_peak_alt_km.value
    };
    let d_region_source = format!(
        "alpha-Chapman with Chapman grazing function Ch(X, chi) at midpoint \
         chi = {chi_deg:.2} deg: realised peak Nm/sqrt(Ch) at +H ln(Ch), staying finite \
         through the terminator. Evaluated at the LOCAL zenith angle at every point on \
         the ray, so a path crossing the terminator is absorbed only on its sunlit part. \
         Overhead anchor {:.1e} m^-3 at {:.0} km \
         (order-of-magnitude, not a fitted model)",
        ion.d_peak_ne_overhead.value, ion.d_peak_alt_km.value
    );

    // E region: the same generalised layer on the same grazing branch, so foE
    // follows (cos chi)^{1/4} without ever touching the engine's plane-parallel
    // 85 deg limit. The overhead anchor is the only free number.
    let foe_overhead_mhz = fof2::foe_overhead(inputs.ssn, ion.foe_overhead_quiet_mhz.value);
    let e_disp = SolarChapmanLayer::new(
        Box::new(ConstantPeak(fof2::e_layer_peak_ne(
            inputs.ssn,
            ion.foe_overhead_quiet_mhz.value,
        ))),
        SlantFactor::solar(solar.declination_deg, inputs.utc_hours),
        EARTH_RADIUS_M + ion.e_peak_alt_km.value * 1e3,
        ion.e_scale_height_km.value * 1e3,
    );
    let foe_midpoint_mhz = critical_frequency(PerCubicMeter::new(
        e_disp.realised_peak_ne(mid_colat, mid_lon_rad),
    ))
    .get()
        / 1e6;
    let foe_source = format!(
        "overhead foE {foe_overhead_mhz:.2} MHz from SSN = {:.0} via \
         foE^4 proportional to (1 + {:.4} R), realised as foE (cos chi)^(1/4) BY THE LAYER \
         through the same Chapman grazing function the D region uses - so it thins smoothly \
         through the terminator instead of being cut off at 85 deg. Peak {:.0} km, \
         scale {:.0} km (order-of-magnitude anchors, not a fitted model)",
        inputs.ssn,
        fof2::FOE_SOLAR_COEFF,
        ion.e_peak_alt_km.value,
        ion.e_scale_height_km.value,
    );

    // Sporadic E: probabilistic, so it never enters the deterministic verdict.
    let sporadic = if inputs.es_manual {
        SporadicE::manual(inputs.foes_mhz, inputs.es_probability)
    } else {
        SporadicE::derive(
            season,
            lst,
            mid_lat,
            ion.es_foes_max_mhz.value,
            ion.es_peak_probability.value,
        )
    };
    let es_solved = inputs.es_enabled && sporadic.is_worth_solving();

    // F2: the peak density now comes from a field, not a scalar. `fof2_mhz` is
    // its value AT THE MIDPOINT, for display; the engine gets the whole field.
    let (fof2_backend, fof2_at) = resolve_fof2(inputs, season);
    let fof2 = fof2_at(mid_lat, mid_lon);
    let fof2_tx_mhz = fof2_at(inputs.tx_lat, inputs.tx_lon);
    let fof2_rx_mhz = fof2_at(inputs.rx_lat, inputs.rx_lon);
    let fof2_source =
        fof2_source_text(inputs, fof2_backend, season, fof2, fof2_tx_mhz, fof2_rx_mhz);
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
            ion.nu_ref_per_s.value,
            ion.nu_ref_alt_km.value,
            ion.nu_scale_height_km.value,
            format!(
                "neutral-atmosphere exponential, {:.1e} /s at {:.0} km, \
                 scale {:.1} km (order-of-magnitude anchor). nu follows neutral \
                 density and is NOT a function of solar zenith angle",
                ion.nu_ref_per_s.value, ion.nu_ref_alt_km.value, ion.nu_scale_height_km.value
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
        fof2_tx_mhz,
        fof2_rx_mhz,
        fof2_backend,
        foe_overhead_mhz,
        foe_midpoint_mhz,
        foe_source,
        e_region_peak_alt_km: ion.e_peak_alt_km.value,
        sporadic_e: sporadic,
        es_solved,
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

/// Which foF2 backend will actually run, and a closure sampling it in degrees.
///
/// The fallback is deliberately not silent: if the operator asked for the grid
/// and it could not be parsed, this returns `ConstantSsn`, and
/// [`fof2_source_text`] says so in the string the panel displays.
fn resolve_fof2(
    inputs: &Inputs,
    season: Season,
) -> (Fof2Backend, Box<dyn Fn(f64, f64) -> f64 + '_>) {
    let utc = inputs.utc_hours;
    match inputs.fof2_backend {
        Fof2Backend::Gridded => match Fof2Grid::bundled() {
            Ok(grid) => {
                let peak = GriddedF2Peak::new(grid.plane(season, inputs.ssn), utc);
                (
                    Fof2Backend::Gridded,
                    Box::new(move |lat, lon| peak.fof2_at(lat, lon)),
                )
            }
            Err(_) => {
                let f = fof2_from_ssn(inputs.ssn);
                (Fof2Backend::ConstantSsn, Box::new(move |_, _| f))
            }
        },
        Fof2Backend::ConstantSsn => {
            let f = fof2_from_ssn(inputs.ssn);
            (Fof2Backend::ConstantSsn, Box::new(move |_, _| f))
        }
    }
}

/// The provenance string for whichever backend ran. Every value the F2 layer
/// was built from appears here; nothing about it is hidden from the panel.
fn fof2_source_text(
    inputs: &Inputs,
    ran: Fof2Backend,
    season: Season,
    mid: f64,
    tx: f64,
    rx: f64,
) -> String {
    let fell_back = inputs.fof2_backend == Fof2Backend::Gridded && ran == Fof2Backend::ConstantSsn;
    match ran {
        Fof2Backend::ConstantSsn => {
            let why = if fell_back {
                format!(
                    "FELL BACK to the constant model: the bundled foF2 grid failed to load ({}). ",
                    Fof2Grid::bundled().err().unwrap_or("unknown error")
                )
            } else {
                String::new()
            };
            format!(
                "{why}constant {mid:.2} MHz over the whole domain, derived from SSN = {:.0} \
                 (NmF2 linear in SSN, coarse midlat anchor; no latitude, local-time or \
                 seasonal branch)",
                inputs.ssn
            )
        }
        Fof2Backend::Gridded => format!(
            "bundled climatology grid sampled at each ray point's own latitude and local solar \
             time: {mid:.2} MHz at the midpoint, {tx:.2} at the transmitter, {rx:.2} at the \
             receiver ({} season, SSN = {:.0}, interpolated in NmF2 between the SSN 0 and SSN 100 \
             pages, bicubic in latitude and local time). NOT CCIR / URSI / IRI coefficient data: \
             the layout follows the operational maps but the values are an order-of-magnitude \
             climatology calibrated only to the same SSN anchor the constant model uses, \
             reproducing the diurnal maximum, the equatorial anomaly, the high-latitude trough \
             and the winter anomaly. Magnitudes are indicative; the VARIATION is the defensible \
             part",
            season.label(),
            inputs.ssn,
        ),
    }
}

pub fn build_models(inputs: &Inputs, a: &Assumptions) -> Result<Models, String> {
    // F2. The peak density is a FIELD now, not a scalar, so the layer varies
    // with latitude and local solar time; the vertical shape is unchanged.
    //
    // The slant factor is deliberately `Overhead`, not the solar grazing branch
    // the D and E layers use. The F2 region is transport-dominated rather than
    // in photochemical equilibrium - which is why it survives the night and why
    // the winter anomaly exists - so its day/night behaviour belongs in the
    // climatology, not in a zenith-angle law. Applying both would double-count
    // it. (Using the engine's `ChapmanLayer::with_zenith_angle` here would be
    // worse still: it refuses past 85 deg, which would put a hard absorption
    // cliff on F2 at every terminator - see docs/derivations/chapman-grazing.md.)
    let f2_source: Box<dyn crate::chapman::PeakDensitySource> = match a.fof2_backend {
        Fof2Backend::Gridded => {
            let grid = Fof2Grid::bundled()
                .map_err(|e| format!("bundled foF2 grid failed to load: {e}"))?;
            Box::new(GriddedF2Peak::new(
                grid.plane(a.season, inputs.ssn),
                inputs.utc_hours,
            ))
        }
        Fof2Backend::ConstantSsn => Box::new(ConstantPeak(a.nm_per_m3)),
    };
    let f2 = SolarChapmanLayer::new(
        f2_source,
        SlantFactor::Overhead,
        EARTH_RADIUS_M + a.hmf2_km * 1e3,
        a.scale_height_km * 1e3,
    );

    // The D region is always present and is day/night-aware: it evaluates the
    // Chapman grazing function at the local solar zenith angle of each sampled
    // point, self-zeroing smoothly on the night side rather than being switched
    // off at a midpoint zenith-angle threshold (docs/derivations/chapman-grazing.md).
    let ion = inputs.anchors.ionosphere;
    let d = SolarChapmanLayer::d_region(
        ion.d_peak_ne_overhead.value,
        EARTH_RADIUS_M + ion.d_peak_alt_km.value * 1e3,
        ion.d_scale_height_km.value * 1e3,
        a.solar.declination_deg,
        inputs.utc_hours,
        ion.d_night_floor_fraction.value,
    );

    // E region: same treatment as D, one region up. This is what gives short
    // paths somewhere to reflect from in daylight when F2 has no solution at
    // that geometry.
    let e = SolarChapmanLayer::new(
        Box::new(ConstantPeak(fof2::e_layer_peak_ne(
            inputs.ssn,
            ion.foe_overhead_quiet_mhz.value,
        ))),
        SlantFactor::solar(a.solar.declination_deg, inputs.utc_hours),
        EARTH_RADIUS_M + ion.e_peak_alt_km.value * 1e3,
        ion.e_scale_height_km.value * 1e3,
    );

    let layers: Vec<Box<dyn ElectronDensity + Send + Sync>> =
        vec![Box::new(f2), Box::new(e), Box::new(d)];
    let density = MultiLayer::new(layers);

    // The probabilistic stack: everything above, plus the Es sheet. Rebuilt
    // rather than shared because `MultiLayer` owns its layers; the cost is one
    // extra construction per solve, which is nothing next to the tracing.
    let density_with_es = if a.es_solved {
        let es = a
            .sporadic_e
            .layer(EARTH_RADIUS_M)
            .map_err(|e| format!("sporadic-E layer rejected: {e}"))?;
        let f2_source_2: Box<dyn crate::chapman::PeakDensitySource> = match a.fof2_backend {
            Fof2Backend::Gridded => {
                let grid = Fof2Grid::bundled()
                    .map_err(|e| format!("bundled foF2 grid failed to load: {e}"))?;
                Box::new(GriddedF2Peak::new(
                    grid.plane(a.season, inputs.ssn),
                    inputs.utc_hours,
                ))
            }
            Fof2Backend::ConstantSsn => Box::new(ConstantPeak(a.nm_per_m3)),
        };
        let layers: Vec<Box<dyn ElectronDensity + Send + Sync>> = vec![
            Box::new(SolarChapmanLayer::new(
                f2_source_2,
                SlantFactor::Overhead,
                EARTH_RADIUS_M + a.hmf2_km * 1e3,
                a.scale_height_km * 1e3,
            )),
            Box::new(SolarChapmanLayer::new(
                Box::new(ConstantPeak(fof2::e_layer_peak_ne(
                    inputs.ssn,
                    ion.foe_overhead_quiet_mhz.value,
                ))),
                SlantFactor::solar(a.solar.declination_deg, inputs.utc_hours),
                EARTH_RADIUS_M + ion.e_peak_alt_km.value * 1e3,
                ion.e_scale_height_km.value * 1e3,
            )),
            Box::new(SolarChapmanLayer::d_region(
                ion.d_peak_ne_overhead.value,
                EARTH_RADIUS_M + ion.d_peak_alt_km.value * 1e3,
                ion.d_scale_height_km.value * 1e3,
                a.solar.declination_deg,
                inputs.utc_hours,
                ion.d_night_floor_fraction.value,
            )),
            Box::new(es),
        ];
        Some(MultiLayer::new(layers))
    } else {
        None
    };

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
        density_with_es,
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

    fn sample_at(models: &Models, alt_km: f64, lat: f64, lon: f64) -> f64 {
        models
            .density
            .sample(&SphericalPoint::new(
                Meters::new(EARTH_RADIUS_M + alt_km * 1e3),
                Radians::from_degrees(90.0 - lat),
                Radians::from_degrees(lon),
            ))
            .ne
    }

    /// The headline change: the F2 layer is no longer one number for the whole
    /// Earth. Sampled at the same altitude at two places at the same instant,
    /// the density must genuinely differ - and it must differ in the direction
    /// the climatology claims, with the daylit side denser.
    #[test]
    fn f2_layer_varies_across_the_domain() {
        let inputs = Inputs {
            utc_hours: 12.0,
            month: 4,
            day_of_month: 15,
            ..Inputs::default()
        };
        let a = resolve(&inputs);
        assert_eq!(a.fof2_backend, Fof2Backend::Gridded, "grid must be in use");
        let models = build_models(&inputs, &a).expect("models");

        // At 12 UTC, longitude 30 E is local mid-afternoon and longitude 150 W
        // is local pre-dawn. Same latitude, same altitude, same instant.
        let day = sample_at(&models, 300.0, 40.0, 30.0);
        let night = sample_at(&models, 300.0, 40.0, -150.0);
        assert!(
            day > 1.6 * night,
            "afternoon F2 {day:.3e} should stand well above pre-dawn {night:.3e}"
        );

        // The equatorial anomaly crest at the same local time as the midlatitude
        // point beats it.
        let crest = sample_at(&models, 300.0, 16.0, 30.0);
        assert!(
            crest > day,
            "anomaly crest {crest:.3e} should exceed midlatitude {day:.3e}"
        );

        // ...and none of this happens under the constant backend. Its F2 layer
        // is exactly flat (pinned bit for bit by
        // `constant_backend_reproduces_the_engine_f2_layer_exactly`); what is
        // checked here is the whole stack, where the only residual day/night
        // difference at F2 heights is the E layer's own exponential tail. That
        // must be negligible - two orders below the structure the grid adds.
        let flat_inputs = Inputs {
            fof2_backend: Fof2Backend::ConstantSsn,
            ..inputs.clone()
        };
        let flat_a = resolve(&flat_inputs);
        let flat = build_models(&flat_inputs, &flat_a).expect("models");
        let flat_day = sample_at(&flat, 300.0, 40.0, 30.0);
        let flat_night = sample_at(&flat, 300.0, 40.0, -150.0);
        let flat_swing = (flat_day - flat_night).abs() / flat_day;
        let grid_swing = (day - night).abs() / day;
        assert!(
            flat_swing < 1e-3,
            "the constant backend's stack should be flat at F2 heights, swing {flat_swing:.2e}"
        );
        assert!(
            grid_swing > 100.0 * flat_swing,
            "the grid must add real structure: {grid_swing:.3} vs residual {flat_swing:.2e}"
        );
    }

    /// The constant backend must reproduce the OLD F2 layer bit for bit: the
    /// engine's `ChapmanLayer::new` at the same NmF2, height and scale height.
    /// This is what makes the generalisation safe to adopt - the previous
    /// behaviour is still reachable and still identical.
    #[test]
    fn constant_backend_reproduces_the_engine_f2_layer_exactly() {
        use skipzone::density::ChapmanLayer;

        let inputs = Inputs {
            fof2_backend: Fof2Backend::ConstantSsn,
            ..Inputs::default()
        };
        let a = resolve(&inputs);
        assert!(
            (a.fof2_mhz - fof2_from_ssn(inputs.ssn)).abs() < 1e-12,
            "constant backend must return the scalar anchor"
        );

        let old = ChapmanLayer::new(
            PerCubicMeter::new(a.nm_per_m3),
            Meters::new(EARTH_RADIUS_M + a.hmf2_km * 1e3),
            Meters::new(a.scale_height_km * 1e3),
        )
        .unwrap();
        let new = SolarChapmanLayer::new(
            Box::new(ConstantPeak(a.nm_per_m3)),
            SlantFactor::Overhead,
            EARTH_RADIUS_M + a.hmf2_km * 1e3,
            a.scale_height_km * 1e3,
        );
        for i in 0..=300 {
            let r = EARTH_RADIUS_M + 60e3 + (700e3 - 60e3) * f64::from(i) / 300.0;
            let p = SphericalPoint::new(Meters::new(r), Radians::new(1.1), Radians::new(-0.4));
            assert_eq!(old.sample(&p).ne.to_bits(), new.sample(&p).ne.to_bits());
            assert_eq!(
                old.sample(&p).d_ne[0].to_bits(),
                new.sample(&p).d_ne[0].to_bits()
            );
        }
    }

    /// The E layer exists, peaks where it says it does, and follows the
    /// zenith-angle law: daylight E is far denser than night E, and it does NOT
    /// vanish at the terminator the way a plane-parallel layer would (the
    /// engine refuses past 85 deg; this one keeps going).
    #[test]
    fn e_layer_follows_solar_zenith_angle_through_the_terminator() {
        let inputs = Inputs {
            utc_hours: 12.0,
            month: 3,
            day_of_month: 21,
            ..Inputs::default()
        };
        let a = resolve(&inputs);
        let models = build_models(&inputs, &a).expect("models");

        // Local noon at the equator (12 UTC, lon 0 on an equinox) is chi ~ 0.
        let noon = sample_at(&models, E_REGION_PEAK_ALT_KM, 0.0, 0.0);
        // 88 deg of longitude away is chi ~ 88 deg: past the engine's limit,
        // still producing.
        let terminator = sample_at(&models, E_REGION_PEAK_ALT_KM + 8.0, 0.0, -88.0);
        // Deep night.
        let night = sample_at(&models, E_REGION_PEAK_ALT_KM, 0.0, 180.0);

        assert!(noon > 1e11, "noon E layer Ne = {noon:.3e}");
        assert!(
            terminator > 1e-3 * noon,
            "terminator E layer collapsed to {terminator:.3e} against noon {noon:.3e}"
        );
        assert!(
            terminator < 0.5 * noon,
            "terminator E should be thinned, got {terminator:.3e} vs {noon:.3e}"
        );
        assert!(night < 1e-3 * noon, "night E layer Ne = {night:.3e}");

        // foE at the midpoint follows (cos chi)^(1/4) off the overhead anchor.
        let chi = a.solar.zenith_angle_deg.to_radians();
        if a.solar.is_day() {
            let want = a.foe_overhead_mhz * chi.cos().max(0.0).powf(0.25);
            assert!(
                (a.foe_midpoint_mhz - want).abs() < 0.05 * want.max(0.1),
                "foE {} vs (cos chi)^(1/4) law {want}",
                a.foe_midpoint_mhz
            );
        }
    }

    /// The Es stack is the deterministic stack plus a thin sheet, and nothing
    /// else: below and above the sheet the two must agree exactly, so an Es
    /// result can never be contaminated by an unintended change elsewhere.
    #[test]
    fn es_stack_differs_from_the_deterministic_one_only_at_the_sheet() {
        let inputs = Inputs {
            month: 7,
            day_of_month: 15,
            utc_hours: 16.0,
            tx_lat: 45.0,
            tx_lon: 0.0,
            rx_lat: 47.0,
            rx_lon: 4.0,
            ..Inputs::default()
        };
        let a = resolve(&inputs);
        assert!(
            a.es_solved,
            "summer afternoon midlatitude Es should be solved"
        );
        let models = build_models(&inputs, &a).expect("models");
        let with_es = models.density_with_es.as_ref().expect("an Es stack");

        let at = |d: &MultiLayer, alt_km: f64| {
            d.sample(&SphericalPoint::new(
                Meters::new(EARTH_RADIUS_M + alt_km * 1e3),
                Radians::from_degrees(90.0 - 46.0),
                Radians::from_degrees(2.0),
            ))
            .ne
        };
        for alt in [60.0, 80.0, 90.0, 93.0, 120.0, 200.0, 300.0, 500.0] {
            assert_eq!(
                at(&models.density, alt).to_bits(),
                at(with_es, alt).to_bits(),
                "the two stacks must agree at {alt} km, away from the sheet"
            );
        }
        let sheet =
            at(with_es, a.sporadic_e.height_km) - at(&models.density, a.sporadic_e.height_km);
        assert!(
            (sheet - a.sporadic_e.peak_ne()).abs() < 1e-6 * a.sporadic_e.peak_ne(),
            "the sheet should add exactly its own peak density"
        );

        // Deep winter night: no sheet is built at all.
        let quiet = Inputs {
            month: 1,
            utc_hours: 2.0,
            tx_lat: 5.0,
            rx_lat: 6.0,
            ..inputs.clone()
        };
        let qa = resolve(&quiet);
        assert!(!qa.es_solved, "negligible Es should not be solved");
        assert!(build_models(&quiet, &qa).unwrap().density_with_es.is_none());
    }

    /// Every assumed value the new layers introduce is surfaced with a source
    /// string, and the strings say what they actually are. This is the
    /// transparency contract of `Assumptions`, not decoration.
    #[test]
    fn new_assumptions_are_all_surfaced_with_provenance() {
        let a = resolve(&Inputs::default());
        assert!(a.fof2_source.contains("NOT CCIR"), "{}", a.fof2_source);
        assert!(a.fof2_source.contains("climatology"));
        assert!(a.foe_source.contains("order-of-magnitude"));
        assert!(a.foe_source.contains("cos chi"));
        assert!(a.sporadic_e.source.contains("order-of-magnitude"));
        assert!(a.sporadic_e.source.contains("NOT a Chapman layer"));
        assert!(a.foe_overhead_mhz > 0.0);
        assert!(a.sporadic_e.foes_mhz > 0.0);
        assert!((0.0..=1.0).contains(&a.sporadic_e.probability));

        // The fallback path names itself rather than quietly substituting.
        let constant = resolve(&Inputs {
            fof2_backend: Fof2Backend::ConstantSsn,
            ..Inputs::default()
        });
        assert!(
            constant.fof2_source.contains("constant"),
            "{}",
            constant.fof2_source
        );
        assert!(!constant.fof2_source.contains("FELL BACK"));
    }
}
