//! The map projection and the camera that frames it.
//!
//! # Why equirectangular, and nothing else
//!
//! The projection is plain equirectangular (Plate Carrée): `x` is linear in
//! longitude, `y` is linear in latitude, at the same number of pixels per degree
//! on both axes. It is chosen for one reason - it is the only projection in
//! which the map means exactly what the numbers in the rest of the interface
//! say.
//!
//! * A coverage cell is a lat/lon box half a step either side of a solved grid
//!   point. Here it draws as a rectangle of constant size anywhere on the map,
//!   which is what it is. Under Web Mercator the same box grew without bound
//!   towards the poles and the polar rows had to be thrown away entirely.
//! * The graticule is a square lattice, so "how far north is that hop" is read
//!   off the map by eye instead of estimated through a nonlinear vertical scale.
//! * The whole world exists. Mercator has no poles: it has to be cut, and the
//!   cut is arbitrary. Latitude runs to +/-90 here and every degree of it is
//!   drawn.
//!
//! What it costs is the thing every world projection costs somewhere: east-west
//! distances are stretched by `1/cos(lat)`, so a great-circle path is a curve,
//! not a straight line. That is honest and legible - a high-latitude path bows
//! visibly north because it really does go north - and it is the same curve
//! every HF area-coverage program has drawn for decades. It is emphatically not
//! the same thing as the *geometry itself* deforming, which is what Mercator
//! does to the coverage tiles.
//!
//! # The camera
//!
//! [`Camera`] is the whole view state: a centre in degrees and a scale in screen
//! pixels per degree. [`Camera::clamped`] is the one place the view is
//! constrained, and it enforces two rules:
//!
//! * You can never zoom out past the whole world. The minimum scale puts 360
//!   degrees of longitude exactly across the viewport, so there is never a
//!   second copy of the world on screen and never a void either side of it.
//! * You can never pan off the top or bottom. Latitude is clamped so the poles
//!   stay at or outside the edges whenever the world is taller than the
//!   viewport, and pinned to the equator when it is not.
//!
//! Longitude is deliberately *not* clamped: it wraps, so a Pacific path can be
//! centred without the map hitting a wall at the antimeridian.

use egui::{Pos2, Rect, Vec2, pos2};

/// Hard limit on how far in the map can be zoomed, screen pixels per degree.
/// A degree is roughly 111 km, so this is about 2 km per pixel - far finer than
/// the 1:50m basemap or anything the ray tracer resolves, and the point past
/// which zooming in only magnifies the generalisation of the coastline.
const MAX_PIXELS_PER_DEGREE: f64 = 60.0;

/// Longitude folded into [-180, 180).
#[must_use]
pub fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Where the map is centred and how big it is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub lat: f64,
    pub lon: f64,
    /// Screen pixels per degree, the same on both axes.
    pub ppd: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            lat: 0.0,
            lon: 0.0,
            ppd: 2.0,
        }
    }
}

impl Camera {
    /// The scale at which the full 360 degrees of longitude exactly spans
    /// `rect`. This is the minimum: zooming out further would either repeat the
    /// world or leave dead space beside it.
    #[must_use]
    pub fn min_ppd(rect: Rect) -> f64 {
        f64::from(rect.width().max(1.0)) / 360.0
    }

    /// The camera with its scale and latitude brought inside the legal range for
    /// `rect`. Every draw goes through this, so no input path can leave the view
    /// somewhere the projection does not cover.
    #[must_use]
    pub fn clamped(mut self, rect: Rect) -> Self {
        let min_ppd = Self::min_ppd(rect);
        self.ppd = self.ppd.clamp(min_ppd, MAX_PIXELS_PER_DEGREE.max(min_ppd));

        // Half the viewport height in degrees. When the world is shorter than
        // the viewport there is nothing to pan to, so the equator is pinned to
        // the middle and the map is letterboxed.
        let half_h_deg = f64::from(rect.height().max(1.0)) / (2.0 * self.ppd);
        let slack = 90.0 - half_h_deg;
        self.lat = if slack <= 0.0 {
            0.0
        } else {
            self.lat.clamp(-slack, slack)
        };
        self.lon = wrap_lon(self.lon);
        self
    }

