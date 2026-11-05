//! CCIR foF2 and M(3000)F2: the real numerical maps, evaluated.
//!
//! # Why this module exists
//!
//! [`crate::fof2`] says, in its own header, that the bundled `fof2_grid.tsv` is
//! "an ORDER-OF-MAGNITUDE CLIMATOLOGY OF MY OWN CONSTRUCTION" and that "dropping
//! a real IRI or CCIR table into the same file layout is a file replacement, not
//! a code change". That statement was half right. The data really was already in
//! the repository - `data/nequick/ccir11.txt` .. `ccir22.txt`, vendored and
//! licensed and, until now, read by nothing. What it needed was not a file
//! replacement but this module: the CCIR maps are not a latitude-by-local-time
//! table, they are a Fourier series in universal time over a Legendre-like
//! expansion in MODIFIED DIP LATITUDE, and nothing in the app knew what modip
//! was.
//!
//! `bin/iono_check` measures what that buys against measured GIRO soundings.
//!
//! # What the numbers are
//!
//! CCIR (Oslo, 1966) numerical map coefficients, as adopted by ITU-R P.1239 and
//! as distributed with the Galileo NeQuick-G reference implementation. Twelve
//! monthly files, each holding
//!
//! * `F2[2][76][13]` - 1976 foF2 coefficients, and
//! * `Fm3[2][49][9]` -  882 M(3000)F2 coefficients,
//!
//! where the leading index is a solar-activity pair, R12 = 0 and R12 = 100.
//! Provenance, verification and the EUPL v1.2 licence are in
//! `data/nequick/_SOURCE_AND_LICENSE.txt`, which must travel with the data.
//!
//! # The evaluation, term by term
//!
//! Solar activity is linear in the coefficients between the two tabulated
//! levels, which is the map's own convention:
//!
//! ```text
//! A[j][k] = F2[0][j][k] (1 - R12/100) + F2[1][j][k] (R12/100)
//! ```
//!
//! Universal time enters as a six-harmonic Fourier series on the hour angle
//! `T = 15 UT - 180` degrees, collapsing the 13 time coefficients of each term
//! to one number:
//!
//! ```text
//! CF2[j] = A[j][0] + sum_{i=1..6} ( A[j][2i-1] sin(i T) + A[j][2i] cos(i T) )
//! ```
//!
//! Geography enters as a product of `sin(modip)^k` with `cos(lat)^m cos(m lon)`
//! and `cos(lat)^m sin(m lon)`, the term counts per longitude order given by
//! [`crate::ccir::F2_GEO_DEGREE`]. Order 0 has no longitude factor and contributes 12 terms;
//! orders 1 to 8 contribute a cosine and a sine each, `12 + 2*32 = 76` in total,
//! which is where the 76 comes from.
//!
//! # Modified dip latitude
//!
//! `tan(modip) = I / sqrt(cos(lat))`, with `I` the magnetic dip at 300 km. It is
//! the coordinate the F2 layer actually organises itself along - the equatorial
//! anomaly straddles the magnetic equator, not the geographic one - and it is
//! why [`crate::fof2::climatology_fof2`]'s own comment concedes that using
//! geographic latitude for the anomaly crests "is a real limitation of this
//! table".
//!
//! It is read from `data/nequick/modipNeQG_wrapped.txt` rather than computed
//! from the IGRF model this project already carries, because the CCIR
//! coefficients were fitted in the modip of their own era: recomputing the
//! coordinate under a modern field would evaluate the map away from the
//! coordinate it was fitted in. `tests::grid_modip_agrees_with_igrf_dip` checks
//! the shipped grid against this project's IGRF anyway, which is what catches a
//! transposed or misaligned lattice.

use std::sync::OnceLock;

// --- MODIP -----------------------------------------------------------------

/// Rows in the shipped modip grid: 37 latitudes plus one wrapped row beyond
/// each pole.
pub const MODIP_ROWS: usize = 39;
/// Columns in the shipped modip grid: 37 longitudes (-180 to +180 inclusive,
/// both ends present) plus one wrapped column on each side.
pub const MODIP_COLS: usize = 39;
/// Latitude step of the modip grid, degrees.
pub const MODIP_LAT_STEP_DEG: f64 = 5.0;
/// Longitude step of the modip grid, degrees.
pub const MODIP_LON_STEP_DEG: f64 = 10.0;

const MODIP_TEXT: &str = include_str!("../../data/nequick/modipNeQG_wrapped.txt");

/// The modified-dip-latitude grid, in degrees.
///
/// Row `r` is latitude `-90 + 5 (r - 1)` and column `c` is longitude
/// `-180 + 10 (c - 1)`, so row 1 is the south pole, row 37 the north pole,
/// column 1 is 180 W and column 37 is 180 E. Rows 0 and 38 are the same
/// latitudes as rows 2 and 36 phased half a turn in longitude - the physical
/// continuation across the pole - and columns 0 and 38 repeat columns 36 and 2.
/// The wrapping is what lets a four-by-four cubic run anywhere without an edge
/// case.
pub struct ModipGrid {
    v: Box<[[f64; MODIP_COLS]; MODIP_ROWS]>,
}

