//! Video transcoding in the worker (SPEC 7.3 to 7.5).
//!
//! `ffmpeg` builds the probe rules, the ladder and the argument vectors, `runner` runs the encode
//! with progress and cancellation, `repository` is the worker's SQL and `worker` the pipeline
//! from a claimed job to a switched variant, `housekeeping` subtitle promotion and encode GC.

pub mod ffmpeg;
pub mod housekeeping;
pub mod repository;
pub mod runner;
pub mod worker;
