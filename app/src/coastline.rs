//! Coastline lookup: decides whether a point on the globe is sea, fresh water
//! or land, from the Natural Earth 1:50m `land` and `lakes` polygon datasets
//! shipped alongside this crate's source, under `app/src/assets/`.
//!
//! This is a *geographic* classification only. It answers "is this point on
//! land, in a lake, or at sea?" and nothing else - notably it says nothing
//! about soil moisture, because the datasets carry no such attribute. The
//! electrical constants themselves are never invented here: every answer is one
//! of the existing [`GroundType`] presets, so auto-detection can only pick
//! between values the operator could have picked by hand.
//!
//! The shapefiles are read once, on first use, with the `shapefile` crate (no
//! hand-rolled parsing). Only the `.shp` geometry stream is read - there is no
//! `.dbf`/`.shx` beside them and none is needed, since nothing but the rings
//! matters here.
//!
//! Resolution is the honest limit here: at 1:50m the coastline is generalised
//! to roughly a kilometre or two, so a point that close to the shore can land
//! on the wrong side of it (Plymouth and Lisbon, both sitting up generalised-
//! away estuaries, read as sea). That is immaterial for its actual use - a
//! ground reflection sits hundreds of kilometres from the previous one, and the
//! sea/land contrast it is resolving is enormous - but it means this is not a
//! shoreline authority and must not be used as one.
//!
//! Point-in-polygon is the standard even-odd ray crossing test, run against
//! *all* rings of a shape at once. That handles holes for free: a point inside
//! a lake-island hole crosses the outer ring and the inner ring, an even count,
//! and so reads as outside - which is what the data means.

use std::sync::OnceLock;

use crate::scenario::GroundType;

/// The two datasets, compiled into the binary rather than read from disk at
/// startup.
///
/// They used to be opened by path under `CARGO_MANIFEST_DIR`, which meant the
/// build machine's source tree had to still exist at the same path when the app
/// ran. Embedding removes that (and is what makes the data reachable at all in
/// the web build, where there is no filesystem to read). About 1.4 MB together -
/// the same bytes that were being read at startup anyway.
const LAND_FILE: &str = "ne_50m_land.shp";
const LAKES_FILE: &str = "ne_50m_lakes.shp";
const LAND_BYTES: &[u8] = include_bytes!("assets/ne_50m_land.shp");
const LAKES_BYTES: &[u8] = include_bytes!("assets/ne_50m_lakes.shp");

/// Target point count per ring for the map overlay. The classification always
/// uses the full-detail rings; only the drawing is thinned, since the overlay
/// exists to be eyeballed, not measured.
const OUTLINE_MAX_POINTS: usize = 120;

/// Rings of one polygon shape, with its bounding box for a cheap reject.
struct Poly {
    min_lon: f64,
    max_lon: f64,
    min_lat: f64,
    max_lat: f64,
    /// Every ring (outer and inner) as (lon, lat).
    rings: Vec<Vec<(f64, f64)>>,
}

impl Poly {
    fn contains(&self, lon: f64, lat: f64) -> bool {
        if lon < self.min_lon || lon > self.max_lon || lat < self.min_lat || lat > self.max_lat {
            return false;
        }
        let mut inside = false;
        for ring in &self.rings {
            if ray_crossings_odd(ring, lon, lat) {
                inside = !inside;
            }
        }
        inside
    }
}