impl ModipGrid {
    /// Parse the grid text: 39 lines of 39 free-format values in degrees.
    ///
    /// # Errors
    /// The first structural problem found, described. A broken asset must be
    /// reportable rather than a panic, for the same reason
    /// [`crate::fof2::Fof2Grid::parse`] is fallible.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut v = Box::new([[f64::NAN; MODIP_COLS]; MODIP_ROWS]);
        let mut rows = 0usize;
        for (lineno, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if rows >= MODIP_ROWS {
                return Err(format!("modip grid: more than {MODIP_ROWS} rows"));
            }
            let mut cols = 0usize;
            for tok in line.split_whitespace() {
                if cols >= MODIP_COLS {
                    return Err(format!(
                        "modip grid line {}: more than {MODIP_COLS} values",
                        lineno + 1
                    ));
                }
                let x: f64 = tok
                    .parse()
                    .map_err(|e| format!("modip grid line {}: bad value {tok}: {e}", lineno + 1))?;
                if !(x.is_finite() && (-90.0..=90.0).contains(&x)) {
                    return Err(format!(
                        "modip grid line {}: {x} is outside [-90, 90]",
                        lineno + 1
                    ));
                }
                v[rows][cols] = x;
                cols += 1;
            }
            if cols != MODIP_COLS {
                return Err(format!(
                    "modip grid line {}: expected {MODIP_COLS} values, got {cols}",
                    lineno + 1
                ));
            }
            rows += 1;
        }
        if rows != MODIP_ROWS {
            return Err(format!(
                "modip grid: expected {MODIP_ROWS} rows, got {rows}"
            ));
        }
        Ok(Self { v })
    }

    /// The bundled grid, parsed once per process.
    ///
    /// # Errors
    /// The parse error verbatim, so a caller can put it in front of the operator
    /// rather than silently substituting a different coordinate.
    pub fn bundled() -> Result<&'static Self, &'static str> {
        static GRID: OnceLock<Result<ModipGrid, String>> = OnceLock::new();
        match GRID.get_or_init(|| Self::parse(MODIP_TEXT)) {
            Ok(g) => Ok(g),
            Err(e) => Err(e.as_str()),
        }
    }

    /// Modified dip latitude (deg) at a geographic point, by the four-by-four
    /// third-order interpolation the maps were built to be read with.
    #[must_use]
    pub fn at(&self, lat_deg: f64, lon_deg: f64) -> f64 {
        // Fractional row and column in the lattice described on the struct.
        let a = (lat_deg.clamp(-90.0, 90.0) + 90.0) / MODIP_LAT_STEP_DEG + 1.0;
        let lon = ((lon_deg + 180.0).rem_euclid(360.0)) - 180.0;
        let b = (lon + 180.0) / MODIP_LON_STEP_DEG + 1.0;

        // The window is the four rows (columns) centred on the cell. Clamping
        // the anchor one short of the end is what keeps `lat = +90` and
        // `lon = +180` in range: there the fraction becomes exactly 1 and the
        // cubic returns its third node, which is the row (column) asked for.
        #[allow(clippy::cast_possible_truncation)]
        let ia = (a.floor() as isize).clamp(1, MODIP_ROWS as isize - 4 + 1);
        #[allow(clippy::cast_possible_truncation)]
        let ib = (b.floor() as isize).clamp(1, MODIP_COLS as isize - 4 + 1);
        #[allow(clippy::cast_precision_loss)]
        let x = a - ia as f64;
        #[allow(clippy::cast_precision_loss)]
        let y = b - ib as f64;

        let mut along_lat = [0.0_f64; 4];
        for (k, out) in along_lat.iter_mut().enumerate() {
            #[allow(clippy::cast_sign_loss)]
            let c = (ib - 1 + k as isize) as usize;
            let column = [
                #[allow(clippy::cast_sign_loss)]
                self.v[(ia - 1) as usize][c],
                #[allow(clippy::cast_sign_loss)]
                self.v[ia as usize][c],
                #[allow(clippy::cast_sign_loss)]
                self.v[(ia + 1) as usize][c],
                #[allow(clippy::cast_sign_loss)]
                self.v[(ia + 2) as usize][c],
            ];
            *out = lagrange4(column, x);
        }
        lagrange4(along_lat, y)
    }

    /// Modip (deg) and its partials `(d/dlat, d/dlon)` in degrees per degree,
    /// through a C1 interpolant.
    ///
    /// [`Self::at`] is the reference reading of the table and is what every
    /// comparison against another implementation should use. It cannot be used
    /// to build a ray-traced layer: the four-point Lagrange form is continuous
    /// but its DERIVATIVE jumps at every cell edge, and the ray equations
    /// integrate straight through those jumps - the same reason
    /// [`crate::fof2::Fof2Plane::sample`] is bicubic Catmull-Rom rather than
    /// bilinear.
    ///
    /// So the tracer reads the same table through a Catmull-Rom tensor product,
    /// which is C1 across cells and passes through the same nodes. The two
    /// interpolants differ only between nodes and only because they choose
    /// different tangents; `tests::the_two_modip_interpolants_agree` bounds the
    /// disagreement over the whole globe.
    #[must_use]
    pub fn sample_c1(&self, lat_deg: f64, lon_deg: f64) -> (f64, f64, f64) {
        let a = (lat_deg.clamp(-90.0, 90.0) + 90.0) / MODIP_LAT_STEP_DEG + 1.0;
        let lon = ((lon_deg + 180.0).rem_euclid(360.0)) - 180.0;
        let b = (lon + 180.0) / MODIP_LON_STEP_DEG + 1.0;
        #[allow(clippy::cast_possible_truncation)]
        let ia = (a.floor() as isize).clamp(1, MODIP_ROWS as isize - 3);
        #[allow(clippy::cast_possible_truncation)]
        let ib = (b.floor() as isize).clamp(1, MODIP_COLS as isize - 3);
        #[allow(clippy::cast_precision_loss)]
        let x = a - ia as f64;
        #[allow(clippy::cast_precision_loss)]
        let y = b - ib as f64;

        // Interpolate along latitude first, for each of the four columns, then
        // along longitude. The longitude derivative comes from interpolating
        // the latitude-interpolated VALUES; the latitude derivative from
        // interpolating the latitude DERIVATIVES. Same tensor-product structure
        // as `Fof2Plane::sample`, and the partials are the interpolant's own.
        let mut vals = [0.0_f64; 4];
        let mut dlat = [0.0_f64; 4];
        for (k, (v, d)) in vals.iter_mut().zip(dlat.iter_mut()).enumerate() {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
            let c = (ib - 1 + k as isize) as usize;
            #[allow(clippy::cast_sign_loss)]
            let column = [
                self.v[(ia - 1) as usize][c],
                self.v[ia as usize][c],
                self.v[(ia + 1) as usize][c],
                self.v[(ia + 2) as usize][c],
            ];
            let (value, deriv) = crate::fof2::catmull_rom(column, x);
            *v = value;
            *d = deriv;
        }
        let (m, dm_dy) = crate::fof2::catmull_rom(vals, y);
        let (dm_dx, _) = crate::fof2::catmull_rom(dlat, y);
        (m, dm_dx / MODIP_LAT_STEP_DEG, dm_dy / MODIP_LON_STEP_DEG)
    }
}

/// Cubic Lagrange through four equally spaced samples at parameter positions
/// `-1, 0, 1, 2`, evaluated at `x` in `[0, 1]`.
///
/// Written in the `delta = 2x - 1` form the NeQuick-G reference implementation
/// uses, so a value computed here can be compared with that implementation term
/// for term. `tests::lagrange4_is_the_cubic_through_its_nodes` proves the form
/// is the plain cubic interpolant and nothing else.
#[must_use]
fn lagrange4(z: [f64; 4], x: f64) -> f64 {
    if x.abs() < 1e-10 {
        return z[1];
    }
    let delta = 2.0 * x - 1.0;
    let g1 = z[2] + z[1];
    let g2 = z[2] - z[1];
    let g3 = z[3] + z[0];
    let g4 = (z[3] - z[0]) / 3.0;
    let a0 = 9.0 * g1 - g3;
    let a1 = 9.0 * g2 - g4;
    let a2 = g3 - g1;
    let a3 = g4 - g2;
    (a0 + delta * (a1 + delta * (a2 + delta * a3))) / 16.0
}

/// Modified dip latitude (deg) from the bundled grid.
///
/// # Errors
/// The grid's parse error, if the bundled asset is unusable.
pub fn modip_deg(lat_deg: f64, lon_deg: f64) -> Result<f64, &'static str> {
    Ok(ModipGrid::bundled()?.at(lat_deg, lon_deg))
}

// --- The coefficient maps --------------------------------------------------

/// foF2 terms per month and solar-activity level.
pub const F2_TERMS: usize = 76;
/// Universal-time coefficients per foF2 term: a constant plus six harmonics in
/// sine and cosine.
pub const F2_UT_TERMS: usize = 13;
/// M(3000)F2 terms per month and solar-activity level.
pub const M3000_TERMS: usize = 49;
/// Universal-time coefficients per M(3000)F2 term: a constant plus four
/// harmonics.
pub const M3000_UT_TERMS: usize = 9;

/// How many `sin(modip)^k` powers each longitude order carries, for foF2.
/// Order 0 has no longitude factor and so is counted once; every other order
/// contributes a cosine and a sine term for each power, which is what makes
/// `12 + 2 (12+9+5+2+1+1+1+1) = 76`.
pub const F2_GEO_DEGREE: [usize; 9] = [12, 12, 9, 5, 2, 1, 1, 1, 1];
/// The same, for M(3000)F2: `7 + 2 (8+6+3+2+1+1) = 49`.
pub const M3000_GEO_DEGREE: [usize; 7] = [7, 8, 6, 3, 2, 1, 1];

/// The two solar-activity levels the maps are tabulated at, as R12.
pub const R12_LEVELS: [f64; 2] = [0.0, 100.0];

/// Total numbers in one monthly file.
pub const VALUES_PER_MONTH: usize = 2 * F2_TERMS * F2_UT_TERMS + 2 * M3000_TERMS * M3000_UT_TERMS;