    /// The slippy-map-style zoom level this scale corresponds to, for the
    /// readout in the corner. Kept in the familiar `z0 = whole world in 256 px`
    /// units so the number means the same thing it did when the map was tiled.
    #[must_use]
    pub fn zoom_level(self) -> f64 {
        (self.ppd * 360.0 / 256.0).log2()
    }
}

/// A camera resolved against a concrete screen rectangle: converts degrees to
/// pixels and back.
#[derive(Clone, Copy)]
pub struct Proj {
    rect: Rect,
    cam: Camera,
}

impl Proj {
    #[must_use]
    pub fn new(rect: Rect, cam: Camera) -> Self {
        Self { rect, cam }
    }

    #[must_use]
    pub fn rect(&self) -> Rect {
        self.rect
    }

    #[must_use]
    pub fn camera(&self) -> Camera {
        self.cam
    }

    #[must_use]
    pub fn pixels_per_degree(&self) -> f64 {
        self.cam.ppd
    }

    /// Project a position. Longitude takes whichever copy of itself is nearest
    /// the centre of the view, which is what makes panning across the
    /// antimeridian seamless: there is no seam, only a choice of representative.
    #[must_use]
    pub fn project(&self, lat: f64, lon: f64) -> Pos2 {
        self.project_unwrapped(lat, self.cam.lon + wrap_lon(lon - self.cam.lon))
    }

    /// Project a position whose longitude has already been unwrapped onto a
    /// continuous scale (it may lie outside [-180, 180)). Polylines use this so
    /// that a path running east past the antimeridian keeps going east instead
    /// of jumping back across the map.
    #[must_use]
    pub fn project_unwrapped(&self, lat: f64, lon: f64) -> Pos2 {
        let c = self.rect.center();
        #[allow(clippy::cast_possible_truncation)]
        let x = f64::from(c.x) + (lon - self.cam.lon) * self.cam.ppd;
        #[allow(clippy::cast_possible_truncation)]
        let y = f64::from(c.y) - (lat - self.cam.lat) * self.cam.ppd;
        pos2(x as f32, y as f32)
    }

    /// Screen position back to (lat, lon). The latitude can come back outside
    /// +/-90 when the pointer is in the void above or below the world; callers
    /// that care must check.
    #[must_use]
    pub fn unproject(&self, pos: Pos2) -> (f64, f64) {
        let c = self.rect.center();
        let lat = self.cam.lat - f64::from(pos.y - c.y) / self.cam.ppd;
        let lon = wrap_lon(self.cam.lon + f64::from(pos.x - c.x) / self.cam.ppd);
        (lat, lon)
    }

    /// The whole world as a screen rectangle: everything outside it is void, not
    /// map, and nothing may be drawn there.
    #[must_use]
    pub fn world_rect(&self) -> Rect {
        Rect::from_two_pos(
            self.project_unwrapped(90.0, self.cam.lon - 180.0),
            self.project_unwrapped(-90.0, self.cam.lon + 180.0),
        )
    }

    /// The part of the world that is actually on screen, as
    /// `(south, north, west, east)` in degrees. West may exceed east numerically
    /// (it is an unwrapped span, not a wrapped pair) so that callers can walk it
    /// with a plain loop.
    #[must_use]
    pub fn visible_bounds(&self) -> (f64, f64, f64, f64) {
        let half_w = f64::from(self.rect.width()) / (2.0 * self.cam.ppd);
        let half_h = f64::from(self.rect.height()) / (2.0 * self.cam.ppd);
        (
            (self.cam.lat - half_h).max(-90.0),
            (self.cam.lat + half_h).min(90.0),
            self.cam.lon - half_w,
            self.cam.lon + half_w,
        )
    }

    /// Move the camera by a screen-space drag.
    #[must_use]
    pub fn panned(&self, delta: Vec2) -> Camera {
        let mut cam = self.cam;
        cam.lon = wrap_lon(cam.lon - f64::from(delta.x) / cam.ppd);
        cam.lat += f64::from(delta.y) / cam.ppd;
        cam
    }