/// Even-odd ray crossing test of one ring, casting east from the point.
fn ray_crossings_odd(ring: &[(f64, f64)], lon: f64, lat: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = ring[i];
        let (xj, yj) = ring[j];
        // Half-open latitude interval, so a vertex exactly at the ray latitude
        // is counted once rather than twice.
        if (yi > lat) != (yj > lat) {
            let t = (lat - yi) / (yj - yi);
            if lon < xi + t * (xj - xi) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// What auto-detection decided at one point, and the reason, so the per-hop
/// readout can be checked rather than trusted.
pub struct GroundPick {
    pub ground: GroundType,
    pub reason: String,
}

/// One thinned ring for the debug overlay, carrying its own geographic bounds
/// so the map plugin can drop everything off screen without projecting a single
/// point of it. At any useful zoom that is nearly all of them.
pub struct Outline {
    /// (min_lon, min_lat, max_lon, max_lat).
    pub bounds: (f64, f64, f64, f64),
    /// Ring vertices as (lon, lat).
    pub points: Vec<(f64, f64)>,
}

pub struct Coastline {
    land: Vec<Poly>,
    lakes: Vec<Poly>,
    /// Thinned rings for the debug overlay.
    land_outlines: Vec<Outline>,
    lake_outlines: Vec<Outline>,
}

impl Coastline {
    /// Classify one point. Lakes win over land (a lake polygon lies inside a
    /// land polygon), land falls back to the operator's chosen land type, and
    /// anything outside both is sea.
    #[must_use]
    pub fn classify(&self, lat: f64, lon: f64, land_fallback: GroundType) -> GroundPick {
        let lon = normalise_lon(lon);
        let at = format!("reflection point {}", format_lat_lon(lat, lon));
        if self.lakes.iter().any(|p| p.contains(lon, lat)) {
            return GroundPick {
                ground: GroundType::FreshWater,
                reason: format!("{at} falls inside a lakes polygon"),
            };
        }
        if self.land.iter().any(|p| p.contains(lon, lat)) {
            return GroundPick {
                ground: land_fallback,
                reason: format!(
                    "{at} falls inside a land polygon (soil type not in the data - using the \
                     selected land fallback)"
                ),
            };
        }
        GroundPick {
            ground: GroundType::SeaWater,
            reason: format!("{at} falls outside every land polygon"),
        }
    }

    /// Thinned land rings for the debug overlay.
    #[must_use]
    pub fn land_outlines(&self) -> &[Outline] {
        &self.land_outlines
    }

    /// Thinned lake rings for the debug overlay.
    #[must_use]
    pub fn lake_outlines(&self) -> &[Outline] {
        &self.lake_outlines
    }

    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} land polygons, {} lake polygons, from embedded {LAND_FILE} / {LAKES_FILE}",
            self.land.len(),
            self.lakes.len(),
        )
    }
}

/// Longitude folded into (-180, 180].
fn normalise_lon(lon: f64) -> f64 {
    ((lon + 180.0).rem_euclid(360.0)) - 180.0
}

/// `51.50N, 0.13W` - the form the per-hop reason strings quote.
#[must_use]
pub fn format_lat_lon(lat: f64, lon: f64) -> String {
    format!(
        "{:.2}{}, {:.2}{}",
        lat.abs(),
        if lat >= 0.0 { 'N' } else { 'S' },
        lon.abs(),
        if lon >= 0.0 { 'E' } else { 'W' },
    )
}

fn thin(ring: &[(f64, f64)]) -> Outline {
    let points = if ring.len() <= OUTLINE_MAX_POINTS {
        ring.to_vec()
    } else {
        let stride = ring.len().div_ceil(OUTLINE_MAX_POINTS);
        let mut out: Vec<(f64, f64)> = ring.iter().step_by(stride).copied().collect();
        // Close the ring back up, since the last point is usually dropped.
        if let Some(&first) = out.first()
            && out.last() != Some(&first)
        {
            out.push(first);
        }
        out
    };
    let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(lon, lat) in &points {
        bounds.0 = bounds.0.min(lon);
        bounds.1 = bounds.1.min(lat);
        bounds.2 = bounds.2.max(lon);
        bounds.3 = bounds.3.max(lat);
    }
    Outline { bounds, points }
}

fn read_polygons(name: &str, bytes: &'static [u8]) -> Result<Vec<Poly>, String> {
    let shapes = shapefile::ShapeReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("{name}: {e}"))?
        .read()
        .map_err(|e| format!("{name}: {e}"))?;

    let mut polys = Vec::new();
    for shape in shapes {
        let shapefile::Shape::Polygon(p) = shape else {
            continue;
        };
        let bbox = p.bbox();
        let rings: Vec<Vec<(f64, f64)>> = p
            .rings()
            .iter()
            .map(|r| r.points().iter().map(|pt| (pt.x, pt.y)).collect())
            .collect();
        if rings.is_empty() {
            continue;
        }
        polys.push(Poly {
            min_lon: bbox.min.x,
            max_lon: bbox.max.x,
            min_lat: bbox.min.y,
            max_lat: bbox.max.y,
            rings,
        });
    }
    if polys.is_empty() {
        return Err(format!("{name}: no polygon shapes"));
    }
    Ok(polys)
}

