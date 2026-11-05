//! What the interface can be asked to do, in one enum.
//!
//! Menu items, overlay buttons and dialog buttons all produce an [`Action`]
//! rather than mutating state where they are drawn. That keeps a command's
//! meaning in one place - "Calculate" always means *dispatch a solve and show
//! the trace* whether it was invoked from the menu or from the dialog - and
//! leaves the widgets themselves free of side effects. [`crate::update`]
//! applies them.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// Run the point-to-point solve and show the trace readout.
    Calculate,
    /// Run the frequency sweep and show its charts.
    BestFrequency,
    /// Re-frame the map on the current path.
    FitPath,
    ZoomIn,
    ZoomOut,
    /// Adopt a frequency found by the sweep as the tuned frequency.
    TuneTo(f64),
    /// Start the area coverage grid.
    RunCoverage,
    /// Stop the running grid, keeping every tile computed so far.
    CancelCoverage,
    /// Clear the coverage tiles off the map.
    ResetCoverage,
    ShowCoverage,
    ShowSettings,
    ShowAntennas,
    ShowAbout,
}
