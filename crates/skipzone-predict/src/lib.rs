//! Headless HF link prediction on the Skipzone engine.
//!
//! This crate builds a scenario from station inputs, solves it into hop and
//! mode reports from the engine's ray traces, and scores each path with a link
//! budget: noise, antenna patterns, ground reflection over the land mask, a
//! frequency sweep, area coverage and the calibration anchors those use.
//!
//! It depends only on the engine (`skipzone`), the ionosphere model
//! (`skipzone-iono`) and rayon for data-parallel batches; no GUI, no network.
//!
//! The main path is [`scenario::resolve`] then [`scenario::build_models`],
//! then [`solve::solve`], which returns a [`solve::SolveOutcome`].

pub mod antenna;
pub mod calib;
pub mod coastline;
pub mod compute;
pub mod coverage;
pub mod noise;
pub mod run;
pub mod scenario;
pub mod solve;
pub mod sweep;
