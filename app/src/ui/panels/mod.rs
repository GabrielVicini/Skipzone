//! The trace readouts, one file per panel. Dense and complete, like an
//! instrument panel.
//!
//! Each panel is a free function over the result types that renders one
//! collapsing section, so they can be composed in any order; all are currently
//! in the trace panel. A new readout is a sibling file plus one call.

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
