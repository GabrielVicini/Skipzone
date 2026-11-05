//! The map: the projection, the basemap raster, and the layers drawn over it.
//!
//! [`projection`] is the geometry - equirectangular, and the camera that frames
//! it. [`basemap`] turns the bundled coastline polygons into the one image the
//! world is drawn from. [`layers`] holds the pure drawing overlays: graticule,
//! day/night shading, coverage tiles, coastline outlines and the ray paths.
//! [`view`] assembles a frame out of the rest, from the camera in the state
//! and the raster in the view scratch.

pub mod basemap;
pub mod layers;
pub mod projection;
pub mod view;
