//! The job queue (SPEC 7.2). Workers claim with `for update skip locked`, heartbeat, and end
//! each job as succeeded, retried, requeued for storage, or failed.

pub mod repository;