const CCIR_TEXT: [&str; 12] = [
    include_str!("../../data/nequick/ccir11.txt"),
    include_str!("../../data/nequick/ccir12.txt"),
    include_str!("../../data/nequick/ccir13.txt"),
    include_str!("../../data/nequick/ccir14.txt"),
    include_str!("../../data/nequick/ccir15.txt"),
    include_str!("../../data/nequick/ccir16.txt"),
    include_str!("../../data/nequick/ccir17.txt"),
    include_str!("../../data/nequick/ccir18.txt"),
    include_str!("../../data/nequick/ccir19.txt"),
    include_str!("../../data/nequick/ccir20.txt"),
    include_str!("../../data/nequick/ccir21.txt"),
    include_str!("../../data/nequick/ccir22.txt"),
];

/// One month's CCIR coefficients, both solar-activity levels.
pub struct CcirMonth {
    /// `[solar level][term][ut coefficient]`.
    f2: Box<[[[f64; F2_UT_TERMS]; F2_TERMS]; 2]>,
    fm3: Box<[[[f64; M3000_UT_TERMS]; M3000_TERMS]; 2]>,
}

impl CcirMonth {
    /// Parse one monthly file: `VALUES_PER_MONTH` free-format values, the foF2
    /// block first, in C row-major order with the last index varying fastest.
    ///
    /// # Errors
    /// A count mismatch or an unparsable token, described. The count check is
    /// the load-bearing one: it is what proves the two blocks were split at the
    /// right offset.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut vals = Vec::with_capacity(VALUES_PER_MONTH);
        for tok in text.split_whitespace() {
            let x: f64 = tok
                .parse()
                .map_err(|e| format!("ccir: bad value {tok}: {e}"))?;
            if !x.is_finite() {
                return Err(format!("ccir: non-finite value {tok}"));
            }
            vals.push(x);
        }
        if vals.len() != VALUES_PER_MONTH {
            return Err(format!(
                "ccir: expected {VALUES_PER_MONTH} values, got {}",
                vals.len()
            ));
        }

        let mut f2 = Box::new([[[0.0; F2_UT_TERMS]; F2_TERMS]; 2]);
        let mut fm3 = Box::new([[[0.0; M3000_UT_TERMS]; M3000_TERMS]; 2]);
        let mut i = 0usize;
        for level in f2.iter_mut() {
            for term in level.iter_mut() {
                for c in term.iter_mut() {
                    *c = vals[i];
                    i += 1;
                }
            }
        }
        for level in fm3.iter_mut() {
            for term in level.iter_mut() {
                for c in term.iter_mut() {
                    *c = vals[i];
                    i += 1;
                }
            }
        }
        debug_assert_eq!(i, VALUES_PER_MONTH);
        Ok(Self { f2, fm3 })
    }

    /// The bundled coefficients for a month (1 = January), parsed once per
    /// process per month.
    ///
    /// # Errors
    /// The parse error for that month, verbatim.
    pub fn bundled(month: u32) -> Result<&'static Self, &'static str> {
        static MONTHS: [OnceLock<Result<CcirMonth, String>>; 12] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        let m = (month.clamp(1, 12) - 1) as usize;
        match MONTHS[m].get_or_init(|| Self::parse(CCIR_TEXT[m])) {
            Ok(c) => Ok(c),
            Err(e) => Err(e.as_str()),
        }
    }

    /// Collapse the month to one universal time and one solar activity, which
    /// is everything a single solve needs.
    ///
    /// R12 below zero is clamped; above 100 it extrapolates along the same line
    /// rather than saturating, which is the same convention
    /// [`crate::fof2::Fof2Grid::plane`] uses for its own two levels.
    #[must_use]
    pub fn plane(&self, utc_hours: f64, r12: f64) -> CcirPlane {
        let t = (r12.max(0.0) - R12_LEVELS[0]) / (R12_LEVELS[1] - R12_LEVELS[0]);
        // The universal-time hour angle, in radians.
        let big_t = (15.0 * utc_hours - 180.0).to_radians();

        let mut sin_ut = [0.0_f64; 6];
        let mut cos_ut = [0.0_f64; 6];
        for (i, (s, c)) in sin_ut.iter_mut().zip(cos_ut.iter_mut()).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let harmonic = (i + 1) as f64 * big_t;
            let (sn, cs) = harmonic.sin_cos();
            *s = sn;
            *c = cs;
        }

        let mut cf2 = [0.0_f64; F2_TERMS];
        for (j, out) in cf2.iter_mut().enumerate() {
            let a = |k: usize| self.f2[0][j][k] + (self.f2[1][j][k] - self.f2[0][j][k]) * t;
            let mut acc = a(0);
            for i in 0..6 {
                acc += a(2 * i + 1) * sin_ut[i] + a(2 * i + 2) * cos_ut[i];
            }
            *out = acc;
        }
        let mut cm3 = [0.0_f64; M3000_TERMS];
        for (j, out) in cm3.iter_mut().enumerate() {
            let a = |k: usize| self.fm3[0][j][k] + (self.fm3[1][j][k] - self.fm3[0][j][k]) * t;
            let mut acc = a(0);
            for i in 0..4 {
                acc += a(2 * i + 1) * sin_ut[i] + a(2 * i + 2) * cos_ut[i];
            }
            *out = acc;
        }
        CcirPlane { cf2, cm3 }
    }
}

/// One month, universal time and solar activity: the geographic maps of foF2
/// and M(3000)F2, ready to evaluate anywhere.
pub struct CcirPlane {
    cf2: [f64; F2_TERMS],
    cm3: [f64; M3000_TERMS],
}

/// The geographic basis shared by both maps: `sin(modip)^k` and
/// `cos(lat)^m cos(m lon)` / `cos(lat)^m sin(m lon)`.
struct Basis {
    sin_modip_pow: [f64; 12],
    cos_lat_pow: [f64; 9],
    cos_m_lon: [f64; 9],
    sin_m_lon: [f64; 9],
}

impl Basis {
    fn new(modip_deg: f64, lat_deg: f64, lon_deg: f64) -> Self {
        let sm = modip_deg.to_radians().sin();
        let mut sin_modip_pow = [1.0_f64; 12];
        for k in 1..12 {
            sin_modip_pow[k] = sin_modip_pow[k - 1] * sm;
        }
        // The maps were fitted over the whole sphere, so `cos(lat)` is taken
        // straight; it is non-negative for every latitude in range.
        let cl = lat_deg.to_radians().cos();
        let mut cos_lat_pow = [1.0_f64; 9];
        for m in 1..9 {
            cos_lat_pow[m] = cos_lat_pow[m - 1] * cl;
        }
        let lon = lon_deg.to_radians();
        let mut cos_m_lon = [1.0_f64; 9];
        let mut sin_m_lon = [0.0_f64; 9];
        // Angle addition rather than nine calls to sin_cos; exact identity,
        // and the error over m <= 8 is a few ulp.
        let (sl, cl_lon) = lon.sin_cos();
        for m in 1..9 {
            cos_m_lon[m] = cos_m_lon[m - 1] * cl_lon - sin_m_lon[m - 1] * sl;
            sin_m_lon[m] = sin_m_lon[m - 1] * cl_lon + cos_m_lon[m - 1] * sl;
        }
        Self {
            sin_modip_pow,
            cos_lat_pow,
            cos_m_lon,
            sin_m_lon,
        }
    }

