//! Validation and calibration of the Skipzone model against measurements.
//!
//! This crate retrieves WSPR spots, saves and reloads reproducible corpora,
//! fetches observed solar indices, and fits the model's unverified anchors with
//! a two-way fixed-effects calibration. The harness binaries in `src/bin/` drive
//! it, and they run the same prediction code as the GUI.
//!
//! It is the only crate with network access. It depends on the engine
//! (`skipzone`), `skipzone-iono` and `skipzone-predict`, and never on the app.

/// A saved, reproducible WSPR corpus for calibration. Harness-only.
pub mod corpus;
/// Two-way fixed-effects calibration against measured WSPR spots. Harness-only.
pub mod fit;
/// Outbound HTTP, used only by the validation harnesses.
pub mod net;
/// Observed solar indices, fetched rather than assumed.
pub mod spaceweather;
pub mod wspr;
pub mod wspr_report;
/// Live WSPR spot retrieval.
pub mod wsprlive;
