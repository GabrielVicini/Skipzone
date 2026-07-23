//! The debug readouts, one file per panel. Deliberately dense and complete
//! rather than tidy: this is an instrument panel, not a product screen. Adding
//! a new panel means adding a sibling file here and wiring it into the shell,
//! not editing an existing one.

mod assumptions;
mod diagnostics;
mod inputs;
mod profile;
mod reference;
mod solution;
mod sweep_chart;

pub use assumptions::assumptions_panel;
pub use diagnostics::{errors_panel, near_miss_panel};
pub use inputs::inputs_panel;
pub use profile::profile_panel;
pub use reference::reference_panel;
pub use solution::{legend_panel, solution_panel};
pub use sweep_chart::{state_legend, sweep_chart, sweep_verdict_text};
