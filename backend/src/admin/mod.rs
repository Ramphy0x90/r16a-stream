//! Admin actions. Every route takes the [`crate::auth::Admin`] extractor, and every mutation
//! writes an `audit_log` row in its own transaction (SPEC 5.2).

pub mod audit;
pub mod repository;
pub mod users;