fn load() -> Result<Coastline, String> {
    let land = read_polygons(LAND_FILE, LAND_BYTES)?;
    let lakes = read_polygons(LAKES_FILE, LAKES_BYTES)?;
    let land_outlines = land
        .iter()
        .flat_map(|p| p.rings.iter().map(|r| thin(r)))
        .collect();
    let lake_outlines = lakes
        .iter()
        .flat_map(|p| p.rings.iter().map(|r| thin(r)))
        .collect();

    Ok(Coastline {
        land,
        lakes,
        land_outlines,
        lake_outlines,
    })
}

static COASTLINE: OnceLock<Result<Coastline, String>> = OnceLock::new();

/// The shared coastline data, loaded on first use. Both files together are a
/// few MB of rings, so this is read once per process and shared; a failure is
/// cached too, and surfaced as text rather than a panic, because a missing
/// dataset must degrade to "auto-detect unavailable", not take the app down.
pub fn get() -> Result<&'static Coastline, &'static str> {
    match COASTLINE.get_or_init(load) {
        Ok(c) => Ok(c),
        Err(e) => Err(e.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference points with an unambiguous, independently-known answer: a
    /// coastal city must read as land, deep ocean as sea water, and the middle
    /// of a Great Lake as fresh water. This is the check that the whole feature
    /// rests on - if these three move, the classification is wrong.
    #[test]
    fn known_reference_points_classify_correctly() {
        let c = get().expect("coastline data in the project root");
        println!("{}", c.summary());

        let cases = [
            (
                "San Francisco (coastal city)",
                37.77,
                -122.42,
                "medium ground",
            ),
            ("mid-Atlantic (30N, 40W)", 30.0, -40.0, "sea water"),
            ("Lake Michigan centre", 43.60, -87.00, "fresh water"),
        ];
        for (name, lat, lon, expect) in cases {
            let pick = c.classify(lat, lon, GroundType::MediumGround);
            println!("{name}: {} - {}", pick.ground.label(), pick.reason);
            assert_eq!(pick.ground.label(), expect, "{name}");
        }
    }

    /// A hole in a land polygon (a lake) is not land: the even-odd test must
    /// see two crossings there. Checked on the Caspian Sea, which Natural Earth
    /// carves out of the land layer, and on a lake island (Manitoulin), which
    /// must come back to land again.
    #[test]
    fn holes_and_islands_invert_correctly() {
        let c = get().expect("coastline data");
        let caspian = c.classify(41.0, 51.0, GroundType::MediumGround);
        println!("Caspian: {} - {}", caspian.ground.label(), caspian.reason);
        assert_ne!(caspian.ground.label(), "medium ground");

        let manitoulin = c.classify(45.75, -82.20, GroundType::MediumGround);
        println!(
            "Manitoulin Island: {} - {}",
            manitoulin.ground.label(),
            manitoulin.reason
        );
        assert_eq!(manitoulin.ground.label(), "medium ground");
    }

    /// The land fallback is the operator's choice, not a constant: auto-detect
    /// decides water vs. land only.
    #[test]
    fn land_fallback_is_the_selected_type() {
        let c = get().expect("coastline data");
        for fallback in [
            GroundType::DryGround,
            GroundType::WetGround,
            GroundType::MediumGround,
        ] {
            let pick = c.classify(39.74, -104.99, fallback);
            assert_eq!(pick.ground.label(), fallback.label(), "Denver over land");
        }
        // Sea is sea whatever the land fallback says.
        let pick = c.classify(30.0, -40.0, GroundType::DryGround);
        assert_eq!(pick.ground.label(), "sea water");
    }

    /// The overlay geometry must exist and be thinned, or the debug view has
    /// nothing to draw.
    #[test]
    fn outlines_are_present_and_thinned() {
        let c = get().expect("coastline data");
        assert!(!c.land_outlines().is_empty());
        assert!(!c.lake_outlines().is_empty());
        assert!(
            c.land_outlines()
                .iter()
                .all(|r| r.points.len() <= OUTLINE_MAX_POINTS + 1)
        );
        // The cached bounds must actually bound the ring, or culling would
        // silently drop rings that are on screen.
        for r in c.land_outlines() {
            for &(lon, lat) in &r.points {
                assert!(lon >= r.bounds.0 && lon <= r.bounds.2);
                assert!(lat >= r.bounds.1 && lat <= r.bounds.3);
            }
        }
    }
}