    /// Contract a coefficient vector against this basis, with the per-order term
    /// counts of whichever map it belongs to.
    fn contract(&self, coeff: &[f64], degree: &[usize]) -> f64 {
        let mut acc = 0.0;
        let mut i = 0usize;
        for k in 0..degree[0] {
            acc += coeff[i] * self.sin_modip_pow[k];
            i += 1;
        }
        for (m, &terms) in degree.iter().enumerate().skip(1) {
            for k in 0..terms {
                let g = self.sin_modip_pow[k] * self.cos_lat_pow[m];
                acc += coeff[i] * g * self.cos_m_lon[m];
                acc += coeff[i + 1] * g * self.sin_m_lon[m];
                i += 2;
            }
        }
        debug_assert_eq!(i, coeff.len());
        acc
    }
}

/// Radians per degree, written once because every partial below is a chain
/// factor through it.
const RAD_PER_DEG: f64 = std::f64::consts::PI / 180.0;

/// The same basis, carrying each factor's partials with respect to geographic
/// latitude and longitude in degrees.
///
/// Kept separate from [`Basis`] rather than folded into it because the ray
/// tracer needs the gradient on every density sample and a plain evaluation
/// (the harnesses, the UI readout) needs none of it, and the derivative arrays
/// are most of the work.
struct GradBasis {
    /// `(value, d/dlat, d/dlon)` for `sin(modip)^k`.
    s: [[f64; 3]; 12],
    /// `(value, d/dlat)` for `cos(lat)^m`; it has no longitude dependence.
    l: [[f64; 2]; 9],
    /// `(value, d/dlon)` for `cos(m lon)` and for `sin(m lon)`.
    c: [[f64; 2]; 9],
    sn: [[f64; 2]; 9],
}

impl GradBasis {
    /// `modip` and its partials come from [`ModipGrid::sample_c1`]; everything
    /// else is closed form.
    fn new(modip: (f64, f64, f64), lat_deg: f64, lon_deg: f64) -> Self {
        let (md, dmd_dlat, dmd_dlon) = modip;
        let (sm, cm) = (md * RAD_PER_DEG).sin_cos();
        // d sin(modip)/dlat = cos(modip) * d(modip [rad])/dlat.
        let (ds_dlat, ds_dlon) = (cm * RAD_PER_DEG * dmd_dlat, cm * RAD_PER_DEG * dmd_dlon);
        let mut s = [[0.0_f64; 3]; 12];
        s[0] = [1.0, 0.0, 0.0];
        for k in 1..12 {
            // (s^k)' = k s^(k-1) s'  -  written as a recurrence on the product
            // rule so no power is formed twice.
            s[k] = [
                s[k - 1][0] * sm,
                s[k - 1][1] * sm + s[k - 1][0] * ds_dlat,
                s[k - 1][2] * sm + s[k - 1][0] * ds_dlon,
            ];
        }

        let (sl, cl) = (lat_deg * RAD_PER_DEG).sin_cos();
        let dcl_dlat = -sl * RAD_PER_DEG;
        let mut l = [[0.0_f64; 2]; 9];
        l[0] = [1.0, 0.0];
        for m in 1..9 {
            l[m] = [l[m - 1][0] * cl, l[m - 1][1] * cl + l[m - 1][0] * dcl_dlat];
        }

        let (slon, clon) = (lon_deg * RAD_PER_DEG).sin_cos();
        let mut c = [[0.0_f64; 2]; 9];
        let mut sn = [[0.0_f64; 2]; 9];
        c[0] = [1.0, 0.0];
        sn[0] = [0.0, 0.0];
        for m in 1..9 {
            let (cc, ss) = (
                c[m - 1][0] * clon - sn[m - 1][0] * slon,
                sn[m - 1][0] * clon + c[m - 1][0] * slon,
            );
            #[allow(clippy::cast_precision_loss)]
            let mf = m as f64;
            c[m] = [cc, -mf * ss * RAD_PER_DEG];
            sn[m] = [ss, mf * cc * RAD_PER_DEG];
        }

        Self { s, l, c, sn }
    }

    /// Contract a coefficient vector, returning `(value, d/dlat, d/dlon)`.
    fn contract(&self, coeff: &[f64], degree: &[usize]) -> (f64, f64, f64) {
        let (mut v, mut dlat, mut dlon) = (0.0, 0.0, 0.0);
        let mut i = 0usize;
        for k in 0..degree[0] {
            v += coeff[i] * self.s[k][0];
            dlat += coeff[i] * self.s[k][1];
            dlon += coeff[i] * self.s[k][2];
            i += 1;
        }
        for (m, &terms) in degree.iter().enumerate().skip(1) {
            for k in 0..terms {
                // g = s^k cos(lat)^m, and then a cosine and a sine in longitude.
                let g = self.s[k][0] * self.l[m][0];
                let g_lat = self.s[k][1] * self.l[m][0] + self.s[k][0] * self.l[m][1];
                let g_lon = self.s[k][2] * self.l[m][0];
                for (cf, trig) in [(coeff[i], &self.c[m]), (coeff[i + 1], &self.sn[m])] {
                    v += cf * g * trig[0];
                    dlat += cf * g_lat * trig[0];
                    dlon += cf * (g_lon * trig[0] + g * trig[1]);
                }
                i += 2;
            }
        }
        debug_assert_eq!(i, coeff.len());
        (v, dlat, dlon)
    }
}

impl CcirPlane {
    /// foF2 (MHz) at a geographic point, given that point's modip.
    ///
    /// Returned raw. The map is a truncated expansion and can undershoot in the
    /// deep polar night; a consumer that needs a physical layer applies its own
    /// floor and says so, rather than having one hidden here.
    #[must_use]
    pub fn fof2_at_modip(&self, modip_deg: f64, lat_deg: f64, lon_deg: f64) -> f64 {
        Basis::new(modip_deg, lat_deg, lon_deg).contract(&self.cf2, &F2_GEO_DEGREE)
    }

    /// M(3000)F2 - the obliquity factor for a 3000 km hop - at a geographic
    /// point, given that point's modip. Dimensionless, physically 2.2 to 3.8.
    ///
    /// Nothing consumes this yet. It is evaluated because it is the second half
    /// of every monthly file, and getting it into its physical range is the
    /// check that the two blocks were split at the right offset.
    #[must_use]
    pub fn m3000_at_modip(&self, modip_deg: f64, lat_deg: f64, lon_deg: f64) -> f64 {
        Basis::new(modip_deg, lat_deg, lon_deg).contract(&self.cm3, &M3000_GEO_DEGREE)
    }

    /// foF2 (MHz) at a geographic point, taking modip from the bundled grid.
    ///
    /// # Errors
    /// The modip grid's parse error, if the bundled asset is unusable.
    pub fn fof2(&self, lat_deg: f64, lon_deg: f64) -> Result<f64, &'static str> {
        Ok(self.fof2_at_modip(modip_deg(lat_deg, lon_deg)?, lat_deg, lon_deg))
    }
}

impl CcirPlane {
    /// foF2 (MHz) and its partials `(d/dlat, d/dlon)` in MHz per degree, for the
    /// ray tracer. Uses the C1 modip interpolant - see [`ModipGrid::sample_c1`].
    #[must_use]
    pub fn fof2_with_gradient(
        &self,
        grid: &ModipGrid,
        lat_deg: f64,
        lon_deg: f64,
    ) -> (f64, f64, f64) {
        GradBasis::new(grid.sample_c1(lat_deg, lon_deg), lat_deg, lon_deg)
            .contract(&self.cf2, &F2_GEO_DEGREE)
    }
}

