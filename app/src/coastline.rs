//! Coastline lookup: decides whether a point on the globe is sea, fresh water
//! or land, from the Natural Earth 1:50m `land` and `lakes` polygon datasets
//! shipped alongside this crate's source, under `app/src/assets/`.
//!
//! This is a geographic classification only; the datasets say nothing about
//! soil moisture. Every answer is one of the existing [`GroundType`] presets,
//! so auto-detection only picks values the operator could pick by hand.
//!
//! The shapefiles are read once, on first use, with the `shapefile` crate.
//! Only the `.shp` geometry stream is needed.
//!
//! At 1:50m the coastline is generalised to a kilometre or two, so a point that
//! close to the shore can land on the wrong side (Plymouth and Lisbon, on
//! generalised-away estuaries, read as sea). That does not matter for ground
//! reflections hundreds of kilometres apart, but this is not a shoreline
//! authority.
//!
//! Point-in-polygon is the even-odd ray crossing test over all rings of a shape
//! at once, which handles holes: a point inside a lake-island hole crosses two
//! rings and reads as outside.

use std::sync::OnceLock;

use crate::scenario::GroundType;

/// The two datasets, compiled into the binary rather than read from disk at
/// startup.
///
/// Embedding means the app does not depend on the build machine's source tree
/// and works in the web build, which has no filesystem. About 1.4 MB together.
const LAND_FILE: &str = "ne_50m_land.shp";
const LAKES_FILE: &str = "ne_50m_lakes.shp";
const LAND_BYTES: &[u8] = include_bytes!("assets/ne_50m_land.shp");
const LAKES_BYTES: &[u8] = include_bytes!("assets/ne_50m_lakes.shp");

/// How far a ring is allowed to move when it is thinned for drawing, degrees.
///
/// The classification always uses the full-detail rings; only the drawing is
/// thinned. A vertex is kept if it is at least this far from the last one kept,
/// so the thinned ring stays within about this distance of the real one.
/// Thinning by vertex index instead would keep dense fjord coasts and cut
/// across sparse smooth ones.
///
/// 0.12 degrees is roughly 13 km - below one texel of the basemap raster, which
/// is filled from these same rings. Under that the picture cannot tell the
/// difference, and above it the shoreline would start visibly cutting corners.
const OUTLINE_TOLERANCE_DEG: f64 = 0.12;

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

/// One thinned ring for drawing, carrying its own geographic bounds so the map
/// can drop everything off screen without projecting a single point of it. At
/// any useful zoom that is nearly all of them.
pub struct Outline {
    /// (min_lon, min_lat, max_lon, max_lat).
    pub bounds: (f64, f64, f64, f64),
    /// Ring vertices as (lon, lat).
    pub points: Vec<(f64, f64)>,
}

/// One shape's rings, thinned. Grouped rather than flattened because the
/// even-odd rule that carves lakes out of continents only works over a shape's
/// rings *together*.
pub type OutlineShape = Vec<Outline>;

pub struct Coastline {
    land: Vec<Poly>,
    lakes: Vec<Poly>,
    /// Thinned rings for drawing, grouped by shape.
    land_outlines: Vec<OutlineShape>,
    lake_outlines: Vec<OutlineShape>,
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

    /// Land shapes for drawing: thinned rings, grouped by shape.
    ///
    /// The basemap raster is filled and the shoreline stroked from these same
    /// rings, so the coastline follows the land underneath it.
    #[must_use]
    pub fn land_outlines(&self) -> &[OutlineShape] {
        &self.land_outlines
    }

    /// Lake shapes for drawing, as [`Self::land_outlines`].
    #[must_use]
    pub fn lake_outlines(&self) -> &[OutlineShape] {
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

/// Thin one ring by radial distance: keep the first vertex, then every vertex
/// at least [`OUTLINE_TOLERANCE_DEG`] from the last one kept, then close it.
fn thin(ring: &[(f64, f64)]) -> Outline {
    let tol2 = OUTLINE_TOLERANCE_DEG * OUTLINE_TOLERANCE_DEG;
    let mut points: Vec<(f64, f64)> = Vec::new();
    for &(lon, lat) in ring {
        let keep = points.last().is_none_or(|&(plon, plat)| {
            let (dx, dy) = (lon - plon, lat - plat);
            dx * dx + dy * dy >= tol2
        });
        if keep {
            points.push((lon, lat));
        }
    }
    // A shape smaller than the tolerance still has to exist: collapsing an
    // island to a single point would delete it from the map, and the classifier
    // would still call it land. Fall back to the ring as it came.
    if points.len() < 3 {
        points = ring.to_vec();
    }
    // Close the ring back up, since the last vertex is usually dropped.
    if let Some(&first) = points.first()
        && points.last() != Some(&first)
    {
        points.push(first);
    }

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
        .map(|p| p.rings.iter().map(|r| thin(r)).collect())
        .collect();
    let lake_outlines = lakes
        .iter()
        .map(|p| p.rings.iter().map(|r| thin(r)).collect())
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

    /// The drawing geometry must exist, be thinned, and still bound itself.
    ///
    /// One thinned ring per full-detail ring, one group per shape: the basemap
    /// raster relies on the grouping to apply the even-odd rule over a shape,
    /// and on the bounds to cull without projecting.
    #[test]
    fn outlines_are_present_and_thinned() {
        let c = get().expect("coastline data");
        assert!(!c.land_outlines().is_empty());
        assert!(!c.lake_outlines().is_empty());
        assert_eq!(c.land_outlines().len(), c.land.len());
        assert_eq!(c.lake_outlines().len(), c.lakes.len());

        let (mut full, mut thinned) = (0usize, 0usize);
        for (poly, shape) in c.land.iter().zip(c.land_outlines()) {
            assert_eq!(poly.rings.len(), shape.len(), "one outline per ring");
            for (ring, outline) in poly.rings.iter().zip(shape) {
                full += ring.len();
                thinned += outline.points.len();
                // The cached bounds must bound the ring, or culling would drop
                // rings that are on screen.
                for &(lon, lat) in &outline.points {
                    assert!(lon >= outline.bounds.0 && lon <= outline.bounds.2);
                    assert!(lat >= outline.bounds.1 && lat <= outline.bounds.3);
                }
            }
        }
        assert!(thinned < full, "thinning removed nothing: {thinned}/{full}");
    }

    /// Thinning may move the shoreline, but only by about the tolerance it
    /// promises. This is what lets the same rings be used for the raster fill
    /// and the stroke over it: at that tolerance the two agree to within a
    /// texel, so the coastline follows the land it is drawn on.
    #[test]
    fn thinning_stays_within_its_tolerance() {
        let c = get().expect("coastline data");
        for (poly, shape) in c.land.iter().zip(c.land_outlines()).take(200) {
            for (ring, outline) in poly.rings.iter().zip(shape) {
                if outline.points.len() == ring.len() {
                    continue; // untouched
                }
                // Every dropped vertex must be within tolerance of a kept one.
                for &(lon, lat) in ring {
                    let near = outline.points.iter().any(|&(olon, olat)| {
                        let (dx, dy) = (lon - olon, lat - olat);
                        dx * dx + dy * dy <= 4.0 * OUTLINE_TOLERANCE_DEG * OUTLINE_TOLERANCE_DEG
                    });
                    assert!(near, "vertex ({lon}, {lat}) has no kept neighbour");
                }
            }
        }
    }
}
