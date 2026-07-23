//! The trace readouts, one file per panel. Deliberately dense and complete
//! rather than tidy: this is an instrument panel, not a product screen.
//!
//! Each panel is a free function over the result types and renders a single
//! collapsing section, so they can be composed in any order - today they all
//! live in the Calculate dialog. Adding a readout means adding a sibling file
//! here and one call where it belongs, not editing an existing panel.

mod assumptions;
mod diagnostics;
mod profile;
mod reference;
mod solution;
mod verdict;

pub use assumptions::assumptions_panel;
pub use diagnostics::{errors_panel, near_miss_panel};
pub use profile::profile_panel;
pub use reference::reference_panel;
pub use solution::{legend_panel, solution_panel};
pub use verdict::verdict_chip;
