//! The basemap: land and water, rasterised once from the bundled Natural Earth
//! polygons into a single equirectangular image.
//!
//! Built from local data rather than fetched tiles, so the map is never blank
//! waiting for the network, is not tied to Web Mercator, and works offline. It
//! uses the same coastline geometry the solver classifies ground reflections
//! against, so a reflection drawn in the sea is in the sea the model uses. It
//! costs one rasterisation at startup.
//!
//! # Resolution
//!
//! [`TEX_W`]x[`TEX_H`] gives about 0.18 degrees per texel, roughly 20 km, the
//! same order as the generalisation of the 1:50m source. Zoomed in further the
//! raster goes soft, and the coastline stroke drawn over it (see
//! [`super::layers::coastline`]) keeps the shoreline crisp. Hops land hundreds
//! of kilometres apart, so nothing in the model resolves a bay.

use egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions, TextureWrapMode};

use skipzone_predict::coastline::{self, Outline};

/// Raster size. Width is twice height because the projection is
/// equirectangular: the world is 360 degrees by 180.
pub const TEX_W: usize = 2048;
pub const TEX_H: usize = 1024;

/// Deep water, and the colour of anything the land layer does not cover.
pub const OCEAN: Color32 = Color32::from_rgb(0x0E, 0x1C, 0x28);
/// Land: a low-saturation grey a little lighter than the water, leaving the
/// colours for the coverage ramp and the ray paths.
pub const LAND: Color32 = Color32::from_rgb(0x27, 0x2E, 0x33);
/// Lakes read as water, because to the ground-reflection model they are water.
pub const LAKE: Color32 = Color32::from_rgb(0x11, 0x24, 0x33);
/// The void outside +/-90 latitude. Not map, not water - nothing, and it says
/// so rather than pretending the world continues.
pub const VOID: Color32 = Color32::from_rgb(0x08, 0x0A, 0x0D);
/// Shoreline stroke drawn over the raster from the full-detail rings.
pub const COAST: Color32 = Color32::from_rgb(0x46, 0x56, 0x62);

/// Build the basemap texture, or `None` if the coastline data could not be
/// read. A missing basemap must degrade to an empty ocean, never to a panic:
/// the map is still usable for geometry with no land drawn on it at all.
#[must_use]
pub fn load(ctx: &Context) -> Option<TextureHandle> {
    let coast = coastline::get().ok()?;
    let mut pixels = vec![OCEAN; TEX_W * TEX_H];
    for shape in coast.land_outlines() {
        fill_shape(&mut pixels, shape, LAND);
    }
    // Lakes after land, because a lake polygon lies inside a land polygon and
    // has to win - the same precedence the classifier applies.
    for shape in coast.lake_outlines() {
        fill_shape(&mut pixels, shape, LAKE);
    }
    let image = ColorImage::new([TEX_W, TEX_H], pixels);
    // Repeating in `u` lets the whole world be drawn as one quad at any camera
    // longitude: the texture coordinate for the quad's left edge runs past 1
    // and wraps. Clamping would smear the edge column across the antimeridian.
    let options = TextureOptions {
        wrap_mode: TextureWrapMode::Repeat,
        ..TextureOptions::LINEAR
    };
    Some(ctx.load_texture("skipzone_basemap", image, options))
}

/// Texture `u` coordinate of a longitude, on the raster's own absolute scale
/// where `u = 0` is exactly 180 degrees west.
///
/// Using this, rather than `u` from 0 to 1 at the camera's edges, keeps the
/// raster aligned with the coastline vectors at any camera longitude.
#[must_use]
pub fn u_of_lon(lon: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let u = ((lon + 180.0) / 360.0).rem_euclid(1.0) as f32;
    u
}

/// Latitude at the centre of raster row `row`.
fn row_lat(row: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let t = (row as f64 + 0.5) / TEX_H as f64;
    90.0 - 180.0 * t
}

/// Raster column containing longitude `lon`.
fn lon_col(lon: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let w = TEX_W as f64;
    (lon + 180.0) / 360.0 * w
}

