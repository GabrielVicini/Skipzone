//! Everything egui: the theme palette, shared widgets, the map plugins, and
//! the debug panels. Grouping the rendering here keeps the domain modules
//! (scenario, solve, sweep) free of any UI dependency, and leaves room for the
//! planned pop-out windows and interactive graphs to land as new submodules.

pub mod map;
pub mod panels;
pub mod theme;
pub mod widgets;
