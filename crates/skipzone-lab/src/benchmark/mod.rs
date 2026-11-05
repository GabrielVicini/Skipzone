//! The accuracy benchmark: every model scored on the same frozen rows, with
//! hold-outs by month and region and day-block error bars
//! (docs/notes/accuracy-roadmap.md §2).
//!
//! The `benchmark` binary drives it. The logic lives here so it can be
//! tested without the binary.
//!
//! | module | what is in it |
//! |---|---|
//! | [`manifest`](crate::benchmark::manifest) | corpus manifests and their SHA-256 check |
//! | [`sha256`](crate::benchmark::sha256) | the hash itself |
//! | [`corpus`](crate::benchmark::corpus) | loaders for WSPR, RBN, NCDXF beacon and ionosonde files |
//! | [`split`](crate::benchmark::split) | month, region and station hold-outs |
//! | [`predict`](crate::benchmark::predict) | Skipzone's prediction per row |
//! | [`baseline`](crate::benchmark::baseline) | ITURHFProp input files and output parsing |
//! | [`metrics`](crate::benchmark::metrics) | SNR error, Brier, reliability, AUC, MUF error |
//! | [`bootstrap`](crate::benchmark::bootstrap) | day-block and paired bootstrap |
//! | [`report`](crate::benchmark::report) | `results.tsv`, `rows.tsv` and `report.md` |
//! | [`pipeline`](crate::benchmark::pipeline) | a whole run, and the before/after comparison |

pub mod baseline;
pub mod bootstrap;
pub mod corpus;
pub mod manifest;
pub mod metrics;
pub mod pipeline;
pub mod predict;
pub mod report;
pub mod sha256;
pub mod split;