/// Ratio of the SILSO version 2.0 sunspot number to the version 1 series it
/// replaced in July 2015. The recalibration multiplied the whole historical
/// record by about this factor.
///
/// It matters because the two ends of the calculation are on different scales
/// and nothing in between says so. Every sunspot number this project handles -
/// `spaceweather`'s SIDC and NOAA feeds, `corpus/ssn_daily.tsv` - is version 2.
/// The CCIR maps were fitted in 1966, so their `R12 = 100` page means version
/// 1's hundred. Feeding a version 2 number straight in overstates solar
/// activity by 44 %.
///
/// MEASURED, 2026-08-09, `bin/iono_check` over 70 219 GIRO soundings: driving
/// the maps with the raw version 2 R12 leaves a **+0.92 MHz** bias; dividing by
/// this factor leaves **+0.03 MHz**. Nothing was fitted to produce that - the
/// factor is SILSO's published recalibration and the bias fell out.
pub const SSN_V2_OVER_V1: f64 = 1.44;

/// A modern (version 2) sunspot number on the scale the CCIR maps were fitted
/// in. A conversion, not a calibration: see [`SSN_V2_OVER_V1`].
#[must_use]
pub fn map_r12_from_ssn_v2(ssn_v2: f64) -> f64 {
    ssn_v2.max(0.0) / SSN_V2_OVER_V1
}

/// Numerical guard on foF2 (MHz) from the maps.
///
/// This is NOT a model of the night F region - [`crate::fof2::FOF2_FLOOR_MHZ`]
/// is that, for the synthetic climatology, and applying a floor of that size
/// here would lift CCIR's own night values by tens of percent. This one exists
/// only so that a truncated spherical expansion cannot hand the layer a zero or
/// negative critical frequency in some corner nobody sampled.
/// `tests::the_floor_does_not_bind` shows it is never reached.
pub const CCIR_FLOOR_MHZ: f64 = 0.5;

/// Peak density of the F2 layer from the CCIR maps.
///
/// Holds one month, universal time and solar activity, and evaluates foF2 at
/// each sampled point's own geographic position - not at a local solar time.
/// That is the substantive difference from [`crate::fof2::GriddedF2Peak`]: the
/// CCIR maps are functions of longitude AND universal time separately, so two
/// points at the same local solar time in different sectors get different
/// answers, which is most of what the longitude orders are for.
pub struct CcirF2Peak {
    plane: CcirPlane,
    modip: &'static ModipGrid,
}

impl CcirF2Peak {
    /// # Errors
    /// The bundled modip grid's parse error.
    pub fn new(plane: CcirPlane) -> Result<Self, &'static str> {
        Ok(Self {
            plane,
            modip: ModipGrid::bundled()?,
        })
    }

    /// foF2 (MHz) at a geographic position, floored, for display and for the
    /// harnesses.
    #[must_use]
    pub fn fof2_at(&self, lat_deg: f64, lon_deg: f64) -> f64 {
        self.plane
            .fof2_at_modip(self.modip.at(lat_deg, lon_deg), lat_deg, lon_deg)
            .max(CCIR_FLOOR_MHZ)
    }
}

impl crate::chapman::PeakDensitySource for CcirF2Peak {
    fn peak(&self, colat: f64, lon: f64) -> crate::chapman::PeakSample {
        let lat_deg = 90.0 - colat.to_degrees();
        let lon_deg = lon.to_degrees();
        let (f, df_dlat, df_dlon) = self.plane.fof2_with_gradient(self.modip, lat_deg, lon_deg);
        if !f.is_finite() || f <= CCIR_FLOOR_MHZ {
            // At or below the guard the layer is flat by construction, so the
            // gradient is zero rather than the expansion's - a floored value
            // whose gradient still slopes is what drives Hamiltonian drift.
            let nm = skipzone::density::density_at_critical_frequency(skipzone::units::Hertz::new(
                CCIR_FLOOR_MHZ * 1e6,
            ))
            .get();
            return crate::chapman::PeakSample::uniform(nm);
        }
        let nm =
            skipzone::density::density_at_critical_frequency(skipzone::units::Hertz::new(f * 1e6))
                .get();
        // Nm = K f^2, so dNm/dx = (2 Nm / f) df/dx. The chain factors to engine
        // coordinates are the same two `fof2` uses: lat = 90 - theta in degrees
        // (so dlat/dtheta = -180/pi) and lon in degrees (dlon/dphi = 180/pi).
        let k = 2.0 * nm / f;
        const DEG_PER_RAD: f64 = 180.0 / std::f64::consts::PI;
        crate::chapman::PeakSample {
            nm,
            d_nm: [k * df_dlat * -DEG_PER_RAD, k * df_dlon * DEG_PER_RAD],
        }
    }
}