    /// Scale about a fixed screen point, so the geography under the pointer
    /// stays under the pointer.
    #[must_use]
    pub fn zoomed_about(&self, factor: f64, anchor: Pos2) -> Camera {
        let (lat, lon) = self.unproject(anchor);
        let mut cam = self.cam;
        cam.ppd *= factor;
        // Solve for the centre that keeps (lat, lon) at `anchor`.
        let c = self.rect.center();
        cam.lat = lat + f64::from(anchor.y - c.y) / cam.ppd;
        cam.lon = wrap_lon(lon - f64::from(anchor.x - c.x) / cam.ppd);
        cam
    }
}

impl Proj {
    /// A feature's longitude extent `[min_lon, max_lon]` put on the same
    /// continuous scale as the camera, as `(west, east)`.
    ///
    /// The extent's *midpoint* moves to whichever copy of itself is nearest the
    /// camera and the two edges keep their real distance from it. Unwrapping the
    /// two edges independently is the trap: a feature spanning the full world
    /// has `wrap_lon(max - min) == wrap_lon(360) == 0`, so its span collapses to
    /// nothing and it culls itself off the map. Natural Earth holds Africa,
    /// Europe and Asia as one ring joined at Suez, which is exactly such a
    /// feature - and it duly disappeared.
    ///
    /// Anchoring on the midpoint rather than an edge keeps the result centred on
    /// the view, so the untranslated copy of the world is the one that matters
    /// and the +/-360 copies stay the rare case they are meant to be.
    #[must_use]
    pub fn span_near_camera(&self, min_lon: f64, max_lon: f64) -> (f64, f64) {
        let half = 0.5 * (max_lon - min_lon);
        let mid = self.cam.lon + wrap_lon(f64::midpoint(min_lon, max_lon) - self.cam.lon);
        (mid - half, mid + half)
    }
}

