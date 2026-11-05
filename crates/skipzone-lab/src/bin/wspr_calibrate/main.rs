//! Calibrate the model's unverified anchors against a saved WSPR corpus, with
//! transmitter and receiver effects treated as nuisance parameters.
//!
//! ```text
//! cargo run --release -p skipzone-lab --bin wspr_calibrate -- \
//!     --fit corpus/fit.tsv --holdout corpus/holdout.tsv --negatives corpus/fit_neg.tsv
//! ```
//!
//! Read `skipzone_lab::fit`'s module documentation before believing any number
//! this prints. Absolute levels (antenna gain, noise floor) cannot be
//! identified from WSPR, because they are constant per station and absorbed
//! into that station's effect. What is calibrated is how the signal varies
//! with frequency, path length, zenith angle, hop count and layer.
//!
//! The hold-out is separated by day and, separately, by region. A random row
//! split would not test generalisation, since adjacent spots share an
//! ionosphere.
//!
//! # Layout
//!
//! | module | what is in it |
//! |---|---|
//! | [`args`] | the command line surface and its documented defaults |
//! | [`solving`] | reading the corpus and solving it into cached predictions |
//! | [`driver`] | the fit itself, plus the anchor scans |
//! | [`report`] | everything that gets printed; no fitting happens there |
//! | [`negatives`] | paths that heard nothing, and the decode-probability work |
//! | [`jackknife`] | day-level refits, to see how much one day is carrying |

mod args;
mod driver;
mod jackknife;
mod negatives;
mod report;
mod solving;

use std::process::ExitCode;

use args::parse_args;
use driver::run;

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\ncalibration could not run: {e}");
            ExitCode::FAILURE
        }
    }
}