/// foF2 (MHz) from the CCIR maps, for callers that want one value and do not
/// hold a plane.
///
/// `month` is 1 to 12, `utc_hours` is universal time, `r12` is the twelve-month
/// smoothed sunspot number the maps are indexed by - NOT a daily sunspot number,
/// though a caller with only a daily number will pass it and should say so.
///
/// # Errors
/// The bundled asset's parse error, verbatim.
pub fn ccir_fof2(
    lat_deg: f64,
    lon_deg: f64,
    month: u32,
    utc_hours: f64,
    r12: f64,
) -> Result<f64, &'static str> {
    CcirMonth::bundled(month)?
        .plane(utc_hours, r12)
        .fof2(lat_deg, lon_deg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four-point form really is the cubic Lagrange interpolant: it passes
    /// through all four nodes, and it reproduces a cubic polynomial exactly.
    /// If this drifts, every modip value drifts with it.
    #[test]
    fn lagrange4_is_the_cubic_through_its_nodes() {
        // A cubic sampled at -1, 0, 1, 2 must be reproduced everywhere.
        let p = |t: f64| 0.7 - 1.3 * t + 0.45 * t * t - 0.2 * t * t * t;
        let z = [p(-1.0), p(0.0), p(1.0), p(2.0)];
        for i in 0..=40 {
            let x = f64::from(i) / 40.0;
            let got = lagrange4(z, x);
            assert!((got - p(x)).abs() < 1e-12, "x={x}: {got} vs {}", p(x));
        }
        // Nodes, including the short-circuit at x = 0.
        assert!((lagrange4(z, 0.0) - z[1]).abs() < 1e-15);
        assert!((lagrange4(z, 1.0) - z[2]).abs() < 1e-12);
    }

    /// The shipped grid parses to the documented lattice, and the wrap rows and
    /// columns really are the wraps the header claims. A transposed or
    /// misaligned grid would evaluate the CCIR maps in the wrong coordinate and
    /// nothing downstream would notice, so this is checked structurally.
    #[test]
    fn modip_grid_is_wrapped_as_documented() {
        let g = ModipGrid::bundled().expect("bundled modip grid parses");

        // Row 1 is the south pole, row 37 the north pole.
        for c in 0..MODIP_COLS {
            assert!((g.v[1][c] + 90.0).abs() < 1e-9, "row 1 col {c}");
            assert!((g.v[37][c] - 90.0).abs() < 1e-9, "row 37 col {c}");
        }

        // Longitude is periodic with 36 columns (360 degrees).
        for r in 0..MODIP_ROWS {
            for c in 0..3 {
                assert!(
                    (g.v[r][c] - g.v[r][c + 36]).abs() < 1e-9,
                    "column wrap at row {r}, col {c}"
                );
            }
        }

        // The rows beyond each pole are their in-range twins phased half a turn,
        // which is what "across the pole" means on a sphere.
        for c in 0..18 {
            assert!(
                (g.v[0][c] - g.v[2][c + 18]).abs() < 1e-9,
                "south polar wrap at col {c}"
            );
            assert!(
                (g.v[38][c] - g.v[36][c + 18]).abs() < 1e-9,
                "north polar wrap at col {c}"
            );
        }
    }

    /// The interpolant reproduces the tabulated values at the lattice points,
    /// including both extreme corners, and stays inside [-90, 90] between them.
    #[test]
    fn modip_interpolation_hits_its_nodes() {
        let g = ModipGrid::bundled().unwrap();
        for r in 1..=37 {
            #[allow(clippy::cast_precision_loss)]
            let lat = -90.0 + MODIP_LAT_STEP_DEG * (r as f64 - 1.0);
            for c in 1..=37 {
                #[allow(clippy::cast_precision_loss)]
                let lon = -180.0 + MODIP_LON_STEP_DEG * (c as f64 - 1.0);
                let got = g.at(lat, lon);
                assert!(
                    (got - g.v[r][c]).abs() < 1e-9,
                    "node ({lat}, {lon}): {got} vs {}",
                    g.v[r][c]
                );
            }
        }
        for lat in [-90.0_f64, -47.3, -0.5, 12.8, 61.2, 90.0] {
            for lon in [-180.0_f64, -111.0, -3.7, 0.0, 88.4, 179.9, 180.0] {
                let m = g.at(lat, lon);
                assert!((-90.0..=90.0).contains(&m), "modip {m} at ({lat}, {lon})");
            }
        }
    }

    /// Longitude is periodic through the interpolant, not just in the table: a
    /// seam at one meridian would put a discontinuity into every F2 map.
    #[test]
    fn modip_is_periodic_in_longitude() {
        let g = ModipGrid::bundled().unwrap();
        for lat in [-63.0, -12.0, 0.0, 33.0, 71.0] {
            for lon in [-180.0_f64, -95.5, 0.0, 47.2, 179.0] {
                let a = g.at(lat, lon);
                let b = g.at(lat, lon + 360.0);
                assert!((a - b).abs() < 1e-9, "seam at ({lat}, {lon}): {a} vs {b}");
            }
        }
    }

    /// The shipped grid is the coordinate it claims to be, checked against this
    /// project's own IGRF rather than against itself: `tan(modip) = I / sqrt(cos
    /// lat)` with `I` the dip at 300 km.
    ///
    /// This is the test that catches a transposed lattice, a hemisphere flip or
    /// an off-by-one row. It is deliberately loose: the grid is epoch ~2001 and
    /// the IGRF model here runs at a modern epoch, so the two disagree by a
    /// couple of degrees from secular variation alone, and more where the field
    /// is weak. A wrong indexing would miss by tens of degrees.
    #[test]
    fn grid_modip_agrees_with_igrf_dip() {
        use skipzone::geo::SphericalPoint;
        use skipzone::mag::{Igrf, MagneticField};
        use skipzone::units::{Meters, Radians};

        let igrf = Igrf::from_embedded().expect("embedded IGRF");
        let model = igrf.model_at(2001.0).expect("IGRF at 2001");
        let g = ModipGrid::bundled().unwrap();

        let mut worst: f64 = 0.0;
        let mut worst_at = (0.0, 0.0);
        let mut n = 0usize;
        let mut sum_sq = 0.0;
        for lat_i in -8..=8 {
            let lat = f64::from(lat_i) * 10.0;
            // Poles are degenerate in this coordinate; sqrt(cos lat) -> 0.
            if lat.abs() > 80.0 {
                continue;
            }
            for lon_i in -17..=18 {
                let lon = f64::from(lon_i) * 10.0;
                let p = SphericalPoint::new(
                    Meters::new(6_371_200.0 + 300_000.0),
                    Radians::new((90.0 - lat).to_radians()),
                    Radians::new(lon.to_radians()),
                );
                let s = model.sample(&p);
                // Spherical (r, theta, phi) to local (north, east, down):
                // north = -B_theta, east = B_phi, down = -B_r.
                let north = -s.b[1];
                let east = s.b[2];
                let down = -s.b[0];
                let dip = down.atan2(north.hypot(east));
                let want = (dip / lat.to_radians().cos().max(1e-12).sqrt())
                    .atan()
                    .to_degrees();
                let got = g.at(lat, lon);
                let d = (got - want).abs();
                if d > worst {
                    worst = d;
                    worst_at = (lat, lon);
                }
                sum_sq += d * d;
                n += 1;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let rms = (sum_sq / n as f64).sqrt();
        assert!(
            rms < 2.0 && worst < 6.0,
            "grid modip vs IGRF-2001 dip: RMS {rms:.2} deg, worst {worst:.2} deg at {worst_at:?}"
        );
    }

    /// Every monthly file parses to exactly the documented value count. The
    /// count is what proves the foF2 and M(3000)F2 blocks were split at the
    /// right offset; a wrong split would still produce numbers.
    #[test]
    fn every_month_parses_to_the_documented_shape() {
        assert_eq!(VALUES_PER_MONTH, 2858);
        assert_eq!(
            F2_GEO_DEGREE[0] + 2 * F2_GEO_DEGREE[1..].iter().sum::<usize>(),
            F2_TERMS
        );
        assert_eq!(
            M3000_GEO_DEGREE[0] + 2 * M3000_GEO_DEGREE[1..].iter().sum::<usize>(),
            M3000_TERMS
        );
        for month in 1..=12 {
            CcirMonth::bundled(month).expect("bundled CCIR month parses");
        }
    }

    /// foF2 from the maps is a physical ionosphere everywhere on the globe, at
    /// every hour, in every month, at both tabulated activity levels: positive,
    /// bounded, and rising with solar activity.
    ///
    /// This is the test that fails loudly if the term ordering is wrong. A
    /// mis-ordered expansion does not produce a slightly worse ionosphere, it
    /// produces negative critical frequencies and tens of megahertz.
    #[test]
    fn ccir_fof2_is_a_physical_ionosphere_everywhere() {
        let mut lo_min = f64::INFINITY;
        let mut hi_max: f64 = 0.0;
        let mut rising = 0usize;
        let mut total = 0usize;
        for month in [1u32, 3, 6, 9, 12] {
            let c = CcirMonth::bundled(month).unwrap();
            for ut in [0.0_f64, 4.0, 8.0, 12.0, 16.0, 20.0] {
                let lo = c.plane(ut, 0.0);
                let hi = c.plane(ut, 100.0);
                for lat_i in -8..=8 {
                    let lat = f64::from(lat_i) * 10.0;
                    for lon_i in -6..=5 {
                        let lon = f64::from(lon_i) * 30.0;
                        let a = lo.fof2(lat, lon).unwrap();
                        let b = hi.fof2(lat, lon).unwrap();
                        assert!(
                            a.is_finite() && b.is_finite(),
                            "non-finite foF2 at {month} {ut} ({lat}, {lon})"
                        );
                        assert!(
                            (0.5..25.0).contains(&a) && (0.5..25.0).contains(&b),
                            "foF2 out of physical range at month {month}, UT {ut}, \
                             ({lat}, {lon}): R12=0 {a:.2}, R12=100 {b:.2}"
                        );
                        lo_min = lo_min.min(a);
                        hi_max = hi_max.max(b);
                        if b > a {
                            rising += 1;
                        }
                        total += 1;
                    }
                }
            }
        }
        // Solar activity raises foF2 essentially everywhere. Not literally
        // everywhere - the maps are fitted, and a handful of high-latitude
        // night cells invert - so this is asserted as a fraction rather than
        // as an absolute, and the fraction is high enough that a sign error
        // could not pass.
        #[allow(clippy::cast_precision_loss)]
        let frac = rising as f64 / total as f64;
        assert!(
            frac > 0.95,
            "foF2 rises with R12 in only {:.1}% of cells",
            100.0 * frac
        );
        // And the global span is the one an ionosphere has: a night-time floor
        // of a megahertz or two and a daytime anomaly crest in the teens.
        assert!(
            lo_min < 3.0,
            "quiet-sun minimum foF2 {lo_min:.2} MHz is too high"
        );
        assert!(
            hi_max > 12.0,
            "active-sun maximum foF2 {hi_max:.2} MHz is too low"
        );
    }

    /// M(3000)F2 lands in its physical range. Independent of foF2, and it is
    /// what confirms the second block of every file is being read as itself
    /// rather than as a continuation of the first.
    #[test]
    fn m3000_is_in_its_physical_range() {
        for month in [1u32, 6, 12] {
            let c = CcirMonth::bundled(month).unwrap();
            for ut in [0.0_f64, 6.0, 12.0, 18.0] {
                let p = c.plane(ut, 60.0);
                for lat_i in -8..=8 {
                    let lat = f64::from(lat_i) * 10.0;
                    for lon_i in -6..=5 {
                        let lon = f64::from(lon_i) * 30.0;
                        let m = p.m3000_at_modip(modip_deg(lat, lon).unwrap(), lat, lon);
                        assert!(
                            (2.0..4.0).contains(&m),
                            "M(3000)F2 {m:.3} outside 2.0-4.0 at month {month}, \
                             UT {ut}, ({lat}, {lon})"
                        );
                    }
                }
            }
        }
    }

    /// The maps are continuous across the date line and across the universal
    /// time wrap. Both are seams that would show up in a coverage map as a wall.
    #[test]
    fn ccir_fof2_has_no_seams() {
        let c = CcirMonth::bundled(4).unwrap();
        let p = c.plane(9.5, 70.0);
        for lat in [-55.0, -10.0, 0.0, 25.0, 60.0] {
            let a = p.fof2(lat, 180.0).unwrap();
            let b = p.fof2(lat, -180.0).unwrap();
            assert!((a - b).abs() < 1e-9, "date line at lat {lat}: {a} vs {b}");
        }
        for (lat, lon) in [(45.0, 10.0), (-20.0, -70.0), (5.0, 120.0)] {
            let a = c.plane(0.0, 70.0).fof2(lat, lon).unwrap();
            let b = c.plane(24.0, 70.0).fof2(lat, lon).unwrap();
            assert!(
                (a - b).abs() < 1e-9,
                "UT wrap at ({lat}, {lon}): {a} vs {b}"
            );
        }
    }

    /// The C1 interpolant used by the tracer and the reference Lagrange form
    /// used for comparison read the same table, so they must agree at the nodes
    /// exactly and closely between them. The bound is what says "reading the
    /// map through a C1 interpolant is not a different map".
    #[test]
    fn the_two_modip_interpolants_agree() {
        let g = ModipGrid::bundled().unwrap();
        let mut worst: f64 = 0.0;
        let mut at = (0.0, 0.0);
        let mut lat = -90.0;
        while lat <= 90.0 {
            let mut lon = -180.0;
            while lon <= 180.0 {
                let d = (g.at(lat, lon) - g.sample_c1(lat, lon).0).abs();
                if d > worst {
                    worst = d;
                    at = (lat, lon);
                }
                lon += 1.0;
            }
            lat += 1.0;
        }
        // The worst disagreement is next to a pole, where modip runs from -90
        // to -76 in one grid row and the two forms choose different tangents
        // across the steepest cell on the globe.
        assert!(
            worst < 0.6 && at.0.abs() > 80.0,
            "modip interpolants differ by {worst:.3} deg at {at:?}"
        );

        // That is the wrong quantity to bound, though, and this is the right
        // one. foF2 depends on modip only through sin(modip), whose derivative
        // vanishes at exactly the poles where the two forms disagree most, so
        // the disagreement that reaches the tracer is far smaller than the
        // disagreement in the coordinate.
        let mut worst_f: f64 = 0.0;
        let mut at_f = (0u32, 0.0, 0.0, 0.0);
        for month in [1u32, 4, 7, 10] {
            let c = CcirMonth::bundled(month).unwrap();
            for ut_i in 0..6 {
                let ut = f64::from(ut_i) * 4.0;
                let p = c.plane(ut, 70.0);
                let mut lat = -90.0;
                while lat <= 90.0 {
                    let mut lon = -180.0;
                    while lon < 180.0 {
                        let a = p.fof2_at_modip(g.at(lat, lon), lat, lon);
                        let b = p.fof2_with_gradient(g, lat, lon).0;
                        if (a - b).abs() > worst_f {
                            worst_f = (a - b).abs();
                            at_f = (month, ut, lat, lon);
                        }
                        lon += 5.0;
                    }
                    lat += 2.5;
                }
            }
        }
        assert!(
            worst_f < 0.05,
            "reading the map through the C1 interpolant moves foF2 by \
             {worst_f:.4} MHz at {at_f:?}"
        );

        // At the nodes both are the tabulated value, so they agree exactly.
        for r in 1..=37 {
            #[allow(clippy::cast_precision_loss)]
            let lat = -90.0 + MODIP_LAT_STEP_DEG * (r as f64 - 1.0);
            for c in 1..=37 {
                #[allow(clippy::cast_precision_loss)]
                let lon = -180.0 + MODIP_LON_STEP_DEG * (c as f64 - 1.0);
                assert!(
                    (g.sample_c1(lat, lon).0 - g.v[r][c]).abs() < 1e-9,
                    "C1 node ({lat}, {lon})"
                );
            }
        }
    }

    /// The C1 sampler's partials are the partials of the function it is
    /// actually returning, checked against central differences of itself, and
    /// they are continuous across a cell boundary. The second half is the whole
    /// reason this interpolant exists: a gradient that jumps is what the
    /// Lagrange form does and what the ray equations cannot integrate.
    #[test]
    fn modip_c1_partials_are_exact_and_continuous() {
        let g = ModipGrid::bundled().unwrap();
        for &lat in &[-71.3_f64, -18.0, 0.7, 22.4, 47.9, 63.2] {
            for &lon in &[-177.0_f64, -95.5, -3.7, 41.2, 122.8, 176.0] {
                let (_, dlat, dlon) = g.sample_c1(lat, lon);
                let h = 1e-6;
                let fd_lat =
                    (g.sample_c1(lat + h, lon).0 - g.sample_c1(lat - h, lon).0) / (2.0 * h);
                let fd_lon =
                    (g.sample_c1(lat, lon + h).0 - g.sample_c1(lat, lon - h).0) / (2.0 * h);
                assert!(
                    (dlat - fd_lat).abs() < 1e-5 * fd_lat.abs().max(1.0),
                    "d/dlat at ({lat}, {lon}): {dlat} vs {fd_lat}"
                );
                assert!(
                    (dlon - fd_lon).abs() < 1e-5 * fd_lon.abs().max(1.0),
                    "d/dlon at ({lat}, {lon}): {dlon} vs {fd_lon}"
                );
            }
        }

        // Straddle a lattice line in each axis. The C1 form must match across
        // it; the reference Lagrange form is allowed not to, and is shown not
        // to, so this is measuring something real.
        let eps = 1e-7;
        let mut jump_c1: f64 = 0.0;
        let mut jump_ref: f64 = 0.0;
        for lat_node in [-40.0_f64, -5.0, 25.0, 60.0] {
            for lon in [-120.0_f64, 10.0, 95.0] {
                let a = g.sample_c1(lat_node - eps, lon).1;
                let b = g.sample_c1(lat_node + eps, lon).1;
                jump_c1 = jump_c1.max((a - b).abs());
                let fd = |l: f64| (g.at(l + 1e-6, lon) - g.at(l - 1e-6, lon)) / 2e-6;
                jump_ref = jump_ref.max((fd(lat_node - 1e-4) - fd(lat_node + 1e-4)).abs());
            }
        }
        assert!(jump_c1 < 1e-6, "C1 gradient jumps by {jump_c1:.3e}");
        assert!(
            jump_ref > 1e-3,
            "the reference form was expected to jump, but jumped by only {jump_ref:.3e} - \
             if it no longer does, sample_c1 has stopped earning its keep"
        );
    }

    /// The foF2 gradient the tracer consumes matches central differences of the
    /// foF2 the tracer consumes. A gradient that disagrees with its own field
    /// does not produce a slightly wrong number here; it drives Hamiltonian
    /// drift far away in the solver.
    #[test]
    fn fof2_gradient_matches_finite_differences() {
        let grid = ModipGrid::bundled().unwrap();
        let p = CcirMonth::bundled(5).unwrap().plane(11.0, 70.0);
        for &lat in &[-62.0_f64, -21.5, 3.0, 18.7, 44.0, 68.5] {
            for &lon in &[-150.0_f64, -62.5, 0.0, 33.3, 128.0] {
                let (_, dlat, dlon) = p.fof2_with_gradient(grid, lat, lon);
                let h = 1e-6;
                let f = |la: f64, lo: f64| p.fof2_with_gradient(grid, la, lo).0;
                let fd_lat = (f(lat + h, lon) - f(lat - h, lon)) / (2.0 * h);
                let fd_lon = (f(lat, lon + h) - f(lat, lon - h)) / (2.0 * h);
                assert!(
                    (dlat - fd_lat).abs() < 1e-5 * fd_lat.abs().max(1e-3),
                    "dfoF2/dlat at ({lat}, {lon}): {dlat} vs {fd_lat}"
                );
                assert!(
                    (dlon - fd_lon).abs() < 1e-5 * fd_lon.abs().max(1e-3),
                    "dfoF2/dlon at ({lat}, {lon}): {dlon} vs {fd_lon}"
                );
            }
        }
    }

    /// The peak source's partials, in ENGINE coordinates, against central
    /// differences of its own Ne. This is the check that the two chain-rule
    /// factors carry the right signs; a flipped sign is invisible in the
    /// density and fatal in the tracer. Mirrors
    /// `fof2::tests::gridded_peak_partials_match_fd`.
    #[test]
    fn ccir_peak_partials_match_fd() {
        use crate::chapman::PeakDensitySource;
        let src = CcirF2Peak::new(CcirMonth::bundled(9).unwrap().plane(15.0, 60.0)).unwrap();
        for (colat, lon) in [
            (0.9_f64, 0.3_f64),
            (1.6, -2.2),
            (2.4, 1.1),
            (0.35, 3.0),
            (2.8, -0.4),
        ] {
            let s = src.peak(colat, lon);
            let h = 1e-6;
            let fd_theta = (src.peak(colat + h, lon).nm - src.peak(colat - h, lon).nm) / (2.0 * h);
            let fd_phi = (src.peak(colat, lon + h).nm - src.peak(colat, lon - h).nm) / (2.0 * h);
            assert!(
                (s.d_nm[0] - fd_theta).abs() < 1e-5 * fd_theta.abs().max(1e-3 * s.nm),
                "dtheta at ({colat}, {lon}): {} vs {fd_theta}",
                s.d_nm[0]
            );
            assert!(
                (s.d_nm[1] - fd_phi).abs() < 1e-5 * fd_phi.abs().max(1e-3 * s.nm),
                "dphi at ({colat}, {lon}): {} vs {fd_phi}",
                s.d_nm[1]
            );
        }
    }

    /// The numerical guard is a guard and not a model: it is never reached, at
    /// either tabulated activity level, in any month, anywhere.
    #[test]
    fn the_floor_does_not_bind() {
        let mut lowest = f64::INFINITY;
        let mut at = (0u32, 0.0, 0.0, 0.0);
        for month in 1..=12u32 {
            let c = CcirMonth::bundled(month).unwrap();
            for ut_i in 0..12 {
                let ut = f64::from(ut_i) * 2.0;
                let p = c.plane(ut, 0.0);
                let mut lat = -90.0;
                while lat <= 90.0 {
                    let mut lon = -180.0;
                    while lon < 180.0 {
                        let f = p.fof2(lat, lon).unwrap();
                        if f < lowest {
                            lowest = f;
                            at = (month, ut, lat, lon);
                        }
                        lon += 10.0;
                    }
                    lat += 5.0;
                }
            }
        }
        assert!(
            lowest > CCIR_FLOOR_MHZ,
            "the guard binds: lowest map foF2 is {lowest:.3} MHz at {at:?}"
        );
    }

    /// The sunspot-scale conversion is a division and stays one - clamped at
    /// zero, never negative, and exactly the published factor.
    #[test]
    fn the_sunspot_scale_conversion_is_a_conversion() {
        assert!((map_r12_from_ssn_v2(144.0) - 100.0).abs() < 1e-9);
        assert!((map_r12_from_ssn_v2(0.0)).abs() < 1e-12);
        assert!((map_r12_from_ssn_v2(-30.0)).abs() < 1e-12);
        assert!((SSN_V2_OVER_V1 - 1.44).abs() < 1e-12);
    }

    /// The behaviours the F2 layer is known for, as trends rather than
    /// absolutes. These are the same claims `fof2::climatology_trends_are_as_
    /// documented` makes about the synthetic climatology, asked of the real
    /// maps - and unlike that one, nothing here was constructed to satisfy them.
    #[test]
    fn ccir_reproduces_the_known_f2_behaviours() {
        // Afternoon exceeds pre-dawn at a midlatitude station. Lindau, Germany
        // (52 N, 10 E): local solar time is UT + 0.67 h.
        let (lat, lon) = (52.0, 10.0);
        let c = CcirMonth::bundled(3).unwrap();
        let day = c.plane(13.0, 80.0).fof2(lat, lon).unwrap();
        let night = c.plane(3.0, 80.0).fof2(lat, lon).unwrap();
        assert!(day > night, "equinox day {day:.2} <= night {night:.2}");

        // The equatorial anomaly: crests either side of the MAGNETIC equator in
        // the afternoon, standing above the trough between them. Taken along
        // 285 E (the American sector), where the magnetic equator sits well
        // north of the geographic one - which is exactly the structure a
        // geographic-latitude climatology cannot produce.
        let p = c.plane(19.0, 80.0); // ~14 LST at -75 E
        let dip_eq = (-40..=40)
            .map(f64::from)
            .min_by(|a, b| {
                modip_deg(*a, -75.0)
                    .unwrap()
                    .abs()
                    .total_cmp(&modip_deg(*b, -75.0).unwrap().abs())
            })
            .unwrap();
        let trough = p.fof2(dip_eq, -75.0).unwrap();
        let north_crest = p.fof2(dip_eq + 15.0, -75.0).unwrap();
        let south_crest = p.fof2(dip_eq - 15.0, -75.0).unwrap();
        assert!(
            north_crest > trough && south_crest > trough,
            "no anomaly crests about the magnetic equator at {dip_eq} N: \
             {south_crest:.2} / {trough:.2} / {north_crest:.2}"
        );

        // The winter anomaly: midlatitude winter DAY exceeds summer day.
        let jan = CcirMonth::bundled(1).unwrap().plane(12.0, 80.0);
        let jul = CcirMonth::bundled(7).unwrap().plane(12.0, 80.0);
        let (w, s) = (jan.fof2(50.0, 0.0).unwrap(), jul.fof2(50.0, 0.0).unwrap());
        assert!(w > s, "winter day {w:.2} <= summer day {s:.2}");
    }
}