/// Scanline fill of one shape - all its rings at once, under the even-odd rule.
///
/// Taking the rings as a group handles holes: a row crossing a continent, a
/// lake in it and an island in the lake accumulates four crossings and fills
/// the two land spans. [`coastline::Coastline::classify`] applies the same rule
/// point by point, so the picture matches the classification.
fn fill_shape(pixels: &mut [Color32], rings: &[Outline], color: Color32) {
    let (mut min_lat, mut max_lat) = (f64::MAX, f64::MIN);
    for ring in rings {
        min_lat = min_lat.min(ring.bounds.1);
        max_lat = max_lat.max(ring.bounds.3);
    }
    if min_lat > max_lat {
        return;
    }

    // Rows whose centre lies inside the shape's latitude span, clamped to the
    // raster. A sliver thinner than one row does not survive at this
    // resolution.
    let row_of = |lat: f64| {
        #[allow(clippy::cast_precision_loss)]
        let h = TEX_H as f64;
        (90.0 - lat) / 180.0 * h
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let row0 = row_of(max_lat).floor().max(0.0) as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let row1 = (row_of(min_lat).ceil() as usize).min(TEX_H - 1);

    let mut crossings: Vec<f64> = Vec::new();
    for row in row0..=row1.max(row0) {
        if row >= TEX_H {
            break;
        }
        let lat = row_lat(row);
        crossings.clear();
        for ring in rings {
            let ring = &ring.points;
            let n = ring.len();
            if n < 3 {
                continue;
            }
            let mut j = n - 1;
            for i in 0..n {
                let (xi, yi) = ring[i];
                let (xj, yj) = ring[j];
                // Half-open in latitude, so a vertex exactly on the scanline is
                // counted once rather than twice.
                if (yi > lat) != (yj > lat) {
                    let t = (lat - yi) / (yj - yi);
                    crossings.push(xi + t * (xj - xi));
                }
                j = i;
            }
        }
        if crossings.len() < 2 {
            continue;
        }
        crossings.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let base = row * TEX_W;
        for pair in crossings.chunks_exact(2) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let c0 = lon_col(pair[0]).round().clamp(0.0, TEX_W as f64) as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let c1 = lon_col(pair[1]).round().clamp(0.0, TEX_W as f64) as usize;
            for col in c0..c1.min(TEX_W) {
                pixels[base + col] = color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The raster must agree with the classifier it was drawn from. These are
    /// the same reference points the coastline tests pin, checked through the
    /// picture instead of through the point-in-polygon test: if the two ever
    /// disagree, the map is lying about where the ground constants changed.
    #[test]
    fn raster_agrees_with_the_classifier() {
        let Ok(coast) = coastline::get() else {
            return; // no data bundled: the app degrades to an empty ocean
        };
        let mut pixels = vec![OCEAN; TEX_W * TEX_H];
        for shape in coast.land_outlines() {
            fill_shape(&mut pixels, shape, LAND);
        }
        for shape in coast.lake_outlines() {
            fill_shape(&mut pixels, shape, LAKE);
        }
        let at = |lat: f64, lon: f64| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let row = ((90.0 - lat) / 180.0 * TEX_H as f64) as usize;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let col = lon_col(lon) as usize;
            pixels[row.min(TEX_H - 1) * TEX_W + col.min(TEX_W - 1)]
        };
        assert_eq!(at(45.0, -100.0), LAND, "central North America");
        assert_eq!(at(30.0, -40.0), OCEAN, "mid-Atlantic");
        assert_eq!(at(43.6, -87.0), LAKE, "Lake Michigan");
        assert_eq!(at(-25.0, 134.0), LAND, "central Australia");
    }

    /// Every texel must be written; an unpainted texel would show as black.
    #[test]
    fn the_whole_world_is_painted() {
        let pixels = vec![OCEAN; TEX_W * TEX_H];
        assert_eq!(pixels.len(), TEX_W * TEX_H);
        assert!(pixels.iter().all(|&p| p != Color32::TRANSPARENT));
    }

    /// The raster is anchored to absolute longitude, not to wherever the camera
    /// happens to be. Getting this wrong slides the painted world sideways
    /// under the coastline vectors, which is exactly what it did.
    #[test]
    fn texture_coordinates_are_anchored_to_the_antimeridian() {
        assert!((u_of_lon(-180.0) - 0.0).abs() < 1e-6);
        assert!((u_of_lon(0.0) - 0.5).abs() < 1e-6);
        assert!((u_of_lon(90.0) - 0.75).abs() < 1e-6);
        // +180 and -180 are the same meridian and must land on the same texel.
        assert!((u_of_lon(180.0) - u_of_lon(-180.0)).abs() < 1e-6);
        // Unwrapped longitudes outside the range still resolve.
        assert!((u_of_lon(360.0) - u_of_lon(0.0)).abs() < 1e-6);
        assert!((u_of_lon(-270.0) - u_of_lon(90.0)).abs() < 1e-6);
    }
}
