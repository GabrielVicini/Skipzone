//! The map: the tile/plugin resources and the drawing overlays that sit on it.
//!
//! [`view::MapView`] owns the things that must survive between frames (the tile
//! cache and the pan/zoom memory); [`plugins`] holds the pure drawing overlays
//! for the ray paths and the day/night terminator.

pub mod plugins;
pub mod view;

pub use view::MapView;
