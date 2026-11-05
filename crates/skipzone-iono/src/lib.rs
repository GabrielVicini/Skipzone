//! The ionosphere model for the Skipzone engine.
//!
//! This crate holds Chapman layers, the CCIR foF2 and M(3000)F2 maps, the
//! sporadic-E climatology, solar geometry, and the time and Maidenhead grid
//! conversions they need. It depends only on the `skipzone` engine crate, does
//! no I/O and starts no threads. Its main entry type is
//! [`chapman::SolarChapmanLayer`], which implements the engine's
//! `ElectronDensity` trait, so the engine tracer samples it like any other
//! profile. Peak density comes from a [`chapman::PeakDensitySource`], such as
//! the CCIR maps or the gridded foF2 climatology.

/// The CCIR foF2 / M(3000)F2 numerical maps, evaluated from the vendored
/// coefficients.
pub mod ccir;
pub mod chapman;
pub mod clock;
pub mod fof2;
pub mod grid;
pub mod solar;
pub mod sporadic_e;
