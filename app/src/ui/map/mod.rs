//! The map: the projection, the basemap raster, and the layers drawn over it.
//!
//! [`projection`] is the geometry - equirectangular, and the camera that frames
//! it. [`basemap`] turns the bundled coastline polygons into the one image the
//! world is drawn from. [`layers`] holds the pure drawing overlays: graticule,
//! day/night shading, coverage tiles, coastline outlines and the ray paths.
//! [`view::MapView`] owns the two things that must survive between frames, the
//! camera and that raster, and assembles a frame out of the rest.

pub mod basemap;
pub mod layers;
pub mod projection;
pub mod view;

pub use view::MapView;