/// Unwrap a sequence of longitudes onto a continuous scale, so that consecutive
/// points never differ by more than 180 degrees. A ground track that crosses the
/// antimeridian comes out as a single monotone run instead of a jump, which is
/// what lets it be drawn as one unbroken polyline.
pub fn unwrap_run(lons: impl IntoIterator<Item = f64>, out: &mut Vec<f64>) {
    out.clear();
    let mut prev: Option<f64> = None;
    for lon in lons {
        let lon = match prev {
            None => wrap_lon(lon),
            Some(p) => p + wrap_lon(lon - p),
        };
        out.push(lon);
        prev = Some(lon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1200.0, 800.0))
    }

    /// Screen positions are `f32`, so a round trip is only ever exact to the
    /// precision of a pixel coordinate. The tolerance is that limit and no
    /// looser: a thousandth of a degree is about a hundred metres, far below
    /// anything the map is read to, and far above `f32` rounding at a
    /// four-figure pixel coordinate.
    #[test]
    fn project_and_unproject_round_trip() {
        let cam = Camera {
            lat: 20.0,
            lon: -70.0,
            ppd: 3.0,
        }
        .clamped(rect());
        let p = Proj::new(rect(), cam);
        for &(lat, lon) in &[(0.0, 0.0), (51.5, -0.13), (-33.9, 151.2), (80.0, 179.0)] {
            let (rlat, rlon) = p.unproject(p.project(lat, lon));
            assert!((rlat - lat).abs() < 1e-3, "lat {lat} -> {rlat}");
            assert!(wrap_lon(rlon - lon).abs() < 1e-3, "lon {lon} -> {rlon}");
        }
    }

    /// The projection must be linear in both axes: equal steps in degrees are
    /// equal steps in pixels anywhere on the map. This is the whole reason the
    /// projection was changed, so it is pinned.
    #[test]
    fn scale_is_constant_everywhere() {
        let cam = Camera {
            lat: 0.0,
            lon: 0.0,
            ppd: 4.0,
        }
        .clamped(rect());
        let p = Proj::new(rect(), cam);
        for &lat in &[-85.0, -45.0, 0.0, 45.0, 85.0] {
            let a = p.project(lat, 0.0);
            let b = p.project(lat, 1.0);
            let c = p.project(lat + 1.0, 0.0);
            assert!(
                (f64::from(b.x - a.x) - 4.0).abs() < 1e-4,
                "lon scale at {lat}"
            );
            assert!(
                (f64::from(a.y - c.y) - 4.0).abs() < 1e-4,
                "lat scale at {lat}"
            );
        }
    }

    /// Zooming all the way out shows the whole world exactly once: no repeat, no
    /// void beside it. This is the reported "gaping hole" made impossible.
    #[test]
    fn cannot_zoom_out_past_the_whole_world() {
        let r = rect();
        let cam = Camera {
            lat: 0.0,
            lon: 0.0,
            ppd: 1e-6,
        }
        .clamped(r);
        let p = Proj::new(r, cam);
        let world = p.world_rect();
        assert!((world.width() - r.width()).abs() < 0.5, "{world:?}");
        assert!(world.height() <= r.height() + 0.5);
    }

    /// Panning north can never reveal anything above the pole.
    #[test]
    fn latitude_is_clamped_to_the_poles() {
        let r = rect();
        let cam = Camera {
            lat: 89.0,
            lon: 0.0,
            ppd: 20.0,
        }
        .clamped(r);
        let p = Proj::new(r, cam);
        let world = p.world_rect();
        assert!(world.top() <= r.top() + 0.5, "world top {}", world.top());
    }

    /// A feature's longitude span must survive being put on the camera's scale,
    /// however wide it is. The world-spanning case is the one that broke: it
    /// collapsed to zero width, so the ring culled itself and the whole of
    /// Afro-Eurasia lost its coastline as soon as the map was zoomed in.
    #[test]
    fn a_world_spanning_extent_keeps_its_width() {
        let r = rect();
        for cam_lon in [-179.0, -90.0, 0.0, 37.0, 179.0] {
            let p = Proj::new(
                r,
                Camera {
                    lat: 0.0,
                    lon: cam_lon,
                    ppd: 20.0,
                }
                .clamped(r),
            );
            let (west, east) = p.span_near_camera(-180.0, 180.0);
            assert!(
                (east - west - 360.0).abs() < 1e-9,
                "cam {cam_lon}: {west}..{east}"
            );
            // And the span must straddle the camera, or it would sit off screen.
            assert!(west <= p.camera().lon && p.camera().lon <= east);

            // An ordinary extent keeps its width too, and lands near the camera.
            let (w2, e2) = p.span_near_camera(-17.6, 51.4);
            assert!((e2 - w2 - 69.0).abs() < 1e-9);
            assert!(wrap_lon(w2 - (-17.6)).abs() < 1e-9);
        }
    }

    /// A ground track crossing the antimeridian must come out as one run, not as
    /// a jump back across the map.
    #[test]
    fn runs_unwrap_across_the_antimeridian() {
        let mut out = Vec::new();
        unwrap_run([170.0, 175.0, -179.0, -174.0], &mut out);
        for w in out.windows(2) {
            assert!((w[1] - w[0]).abs() < 180.0, "{out:?}");
        }
        assert!(out[3] > out[0], "{out:?}");
    }

    /// Zooming about a point keeps the geography under that point.
    #[test]
    fn zoom_about_a_point_holds_it_still() {
        let r = rect();
        let cam = Camera {
            lat: 10.0,
            lon: 30.0,
            ppd: 3.0,
        }
        .clamped(r);
        let p = Proj::new(r, cam);
        let anchor = pos2(900.0, 200.0);
        let before = p.unproject(anchor);
        let p2 = Proj::new(r, p.zoomed_about(2.0, anchor).clamped(r));
        let after = p2.unproject(anchor);
        assert!((after.0 - before.0).abs() < 1e-6);
        assert!(wrap_lon(after.1 - before.1).abs() < 1e-6);
    }
}
